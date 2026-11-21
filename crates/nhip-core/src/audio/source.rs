//! Where samples come from: an audio file, or a generated demo song.

use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::fs::File;
use std::path::Path;

use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::units::{Time, TimeBase};

pub trait Source: Send {
    fn rate(&self) -> u32;
    fn duration_ms(&self) -> u64;
    /// Appends stereo frames; returns false once the stream has ended.
    fn fill(&mut self, out: &mut VecDeque<f32>) -> bool;
    fn seek(&mut self, ms: u64) -> bool;
}

pub struct FileSource {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    rate: u32,
    duration_ms: u64,
    time_base: Option<TimeBase>,
    scratch: Vec<f32>,
}

impl FileSource {
    pub fn open(path: &str) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| e.to_string())?;
        let ext = Path::new(path).extension().and_then(|e| e.to_str());
        Self::from_media_source(Box::new(file), ext)
    }

    pub fn from_media_source(input: Box<dyn MediaSource>, extension: Option<&str>) -> Result<Self, String> {
        let mss = MediaSourceStream::new(input, Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = extension {
            hint.with_extension(ext);
        }
        let format = symphonia::default::get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| e.to_string())?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| "no audio track".to_string())?;
        let track_id = track.id;
        let params = track
            .codec_params
            .as_ref()
            .and_then(|p| p.audio())
            .ok_or_else(|| "unsupported codec".to_string())?;
        let rate = params.sample_rate.unwrap_or(44_100);
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(params, &AudioDecoderOptions::default())
            .map_err(|e| e.to_string())?;
        let duration_ms = track
            .time_base
            .zip(track.num_frames)
            .and_then(|(tb, frames)| {
                let ts = symphonia::core::units::Timestamp::new(frames as i64);
                tb.calc_time(ts).map(|t| (t.as_secs_f64() * 1000.0) as u64)
            })
            .unwrap_or(0);
        let time_base = track.time_base;
        Ok(Self { format, decoder, track_id, rate, duration_ms, time_base, scratch: Vec::new() })
    }

    pub fn seek_to_ms(&mut self, ms: u64) -> Result<Option<u64>, String> {
        let time = Time::try_from_secs_f64(ms as f64 / 1000.0).ok_or("invalid seek time")?;
        let result = self.format.seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) });
        self.decoder.reset();
        let seeked = result.map_err(|e| e.to_string())?;
        Ok(self.time_base.and_then(|tb| tb.calc_time(seeked.actual_ts)).map(|t| (t.as_secs_f64() * 1000.0) as u64))
    }
}

impl Source for FileSource {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn duration_ms(&self) -> u64 {
        self.duration_ms
    }

    fn fill(&mut self, out: &mut VecDeque<f32>) -> bool {
        let mut errors = 0;
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(p)) => p,
                Ok(None) => return false,
                Err(SymError::ResetRequired) => {
                    self.decoder.reset();
                    continue;
                }
                Err(_) => return false,
            };
            if packet.track_id != self.track_id {
                continue;
            }
            match self.decoder.decode(&packet) {
                Ok(buf) => {
                    let spec = buf.spec().clone();
                    self.rate = spec.rate();
                    let channels = spec.channels().count().max(1);
                    let n = buf.samples_interleaved();
                    self.scratch.resize(n, 0.0);
                    buf.copy_to_slice_interleaved(&mut self.scratch);
                    for frame in self.scratch.chunks_exact(channels) {
                        let (l, r) = if channels == 1 { (frame[0], frame[0]) } else { (frame[0], frame[1]) };
                        out.push_back(l);
                        out.push_back(r);
                    }
                    if n > 0 {
                        return true;
                    }
                }
                Err(SymError::DecodeError(_)) => {
                    errors += 1;
                    if errors > 64 {
                        return false;
                    }
                }
                Err(_) => return false,
            }
        }
    }

    fn seek(&mut self, ms: u64) -> bool {
        self.seek_to_ms(ms).is_ok()
    }
}

// ---------------------------------------------------------------------------------------------
// Demo songs, so a fresh install has something to play. Every sample is a pure function of time,
// which makes seeking trivial.

const DEMO_RATE: u32 = 44_100;

pub struct DemoSource {
    seed: u32,
    frames: u64,
    pos: u64,
}

impl DemoSource {
    pub fn new(seed: u32, duration_ms: u64) -> Self {
        Self { seed, frames: duration_ms * DEMO_RATE as u64 / 1000, pos: 0 }
    }

    fn hash(&self, a: u64) -> u32 {
        let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (self.seed as u64).wrapping_mul(0xD1B5_4A32_D192_ED03);
        x ^= x >> 29;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        (x ^ (x >> 32)) as u32
    }

