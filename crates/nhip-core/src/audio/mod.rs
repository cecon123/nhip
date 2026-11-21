//! The playback engine: one worker thread decodes and mixes, the sound card pulls from a ring.

mod automix;
mod dsp;
mod output;
mod source;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dsp::{Equalizer, Limiter};
use output::Output;
pub use source::{DemoSource, FileSource, Source};

use crate::library::TrackId;
use crate::mechanics::Transition;

const BLOCK_FRAMES: usize = 1024;
/// Silence between songs when the transition is `Transition::None`.
const NON_GAPLESS_SILENCE_MS: u64 = 300;

#[derive(Clone)]
pub enum SourceSpec {
    File(String),
    Demo { seed: u32, duration_ms: u64 },
    YouTube { video_id: String },
}

#[derive(Clone)]
pub struct LoadSpec {
    pub id: TrackId,
    pub source: SourceSpec,
    pub duration_ms: u64,
    pub gain_db: f32,
    /// How this song takes over from the one before it.
    pub transition: Transition,
}

#[derive(Clone, Copy, PartialEq)]
pub struct AudioSettings {
    pub crossfade_ms: u64,
    pub eq_enabled: bool,
    pub eq_gains: [f32; 6],
}

enum Cmd {
    Load { spec: LoadSpec, start_ms: u64, play: bool, epoch: u64 },
    SetNext(Option<LoadSpec>),
    SetGain { id: TrackId, gain_db: f32 },
    Play,
    Pause,
    Seek { id: TrackId, ms: u64 },
    Stop,
    Settings(AudioSettings),
    Quit,
}

#[derive(Debug)]
pub enum Event {
    /// The next song began by itself (gapless or crossfade) and is now the current one.
    Advanced(TrackId),
    /// The current song ended and nothing was lined up after it.
    Finished,
    Error(TrackId, String),
    /// The real duration of a song became known from its file.
    Duration(TrackId, u64),
}

pub struct Shared {
    position_ms: AtomicU64,
    duration_ms: AtomicU64,
    volume_bits: AtomicU32,
    paused: AtomicBool,
    loaded: AtomicBool,
    flush_epoch: AtomicU32,
    flushed_epoch: AtomicU32,
}

impl Shared {
    fn new() -> Self {
        Self {
            position_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            volume_bits: AtomicU32::new(0.8f32.to_bits()),
            paused: AtomicBool::new(true),
            loaded: AtomicBool::new(false),
            flush_epoch: AtomicU32::new(0),
            flushed_epoch: AtomicU32::new(0),
        }
    }

    pub(crate) fn volume(&self) -> f32 {
        f32::from_bits(self.volume_bits.load(Ordering::Relaxed))
    }
    pub(crate) fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }
    pub(crate) fn flush_epoch(&self) -> u32 {
        self.flush_epoch.load(Ordering::Acquire)
    }

    fn acknowledge_flush(&self, epoch: u32) {
        self.flushed_epoch.store(epoch, Ordering::Release);
    }

    fn waiting_for_flush(&self) -> bool {
        self.flush_epoch() != self.flushed_epoch.load(Ordering::Acquire)
    }
}

pub(crate) enum CancelSignal {
    Flag(Arc<AtomicBool>),
    Epoch { current: Arc<AtomicU64>, expected: u64 },
    Prepare {
        load_epoch: Arc<AtomicU64>,
        expected_epoch: u64,
        prepare_epoch: Arc<AtomicU64>,
        expected_prepare: u64,
        preparing: Arc<AtomicBool>,
    },
}

impl CancelSignal {
    fn is_cancelled(&self) -> bool {
        match self {
            Self::Flag(flag) => flag.load(Ordering::Acquire),
            Self::Epoch { current, expected } => current.load(Ordering::Acquire) != *expected,
            Self::Prepare {
                load_epoch,
                expected_epoch,
                prepare_epoch,
                expected_prepare,
                preparing,
            } => {
                load_epoch.load(Ordering::Acquire) != *expected_epoch
                    || (preparing.load(Ordering::Acquire)
                        && prepare_epoch.load(Ordering::Acquire) != *expected_prepare)
            }
        }
    }
}

pub struct Engine {
    tx: Sender<Cmd>,
    events: Receiver<Event>,
    shared: Arc<Shared>,
    load_epoch: Arc<AtomicU64>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    pub fn start(settings: AudioSettings, on_event: impl Fn() + Send + 'static) -> Self {
        Self::start_inner(settings, cfg!(test), on_event)
    }

    pub fn start_silent(settings: AudioSettings, on_event: impl Fn() + Send + 'static) -> Self {
        Self::start_inner(settings, true, on_event)
    }

