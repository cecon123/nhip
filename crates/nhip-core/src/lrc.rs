//! Lyrics shared by LRC, plain text, TTML, LyricsPlus, and Spicy Lyrics API.

use std::ops::Range;

use serde::{Deserialize, Serialize};

/// Verified from Spicy Lyrics `src/utils/Lyrics/lyrics.ts:12` (`getLyricsBetweenShow()` = 3s).
pub const INTERLUDE_GAP_MS: u64 = 3_000;
/// Minimum post-vocal gap required to infer an instrumental interlude on plain LRC / line-only tracks.
pub const LRC_INFERRED_INTERLUDE_GAP_MS: u64 = 3_000;
/// Conservative outro gap threshold so short track fadeouts do not flash spurious dots.
pub const OUTRO_INTERLUDE_GAP_MS: u64 = 5_000;

/// Computes a song's typical singing cadence in milliseconds per character from consecutive vocal
/// line pairs, so fast pop/rap tracks infer shorter instrumental breaks (~5-6s) while slow ballads
/// (~10s long lines) never emit false-positive interlude dots.
fn song_cadence_ms_per_char(lines: &[LyricLine]) -> f64 {
    let mut rates: Vec<f64> = Vec::new();
    let mut prev: Option<(u64, usize)> = None;
    for line in lines {
        if line.background || is_instrumental_text(&line.text) {
            continue;
        }
        let Some(start) = line.time_ms else { continue };
        let chars = line.text.trim().chars().count();
        if let Some((p_start, p_chars)) = prev {
            let delta = start.saturating_sub(p_start);
            if (1_200..13_500).contains(&delta) && p_chars >= 6 {
                rates.push(delta as f64 / (p_chars.max(14) as f64));
            }
        }
        prev = Some((start, chars));
    }
    if rates.len() < 3 {
        return 110.0;
    }
    rates.sort_by(|a, b| a.total_cmp(b));
    rates[rates.len() / 2].clamp(62.0, 145.0)
}