    fn sample(&self, frame: u64) -> (f32, f32) {
        let t = frame as f32 / DEMO_RATE as f32;
        let bpm = 84.0 + (self.seed % 7) as f32 * 8.0;
        let beat = t * bpm / 60.0;
        let bar = (beat / 4.0) as usize;
        let beat_i = beat as u64;
        let beat_f = beat.fract();

        // Minor or major pentatonic chosen per song, with a four-chord loop.
        let root_note = 45.0 + (self.seed % 5) as f32;
        let chords: [f32; 4] = match self.seed % 3 {
            0 => [0.0, 8.0, 3.0, 10.0],
            1 => [0.0, 5.0, 10.0, 3.0],
            _ => [0.0, 7.0, 3.0, 5.0],
        };
        let chord = chords[bar % 4];
        let note_hz = |n: f32| 440.0 * 2f32.powf((n - 69.0) / 12.0);

        // Kick on every beat.
        let kick_env = (-beat_f * 9.0).exp();
        let kick = (TAU * (45.0 + 90.0 * (-beat_f * 26.0).exp()) * beat_f * 60.0 / bpm).sin() * kick_env * 0.55;

        // Bass follows the chord root.
        let bass_env = (-beat_f * 2.5).exp();
        let bass = (TAU * note_hz(root_note + chord - 12.0) * t).sin() * bass_env * 0.26;

        // Arpeggio on sixteenths, picked from the scale by the seed.
        let scale = [0.0, 3.0, 5.0, 7.0, 10.0, 12.0, 15.0];
        let sixteenth = beat * 4.0;
        let s_i = sixteenth as u64;
        let s_f = sixteenth.fract();
        let idx = self.hash(s_i + (bar as u64 % 2) * 1000) as usize % scale.len();
        let hz = note_hz(root_note + 24.0 + chord + scale[idx]);
        let env = (-s_f * 4.0).exp();
        let phase = t * hz;
        let tone = (TAU * phase).sin() * 0.55 + (TAU * phase * 2.0).sin() * 0.2 + (TAU * phase * 3.0).sin() * 0.08;
        let pan = if s_i % 2 == 0 { 0.3 } else { -0.3 };
        let lead = tone * env * 0.2;

        // Soft pad holding the chord.
        let pad = [0.0, 7.0, 12.0]
            .iter()
            .map(|n| (TAU * note_hz(root_note + 12.0 + chord + n) * t).sin())
            .sum::<f32>()
            * 0.045;

        // Hats on off-beats from hashed noise.
        let hat = if beat_i % 2 == 1 || s_i % 2 == 1 {
            let n = self.hash(frame) as f32 / u32::MAX as f32 * 2.0 - 1.0;
            n * (-s_f * 18.0).exp() * 0.05
        } else {
            0.0
        };

        // Fade in and out so songs have a beginning and an end.
        let end = self.frames as f32 / DEMO_RATE as f32;
        let fade = (t / 1.5).min(1.0).min(((end - t) / 2.5).max(0.0));
        let fade = (fade * PI / 2.0).sin();

        let mid = kick + bass + pad;
        let l = (mid + lead * (1.0 + pan) + hat) * fade;
        let r = (mid + lead * (1.0 - pan) - hat) * fade;
        (l * 0.8, r * 0.8)
    }
}

impl Source for DemoSource {
    fn rate(&self) -> u32 {
        DEMO_RATE
    }

    fn duration_ms(&self) -> u64 {
        self.frames * 1000 / DEMO_RATE as u64
    }

    fn fill(&mut self, out: &mut VecDeque<f32>) -> bool {
        if self.pos >= self.frames {
            return false;
        }
        let end = (self.pos + 2048).min(self.frames);
        for f in self.pos..end {
            let (l, r) = self.sample(f);
            out.push_back(l);
            out.push_back(r);
        }
        self.pos = end;
        true
    }

    fn seek(&mut self, ms: u64) -> bool {
        self.pos = (ms * DEMO_RATE as u64 / 1000).min(self.frames);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_song_has_sound_and_ends() {
        let mut s = DemoSource::new(3, 2_000);
        let mut out = VecDeque::new();
        while s.fill(&mut out) {}
        assert_eq!(out.len() as u64, 2 * 2 * 44_100);
        let peak = out.iter().fold(0f32, |a, b| a.max(b.abs()));
        assert!(peak > 0.05 && peak <= 1.2, "peak {peak}");
    }

    #[test]
    fn demo_seek_lands_where_asked() {
        let mut a = DemoSource::new(1, 10_000);
        a.seek(4_000);
        let mut out = VecDeque::new();
        a.fill(&mut out);
        let b = DemoSource::new(1, 10_000);
        assert!((out[0] - b.sample(4 * 44_100).0).abs() < 1e-6);
    }
}