    fn start_inner(
        settings: AudioSettings,
        silent: bool,
        on_event: impl Fn() + Send + 'static,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let (etx, events) = mpsc::channel();
        let shared = Arc::new(Shared::new());
        let load_epoch = Arc::new(AtomicU64::new(0));
        let s = shared.clone();
        let ep = load_epoch.clone();
        let worker = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || Worker::new(rx, etx, s, ep, settings, silent, on_event).run())
            .expect("audio thread");
        Self { tx, events, shared, load_epoch, worker: Some(worker) }
    }

    pub fn load(&self, spec: LoadSpec, start_ms: u64, play: bool) {
        let epoch = self.load_epoch.fetch_add(1, Ordering::AcqRel) + 1;
        let _ = self.tx.send(Cmd::Load { spec, start_ms, play, epoch });
    }
    pub fn set_next(&self, spec: Option<LoadSpec>) {
        let _ = self.tx.send(Cmd::SetNext(spec));
    }
    pub fn set_gain(&self, id: TrackId, gain_db: f32) {
        let _ = self.tx.send(Cmd::SetGain { id, gain_db });
    }
    pub fn play(&self) {
        let _ = self.tx.send(Cmd::Play);
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn seek(&self, id: TrackId, ms: u64) {
        let _ = self.tx.send(Cmd::Seek { id, ms });
    }
    pub fn stop(&self) {
        self.load_epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.send(Cmd::Stop);
    }
    pub fn apply_settings(&self, s: AudioSettings) {
        let _ = self.tx.send(Cmd::Settings(s));
    }
    pub fn set_volume(&self, v: f32) {
        self.shared.volume_bits.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn position_ms(&self) -> u64 {
        self.shared.position_ms.load(Ordering::Relaxed)
    }
    pub fn duration_ms(&self) -> u64 {
        self.shared.duration_ms.load(Ordering::Relaxed)
    }
    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::Relaxed)
    }
    /// False once the last song has ended or was stopped: playing again needs a fresh load.
    pub fn has_song(&self) -> bool {
        self.shared.loaded.load(Ordering::Relaxed)
    }
    pub fn poll(&self) -> Option<Event> {
        self.events.try_recv().ok()
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.load_epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.tx.send(Cmd::Quit);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

struct Deck {
    id: TrackId,
    spec: SourceSpec,
    src: Box<dyn Source>,
    buf: VecDeque<f32>,
    /// Fractional read position between the first two buffered frames.
    frac: f64,
    consumed: f64,
    base_ms: u64,
    ended: bool,
    gain: f32,
    tempo_map: Option<automix::TempoMap>,
}

impl Deck {
    fn open(spec: &LoadSpec) -> Result<Self, String> {
        Self::open_with_cancel(spec, CancelSignal::Flag(Arc::new(AtomicBool::new(false))))
    }

    fn open_with_cancel(spec: &LoadSpec, cancel: CancelSignal) -> Result<Self, String> {
        Self::open_with_cancel_mode(spec, cancel, false)
    }

    fn open_with_cancel_mode(spec: &LoadSpec, cancel: CancelSignal, force_fresh: bool) -> Result<Self, String> {
        let src: Box<dyn Source> = match &spec.source {
            SourceSpec::File(path) => Box::new(FileSource::open(path)?),
            SourceSpec::Demo { seed, duration_ms } => Box::new(DemoSource::new(*seed, *duration_ms)),
            SourceSpec::YouTube { video_id } => {
                let media = if force_fresh {
                    http_range::HttpRangeSource::open_fresh(video_id.clone(), cancel)
                } else {
                    http_range::HttpRangeSource::open(video_id.clone(), cancel)
                }
                .map_err(|e| e.to_string())?;
                Box::new(FileSource::from_media_source(Box::new(media), Some("m4a"))?)
            }
        };
        Ok(Self {
            id: spec.id,
            spec: spec.source.clone(),
            src,
            buf: VecDeque::new(),
            frac: 0.0,
            consumed: 0.0,
            base_ms: 0,
            ended: false,
            gain: db_to_amp(spec.gain_db),
            tempo_map: None,
        })
    }

    fn position_ms(&self) -> u64 {
        self.position_at(self.consumed)
    }

    fn position_at(&self, frame: f64) -> u64 {
        let source = self.tempo_map.as_ref().map_or(frame, |map| map.source_frame(frame));
        self.base_ms + (source * 1000.0 / self.src.rate() as f64) as u64
    }

    fn remaining_ms(&self) -> Option<u64> {
        let d = self.src.duration_ms();
        (d > 0).then(|| d.saturating_sub(self.position_ms()))
    }

    fn seek(&mut self, ms: u64) -> Option<u64> {
        let actual_ms = self.src.seek(ms)?;
        self.buf.clear();
        self.frac = 0.0;
        self.consumed = 0.0;
        self.base_ms = actual_ms;
        self.ended = false;
        self.tempo_map = None;
        Some(actual_ms)
    }

    /// Writes stereo frames resampled to `dev_rate`; returns how many frames were produced.
    fn pull(&mut self, out: &mut [f32], dev_rate: f64) -> usize {
        let step = self.src.rate() as f64 / dev_rate;
        let mut written = 0;
        for frame in out.chunks_exact_mut(2) {
            while self.buf.len() < 4 && !self.ended {
                if !self.src.fill(&mut self.buf) {
                    self.ended = true;
                }
            }
            if self.buf.len() < 4 {
                break;
            }
            let f = self.frac as f32;
            frame[0] = (self.buf[0] + (self.buf[2] - self.buf[0]) * f) * self.gain;
            frame[1] = (self.buf[1] + (self.buf[3] - self.buf[1]) * f) * self.gain;
            self.frac += step;
            self.consumed += step;
            while self.frac >= 1.0 && self.buf.len() >= 2 {
                self.buf.pop_front();
                self.buf.pop_front();
                self.frac -= 1.0;
            }
            written += 1;
        }
        written
    }

    fn finished(&self) -> bool {
        self.ended && self.buf.len() < 4
    }
}

struct Outgoing {
    deck: Deck,
    done_frames: u64,
    total_frames: u64,
}

/// Something that happened in the decoder but hasn't reached the speakers yet.
enum Pending {
    Advanced { id: TrackId, duration_ms: u64 },
    Finished,
}

struct Worker<F: Fn()> {
    rx: Receiver<Cmd>,
    pending_cmd: Option<Cmd>,
    tx: Sender<Event>,
    shared: Arc<Shared>,
    load_epoch: Arc<AtomicU64>,
    active_epoch: u64,
    silent: bool,
    output: Output,
    flush_waiting_since: Option<Instant>,
    output_stalled_since: Option<Instant>,
    last_device_retry: Instant,
    settings: AudioSettings,
    eq: Equalizer,
    limiter: Limiter,
    current: Option<Deck>,
    outgoing: Option<Outgoing>,
    next: Option<LoadSpec>,
    /// Pre-opened next deck for Gapless/Crossfade/manual Next so handovers never stall the mixer thread on HTTPS/IO.
    prepared_next: Option<(TrackId, Deck)>,
    preload_rx: Option<(u64, TrackId, Receiver<Result<Deck, String>>)>,
    preload_gen: u64,
    mix_analyzer: Option<automix::Analyzer>,
    prepared_mix: Option<automix::PreparedMix>,
    silence_frames: u64,
    /// Frames ever pushed to the output; with what's still buffered this gives what was heard.
    written_frames: u64,
    pending: VecDeque<(u64, Pending)>,
    on_event: F,
    mix_a: Vec<f32>,
    mix_b: Vec<f32>,
}

impl<F: Fn()> Worker<F> {
    fn new(
        rx: Receiver<Cmd>,
        tx: Sender<Event>,
        shared: Arc<Shared>,
        load_epoch: Arc<AtomicU64>,
        settings: AudioSettings,
        silent: bool,
        on_event: F,
    ) -> Self {
        let output = if silent {
            Output::timer(shared.clone())
        } else {
            Output::create(shared.clone())
        };
        let mut eq = Equalizer::new();
        eq.configure(settings.eq_enabled, &settings.eq_gains, output.rate as f32);
        let limiter = Limiter::new(output.rate as f32);
        Self {
            rx,
            pending_cmd: None,
            tx,
            shared,
            load_epoch,
            active_epoch: 0,
            silent,
            output,
            flush_waiting_since: None,
            output_stalled_since: None,
            last_device_retry: Instant::now(),
            settings,
            eq,
            limiter,
            current: None,
            outgoing: None,
            next: None,
            prepared_next: None,
            preload_rx: None,
            preload_gen: 0,
            mix_analyzer: None,
            prepared_mix: None,
            silence_frames: 0,
            written_frames: 0,
            pending: VecDeque::new(),
            on_event,
            mix_a: vec![0.0; BLOCK_FRAMES * 2],
            mix_b: vec![0.0; BLOCK_FRAMES * 2],
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.on_event)();
    }

    fn flush_output(&mut self) {
        self.pending.clear();
        if self.output.buffered_frames() != 0 {
            self.shared.flush_epoch.fetch_add(1, Ordering::Release);
            self.flush_waiting_since = Some(Instant::now());
        } else {
            self.flush_waiting_since = None;
        }
    }

    fn rebuild_output(&mut self) {
        self.output = if self.silent {
            Output::timer(self.shared.clone())
        } else {
            Output::create(self.shared.clone())
        };
        self.flush_waiting_since = None;
        self.output_stalled_since = None;
        self.last_device_retry = Instant::now();
        self.reconfigure_eq();
        self.limiter.set_rate(self.output.rate as f32);
    }

    /// Detects and recovers from Windows WASAPI device switches, stalled audio callbacks,
    /// or silent timer fallback when a hardware output device becomes available again.
    fn check_output_health(&mut self) {
        if self.output.failed() {
            self.rebuild_output();
            return;
        }
        if self.silent {
            return;
        }
        if self.shared.waiting_for_flush() {
            let since = *self.flush_waiting_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(250) {
                self.rebuild_output();
                return;
            }
        } else {
            self.flush_waiting_since = None;
        }

        if self.current.is_some()
            && !self.shared.is_paused()
            && !self.shared.waiting_for_flush()
            && self.output.free_frames() < BLOCK_FRAMES
        {
            let since = *self.output_stalled_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_millis(600) {
                self.rebuild_output();
                return;
            }
        } else {
            self.output_stalled_since = None;
        }

        if self.output.is_timer()
            && !self.shared.is_paused()
            && self.last_device_retry.elapsed() >= Duration::from_secs(2)
        {
            self.last_device_retry = Instant::now();
            if let Some(dev) = Output::try_device(self.shared.clone()) {
                self.output = dev;
                self.flush_waiting_since = None;
                self.output_stalled_since = None;
                self.reconfigure_eq();
                self.limiter.set_rate(self.output.rate as f32);
            }
        }
    }

    /// Pulls the next command, coalescing bursts of `Cmd::Seek` on the same track and dropping
    /// already-superseded `Cmd::Load` commands so scrubbing/skipping never queues up HTTPS stalls.
    fn next_command(&mut self) -> Result<Option<Cmd>, ()> {
        let mut cmd = if let Some(c) = self.pending_cmd.take() {
            Some(c)
        } else if !self.can_render() {
            match self.rx.recv_timeout(Duration::from_millis(5)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Err(()),
            }
        } else {
            self.rx.try_recv().ok()
        };

        while let Some(current) = cmd.take() {
            match current {
                Cmd::Load { epoch, .. } if epoch != self.load_epoch.load(Ordering::Acquire) => {
                    // Superseded by a newer load before we even started; drain immediately.
                    cmd = self.rx.try_recv().ok();
                }
                Cmd::Seek { id, mut ms } => {
                    loop {
                        match self.rx.try_recv() {
                            Ok(Cmd::Seek { id: next_id, ms: next_ms }) if next_id == id => {
                                ms = next_ms;
                            }
                            Ok(other) => {
                                self.pending_cmd = Some(other);
                                break;
                            }
                            Err(_) => break,
                        }
                    }
                    return Ok(Some(Cmd::Seek { id, ms }));
                }
                other => return Ok(Some(other)),
            }
        }
        Ok(None)
    }

    /// Announces decoder events once their audio is actually being heard. Reporting the end of a
    /// song early would let the next load cut off its last moments.
    fn deliver_pending(&mut self) {
        let heard = self.written_frames.saturating_sub(self.output.buffered_frames() as u64);
        while self.pending.front().is_some_and(|(at, _)| *at <= heard) {
            match self.pending.pop_front() {
                Some((_, Pending::Advanced { id, duration_ms })) => {
                    self.shared.duration_ms.store(duration_ms, Ordering::Relaxed);
                    self.shared.position_ms.store(0, Ordering::Relaxed);
                    if duration_ms > 0 {
                        self.emit(Event::Duration(id, duration_ms));
                    }
                    self.emit(Event::Advanced(id));
                }
                Some((_, Pending::Finished)) => self.emit(Event::Finished),
                None => {}
            }
        }
    }

    fn run(mut self) {
        loop {
            self.deliver_pending();
            self.poll_preloaded_next();
            self.check_output_health();
            let cmd = match self.next_command() {
                Ok(c) => c,
                Err(()) => return,
            };
            if let Some(cmd) = cmd {
                if !self.handle(cmd) {
                    return;
                }
                continue;
            }
            if self.can_render() {
                self.render_block();
            }
            self.publish_position();
        }
    }

    fn can_render(&self) -> bool {
        // Wait for the consumer to discard the old frames before writing the new deck's audio.
        self.current.is_some() && !self.shared.is_paused() && !self.shared.waiting_for_flush()
            && self.output.free_frames() >= BLOCK_FRAMES
    }

    fn reconfigure_eq(&mut self) {
        self.eq.configure(self.settings.eq_enabled, &self.settings.eq_gains, self.output.rate as f32);
    }

    fn publish_position(&self) {
        // While a change is on its way to the speakers the clock still belongs to the old song.
        if !self.pending.is_empty() || self.shared.waiting_for_flush() {
            return;
        }
        if let Some(deck) = &self.current {
            let buffered = self.output.buffered_frames() as f64 * deck.src.rate() as f64 / self.output.rate.max(1) as f64;
            let raw_pos = deck.position_at((deck.consumed - buffered).max(0.0));
            let pos = if self.shared.is_paused() {
                raw_pos
            } else {
                raw_pos.saturating_sub(self.output.driver_latency_ms())
            };
            self.shared.position_ms.store(pos, Ordering::Relaxed);
        }
    }

    fn poll_preloaded_next(&mut self) {
        let Some((gen, id, rx)) = self.preload_rx.take() else { return };
        if gen != self.preload_gen {
            return;
        }
        match rx.try_recv() {
            Ok(Ok(deck)) => {
                if self.next.as_ref().is_some_and(|n| n.id == id) {
                    self.prepared_next = Some((id, deck));
                }
            }
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {}
            Err(mpsc::TryRecvError::Empty) => {
                self.preload_rx = Some((gen, id, rx));
            }
        }
    }

    fn start_preload_next(&mut self) {
        self.preload_gen = self.preload_gen.wrapping_add(1);
        self.preload_rx = None;
        let Some(spec) = self.next.clone() else {
            self.prepared_next = None;
            return;
        };
        if spec.transition == Transition::AutoMix {
            // AutoMix has its own beat-matching background analyzer (`prepare_mix`).
            self.prepared_next = None;
            return;
        }
        if self.prepared_next.as_ref().is_some_and(|(id, _)| *id == spec.id) {
            return;
        }
        self.prepared_next = None;
        let gen = self.preload_gen;
        let id = spec.id;
        let epoch = self.active_epoch;
        let load_epoch = self.load_epoch.clone();
        let (tx, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("audio-preload".into()).spawn(move || {
            let cancel = CancelSignal::Epoch { current: load_epoch, expected: epoch };
            let _ = tx.send(Deck::open_with_cancel(&spec, cancel));
        });
        self.preload_rx = Some((gen, id, rx));
    }

    fn take_prepared_next(&mut self, id: TrackId, gain_db: f32) -> Option<Deck> {
        self.poll_preloaded_next();
        if self.prepared_next.as_ref().is_some_and(|(pid, _)| *pid == id) {
            self.prepared_next.take().map(|(_, mut deck)| {
                deck.gain = db_to_amp(gain_db);
                deck
            })
        } else {
            None
        }
    }

    fn prepare_mix(&mut self) {
        self.prepared_mix = None;
        let tracks = self.current.as_ref().zip(self.next.as_ref())
            .filter(|(now, next)| now.id != next.id && next.transition == Transition::AutoMix)
            .map(|(now, next)| (now.spec.clone(), next.clone()));
        if self.mix_analyzer.is_none() && tracks.is_some() {
            self.mix_analyzer = automix::Analyzer::start(self.load_epoch.clone()).ok();
        }
        if let Some(analyzer) = &mut self.mix_analyzer {
            analyzer.select(tracks, self.active_epoch);
        }
        self.start_preload_next();
    }

    fn handle(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Load { spec, start_ms, play, epoch } => {
                self.flush_output();
                self.outgoing = None;
                self.silence_frames = 0;
                // Reuse preloaded deck on manual Next skip if it already finished opening for this track.
                let reused = self.take_prepared_next(spec.id, spec.gain_db);
                self.next = None;
                self.active_epoch = epoch;
                if self.load_epoch.load(Ordering::Acquire) != epoch {
                    self.set_current(None);
                    self.shared.paused.store(true, Ordering::Relaxed);
                    return true;
                }
                if let Some(mut deck) = reused {
                    if start_ms > 0 {
                        deck.seek(start_ms);
                    }
                    self.publish_new(&deck, start_ms);
                    self.set_current(Some(deck));
                } else {
                    self.start_deck(&spec, start_ms, epoch);
                }
                self.prepare_mix();
                self.shared.paused.store(!play || self.current.is_none(), Ordering::Relaxed);
            }
            Cmd::SetNext(spec) => {
                self.next = spec;
                self.prepare_mix();
            }
            Cmd::SetGain { id, gain_db } => {
                if let Some(d) = self.current.as_mut().filter(|d| d.id == id) {
                    d.gain = db_to_amp(gain_db);
                }
                if let Some(n) = self.next.as_mut().filter(|n| n.id == id) {
                    n.gain_db = gain_db;
                }
            }
            Cmd::Play => {
                if self.current.is_some() {
                    self.shared.paused.store(false, Ordering::Relaxed);
                }
            }
            Cmd::Pause => self.shared.paused.store(true, Ordering::Relaxed),
            Cmd::Seek { id, ms } => {
                if !self.current.as_ref().is_some_and(|deck| deck.id == id) {
                    let changing = self.pending.iter().any(|(_, event)| matches!(event, Pending::Advanced { .. }));
                    if !changing || !self.outgoing.as_ref().is_some_and(|old| old.deck.id == id) {
                        return true;
                    }
                    self.current = self.outgoing.take().map(|old| old.deck);
                }
                self.flush_output();
                self.outgoing = None;
                let mut seek_failed_yt: Option<(TrackId, SourceSpec, f32, u64)> = None;
                if let Some(d) = self.current.as_mut() {
                    if let Some(actual_ms) = d.seek(ms) {
                        self.shared.position_ms.store(actual_ms, Ordering::Relaxed);
                    } else if matches!(d.spec, SourceSpec::YouTube { .. }) {
                        seek_failed_yt = Some((d.id, d.spec.clone(), d.gain, d.src.duration_ms()));
                    }
                }
                if let Some((deck_id, source, prev_gain, duration_ms)) = seek_failed_yt {
                    let reopen_spec = LoadSpec {
                        id: deck_id,
                        source,
                        duration_ms,
                        gain_db: 0.0,
                        transition: Transition::Gapless,
                    };
                    let cancel = CancelSignal::Epoch {
                        current: self.load_epoch.clone(),
                        expected: self.active_epoch,
                    };
                    if let Ok(mut fresh_deck) = Deck::open_with_cancel_mode(&reopen_spec, cancel, true) {
                        fresh_deck.gain = prev_gain;
                        let actual_ms = fresh_deck.seek(ms).unwrap_or(0);
                        self.shared.position_ms.store(actual_ms, Ordering::Relaxed);
                        self.set_current(Some(fresh_deck));
                    }
                }
                if self.next.as_ref().is_some_and(|n| n.transition == Transition::AutoMix) {
                    self.prepare_mix();
                }
            }
            Cmd::Stop => {
                self.flush_output();
                self.set_current(None);
                self.outgoing = None;
                self.prepare_mix();
                self.shared.paused.store(true, Ordering::Relaxed);
                self.shared.position_ms.store(0, Ordering::Relaxed);
            }
            Cmd::Settings(s) => {
                self.settings = s;
                self.reconfigure_eq();
            }
            Cmd::Quit => return false,
        }
        true
    }

    fn start_deck(&mut self, spec: &LoadSpec, start_ms: u64, epoch: u64) {
        let cancel = CancelSignal::Epoch { current: self.load_epoch.clone(), expected: epoch };
        match Deck::open_with_cancel(spec, cancel) {
            Ok(mut deck) => {
                if self.load_epoch.load(Ordering::Acquire) != epoch {
                    self.set_current(None);
                    return;
                }
                if start_ms > 0 {
                    deck.seek(start_ms);
                }
                self.publish_new(&deck, start_ms);
                self.set_current(Some(deck));
            }
            Err(e) => {
                self.set_current(None);
                if self.load_epoch.load(Ordering::Acquire) == epoch {
                    self.emit(Event::Error(spec.id, e));
                }
            }
        }
    }

    fn set_current(&mut self, deck: Option<Deck>) {
        self.shared.loaded.store(deck.is_some(), Ordering::Relaxed);
        self.current = deck;
    }

    fn publish_new(&self, deck: &Deck, start_ms: u64) {
        let d = deck.src.duration_ms();
        self.shared.duration_ms.store(d, Ordering::Relaxed);
        self.shared.position_ms.store(start_ms, Ordering::Relaxed);
        if d > 0 {
            self.emit(Event::Duration(deck.id, d));
        }
    }

    /// Replaces the current deck with the queued one. Returns false if there is none to start.
    fn advance_to_next(&mut self, fade_with_old: Option<u64>, at_frame: u64, prepared: Option<Deck>) -> bool {
        let Some(spec) = self.next.take() else { return false };
        let preloaded = prepared.or_else(|| self.take_prepared_next(spec.id, spec.gain_db));
        self.prepare_mix();
        let epoch = self.active_epoch;
        let cancel = CancelSignal::Epoch { current: self.load_epoch.clone(), expected: epoch };
        let incoming = match preloaded {
            Some(mut deck) => {
                deck.gain = db_to_amp(spec.gain_db);
                Ok(deck)
            }
            None => Deck::open_with_cancel(&spec, cancel),
        };
        match incoming {
            Ok(deck) => {
                let duration_ms = deck.src.duration_ms();
                self.shared.loaded.store(true, Ordering::Relaxed);
                let old = self.current.replace(deck);
                if let (Some(deck), Some(ms)) = (old, fade_with_old) {
                    let total_frames = ms * self.output.rate as u64 / 1000;
                    self.outgoing = Some(Outgoing { deck, done_frames: 0, total_frames: total_frames.max(1) });
                }
                self.pending.push_back((at_frame, Pending::Advanced { id: spec.id, duration_ms }));
                true
            }
            Err(e) => {
                if self.load_epoch.load(Ordering::Acquire) == epoch {
                    self.emit(Event::Error(spec.id, e));
                }
                false
            }
        }
    }

    fn maybe_start_crossfade(&mut self) {
        if self.outgoing.is_some() {
            return;
        }
        let Some(next) = &self.next else { return };
        let Some(current) = &self.current else { return };
        let Some(remaining) = current.remaining_ms() else { return };
        let fade_ms = match next.transition {
            Transition::Crossfade => self.settings.crossfade_ms,
            Transition::AutoMix => {
                if let Some(plan) = self.mix_analyzer.as_ref().and_then(automix::Analyzer::poll) {
                    self.prepared_mix = Some(plan);
                }
                if let Some(plan) = &self.prepared_mix {
                    if current.position_ms() >= plan.start_ms {
                        let prepared = self.prepared_mix.take().map(|mix| mix.deck);
                        self.advance_to_next(Some(remaining.max(1)), self.written_frames, prepared);
                    }
                    return;
                }
                let next_cap = if next.duration_ms > 0 { next.duration_ms / 2 } else { u64::MAX };
                automix::FALLBACK_MS.min(current.src.duration_ms() / 2).min(next_cap)
            }
            _ => return,
        };
        if fade_ms != 0 && remaining <= fade_ms {
            self.advance_to_next(Some(remaining.max(1)), self.written_frames, None);
        }
    }

    fn render_block(&mut self) {
        self.maybe_start_crossfade();
        let rate = self.output.rate as f64;
        let n = BLOCK_FRAMES;
        self.mix_a[..n * 2].fill(0.0);
        let mut filled = 0;

        while filled < n {
            if self.silence_frames > 0 {
                let take = (self.silence_frames.min((n - filled) as u64)) as usize;
                self.silence_frames -= take as u64;
                filled += take;
                if self.silence_frames > 0 {
                    continue;
                }
            }
            let got = match self.current.as_mut() {
                Some(deck) => deck.pull(&mut self.mix_a[filled * 2..n * 2], rate),
                None => break,
            };
            filled += got;
            if filled < n {
                let finished = self.current.as_ref().is_some_and(|d| d.finished());
                if !finished {
                    continue;
                }
                if self.next.as_ref().is_some_and(|n| n.transition == Transition::None) {
                    self.silence_frames = NON_GAPLESS_SILENCE_MS * self.output.rate as u64 / 1000;
                }
                let at_frame = self.written_frames + filled as u64 + self.silence_frames;
                if !self.advance_to_next(None, at_frame, None) {
                    self.set_current(None);
                    self.outgoing = None;
                    self.pending.push_back((self.written_frames + filled as u64, Pending::Finished));
                    break;
                }
            }
        }

        if let Some(out) = self.outgoing.as_mut() {
            self.mix_b[..n * 2].fill(0.0);
            let got = out.deck.pull(&mut self.mix_b[..n * 2], rate);
            for i in 0..n {
                let x = ((out.done_frames + i as u64) as f32 / out.total_frames as f32).min(1.0);
                let (fade_in, fade_out) = ((x * std::f32::consts::FRAC_PI_2).sin(), (x * std::f32::consts::FRAC_PI_2).cos());
                self.mix_a[i * 2] *= fade_in;
                self.mix_a[i * 2 + 1] *= fade_in;
                if i < got {
                    self.mix_a[i * 2] += self.mix_b[i * 2] * fade_out;
                    self.mix_a[i * 2 + 1] += self.mix_b[i * 2 + 1] * fade_out;
                }
            }
            out.done_frames += n as u64;
            if got < n || out.done_frames >= out.total_frames {
                self.outgoing = None;
            }
        }

        if filled == 0 && self.current.is_none() {
            return;
        }
        let block = &mut self.mix_a[..n * 2];
        self.eq.process(block);
        self.limiter.process(block);
        self.output.push(block);
        self.written_frames += n as u64;
    }
}

pub fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Estimates a file's loudness from gated block energy, roughly in LUFS, and its sample peak in
/// dB. Returns `None` for silence or an unreadable file.
#[cfg(test)]
pub fn measure_loudness(path: &str) -> Option<(f32, f32)> {
    let mut src = FileSource::open(path).ok()?;
    let block = (src.rate() as usize * 2 / 5).max(1);
    let mut buf = VecDeque::new();
    let mut energies = Vec::new();
    let mut sum = 0f64;
    let mut peak = 0f64;
    let mut count = 0usize;
    loop {
        let more = src.fill(&mut buf);
        while buf.len() >= 2 {
            let l = buf.pop_front()? as f64;
            let r = buf.pop_front()? as f64;
            peak = peak.max(l.abs()).max(r.abs());
            sum += (l * l + r * r) * 0.5;
            count += 1;
            if count == block {
                energies.push(sum / count as f64);
                sum = 0.0;
                count = 0;
            }
        }
        if !more {
            break;
        }
    }
    gated_loudness(&energies).map(|l| (l, 20.0 * peak.max(1e-9).log10() as f32))
}

#[cfg(test)]
fn gated_loudness(energies: &[f64]) -> Option<f32> {
    let to_db = |e: f64| -0.691 + 10.0 * e.log10();
    let loud: Vec<f64> = energies.iter().copied().filter(|&e| e > 0.0 && to_db(e) > -70.0).collect();
    if loud.is_empty() {
        return None;
    }
    let mean = loud.iter().sum::<f64>() / loud.len() as f64;
    let gate = to_db(mean) - 10.0;
    let kept: Vec<f64> = loud.into_iter().filter(|&e| to_db(e) > gate).collect();
    if kept.is_empty() {
        return None;
    }
    Some(to_db(kept.iter().sum::<f64>() / kept.len() as f64) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo(id: TrackId, seed: u32, duration_ms: u64) -> LoadSpec {
        LoadSpec { id, source: SourceSpec::Demo { seed, duration_ms }, duration_ms, gain_db: 0.0, transition: Transition::Gapless }
    }

    fn engine(crossfade_ms: u64) -> Engine {
        Engine::start(AudioSettings { crossfade_ms, eq_enabled: false, eq_gains: [0.0; 6] }, || {})
    }

    /// Waits for the engine's next event, ignoring duration notices.
    fn next_event(e: &Engine, limit: Duration) -> Option<Event> {
        let start = std::time::Instant::now();
        while start.elapsed() < limit {
            match e.poll() {
                Some(Event::Duration(..)) | None => std::thread::sleep(Duration::from_millis(5)),
                Some(other) => return Some(other),
            }
        }
        None
    }

    #[test]
    fn plays_into_next_song_then_reports_the_end() {
        let e = engine(0);
        e.load(demo(1, 1, 700), 0, true);
        e.set_next(Some(demo(2, 2, 700)));
        match next_event(&e, Duration::from_secs(4)) {
            Some(Event::Advanced(2)) => {}
            other => panic!("expected the next song, got {other:?}"),
        }
        e.set_next(None);
        assert!(matches!(next_event(&e, Duration::from_secs(4)), Some(Event::Finished)));
        assert!(!e.has_song());
    }

    #[test]
    fn crossfade_starts_the_next_song_before_the_end() {
        let e = engine(800);
        let start = std::time::Instant::now();
        e.load(demo(1, 1, 2000), 0, true);
        e.set_next(Some(LoadSpec { transition: Transition::Crossfade, ..demo(2, 2, 2000) }));
        assert!(matches!(next_event(&e, Duration::from_secs(4)), Some(Event::Advanced(2))));
        let t = start.elapsed().as_millis();
        // The fade starts 800 ms before the end, 1200 ms in, and is announced when it's heard.
        assert!((1000..1600).contains(&t), "advanced after {t} ms");
    }

    #[test]
    fn seeking_the_visible_song_cancels_an_unaudible_handoff() {
        let (_, commands) = mpsc::channel();
        let (events, _) = mpsc::channel();
        let shared = Arc::new(Shared::new());
        let settings = AudioSettings { crossfade_ms: 800, eq_enabled: false, eq_gains: [0.0; 6] };
        let mut worker = Worker::new(commands, events, shared.clone(), Arc::new(AtomicU64::new(0)), settings, true, || {});
        worker.start_deck(&demo(1, 1, 5_000), 0, 0);
        worker.next = Some(demo(2, 2, 5_000));
        assert!(worker.advance_to_next(Some(800), 24_000, None));
        assert!(worker.handle(Cmd::Seek { id: 1, ms: 600 }));
        assert_eq!(worker.current.as_ref().unwrap().id, 1);
        assert!(worker.outgoing.is_none() && worker.pending.is_empty());
        assert_eq!(shared.position_ms.load(Ordering::Relaxed), 600);
    }

    #[test]
    fn set_next_preloads_gapless_deck_and_reuses_it_on_manual_load() {
        let (_, commands) = mpsc::channel();
        let (events, _) = mpsc::channel();
        let shared = Arc::new(Shared::new());
        let settings = AudioSettings { crossfade_ms: 0, eq_enabled: false, eq_gains: [0.0; 6] };
        let mut worker = Worker::new(commands, events, shared, Arc::new(AtomicU64::new(0)), settings, true, || {});
        worker.start_deck(&demo(1, 1, 5_000), 0, 0);
        worker.handle(Cmd::SetNext(Some(demo(2, 2, 5_000))));
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while worker.prepared_next.is_none() && std::time::Instant::now() < deadline {
            worker.poll_preloaded_next();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.prepared_next.as_ref().is_some_and(|(id, _)| *id == 2), "expected background preloader to open next deck");
        // Manual Load (user clicks Next) reuses the preloaded deck and clears prepared_next.
        worker.handle(Cmd::Load { spec: demo(2, 2, 5_000), start_ms: 0, play: true, epoch: 0 });
        assert_eq!(worker.current.as_ref().unwrap().id, 2);
        assert!(worker.prepared_next.is_none());
    }

    #[test]
    fn pausing_stops_the_clock_and_seeking_moves_it() {
        let e = engine(0);
        e.load(demo(1, 1, 30_000), 0, true);
        std::thread::sleep(Duration::from_millis(600));
        e.pause();
        std::thread::sleep(Duration::from_millis(150));
        let a = e.position_ms();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(a, e.position_ms());
        assert!(a > 100, "clock should have run, was {a}");
        e.seek(1, 20_000);
        std::thread::sleep(Duration::from_millis(100));
        assert!((19_900..20_200).contains(&e.position_ms()));
    }

    #[test]
    fn missing_file_is_reported() {
        let e = engine(0);
        let spec = LoadSpec { id: 9, source: SourceSpec::File("Z:/no/such/file.mp3".into()), duration_ms: 0, gain_db: 0.0, transition: Transition::Gapless };
        e.load(spec, 0, true);
        assert!(matches!(next_event(&e, Duration::from_secs(2)), Some(Event::Error(9, _))));
    }

    fn write_wav(path: &std::path::Path, seconds: u32) {
        let rate = 44_100u32;
        let frames = rate * seconds;
        let data_len = frames * 4;
        let mut b = Vec::with_capacity(44 + data_len as usize);
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..frames {
            let s = ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 16000.0) as i16;
            b.extend_from_slice(&s.to_le_bytes());
            b.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn real_files_decode_seek_and_measure() {
        let dir = std::env::temp_dir().join(format!("nhip-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        write_wav(&path, 3);
        let p = path.to_string_lossy().into_owned();

        let mut src = FileSource::open(&p).unwrap();
        assert_eq!(src.rate(), 44_100);
        assert!((src.duration_ms() as i64 - 3000).abs() < 50, "duration {}", src.duration_ms());

        let mut out = VecDeque::new();
        while src.fill(&mut out) {}
        assert!((out.len() as i64 - 2 * 3 * 44_100).abs() < 8192, "samples {}", out.len());

        assert!(src.seek(2_000).is_some());
        out.clear();
        while src.fill(&mut out) {}
        assert!((out.len() as i64 - 2 * 44_100).abs() < 8192, "after seek {}", out.len());

        // A sine at half scale: energy 0.125 per channel gives about -9.7 dB.
        let (l, peak) = measure_loudness(&p).unwrap();
        assert!((l + 9.7).abs() < 1.0, "loudness {l}");
        assert!((peak + 6.2).abs() < 0.2, "peak {peak}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn full_scale_sine_measures_near_minus_three() {
        // A full-scale sine has energy 0.5 per channel.
        let l = gated_loudness(&vec![0.5; 20]).unwrap();
        assert!((l + 3.7).abs() < 0.3, "got {l}");
    }

    #[test]
    fn silence_has_no_loudness() {
        assert!(gated_loudness(&vec![0.0; 20]).is_none());
    }

    #[test]
    fn quiet_intro_does_not_pull_the_measurement_down() {
        let mut blocks = vec![1e-5; 10];
        blocks.extend(vec![0.1; 40]);
        let l = gated_loudness(&blocks).unwrap();
        let expected = -0.691 + 10.0 * 0.1f64.log10();
        assert!((l as f64 - expected).abs() < 0.2, "got {l}");
    }
}

mod http_range {
    use std::collections::VecDeque;
    use std::io::{self, Read, Seek, SeekFrom};
    use std::time::Instant;

    use symphonia::core::io::MediaSource;

    use super::CancelSignal;
    use crate::ytm::{google_media_url, stream_http_agent, ResolvedStream};

    pub(super) const CHUNK_SIZE: usize = 256 * 1024;
    const MAX_CACHED_CHUNKS: usize = 8;
    const READ_SLICE: usize = 32 * 1024;

    enum RangeErr {
        Cancelled,
        AuthOrExpired(u16),
        HttpStatus(u16),
        Io(io::Error),
    }

    impl RangeErr {
        fn into_io(self) -> io::Error {
            match self {
                Self::Cancelled => io::Error::new(io::ErrorKind::Interrupted, "playback load cancelled"),
                Self::AuthOrExpired(s) | Self::HttpStatus(s) => io::Error::other(format!("HTTP {s}")),
                Self::Io(e) => e,
            }
        }
    }

    const MAX_STREAM_REFRESHES: u8 = 3;

    pub(crate) struct HttpRangeSource {
        agent: reqwest::blocking::Client,
        stream: ResolvedStream,
        resolver: Box<dyn FnMut(bool) -> Result<ResolvedStream, String> + Send + Sync>,
        enforce_google_url: bool,
        refreshes: u8,
        total_len: u64,
        pos: u64,
        cache: VecDeque<(u64, Vec<u8>)>,
        cancel: CancelSignal,
    }

    impl HttpRangeSource {
        pub(crate) fn open(video_id: String, cancel: CancelSignal) -> io::Result<Self> {
            Self::open_youtube(video_id, cancel, false)
        }

        pub(crate) fn open_fresh(video_id: String, cancel: CancelSignal) -> io::Result<Self> {
            Self::open_youtube(video_id, cancel, true)
        }

        fn open_youtube(video_id: String, cancel: CancelSignal, initial_fresh: bool) -> io::Result<Self> {
            let agent = stream_http_agent().clone();
            Self::open_inner(
                agent,
                Box::new(move |fresh| {
                    if fresh {
                        crate::ytm::resolve_stream_fresh(&video_id)
                    } else {
                        crate::ytm::resolve_stream(&video_id)
                    }
                }),
                true,
                initial_fresh,
                cancel,
            )
        }

        #[cfg(test)]
        pub(crate) fn open_with_resolver(
            agent: reqwest::blocking::Client,
            mut resolver: Box<dyn FnMut() -> Result<ResolvedStream, String> + Send + Sync>,
            cancel: CancelSignal,
        ) -> io::Result<Self> {
            Self::open_inner(agent, Box::new(move |_fresh| resolver()), false, false, cancel)
        }

        #[cfg(test)]
        pub(crate) fn open_with_fresh_resolver(
            agent: reqwest::blocking::Client,
            resolver: Box<dyn FnMut(bool) -> Result<ResolvedStream, String> + Send + Sync>,
            cancel: CancelSignal,
        ) -> io::Result<Self> {
            Self::open_inner(agent, resolver, false, false, cancel)
        }

        fn open_inner(
            agent: reqwest::blocking::Client,
            mut resolver: Box<dyn FnMut(bool) -> Result<ResolvedStream, String> + Send + Sync>,
            enforce_google_url: bool,
            initial_fresh: bool,
            cancel: CancelSignal,
        ) -> io::Result<Self> {
            if cancel.is_cancelled() {
                return Err(RangeErr::Cancelled.into_io());
            }
            let mut stream = resolver(initial_fresh).map_err(io::Error::other)?;
            if enforce_google_url && google_media_url(&stream.url).is_none() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "untrusted stream host"));
            }
            let mut refreshes = 0u8;
            let (total, first_chunk) = match Self::fetch_full_range(
                &agent,
                &stream,
                0,
                (CHUNK_SIZE as u64) - 1,
                None,
                &cancel,
            ) {
                Ok(ok) => ok,
                Err(RangeErr::AuthOrExpired(_)) if !initial_fresh => {
                    // Cached stream URL expired before first chunk; force a fresh resolution once.
                    stream = resolver(true).map_err(io::Error::other)?;
                    if enforce_google_url && google_media_url(&stream.url).is_none() {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "untrusted stream host"));
                    }
                    refreshes = 1;
                    Self::fetch_full_range(
                        &agent,
                        &stream,
                        0,
                        (CHUNK_SIZE as u64) - 1,
                        None,
                        &cancel,
                    )
                    .map_err(RangeErr::into_io)?
                }
                Err(e) => return Err(e.into_io()),
            };

            let mut cache = VecDeque::with_capacity(MAX_CACHED_CHUNKS);
            cache.push_back((0, first_chunk));

            Ok(Self {
                agent,
                stream,
                resolver,
                enforce_google_url,
                refreshes,
                total_len: total,
                pos: 0,
                cache,
                cancel,
            })
        }

        fn fetch_full_range(
            agent: &reqwest::blocking::Client,
            stream: &ResolvedStream,
            start: u64,
            requested_end: u64,
            expected_total: Option<u64>,
            cancel: &CancelSignal,
        ) -> Result<(u64, Vec<u8>), RangeErr> {
            let mut cursor = start;
            let mut target_end = requested_end;
            let mut known_total = expected_total;
            let mut out = Vec::new();

            loop {
                let (cr_end, cr_total, mut part) =
                    Self::request_once(agent, stream, cursor, target_end, cancel)?;
                if let Some(prev) = known_total {
                    if cr_total != prev {
                        return Err(RangeErr::Io(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "stream content length changed",
                        )));
                    }
                }
                known_total = Some(cr_total);
                if target_end >= cr_total {
                    target_end = cr_total - 1;
                }
                out.append(&mut part);
                if cr_end >= target_end {
                    return Ok((cr_total, out));
                }
                cursor = cr_end + 1;
            }
        }

        fn request_once(
            agent: &reqwest::blocking::Client,
            stream: &ResolvedStream,
            start: u64,
            end: u64,
            cancel: &CancelSignal,
        ) -> Result<(u64, u64, Vec<u8>), RangeErr> {
            if cancel.is_cancelled() {
                return Err(RangeErr::Cancelled);
            }
            let mut req = agent
                .get(&stream.url)
                .header("Range", format!("bytes={start}-{end}"))
                .header("Accept-Encoding", "identity");
            for (k, v) in &stream.headers {
                req = req.header(k.as_str(), v.as_str());
            }

            let mut resp = match req.send() {
                Ok(r) => r,
                Err(e) => {
                    if cancel.is_cancelled() {
                        return Err(RangeErr::Cancelled);
                    }
                    return Err(RangeErr::Io(io::Error::other(e.to_string())));
                }
            };

            let status = resp.status().as_u16();
            if matches!(status, 401 | 403 | 410) {
                return Err(RangeErr::AuthOrExpired(status));
            }
            if status != 206 {
                return Err(RangeErr::HttpStatus(status));
            }

            let cr_header = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| {
                    RangeErr::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "missing Content-Range",
                    ))
                })?;
            let (cr_start, cr_end, cr_total) = parse_content_range(cr_header).ok_or_else(|| {
                RangeErr::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid Content-Range",
                ))
            })?;
            if cr_start != start {
                return Err(RangeErr::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "unexpected Content-Range start",
                )));
            }

            let expected_len = (cr_end - cr_start + 1) as usize;
            let mut body = Vec::with_capacity(expected_len.min(CHUNK_SIZE * 2));
            let mut tmp = [0u8; READ_SLICE];
            while body.len() < expected_len {
                if cancel.is_cancelled() {
                    return Err(RangeErr::Cancelled);
                }
                let want = (expected_len - body.len()).min(tmp.len());
                match resp.read(&mut tmp[..want]) {
                    Ok(0) => {
                        return Err(RangeErr::Io(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "truncated HTTP range body",
                        )));
                    }
                    Ok(n) => body.extend_from_slice(&tmp[..n]),
                    Err(e) => {
                        if cancel.is_cancelled() {
                            return Err(RangeErr::Cancelled);
                        }
                        return Err(RangeErr::Io(e));
                    }
                }
            }
            Ok((cr_end, cr_total, body))
        }

        fn refresh_stream(&mut self) -> io::Result<()> {
            if self.cancel.is_cancelled() {
                return Err(RangeErr::Cancelled.into_io());
            }
            if self.refreshes >= MAX_STREAM_REFRESHES {
                return Err(io::Error::other("stream refresh limit reached"));
            }
            self.refreshes += 1;
            let next = (self.resolver)(true).map_err(io::Error::other)?;
            if self.enforce_google_url && google_media_url(&next.url).is_none() {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "untrusted stream host"));
            }
            self.stream = next;
            Ok(())
        }

        fn get_or_fetch_chunk(&mut self, chunk_idx: u64) -> io::Result<&[u8]> {
            if let Some(pos) = self.cache.iter().position(|(idx, _)| *idx == chunk_idx) {
                let entry = self.cache.remove(pos).expect("valid cache index");
                self.cache.push_back(entry);
                return Ok(&self.cache.back().expect("just pushed").1);
            }

            let start = chunk_idx * CHUNK_SIZE as u64;
            let end = (start + CHUNK_SIZE as u64 - 1).min(self.total_len - 1);
            let mut io_retries = 0u8;
            let mut refreshed_this_chunk = false;

            let data = loop {
                if self.cancel.is_cancelled() {
                    return Err(RangeErr::Cancelled.into_io());
                }
                if self.stream.expires_at.is_some_and(|exp| Instant::now() >= exp) {
                    if refreshed_this_chunk || self.refreshes >= MAX_STREAM_REFRESHES {
                        return Err(io::Error::other("stream URL expired after refresh"));
                    }
                    refreshed_this_chunk = true;
                    self.refresh_stream()?;
                }
                match Self::fetch_full_range(
                    &self.agent,
                    &self.stream,
                    start,
                    end,
                    Some(self.total_len),
                    &self.cancel,
                ) {
                    Ok((_, data)) => break data,
                    Err(RangeErr::AuthOrExpired(_))
                        if !refreshed_this_chunk && self.refreshes < MAX_STREAM_REFRESHES =>
                    {
                        refreshed_this_chunk = true;
                        self.refresh_stream()?;
                    }
                    Err(RangeErr::Io(_)) if io_retries < 1 && !self.cancel.is_cancelled() => {
                        io_retries += 1;
                    }
                    Err(e) => return Err(e.into_io()),
                }
            };

            if self.cache.len() >= MAX_CACHED_CHUNKS {
                self.cache.pop_front();
            }
            self.cache.push_back((chunk_idx, data));
            Ok(&self.cache.back().expect("just pushed").1)
        }
    }

    impl Read for HttpRangeSource {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.cancel.is_cancelled() {
                return Err(RangeErr::Cancelled.into_io());
            }
            if buf.is_empty() || self.pos >= self.total_len {
                return Ok(0);
            }
            let chunk_idx = self.pos / CHUNK_SIZE as u64;
            let offset = (self.pos % CHUNK_SIZE as u64) as usize;
            let chunk = self.get_or_fetch_chunk(chunk_idx)?;
            if offset >= chunk.len() {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "short range chunk",
                ));
            }
            let n = (chunk.len() - offset).min(buf.len());
            buf[..n].copy_from_slice(&chunk[offset..offset + n]);
            self.pos += n as u64;
            Ok(n)
        }
    }

    impl Seek for HttpRangeSource {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            let next: i128 = match pos {
                SeekFrom::Start(n) => n as i128,
                SeekFrom::Current(delta) => self.pos as i128 + delta as i128,
                SeekFrom::End(delta) => self.total_len as i128 + delta as i128,
            };
            if next < 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "seek before start of stream",
                ));
            }
            self.pos = next as u64;
            Ok(self.pos)
        }
    }

    impl MediaSource for HttpRangeSource {
        fn is_seekable(&self) -> bool {
            true
        }

        fn byte_len(&self) -> Option<u64> {
            Some(self.total_len)
        }
    }

    fn parse_content_range(raw: &str) -> Option<(u64, u64, u64)> {
        let rest = raw.trim().strip_prefix("bytes")?.trim_start();
        let (range, total) = rest.split_once('/')?;
        if total.trim() == "*" {
            return None;
        }
        let total: u64 = total.trim().parse().ok()?;
        let (start, end) = range.split_once('-')?;
        let start: u64 = start.trim().parse().ok()?;
        let end: u64 = end.trim().parse().ok()?;
        (start <= end && end < total).then_some((start, end, total))
    }

    #[cfg(test)]
    pub(super) mod tests {
        use std::collections::VecDeque;
        use std::io::{Read, Seek, SeekFrom, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;
        use std::time::{Duration, Instant};

        use super::*;
        use crate::audio::source::{FileSource, Source};

        pub(in crate::audio) fn pcm_wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
            let channels = 2u16;
            let num_frames = samples.len() / 2;
            let data_bytes = (num_frames * channels as usize * 2) as u32;
            let mut wav = Vec::with_capacity(44 + data_bytes as usize);
            wav.extend_from_slice(b"RIFF");
            wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
            wav.extend_from_slice(b"WAVEfmt ");
            wav.extend_from_slice(&16u32.to_le_bytes());
            wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
            wav.extend_from_slice(&channels.to_le_bytes());
            wav.extend_from_slice(&sample_rate.to_le_bytes());
            wav.extend_from_slice(&(sample_rate * channels as u32 * 2).to_le_bytes());
            wav.extend_from_slice(&(channels * 2).to_le_bytes());
            wav.extend_from_slice(&16u16.to_le_bytes());
            wav.extend_from_slice(b"data");
            wav.extend_from_slice(&data_bytes.to_le_bytes());
            for &sample in samples {
                let s = (sample.clamp(-1.0, 1.0) * 32_767.0) as i16;
                wav.extend_from_slice(&s.to_le_bytes());
            }
            wav
        }

        fn make_test_wav(seconds: f32) -> Vec<u8> {
            let sample_rate = 44_100u32;
            let num_frames = (sample_rate as f32 * seconds) as usize;
            let mut samples = Vec::with_capacity(num_frames * 2);
            for i in 0..num_frames {
                let s = ((i as f32 * 440.0 * std::f32::consts::TAU) / sample_rate as f32).sin() * 0.5;
                samples.push(s);
                samples.push(s);
            }
            pcm_wav_bytes(&samples, sample_rate)
        }

        pub(in crate::audio) fn test_agent() -> reqwest::blocking::Client {
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .no_proxy()
                .build()
                .expect("test HTTP client")
        }

        pub(in crate::audio) fn parse_http_request(stream: &mut std::net::TcpStream) -> Option<(String, Option<(u64, u64)>)> {
            let mut req = Vec::new();
            let mut buf = [0u8; 1024];
            while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).ok()?;
                if n == 0 {
                    return None;
                }
                req.extend_from_slice(&buf[..n]);
            }
            let text = String::from_utf8_lossy(&req);
            let mut lines = text.lines();
            let first = lines.next()?;
            let path = first.split_whitespace().nth(1)?.to_string();
            let mut range = None;
            for line in lines {
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("range") {
                        if let Some(spec) = v.trim().strip_prefix("bytes=") {
                            if let Some((a, b)) = spec.split_once('-') {
                                if let (Ok(s), Ok(e)) = (a.parse(), b.parse()) {
                                    range = Some((s, e));
                                }
                            }
                        }
                    }
                }
            }
            Some((path, range))
        }

        pub(in crate::audio) fn send_206(stream: &mut std::net::TcpStream, data: &[u8], start: u64, end: u64) {
            let end = end.min(data.len() as u64 - 1);
            let slice = &data[start as usize..=end as usize];
            let header = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                start,
                end,
                data.len(),
                slice.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(slice);
        }

        #[test]
        fn http_range_decodes_seeks_forward_and_backward() {
            let wav = Arc::new(make_test_wav(3.5)); // > 600 KiB (3 chunks)
            assert!(wav.len() > CHUNK_SIZE * 2);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let hits = Arc::new(AtomicUsize::new(0));

            let srv_data = wav.clone();
            let srv_stop = stop.clone();
            let srv_hits = hits.clone();
            listener.set_nonblocking(true).unwrap();
            let server = thread::spawn(move || {
                while !srv_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut conn, _)) => {
                            let _ = conn.set_nonblocking(false);
                            if let Some((_path, Some((s, e)))) = parse_http_request(&mut conn) {
                                srv_hits.fetch_add(1, Ordering::Relaxed);
                                send_206(&mut conn, &srv_data, s, e);
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });

            let url = format!("http://{addr}/audio");
            let source = HttpRangeSource::open_with_resolver(
                test_agent(),
                Box::new(move || {
                    Ok(ResolvedStream {
                        itag: Some(140),
                        url: url.clone(),
                        headers: vec![],
                        content_length: None,
                        expires_at: None,
                    })
                }),
                CancelSignal::Flag(Arc::new(AtomicBool::new(false))),
            )
            .unwrap();

            let mut file = FileSource::from_media_source(Box::new(source), Some("wav")).unwrap();
            let mut out = VecDeque::new();
            assert!(file.fill(&mut out));
            assert!(!out.is_empty());

            // Seek forward past 512 KiB (chunk 2) and decode.
            assert!(file.seek(2_900).is_some());
            out.clear();
            assert!(file.fill(&mut out));
            assert!(!out.is_empty());

            // Seek backward into chunk 0 and verify cached/re-readable decode works.
            let hits_before_back_seek = hits.load(Ordering::Relaxed);
            assert!(file.seek(200).is_some());
            out.clear();
            assert!(file.fill(&mut out));
            assert!(!out.is_empty());
            assert_eq!(hits.load(Ordering::Relaxed), hits_before_back_seek);

            stop.store(true, Ordering::Relaxed);
            let _ = server.join();
        }

        #[test]
        fn http_range_re_resolves_once_on_403_expiry() {
            let payload: Arc<Vec<u8>> = Arc::new((0..(CHUNK_SIZE * 2 + 128)).map(|i| (i % 251) as u8).collect());
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let always_403_after_v2 = Arc::new(AtomicBool::new(false));

            let srv_data = payload.clone();
            let srv_stop = stop.clone();
            let srv_403 = always_403_after_v2.clone();
            listener.set_nonblocking(true).unwrap();
            let server = thread::spawn(move || {
                while !srv_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut conn, _)) => {
                            let _ = conn.set_nonblocking(false);
                            if let Some((path, Some((s, e)))) = parse_http_request(&mut conn) {
                                if (path == "/v1" && s > 0) || (path != "/v1" && srv_403.load(Ordering::Relaxed)) {
                                    let _ = conn.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                                } else {
                                    send_206(&mut conn, &srv_data, s, e);
                                }
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });

            let resolves = Arc::new(AtomicUsize::new(0));
            let res_count = resolves.clone();
            let mut src = HttpRangeSource::open_with_resolver(
                test_agent(),
                Box::new(move || {
                    let n = res_count.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok(ResolvedStream {
                        itag: Some(140),
                        url: format!("http://{addr}/v{n}"),
                        headers: vec![],
                        content_length: None,
                        expires_at: Some(Instant::now() + Duration::from_secs(60)),
                    })
                }),
                CancelSignal::Flag(Arc::new(AtomicBool::new(false))),
            )
            .unwrap();
            assert_eq!(resolves.load(Ordering::Relaxed), 1);

            // Seeking to chunk 1 on /v1 returns 403 -> re-resolves once to /v2 and succeeds.
            src.seek(SeekFrom::Start(CHUNK_SIZE as u64)).unwrap();
            let mut buf = [0u8; 16];
            let n = src.read(&mut buf).unwrap();
            assert_eq!(n, 16);
            assert_eq!(&buf, &payload[CHUNK_SIZE..CHUNK_SIZE + 16]);
            assert_eq!(resolves.load(Ordering::Relaxed), 2);

            // If chunk 2 returns 403 even after refreshing once for that chunk, fail cleanly without looping forever.
            always_403_after_v2.store(true, Ordering::Relaxed);
            src.seek(SeekFrom::Start((CHUNK_SIZE * 2) as u64)).unwrap();
            let err = src.read(&mut buf).unwrap_err();
            assert!(err.to_string().contains("403"), "got {err}");
            assert_eq!(resolves.load(Ordering::Relaxed), 3);

            stop.store(true, Ordering::Relaxed);
            let _ = server.join();
        }

        #[test]
        fn http_range_forces_fresh_resolve_on_initial_403_and_midstream_403() {
            let wav = Arc::new(make_test_wav(1.5));
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let stop = Arc::new(AtomicBool::new(false));

            let srv_data = wav.clone();
            let srv_stop = stop.clone();
            listener.set_nonblocking(true).unwrap();
            let server = thread::spawn(move || {
                while !srv_stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut conn, _)) => {
                            let _ = conn.set_nonblocking(false);
                            if let Some((path, Some((s, e)))) = parse_http_request(&mut conn) {
                                if path == "/stale" {
                                    let _ = conn.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                                } else {
                                    send_206(&mut conn, &srv_data, s, e);
                                }
                            }
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });

            let fresh_calls = Arc::new(AtomicUsize::new(0));
            let fc = fresh_calls.clone();
            // Simulate a stale cached URL returning 403 on initial open: resolver(false) -> /stale, resolver(true) -> /fresh.
            let src = HttpRangeSource::open_with_fresh_resolver(
                test_agent(),
                Box::new(move |fresh| {
                    let path = if fresh {
                        fc.fetch_add(1, Ordering::Relaxed);
                        "/fresh"
                    } else {
                        "/stale"
                    };
                    Ok(ResolvedStream {
                        itag: Some(140),
                        url: format!("http://{addr}{path}"),
                        headers: vec![],
                        content_length: None,
                        expires_at: Some(Instant::now() + Duration::from_secs(60)),
                    })
                }),
                CancelSignal::Flag(Arc::new(AtomicBool::new(false))),
            )
            .unwrap();
            assert_eq!(fresh_calls.load(Ordering::Relaxed), 1);

            let mut file = FileSource::from_media_source(Box::new(src), Some("wav")).unwrap();
            let mut out = VecDeque::new();
            assert!(file.fill(&mut out));
            assert!(!out.is_empty());

            stop.store(true, Ordering::Relaxed);
            let _ = server.join();
        }

        #[test]
        fn engine_coalesces_seek_burst_to_latest_target() {
            let engine = crate::audio::Engine::start_silent(
                crate::audio::AudioSettings { crossfade_ms: 0, eq_enabled: false, eq_gains: [0.0; 6] },
                || {},
            );
            let spec = crate::audio::LoadSpec {
                id: 77,
                source: crate::audio::SourceSpec::Demo { seed: 3, duration_ms: 30_000 },
                duration_ms: 30_000,
                gain_db: 0.0,
                transition: crate::mechanics::Transition::Gapless,
            };
            engine.load(spec, 0, false);
            let deadline = Instant::now() + Duration::from_secs(2);
            while !engine.has_song() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            assert!(engine.has_song());
            for ms in [1_000, 3_000, 5_000, 9_000, 14_000] {
                engine.seek(77, ms);
            }
            let deadline = Instant::now() + Duration::from_secs(2);
            while engine.position_ms() < 13_500 && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            let pos = engine.position_ms();
            assert!((13_500..=14_500).contains(&pos), "expected settled seek near 14000ms, got {pos}ms");
        }

        #[test]
        fn http_range_aborts_when_cancelled() {
            let cancel = Arc::new(AtomicBool::new(true));
            let resolves = Arc::new(AtomicUsize::new(0));
            let res_count = resolves.clone();
            let err = match HttpRangeSource::open_with_resolver(
                test_agent(),
                Box::new(move || {
                    res_count.fetch_add(1, Ordering::Relaxed);
                    Ok(ResolvedStream {
                        itag: Some(140),
                        url: "http://127.0.0.1:1/audio".into(),
                        headers: vec![],
                        content_length: None,
                        expires_at: None,
                    })
                }),
                CancelSignal::Flag(cancel),
            ) {
                Ok(_) => panic!("expected cancelled open to fail"),
                Err(e) => e,
            };
            assert_eq!(err.kind(), io::ErrorKind::Interrupted);
            assert_eq!(resolves.load(Ordering::Relaxed), 0);
        }
    }
}

