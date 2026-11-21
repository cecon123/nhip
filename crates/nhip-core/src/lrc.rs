//! Lyrics shared by LRC, plain text, and TTML.

use std::ops::Range;

#[derive(Clone)]
pub struct TimedSpan {
    pub bytes: Range<usize>,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone)]
pub struct LyricLine {
    pub time_ms: Option<u64>,
    pub end_ms: Option<u64>,
    pub text: String,
    pub spans: Vec<TimedSpan>,
    /// Secondary / opposite vocalist (`v2` in LyricsPlus / Apple TTML), right-aligned like Spicy Lyrics.
    pub right_align: bool,
    /// Background vocal / ad-lib (`isBackground: true` in LyricsPlus), rendered smaller below parent line.
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

#[derive(Clone)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
    pub songwriter: Option<String>,
    pub provider: Option<&'static str>,
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
        }
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
}
