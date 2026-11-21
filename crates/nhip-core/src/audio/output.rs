//! The sound card side: a ring buffer drained by the device callback, or by a timer when the
//! computer has no audio device.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use rtrb::{Consumer, Producer, RingBuffer};

use super::Shared;

const RING_MS: u64 = 400;
/// Roughly 5 ms to reach a new volume, so changes never click.
const SMOOTHING: f32 = 0.004;

struct Pull {
    consumer: Consumer<f32>,
    shared: Arc<Shared>,
    epoch: u32,
    level: f32,
}

impl Pull {
    fn new(consumer: Consumer<f32>, shared: Arc<Shared>) -> Self {
        let epoch = shared.flush_epoch();
        shared.acknowledge_flush(epoch);
        Self { consumer, shared, epoch, level: 0.0 }
    }

    #[inline]
    fn next_frame(&mut self) -> [f32; 2] {
        let epoch = self.shared.flush_epoch();
        if epoch != self.epoch {
            self.epoch = epoch;
            let n = self.consumer.slots();
            if let Ok(chunk) = self.consumer.read_chunk(n) {
                chunk.commit_all();
            }
            self.shared.acknowledge_flush(epoch);
        }
        let paused = self.shared.is_paused();
        let v = self.shared.volume();
        // Perceived loudness grows much faster than amplitude; the square feels even across the slider.
        let target = if paused { 0.0 } else { v * v };
        self.level += (target - self.level) * SMOOTHING;
        if paused && self.level < 0.001 {
            return [0.0, 0.0];
        }
        match (self.consumer.pop(), self.consumer.pop()) {
            (Ok(l), Ok(r)) => [l * self.level, r * self.level],
            _ => [0.0, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flush_acknowledges_discarded_frames_before_the_next_deck_writes() {
        let shared = Arc::new(Shared::new());
        shared.paused.store(false, Ordering::Relaxed);
        let (mut producer, consumer) = RingBuffer::new(8);
        let mut pull = Pull::new(consumer, shared.clone());
        producer.push(1.0).unwrap();
        producer.push(1.0).unwrap();
        shared.flush_epoch.fetch_add(1, Ordering::Release);
        assert!(shared.waiting_for_flush());
        assert_eq!(pull.next_frame(), [0.0, 0.0]);
        assert!(!shared.waiting_for_flush());
        producer.push(-1.0).unwrap();
        producer.push(-1.0).unwrap();
        assert!(pull.next_frame().into_iter().all(|sample| sample < 0.0));
    }
}

enum Keepalive {
    Device(#[allow(dead_code)] cpal::Stream),
    Timer(#[allow(dead_code)] Arc<AtomicBool>),
}

pub struct Output {
    producer: Producer<f32>,
    capacity_frames: usize,
    pub rate: u32,
    failed: Arc<AtomicBool>,
    _keepalive: Keepalive,
}

impl Output {
    pub fn create(shared: Arc<Shared>) -> Self {
        match Self::try_device(shared.clone()) {
            Some(o) => o,
            None => Self::timer(shared),
        }
    }

    pub fn free_frames(&self) -> usize {
        self.producer.slots() / 2
    }

    pub fn buffered_frames(&self) -> usize {
        self.capacity_frames.saturating_sub(self.free_frames())
    }

    pub fn driver_latency_ms(&self) -> u64 {
        match self._keepalive {
            Keepalive::Device(_) => 75,
            Keepalive::Timer(_) => 0,
        }
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    pub fn is_timer(&self) -> bool {
        matches!(self._keepalive, Keepalive::Timer(_))
    }

    pub fn push(&mut self, stereo: &[f32]) {
        for &s in stereo {
            let _ = self.producer.push(s);
        }
    }

    fn ring(rate: u32) -> (Producer<f32>, Consumer<f32>, usize) {
        let frames = (rate as u64 * RING_MS / 1000) as usize;
        let (p, c) = RingBuffer::new(frames * 2);
        (p, c, frames)
    }

    pub(super) fn try_device(shared: Arc<Shared>) -> Option<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let config = device.default_output_config().ok()?;
        let format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();
        let rate = stream_config.sample_rate;
        let channels = stream_config.channels as usize;
        let (producer, consumer, capacity_frames) = Self::ring(rate);
        let failed = Arc::new(AtomicBool::new(false));
        let pull = Pull::new(consumer, shared);
        let f = failed.clone();
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &stream_config, pull, channels, f),
            SampleFormat::F64 => build::<f64>(&device, &stream_config, pull, channels, f),
            SampleFormat::I16 => build::<i16>(&device, &stream_config, pull, channels, f),
            SampleFormat::I32 => build::<i32>(&device, &stream_config, pull, channels, f),
            SampleFormat::U16 => build::<u16>(&device, &stream_config, pull, channels, f),
            SampleFormat::U8 => build::<u8>(&device, &stream_config, pull, channels, f),
            _ => None,
        }?;
        stream.play().ok()?;
        Some(Self {
            producer,
            capacity_frames,
            rate,
            failed,
            _keepalive: Keepalive::Device(stream),
        })
    }

    /// Without a sound card the song still has to progress in real time.
    pub(super) fn timer(shared: Arc<Shared>) -> Self {
        const RATE: u32 = 44_100;
        let (producer, consumer, capacity_frames) = Self::ring(RATE);
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let mut pull = Pull::new(consumer, shared);
        std::thread::spawn(move || {
            let start = Instant::now();
            let mut simulated = 0;
            while !stop2.load(Ordering::Relaxed) {
                let elapsed = (start.elapsed().as_secs_f64() * RATE as f64) as u64;
                let frames = elapsed.saturating_sub(simulated).min(u64::from(RATE / 4));
                for _ in 0..frames {
                    let _ = pull.next_frame();
                }
                simulated = elapsed;
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        Self {
            producer,
            capacity_frames,
            rate: RATE,
            failed: Arc::new(AtomicBool::new(false)),
            _keepalive: Keepalive::Timer(stop),
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut pull: Pull,
    channels: usize,
    failed: Arc<AtomicBool>,
) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Sample,
{
    device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for frame in data.chunks_mut(channels.max(1)) {
                    let [l, r] = pull.next_frame();
                    match frame.len() {
                        1 => frame[0] = T::from_sample((l + r) * 0.5),
                        _ => {
                            frame[0] = T::from_sample(l);
                            frame[1] = T::from_sample(r);
                            for extra in frame.iter_mut().skip(2) {
                                *extra = T::EQUILIBRIUM;
                            }
                        }
                    }
                }
            },
            move |err: cpal::Error| {
                // Under-runs are harmless; anything else means the device went away.
                if !matches!(err.kind(), cpal::ErrorKind::Xrun | cpal::ErrorKind::RealtimeDenied) {
                    failed.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .ok()
}