/// Returns `(end_ms, inferred)` for a vocal line.
/// - Syllable/word-synced lines (`!spans.is_empty()`) use their exact `end_ms` / last span end.
/// - Plain LRC / line-synced tracks (`spans.is_empty()`) estimate vocal duration using the song's
///   adaptive cadence; this also unwraps upstream Musixmatch/LyricsPlus `Line`-only payloads that
///   stretch `end_ms` all the way to `next_start` across long instrumental breaks.
fn effective_line_end_ms(
    line: &LyricLine,
    next_start_ms: Option<u64>,
    ms_per_char: f64,
) -> Option<(u64, bool)> {
    if let Some(last) = line.spans.last() {
        let end = line.end_ms.map_or(last.end_ms, |e| e.max(last.end_ms));
        return Some((end, false));
    }
    let start = line.time_ms?;
    let chars = line.text.trim().chars().count() as f64;
    let est_dur = (1_100.0 + chars * ms_per_char).clamp(1_900.0, 7_400.0).round() as u64;
    if let Some(end) = line.end_ms {
        let stretched_to_next = next_start_ms.is_some_and(|ns| ns.abs_diff(end) <= 300)
            && end.saturating_sub(start) >= est_dur.saturating_add(LRC_INFERRED_INTERLUDE_GAP_MS);
        if stretched_to_next {
            return Some((start.saturating_add(est_dur), true));
        }
        return Some((end, false));
    }
    Some((start.saturating_add(est_dur), true))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedSpan {
    pub bytes: Range<usize>,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricLine {
    pub time_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub text: String,
    #[serde(default)]
    pub spans: Vec<TimedSpan>,
    /// Secondary / opposite vocalist (`OppositeAligned` in Spicy Lyrics / `v2` in LyricsPlus / Apple TTML).
    #[serde(default)]
    pub right_align: bool,
    /// Background vocal / ad-lib (`Background` in Spicy Lyrics / `isBackground` in LyricsPlus), rendered smaller below parent line.
    #[serde(default)]
    pub background: bool,
}

impl LyricLine {
    fn new(time_ms: Option<u64>, text: &str) -> Self {
        Self {
            time_ms,
            end_ms: None,
            text: text.to_owned(),
            spans: Vec::new(),
            right_align: false,
            background: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsContributor {
    pub id: String,
    pub username: String,
    pub url: String,
    #[serde(default)]
    pub avatar: Option<String>,
}

/// Community attribution required by Spicy Lyrics Terms of Service Section 6 when `source == "spicy_lyrics"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsAttribution {
    pub uploader: LyricsContributor,
    #[serde(default)]
    pub maker: Option<LyricsContributor>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LyricsTrackIds {
    #[serde(default)]
    pub spotify: Option<String>,
    #[serde(default)]
    pub isrc: Option<String>,
    #[serde(default)]
    pub apple_music: Option<String>,
    #[serde(default)]
    pub tidal: Option<String>,
    #[serde(default)]
    pub youtube: Option<String>,
}

/// Virtual interlude generated from qualifying timeline gaps or explicit instrumental markers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InterludeEvent {
    pub start_ms: u64,
    pub end_ms: u64,
    pub right_align: bool,
}

/// Unified lyrics timeline item participating in layout, scrolling, and focus distance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LyricsEvent {
    VocalLine(usize),
    Interlude(InterludeEvent),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
    #[serde(default)]
    pub songwriter: Option<String>,
    #[serde(skip)]
    pub provider: Option<&'static str>,
    /// Canonical upstream catalogue identifier (`"spicy_lyrics"`, `"apple_music"`, `"spotify"`, `"lrclib"`, etc.).
    #[serde(default)]
    pub source: Option<String>,
    /// Present ONLY when `source == "spicy_lyrics"`. Absent for commercial catalogues.
    #[serde(default)]
    pub attribution: Option<LyricsAttribution>,
    #[serde(default)]
    pub track_ids: Option<LyricsTrackIds>,
}

/// Maps a canonical source key back to its static display provider name (also used on cache restore).
pub fn provider_label_for_source(source: &str) -> &'static str {
    match source.trim().to_ascii_lowercase().as_str() {
        "spicy_lyrics" | "spicylyrics" => "Spicy Lyrics",
        "apple_music" | "apple" | "apple music" => "Apple Music",
        "spotify" => "Spotify",
        "musixmatch" | "musixmatch-word" => "Musixmatch",
        "lrclib" => "LRCLIB",
        "youtube" | "youtube music" => "YouTube Music",
        "ttml" => "TTML",
        _ => "LyricsPlus",
    }
}

pub fn is_instrumental_text(text: &str) -> bool {
    let t = text.trim();
    if t.is_empty() {
        return true;
    }
    if t.chars().all(|c| {
        c.is_whitespace()
            || matches!(
                c,
                '♪' | '♫' | '♬' | '♩' | '🎵' | '🎶' | '.' | '…' | '•' | '·' | '・' | '~' | '-' | '—' | '–'
            )
    }) {
        return true;
    }
    let lower = t
        .trim_matches(|c| matches!(c, '(' | ')' | '[' | ']' | '（' | '）' | '【' | '】' | '♪' | '♫' | ' '))
        .to_lowercase();
    matches!(
        lower.as_str(),
        "instrumental" | "music" | "interlude" | "nhạc dạo" | "nhac dao" | "nhạc nền" | "dạo nhạc"
    )
}

impl Lyrics {
    pub fn parse(text: &str) -> Self {
        let mut timed: Vec<LyricLine> = Vec::new();
        let mut plain: Vec<LyricLine> = Vec::new();
        for raw in text.lines() {
            let mut rest = raw.trim();
            let mut stamps = Vec::new();
            while let Some(after) = rest.strip_prefix('[') {
                let Some(end) = after.find(']') else { break };
                match parse_stamp(&after[..end]) {
                    Some(t) => stamps.push(t),
                    None if after[..end].contains(':') && stamps.is_empty() => {
                        rest = "";
                        break;
                    }
                    None => break,
                }
                rest = after[end + 1..].trim_start();
            }
            let primary = rest
                .split_once('^')
                .map(|(head, _)| head.trim_end())
                .unwrap_or(rest);
            if stamps.is_empty() {
                if !primary.is_empty() || !plain.is_empty() {
                    plain.push(LyricLine::new(None, primary));
                }
            } else {
                let (clean_text, spans) = parse_inline_spans(primary);
                for t in stamps {
                    let mut line = LyricLine::new(Some(t), &clean_text);
                    line.spans = spans.clone();
                    timed.push(line);
                }
            }
        }
        if timed.is_empty() {
            while plain.last().is_some_and(|l| l.text.is_empty()) {
                plain.pop();
            }
            return Self {
                lines: plain,
                synced: false,
                songwriter: None,
                provider: None,
                source: None,
                attribution: None,
                track_ids: None,
            };
        }
        timed.sort_by_key(|l| l.time_ms);
        for i in 0..timed.len() {
            let next_start = timed.get(i + 1).and_then(|n| n.time_ms);
            if let Some(last_span) = timed[i].spans.last_mut() {
                if last_span.end_ms <= last_span.start_ms {
                    last_span.end_ms = next_start
                        .filter(|&ns| ns > last_span.start_ms)
                        .map(|ns| ns.min(last_span.start_ms + 3_500))
                        .unwrap_or(last_span.start_ms + 900);
                }
            }
        }
        Self {
            lines: timed,
            synced: true,
            songwriter: None,
            provider: None,
            source: None,
            attribution: None,
            track_ids: None,
        }
    }

    /// Rehydrates the static `provider` label from `source` after deserializing from disk cache.
    pub fn restore_provider_label(&mut self) {
        if self.provider.is_none() {
            if let Some(ref src) = self.source {
                self.provider = Some(provider_label_for_source(src));
            }
        }
    }

    /// Builds the unified timeline of vocal lines and virtual `InterludeEvent`s from qualifying
    /// gaps (`>= INTERLUDE_GAP_MS` = 3000ms, matching Spicy Lyrics `getLyricsBetweenShow()`).
    ///
    /// Rules:
    /// - Static (unsynced) lyrics emit only `VocalLine` events (never interludes).
    /// - Intro gap (`0..first_vocal_ms >= 3000ms`) emits one intro `InterludeEvent` unless an
    ///   explicit instrumental line already precedes it.
    /// - Consecutive explicit instrumental/blank lines collapse into one `InterludeEvent`
    ///   (dropping tiny `< 2200ms` blank LRC spacers).
    /// - Mid-song gaps (`next_main.start_ms - prev_group_end_ms >= 3000ms`, where
    ///   `prev_group_end_ms` accounts for any overlapping trailing background vocals) emit one
    ///   `InterludeEvent` aligned to the upcoming vocal line's `right_align`.
    /// - Outro gap (`track_duration_ms - last_vocal_end_ms >= 5000ms`) emits an outro interlude
    ///   only when valid track duration data is supplied.
    pub fn build_timeline(&self, track_duration_ms: Option<u64>) -> Vec<LyricsEvent> {
        if !self.synced {
            return (0..self.lines.len()).map(LyricsEvent::VocalLine).collect();
        }
        let mut events = Vec::with_capacity(self.lines.len() + 4);
        let ms_per_char = song_cadence_ms_per_char(&self.lines);
        let first_vocal = self
            .lines
            .iter()
            .find(|l| !l.background && !is_instrumental_text(&l.text) && l.time_ms.is_some());

        if let Some(fv) = first_vocal {
            if let Some(first_ms) = fv.time_ms {
                let has_explicit_intro = self.lines.iter().any(|l| {
                    is_instrumental_text(&l.text) && l.time_ms.is_some_and(|t| t < first_ms)
                });
                if !has_explicit_intro && first_ms >= INTERLUDE_GAP_MS {
                    events.push(LyricsEvent::Interlude(InterludeEvent {
                        start_ms: 0,
                        end_ms: first_ms,
                        right_align: fv.right_align,
                    }));
                }
            }
        }

        let mut i = 0usize;
        while i < self.lines.len() {
            let line = &self.lines[i];
            if is_instrumental_text(&line.text) {
                if let Some(start_ms) = line.time_ms {
                let mut run_end = i;
                let mut explicit_symbol = !line.text.trim().is_empty();
                while let Some(next) = self.lines.get(run_end + 1) {
                    if is_instrumental_text(&next.text) && next.time_ms.is_some() {
                        explicit_symbol |= !next.text.trim().is_empty();
                        run_end += 1;
                    } else {
                        break;
                    }
                }
                let next_vocal = self.lines.get(run_end + 1);
                let next_vocal_ms = next_vocal.and_then(|n| n.time_ms);
                let end_ms = self.lines[run_end]
                    .end_ms
                    .or(next_vocal_ms)
                    .unwrap_or(start_ms + 4_000)
                    .max(start_ms + 500);
                if explicit_symbol || end_ms.saturating_sub(start_ms) >= 2_200 {
                    // Avoid pushing a duplicate overlapping interlude immediately after another.
                    let duplicate = matches!(
                        events.last(),
                        Some(LyricsEvent::Interlude(prev)) if prev.end_ms >= start_ms
                    );
                    if !duplicate {
                        events.push(LyricsEvent::Interlude(InterludeEvent {
                            start_ms,
                            end_ms,
                            right_align: next_vocal.is_some_and(|n| n.right_align),
                        }));
                    }
                }
                i = run_end + 1;
                continue;
                }
            }

            events.push(LyricsEvent::VocalLine(i));

            // If this is a main vocal line, also emit its immediately following background vocals
            // before evaluating any interlude gap to the next main vocal line.
            if !line.background {
                let mut j = i + 1;
                let mut has_explicit_gap = false;
                let mut bg_indices = Vec::new();
                while let Some(cand) = self.lines.get(j) {
                    if is_instrumental_text(&cand.text) && cand.time_ms.is_some() {
                        has_explicit_gap = true;
                        break;
                    }
                    if cand.background {
                        events.push(LyricsEvent::VocalLine(j));
                        bg_indices.push(j);
                        j += 1;
                    } else {
                        break;
                    }
                }
                let next_main_start = (!has_explicit_gap)
                    .then(|| self.lines.get(j).and_then(|n| n.time_ms))
                    .flatten();
                let mut group_end = effective_line_end_ms(line, next_main_start, ms_per_char);
                for bg_idx in bg_indices {
                    if let Some((bg_end, bg_inf)) =
                        effective_line_end_ms(&self.lines[bg_idx], next_main_start, ms_per_char)
                    {
                        group_end = Some(match group_end {
                            Some((cur_end, cur_inf)) => {
                                if bg_end > cur_end {
                                    (bg_end, bg_inf)
                                } else {
                                    (cur_end, cur_inf)
                                }
                            }
                            None => (bg_end, bg_inf),
                        });
                    }
                }
                if !has_explicit_gap {
                    if let (Some((end_ms, inferred)), Some(next_main)) =
                        (group_end, self.lines.get(j))
                    {
                        if let Some(next_start) = next_main.time_ms {
                            let min_gap = if inferred {
                                LRC_INFERRED_INTERLUDE_GAP_MS
                            } else {
                                INTERLUDE_GAP_MS
                            };
                            if next_start >= end_ms.saturating_add(min_gap) {
                                events.push(LyricsEvent::Interlude(InterludeEvent {
                                    start_ms: end_ms,
                                    end_ms: next_start,
                                    right_align: next_main.right_align,
                                }));
                            }
                        }
                    }
                }
                i = j;
            } else {
                i += 1;
            }
        }

        // Optional outro interlude only when track duration is known and leaves a long instrumental tail.
        if let Some(dur_ms) = track_duration_ms.filter(|&d| d > 0) {
            let last_vocal_end = self
                .lines
                .iter()
                .filter(|l| !is_instrumental_text(&l.text))
                .filter_map(|l| effective_line_end_ms(l, None, ms_per_char))
                .map(|(end, _)| end)
                .max();
            if let Some(last_end) = last_vocal_end {
                let already_has_outro = matches!(
                    events.last(),
                    Some(LyricsEvent::Interlude(ev)) if ev.start_ms >= last_end
                );
                if !already_has_outro && dur_ms >= last_end.saturating_add(OUTRO_INTERLUDE_GAP_MS) {
                    events.push(LyricsEvent::Interlude(InterludeEvent {
                        start_ms: last_end,
                        end_ms: dur_ms,
                        right_align: false,
                    }));
                }
            }
        }

        events
    }

    /// The line being sung at `position_ms`.
    pub fn current(&self, position_ms: u64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        // Prefer the active main vocal line so overlapping background ad-libs don't hijack scroll focus.
        self.lines
            .iter()
            .rposition(|line| {
                !line.background
                    && line.time_ms.is_some_and(|start| start <= position_ms)
                    && line.end_ms.is_none_or(|end| position_ms < end)
            })
            .or_else(|| {
                self.lines.iter().rposition(|line| {
                    line.time_ms.is_some_and(|start| start <= position_ms)
                        && line.end_ms.is_none_or(|end| position_ms < end)
                })
            })
    }

    /// Scroll/display focus line at `position_ms`: stays smoothly anchored on the most recently
    /// sung main line across short inter-line breath pauses (never jumps back to `None`/top mid-song).
    pub fn display_current(&self, position_ms: u64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        if let Some(exact) = self.current(position_ms) {
            return Some(exact);
        }
        if self.ended(position_ms) {
            return None;
        }
        // Bridge inter-line gaps between two lines once singing has started.
        let last_started = self
            .lines
            .iter()
            .rposition(|line| !line.background && line.time_ms.is_some_and(|start| start <= position_ms));
        if let Some(idx) = last_started {
            return Some(idx);
        }
        None
    }

    /// Whether a specific line (including an overlapping background vocal) is actively singing right now.
    pub fn is_line_singing(&self, index: usize, position_ms: u64) -> bool {
        if !self.synced {
            return false;
        }
        let Some(line) = self.lines.get(index) else {
            return false;
        };
        let Some(start) = line.time_ms else {
            return false;
        };
        if position_ms < start {
            return false;
        }
        if let Some(end) = line.end_ms {
            position_ms < end
        } else {
            self.current(position_ms) == Some(index)
        }
    }

    pub fn ended(&self, position_ms: u64) -> bool {
        self.synced
            && self
                .lines
                .iter()
                .all(|line| line.end_ms.is_some_and(|end| end <= position_ms))
    }
}

fn parse_inline_spans(raw: &str) -> (String, Vec<TimedSpan>) {
    if !raw.contains('<') {
        return (raw.to_owned(), Vec::new());
    }
    let mut out = String::with_capacity(raw.len());
    let mut marks: Vec<(usize, u64)> = Vec::new();
    let mut rest = raw;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let after_lt = &rest[lt + 1..];
        if let Some(gt) = after_lt.find('>') {
            if let Some(ms) = parse_stamp(&after_lt[..gt]) {
                marks.push((out.len(), ms));
                rest = &after_lt[gt + 1..];
                continue;
            }
        }
        out.push('<');
        rest = after_lt;
    }
    out.push_str(rest);
    if marks.is_empty() {
        return (raw.to_owned(), Vec::new());
    }
    let mut spans = Vec::with_capacity(marks.len());
    for (idx, &(byte_start, start_ms)) in marks.iter().enumerate() {
        let (byte_end, end_ms) = marks
            .get(idx + 1)
            .copied()
            .unwrap_or((out.len(), start_ms));
        let trimmed_end = out[byte_start..byte_end].trim_end().len() + byte_start;
        if trimmed_end > byte_start {
            spans.push(TimedSpan {
                bytes: byte_start..trimmed_end,
                start_ms,
                end_ms: end_ms.max(start_ms),
            });
        }
    }
    (out, spans)
}

fn parse_stamp(s: &str) -> Option<u64> {
    let (min, rest) = s.split_once(':')?;
    let min: u64 = min.trim().parse().ok()?;
    let secs: f64 = rest.trim().replace(':', ".").parse().ok()?;
    Some(min * 60_000 + (secs * 1000.0) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_timed_lines_in_order() {
        let l = Lyrics::parse("[ar:Someone]\n[00:12.50]Hai\n[00:05.00]Mot\n");
        assert!(l.synced);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].text, "Mot");
        assert_eq!(l.lines[1].time_ms, Some(12_500));
    }

    #[test]
    fn one_line_with_several_times_repeats() {
        let l = Lyrics::parse("[00:01.00][00:03.00]Lap");
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.current(3_500), Some(1));
        assert_eq!(l.current(500), None);
    }

    #[test]
    fn plain_text_is_not_synced() {
        let l = Lyrics::parse("Dong mot\n\nDong hai\n");
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 3);
    }

    #[test]
    fn enhanced_lrc_angle_stamps_build_word_spans() {
        let l = Lyrics::parse("[00:10.00]<00:10.00>Chẳng <00:10.60>phải <00:11.20>phép màu\n[00:13.00]Sao");
        assert!(l.synced);
        assert_eq!(l.lines[0].text, "Chẳng phải phép màu");
        assert_eq!(l.lines[0].spans.len(), 3);
        assert_eq!(&l.lines[0].text[l.lines[0].spans[0].bytes.clone()], "Chẳng");
        assert_eq!(l.lines[0].spans[0].start_ms, 10_000);
        assert_eq!(l.lines[0].spans[0].end_ms, 10_600);
        assert_eq!(l.lines[0].spans[2].end_ms, 13_000);
    }

    #[test]
    fn strips_inline_caret_translation_suffix() {
        let l = Lyrics::parse("[00:44.00]Mùa đông ghé tới^Winter has arrived\n[00:49.00]Bút Chì Bạc ^ Silver Pencil");
        assert!(l.synced);
        assert_eq!(l.lines.len(), 2);
        assert_eq!(l.lines[0].text, "Mùa đông ghé tới");
        assert_eq!(l.lines[1].text, "Bút Chì Bạc");
    }

    #[test]
    fn build_timeline_generates_intro_mid_outro_interludes_without_duplicates_or_bg_splits() {
        let mk = |start: u64, end: u64, text: &str, bg: bool, right: bool| LyricLine {
            time_ms: Some(start),
            end_ms: Some(end),
            text: text.to_string(),
            spans: Vec::new(),
            right_align: right,
            background: bg,
        };
        let lyrics = Lyrics {
            lines: vec![
                // Intro gap 0..4500 (>= 3000) -> intro interlude right_align=false
                mk(4_500, 7_000, "Line 1", false, false),
                // Background vocal extends group end to 9_200, so gap to Line 2 (11_500) is only 2_300 (< 3000) -> NO interlude!
                mk(6_800, 9_200, "bg adlib", true, false),
                mk(11_500, 14_000, "Line 2", false, false),
                // Gap 14_000..18_500 = 4_500 (>= 3000) -> mid interlude inheriting right_align=true from Line 3!
                mk(18_500, 21_000, "Line 3 duet", false, true),
                // Consecutive explicit instrumental markers collapse to a single interlude
                mk(21_200, 24_000, "", false, false),
                mk(24_000, 27_000, "♪", false, false),
                mk(27_500, 30_000, "Final line", false, false),
            ],
            synced: true,
            songwriter: None,
            provider: None,
            source: None,
            attribution: None,
            track_ids: None,
        };

        // Without track duration: no outro interlude
        let tl_no_dur = lyrics.build_timeline(None);
        assert_eq!(
            tl_no_dur,
            vec![
                LyricsEvent::Interlude(InterludeEvent { start_ms: 0, end_ms: 4_500, right_align: false }),
                LyricsEvent::VocalLine(0),
                LyricsEvent::VocalLine(1),
                LyricsEvent::VocalLine(2),
                LyricsEvent::Interlude(InterludeEvent { start_ms: 14_000, end_ms: 18_500, right_align: true }),
                LyricsEvent::VocalLine(3),
                LyricsEvent::Interlude(InterludeEvent { start_ms: 21_200, end_ms: 27_000, right_align: false }),
                LyricsEvent::VocalLine(6),
            ]
        );

        // With 38s track duration (8s tail >= 5s): outro interlude appended
        let tl_with_outro = lyrics.build_timeline(Some(38_000));
        assert_eq!(
            tl_with_outro.last(),
            Some(&LyricsEvent::Interlude(InterludeEvent {
                start_ms: 30_000,
                end_ms: 38_000,
                right_align: false,
            }))
        );

        // Static lyrics never produce interludes
        let static_lyrics = Lyrics::parse("Plain line 1\nPlain line 2");
        assert_eq!(
            static_lyrics.build_timeline(Some(120_000)),
            vec![LyricsEvent::VocalLine(0), LyricsEvent::VocalLine(1)]
        );
    }

    #[test]
    fn build_timeline_infers_interludes_for_plain_lrc_instrumental_breaks_without_false_positives() {
        let lrc = Lyrics::parse(concat!(
            "[00:02.19]One, two, three, yah\n",
            "[00:24.44]Dường như em đã quen khi anh chẳng ghé qua (anh chẳng ghé qua)\n",
            "[00:34.01]Cùng thật nhiều lý do nhưng anh chọn nói ra (anh chọn nói ra)\n",
            "[00:41.49]Rằng anh không còn niềm tin vào tình yêu dù cho em đang bên cạnh\n",
            "[00:50.85]Người đàn ông mà em yêu dù giông bão em vẫn đứng ngay bên cạnh\n",
            "[01:00.92]Em sẽ không ở mãi nơi đây đợi anh mỗi ngày\n",
            "[01:26.80]Dường như anh đã quen với những ngày có em (những ngày có em)\n",
        ));
        let tl = lrc.build_timeline(Some(110_000));
        // Must infer interludes for the real 22s intro break, the 26s mid-song break, and the outro tail,
        // while NEVER inserting spurious interludes between consecutive long 9.5s–10.0s ballad lines.
        let interludes: Vec<InterludeEvent> = tl
            .iter()
            .filter_map(|e| match *e {
                LyricsEvent::Interlude(ev) => Some(ev),
                _ => None,
            })
            .collect();
        assert_eq!(interludes.len(), 3, "expected 3 real instrumental breaks, got {interludes:?}");
        assert_eq!(interludes[0].end_ms, 24_440, "first break must end cleanly when vocal line 1 starts");
        assert!(interludes[0].start_ms > 2_190 && interludes[0].start_ms < 10_000);
        assert_eq!(interludes[1].end_ms, 86_800, "mid-song break must end cleanly when verse 2 starts");
        assert!(interludes[1].start_ms > 60_920 && interludes[1].start_ms < 72_000);
        assert_eq!(interludes[2].end_ms, 110_000, "outro tail interlude must reach track duration");
        // Verify lines 1..=5 sit consecutively with no spurious interlude between them.
        let pos_1 = tl.iter().position(|e| *e == LyricsEvent::VocalLine(1)).unwrap();
        assert_eq!(tl[pos_1 + 1], LyricsEvent::VocalLine(2));
        assert_eq!(tl[pos_1 + 2], LyricsEvent::VocalLine(3));
        assert_eq!(tl[pos_1 + 3], LyricsEvent::VocalLine(4));
        assert_eq!(tl[pos_1 + 4], LyricsEvent::VocalLine(5));
    }

    #[test]
    fn build_timeline_unwraps_stretched_line_only_end_ms_and_recognizes_common_instrumental_markers() {
        let mk = |start: u64, end: u64, text: &str| LyricLine {
            time_ms: Some(start),
            end_ms: Some(end),
            text: text.to_string(),
            spans: Vec::new(),
            right_align: false,
            background: false,
        };
        // Fast pop track where Musixmatch/LyricsPlus Line-only API stretches line 2's end_ms all the way
        // to line 3 (12s gap across an instrumental chorus break) plus explicit "♪ ♪" and "(Instrumental)".
        let lyrics = Lyrics {
            lines: vec![
                mk(5_000, 7_500, "Đêm nay phố mưa rơi"),
                mk(7_500, 10_200, "Anh bước đi một mình"),
                mk(10_200, 22_500, "Chẳng còn ai đợi chờ"),
                mk(22_500, 25_000, "Gió thổi qua hàng cây"),
                mk(25_000, 31_000, "♪ ♪"),
                mk(31_000, 34_000, "Còn lại câu hát cũ"),
                mk(34_000, 40_000, "(Instrumental)"),
                mk(40_000, 43_000, "Hết bài"),
            ],
            synced: true,
            songwriter: None,
            provider: None,
            source: None,
            attribution: None,
            track_ids: None,
        };
        let tl = lyrics.build_timeline(None);
        let interludes: Vec<InterludeEvent> = tl
            .iter()
            .filter_map(|e| match *e {
                LyricsEvent::Interlude(ev) => Some(ev),
                _ => None,
            })
            .collect();
        // Expect 4 interludes: intro (0..5s), stretched break after line 2 (..22.5s), "♪ ♪" (25..31s), "(Instrumental)" (34..40s).
        assert_eq!(interludes.len(), 4, "got {interludes:?}");
        assert_eq!(interludes[0].start_ms, 0);
        assert_eq!(interludes[0].end_ms, 5_000);
        assert!(interludes[1].start_ms > 10_200 && interludes[1].start_ms < 16_000);
        assert_eq!(interludes[1].end_ms, 22_500);
        assert_eq!(interludes[2].start_ms, 25_000);
        assert_eq!(interludes[2].end_ms, 31_000);
        assert_eq!(interludes[3].start_ms, 34_000);
        assert_eq!(interludes[3].end_ms, 40_000);
    }
}
