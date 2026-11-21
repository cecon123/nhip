//! Prepares one short, key-locked transition off the audio worker.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;

use timestretch::{
    detect_beat_grid, BeatGrid, Parameters, PreparedSession, ProcessStatus, SessionConfig,
    SourceAccess, TempoRate,
};

use super::http_range::HttpRangeSource;
use super::{CancelSignal, Deck, FileSource, LoadSpec, Source, SourceSpec};

const WINDOW_SECONDS: u32 = 12;
pub(super) const FALLBACK_MS: u64 = 3_000;

struct Request {
    revision: u64,
    active_epoch: u64,
    current: SourceSpec,
    next: LoadSpec,
}

pub(super) struct Analyzer {
    requests: Sender<Request>,
    results: Receiver<(u64, Option<PreparedMix>)>,
    revision: u64,
    prepare_epoch: Arc<AtomicU64>,
}

impl Analyzer {
    pub(super) fn start(load_epoch: Arc<AtomicU64>) -> std::io::Result<Self> {
        let (requests, rx) = mpsc::channel::<Request>();
        let (tx, results) = mpsc::channel();
        let prepare_epoch = Arc::new(AtomicU64::new(0));
        let worker_prepare = prepare_epoch.clone();
        std::thread::Builder::new()
            .name("auto-mix".into())
            .spawn(move || {
                while let Ok(mut job) = rx.recv() {
                    while let Ok(latest) = rx.try_recv() {
                        job = latest;
                    }
                    let plan = prepare(
                        &job.current,
                        &job.next,
                        &load_epoch,
                        job.active_epoch,
                        &worker_prepare,
                        job.revision,
                    );
                    if tx.send((job.revision, plan)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests,
            results,
            revision: 0,
            prepare_epoch,
        })
    }

    pub(super) fn select(&mut self, tracks: Option<(SourceSpec, LoadSpec)>, active_epoch: u64) {
        self.revision = self.revision.wrapping_add(1);
        self.prepare_epoch.store(self.revision, Ordering::Release);
        if let Some((current, next)) = tracks {
            let _ = self.requests.send(Request {
                revision: self.revision,
                active_epoch,
                current,
                next,
            });
        }
    }

    pub(super) fn poll(&self) -> Option<PreparedMix> {
        self.results
            .try_iter()
            .find_map(|(revision, plan)| (revision == self.revision).then_some(plan).flatten())
    }
}

impl Drop for Analyzer {
    fn drop(&mut self) {
        self.prepare_epoch.fetch_add(1, Ordering::AcqRel);
    }
}

pub(super) struct PreparedMix {
    pub(super) start_ms: u64,
    pub(super) deck: Deck,
}

fn open_outgoing(current: &SourceSpec, cancel: CancelSignal) -> Option<FileSource> {
    match current {
        SourceSpec::File(path) => FileSource::open(path).ok(),
        SourceSpec::YouTube { video_id } => {
            let media = HttpRangeSource::open(video_id.clone(), cancel).ok()?;
            FileSource::from_media_source(Box::new(media), Some("m4a")).ok()
        }
        SourceSpec::Demo { .. } => None,
    }
}

fn prepare(
    current: &SourceSpec,
    next: &LoadSpec,
    load_epoch: &Arc<AtomicU64>,
    active_epoch: u64,
    prepare_epoch: &Arc<AtomicU64>,
    revision: u64,
) -> Option<PreparedMix> {
    let outgoing_cancel = CancelSignal::Prepare {
        load_epoch: load_epoch.clone(),
        expected_epoch: active_epoch,
        prepare_epoch: prepare_epoch.clone(),
        expected_prepare: revision,
        preparing: Arc::new(AtomicBool::new(true)),
    };
    let incoming_preparing = Arc::new(AtomicBool::new(true));
    let incoming_cancel = CancelSignal::Prepare {
        load_epoch: load_epoch.clone(),
        expected_epoch: active_epoch,
        prepare_epoch: prepare_epoch.clone(),
        expected_prepare: revision,
        preparing: incoming_preparing.clone(),
    };

    let outgoing = open_outgoing(current, outgoing_cancel)?;
    let deck = Deck::open_with_cancel(next, incoming_cancel).ok()?;
    let plan = prepare_sources(outgoing, deck, || {
        load_epoch.load(Ordering::Acquire) != active_epoch
            || prepare_epoch.load(Ordering::Acquire) != revision
    })?;

    // Disarm the prepare-epoch cancel check once promoted, then ensure no newer select() superseded it.
    incoming_preparing.store(false, Ordering::Release);
    if load_epoch.load(Ordering::Acquire) != active_epoch
        || prepare_epoch.load(Ordering::Acquire) != revision
    {
        return None;
    }
    Some(plan)
}

fn prepare_sources(
    mut outgoing: FileSource,
    mut deck: Deck,
    should_abort: impl Fn() -> bool,
) -> Option<PreparedMix> {
    if should_abort() {
        return None;
    }
    let duration = outgoing.duration_ms();
    let tail = outgoing
        .seek_to_ms(duration.checked_sub((WINDOW_SECONDS * 1000) as u64)?)
        .ok()??;
    let outgoing_rate = outgoing.rate();
    let tail_samples = window(&mut outgoing)?;
    if should_abort() {
        return None;
    }
    let outgoing_grid = grid(&tail_samples, outgoing_rate)?;

    let incoming_rate = deck.src.rate();
    let mut samples = window(deck.src.as_mut())?;
    if should_abort() {
        return None;
    }
    let incoming_grid = grid(&samples, incoming_rate)?;
    let (start_ms, tempo) = align(&outgoing_grid, &incoming_grid, tail, duration)?;
    let fade_ms = duration - start_ms;
    let frames = (incoming_rate * WINDOW_SECONDS) as usize;
    let mut remainder = samples.split_off((frames * 2).min(samples.len()));
    let (stretched, timing) =
        render_intro(samples.make_contiguous(), incoming_rate, fade_ms, tempo)?;
    deck.buf = stretched.into();
    deck.buf.append(&mut remainder);
    deck.tempo_map = Some(timing);
    Some(PreparedMix { start_ms, deck })
}

fn window(source: &mut dyn Source) -> Option<VecDeque<f32>> {
    let rate = source.rate();
    if !(32_000..=192_000).contains(&rate) {
        return None;
    }
    let mut pcm = VecDeque::new();
    while pcm.len() < (rate * WINDOW_SECONDS * 2) as usize && source.fill(&mut pcm) {}
    (!pcm.is_empty()).then_some(pcm)
}

fn grid(samples: &VecDeque<f32>, rate: u32) -> Option<BeatGrid> {
    let mut interleaved = samples.iter();
    let mono: Vec<_> =
        std::iter::from_fn(|| Some(0.5 * (interleaved.next()? + interleaved.next()?)))
            .take((rate * WINDOW_SECONDS) as usize)
            .collect();
    let grid = detect_beat_grid(&mono, rate);
    let stable = grid
        .segments
        .iter()
        .all(|segment| (segment.bpm / grid.bpm - 1.0).abs() < 0.05);
    (grid.confidence >= 0.48
        && !grid.phase_untrusted
        && stable
        && grid.bpm.is_finite()
        && grid.bpm > 0.0
        && grid.beats.len() >= 6)
        .then_some(grid)
}

fn align(
    outgoing: &BeatGrid,
    incoming: &BeatGrid,
    tail_ms: u64,
    duration: u64,
) -> Option<(u64, f64)> {
    let tempo = outgoing.bpm / incoming.bpm;
    if !(0.95..=1.05).contains(&tempo) {
        return None;
    }
    let beat_ms = 60_000.0 / outgoing.bpm;
    let entry_delay = incoming.beats.first()? * 1000.0 / (incoming.sample_rate as f64 * tempo);
    if entry_delay > 1_500.0 {
        return None;
    }
    let overlap = (beat_ms * 4.0).clamp(1_000.0, 6_000.0);
    let chosen = outgoing
        .beats
        .iter()
        .map(|sample| tail_ms as f64 + sample * 1000.0 / outgoing.sample_rate as f64 - entry_delay)
        .filter(|&start| (1_000.0..=6_000.0).contains(&(duration as f64 - start)))
        .min_by(|&a, &b| {
            let expected = duration as f64 - overlap;
            (a - expected).abs().total_cmp(&(b - expected).abs())
        })?;
    Some((chosen.round() as u64, tempo))
}

pub(super) struct TempoMap(Vec<(usize, f64)>);

impl TempoMap {
    pub(super) fn source_frame(&self, output_frame: f64) -> f64 {
        let next = self
            .0
            .partition_point(|&(frame, _)| frame as f64 <= output_frame);
        let (left, source) = self.0[next.saturating_sub(1)];
        match self.0.get(next) {
            Some(&(right, end)) => {
                source + (end - source) * (output_frame - left as f64) / (right - left) as f64
            }
            None => source + (output_frame - left as f64),
        }
    }
}

fn render_intro(
    input: &[f32],
    rate: u32,
    fade_ms: u64,
    tempo: f64,
) -> Option<(Vec<f32>, TempoMap)> {
    let frames = input.len() / 2;
    let fade_end = fade_ms * rate as u64 / 1000;
    if frames as u64 <= fade_end + 2 * rate as u64 {
        return None;
    }
    if (tempo - 1.0).abs() < 0.001 {
        return Some((
            input.to_vec(),
            TempoMap(vec![(0, 0.0), (frames, frames as f64)]),
        ));
    }
    let config = SessionConfig {
        source_sample_rate: rate,
        output_sample_rate: rate,
        source_frames: Some(frames as u64),
        source_access: SourceAccess::Seekable,
        parameters: Parameters::keylock(TempoRate::new(tempo).ok()?),
        ..SessionConfig::default()
    };
    let mut session = PreparedSession::new(config).ok()?.activate();
    for step in 1..=12 {
        let at = fade_end + rate as u64 * step / 8;
        let tempo = TempoRate::new(tempo + (1.0 - tempo) * step as f64 / 12.0).ok()?;
        session
            .schedule(step, at, Parameters::keylock(tempo))
            .ok()?;
    }

    let mut samples = Vec::with_capacity(input.len());
    let mut points = vec![(0, 0.0)];
    let mut block = [0.0; 2048];
    for _ in 0..(frames / 128 + 128) {
        if session.finished() {
            let last = points.last_mut()?;
            if last.0 == 0 {
                return None;
            }
            last.1 = frames as f64;
            // Join back to the original signal after the tempo has returned to unity.
            let join_frames = (rate as usize / 50).min(frames).min(last.0);
            let tail = &mut samples[(last.0 - join_frames) * 2..];
            let original = &input[(frames - join_frames) * 2..];
            for (index, (mixed, dry)) in tail.iter_mut().zip(original).enumerate() {
                let weight = (index / 2 + 1) as f32 / join_frames as f32;
                *mixed = *mixed * (1.0 - weight) + dry * weight;
            }
            return Some((samples, TempoMap(points)));
        }
        match session.advance(32).status {
            ProcessStatus::Fault => return None,
            ProcessStatus::NeedSource => {
                let request = session.source_request()?;
                let start = request.start_frame as usize * 2;
                let end = (start + request.frames * 2).min(input.len());
                if start >= end {
                    session.finish_input().ok()?;
                } else {
                    session.supply(request, input.get(start..end)?).ok()?;
                }
            }
            _ => {}
        }
        while session.next_applied().is_some() {}
        let stamp = session.read_output(&mut block).ok()?;
        session
            .acknowledge(stamp.consumed(stamp.frames).ok()?)
            .ok()?;
        if stamp.frames > 0 {
            samples.extend_from_slice(&block[..stamp.frames * 2]);
            points.push((
                samples.len() / 2,
                session.position().clamp(0.0, frames as f64),
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 44_100;

    fn click_track(bpm: f64, offset: f64) -> VecDeque<f32> {
        (0..RATE * WINDOW_SECONDS)
            .flat_map(|frame| {
                let time = frame as f64 / RATE as f64;
                let phase = (time - offset).rem_euclid(60.0 / bpm);
                let hit = if phase < 0.06 && time >= offset {
                    (phase * 105.0 * std::f64::consts::TAU).sin() * (-55.0 * phase).exp()
                } else {
                    0.0
                };
                [hit as f32; 2]
            })
            .collect()
    }

    #[test]
    fn aligns_stable_nearby_tempos_and_rejects_incompatible_or_silent_intros() {
        let outgoing = grid(&click_track(120.0, 0.1), RATE).unwrap();
        let incoming = grid(&click_track(118.0, 0.24), RATE).unwrap();
        let (start, tempo) = align(&outgoing, &incoming, 68_000, 80_000).unwrap();
        assert!((77_500..=78_500).contains(&start), "start={start}");
        assert!((tempo - 120.0 / 118.0).abs() < 0.015, "tempo={tempo}");
        let different = grid(&click_track(96.0, 0.1), RATE).unwrap();
        assert!(align(&outgoing, &different, 68_000, 80_000).is_none());
        assert!(grid(&VecDeque::from(vec![0.0; RATE as usize * 2 * 10]), RATE).is_none());
    }

    #[test]
    fn intro_returns_to_original_tempo_and_keeps_the_source_clock_continuous() {
        let pcm: Vec<_> = (0..RATE * WINDOW_SECONDS)
            .flat_map(|frame| {
                let sine = (frame as f32 * 440.0 * std::f32::consts::TAU / RATE as f32).sin();
                [sine * 0.5; 2]
            })
            .collect();
        let (rendered, map) = render_intro(&pcm, RATE, 2_000, 1.04).unwrap();
        let frames = rendered.len() / 2;
        assert!(frames < pcm.len() / 2);
        assert!(rendered.iter().all(|sample| sample.is_finite()));
        assert!((map.source_frame(RATE as f64) / RATE as f64 - 1.04).abs() < 0.015);
        assert!(
            (map.source_frame(7.0 * RATE as f64)
                - map.source_frame(6.0 * RATE as f64)
                - RATE as f64)
                .abs()
                < 25.0
        );
        assert_eq!(
            map.source_frame(frames as f64 + 500.0),
            (pcm.len() / 2 + 500) as f64
        );
        let range = &rendered[RATE as usize..RATE as usize * 3];
        let crossings = range
            .chunks_exact(2)
            .map(|stereo| stereo[0])
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|pair| pair[0] <= 0.0 && pair[1] > 0.0)
            .count();
        assert!((436..=444).contains(&crossings), "pitch={crossings} Hz");
        assert_eq!(rendered.last(), pcm.last());
    }

    fn click_wav(bpm: f64, offset: f64, seconds: u32) -> Vec<u8> {
        let mut samples = Vec::with_capacity((RATE * seconds * 2) as usize);
        for frame in 0..RATE * seconds {
            let time = frame as f64 / RATE as f64;
            let phase = (time - offset).rem_euclid(60.0 / bpm);
            let hit = if phase < 0.06 && time >= offset {
                (phase * 105.0 * std::f64::consts::TAU).sin() * (-55.0 * phase).exp() * 0.8
            } else {
                0.0
            };
            samples.push(hit as f32);
            samples.push(hit as f32);
        }
        super::super::http_range::tests::pcm_wav_bytes(&samples, RATE)
    }

    #[test]
    fn automix_prepares_and_transitions_over_http_range_sources() {
        use std::io;
        use std::net::TcpListener;
        use std::sync::atomic::AtomicBool;
        use std::thread;
        use std::time::Duration;

        use super::super::http_range::tests::{
            parse_http_request, send_206, test_agent,
        };
        use crate::ytm::ResolvedStream;

        let out_wav = Arc::new(click_wav(120.0, 0.1, 16));
        let in_wav = Arc::new(click_wav(118.0, 0.24, 16));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let srv_stop = stop.clone();
        let srv_out = out_wav.clone();
        let srv_in = in_wav.clone();
        listener.set_nonblocking(true).unwrap();

        let server = thread::spawn(move || {
            while !srv_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut conn, _)) => {
                        let _ = conn.set_nonblocking(false);
                        if let Some((path, Some((s, e)))) = parse_http_request(&mut conn) {
                            let body = if path == "/out" { &srv_out } else { &srv_in };
                            send_206(&mut conn, body, s, e);
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        let out_url = format!("http://{addr}/out");
        let out_src = HttpRangeSource::open_with_resolver(
            test_agent(),
            Box::new(move || {
                Ok(ResolvedStream {
                    itag: Some(140),
                    url: out_url.clone(),
                    headers: vec![],
                    content_length: None,
                    expires_at: None,
                })
            }),
            CancelSignal::Flag(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
        let outgoing = FileSource::from_media_source(Box::new(out_src), Some("wav")).unwrap();

        let in_url = format!("http://{addr}/in");
        let in_src = HttpRangeSource::open_with_resolver(
            test_agent(),
            Box::new(move || {
                Ok(ResolvedStream {
                    itag: Some(140),
                    url: in_url.clone(),
                    headers: vec![],
                    content_length: None,
                    expires_at: None,
                })
            }),
            CancelSignal::Flag(Arc::new(AtomicBool::new(false))),
        )
        .unwrap();
        let in_file = FileSource::from_media_source(Box::new(in_src), Some("wav")).unwrap();
        let deck = Deck {
            id: 2,
            spec: SourceSpec::YouTube { video_id: "in".into() },
            src: Box::new(in_file),
            buf: VecDeque::new(),
            frac: 0.0,
            consumed: 0.0,
            base_ms: 0,
            ended: false,
            gain: 1.0,
            tempo_map: None,
        };

        let mut plan = prepare_sources(outgoing, deck, || false).expect("beat-matched YouTube plan");
        assert!((10_000..=15_000).contains(&plan.start_ms), "start_ms={}", plan.start_ms);
        assert!(plan.deck.tempo_map.is_some());

        // Drain the pre-rendered intro and pull past 12s to verify HTTP range streaming continues smoothly.
        let mut pcm = [0.0f32; 2048];
        let mut total_frames = 0usize;
        while total_frames < (RATE as usize) * 14 {
            let n = plan.deck.pull(&mut pcm, RATE as f64);
            assert!(n > 0, "stalled at frame {total_frames}");
            total_frames += n;
        }

        stop.store(true, Ordering::Relaxed);
        let _ = server.join();
    }

    #[test]
    fn automix_prepare_cancel_signal_disarms_on_promotion() {
        let load_epoch = Arc::new(AtomicU64::new(1));
        let prepare_epoch = Arc::new(AtomicU64::new(3));
        let preparing = Arc::new(AtomicBool::new(true));
        let signal = CancelSignal::Prepare {
            load_epoch: load_epoch.clone(),
            expected_epoch: 1,
            prepare_epoch: prepare_epoch.clone(),
            expected_prepare: 3,
            preparing: preparing.clone(),
        };
        assert!(!signal.is_cancelled());

        // While preparing, bumping prepare_epoch cancels immediately.
        prepare_epoch.store(4, Ordering::Release);
        assert!(signal.is_cancelled());

        // Once disarmed (promoted deck), future prepare_epoch bumps must not cancel it,
        // while bumping load_epoch still cancels immediately.
        prepare_epoch.store(3, Ordering::Release);
        preparing.store(false, Ordering::Release);
        prepare_epoch.store(4, Ordering::Release);
        assert!(!signal.is_cancelled());
        load_epoch.store(2, Ordering::Release);
        assert!(signal.is_cancelled());
    }
}
