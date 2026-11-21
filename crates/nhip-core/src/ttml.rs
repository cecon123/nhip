//! Media-time TTML and the Apple/AMLL convention for absolute word timings.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use roxmltree::{Document, Node, ParsingOptions};

use crate::lrc::{LyricLine, Lyrics, TimedSpan};

const TTML: &str = "http://www.w3.org/ns/ttml";
const METADATA: &str = "http://www.w3.org/ns/ttml#metadata";
const PARAMETER: &str = "http://www.w3.org/ns/ttml#parameter";
const STYLING: &str = "http://www.w3.org/ns/ttml#styling";
const APPLE_NAMESPACES: [&str; 2] = [
    "http://music.apple.com/lyric-ttml-internal",
    "http://itunes.apple.com/lyric-ttml-extensions",
];
const MAX_BYTES: usize = 512 * 1024;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub fn read_text(path: &Path) -> io::Result<String> {
    let mut xml = String::new();
    File::open(path)?
        .take(MAX_BYTES as u64 + 1)
        .read_to_string(&mut xml)?;
    if xml.len() > MAX_BYTES {
        return Err(invalid("TTML exceeds 512 KiB"));
    }
    Ok(xml)
}

pub fn parse(xml: &str) -> io::Result<Lyrics> {
    let options = ParsingOptions {
        nodes_limit: 24_000,
        ..Default::default()
    };
    let doc = Document::parse_with_options(xml.trim_start_matches('\u{feff}'), options)
        .map_err(|error| invalid(error.to_string()))?;
    let root = doc.root_element();
    let namespace = root.tag_name().namespace();
    if !root.has_tag_name("tt") || namespace.is_some_and(|ns| ns != TTML) {
        return Err(invalid("Expected a TTML document"));
    }
    if root
        .attribute((PARAMETER, "timeBase"))
        .is_some_and(|value| value != "media")
    {
        return Err(invalid("Expected TTML media time"));
    }
    let apple = root
        .namespaces()
        .any(|ns| APPLE_NAMESPACES.contains(&ns.uri()));
    let line_only = APPLE_NAMESPACES
        .iter()
        .any(|&ns| root.attribute((ns, "timing")) == Some("Line"));
    let body = root
        .children()
        .find(|node| node.has_tag_name("body") && node.tag_name().namespace() == namespace)
        .ok_or_else(|| invalid("TTML has no body"))?;
    let mut lines = Vec::new();
    for paragraph in body
        .descendants()
        .filter(|node| node.has_tag_name("p") && node.tag_name().namespace() == namespace)
    {
        let line = read_line(paragraph, apple, line_only)?;
        if !line.text.is_empty() {
            lines.push(line);
        }
    }
    if lines.is_empty() {
        return Err(invalid("TTML has no lyrics"));
    }
    let mut writers = Vec::new();
    for node in root
        .descendants()
        .filter(|n| n.has_tag_name("songwriter"))
    {
        if let Some(name) = node.text().map(str::trim).filter(|s| !s.is_empty()) {
            if !writers.iter().any(|w| w == name) {
                writers.push(name.to_owned());
            }
        }
    }
    let songwriter = (!writers.is_empty()).then(|| writers.join(", "));
    let provider = Some(if apple { "Apple Music" } else { "TTML" });
    lines.sort_by_key(|line| line.time_ms.unwrap_or(u64::MAX));
    let synced = lines[0].time_ms.is_some();
    Ok(Lyrics {
        lines,
        synced,
        songwriter,
        provider,
        source: Some(if apple { "apple_music" } else { "ttml" }.to_string()),
        attribution: None,
        track_ids: None,
    })
}

#[derive(Default)]
struct Interval {
    start: Option<u64>,
    end: Option<u64>,
}

fn timing(node: Node<'_, '_>, absolute: bool) -> io::Result<Interval> {
    let mut parents: Vec<_> = std::iter::successors(Some(node), Node::parent)
        .take(65)
        .collect();
    if parents.len() > 64 {
        return Err(invalid("TTML elements are nested too deeply"));
    }
    let mut interval = Interval::default();
    while let Some(node) = parents.pop() {
        // shortcut: sequential/frame timing needs a full TTML scheduler if those profiles are imported.
        if node
            .attribute("timeContainer")
            .is_some_and(|value| value != "par")
        {
            return Err(invalid("Unsupported TTML time container"));
        }
        let base = if absolute {
            0
        } else {
            interval.start.unwrap_or_default()
        };
        let begin = node
            .attribute("begin")
            .map(|value| offset(base, value, absolute))
            .transpose()?;
        let end = node
            .attribute("end")
            .map(|value| offset(base, value, absolute))
            .transpose()?;
        let start = begin.or(interval.start);
        let duration_end = node
            .attribute("dur")
            .map(|value| offset(start.unwrap_or_default(), value, absolute))
            .transpose()?;
        let end = [interval.end, end, duration_end]
            .into_iter()
            .flatten()
            .min();
        if end.is_some_and(|end| end <= start.unwrap_or_default()) {
            return Err(invalid("TTML end must follow begin"));
        }
        interval = Interval { start, end };
    }
    Ok(interval)
}

fn offset(base: u64, value: &str, absolute: bool) -> io::Result<u64> {
    milliseconds(value.trim(), absolute)
        .and_then(|ms| base.checked_add(ms))
        .ok_or_else(|| invalid("Unsupported TTML timestamp"))
}

fn milliseconds(value: &str, absolute: bool) -> Option<u64> {
    let seconds = if let Some((clock, sec)) = value.rsplit_once(':') {
        let (hours, minutes) = match clock.split_once(':') {
            Some((h, m)) => (h.parse::<u64>().ok()?, m.parse::<u64>().ok()?),
            None => (0, clock.parse::<u64>().ok()?),
        };
        let sec = sec.parse::<f64>().ok()?;
        if !(0.0..60.0).contains(&sec) || (clock.contains(':') && minutes >= 60) {
            return None;
        }
        hours as f64 * 3600.0 + minutes as f64 * 60.0 + sec
    } else if let Some(ms) = value.strip_suffix("ms") {
        ms.parse::<f64>().ok()? / 1000.0
    } else if let Some(sec) = value.strip_suffix('s') {
        sec.parse().ok()?
    } else if let Some(min) = value.strip_suffix('m') {
        min.parse::<f64>().ok()? * 60.0
    } else if let Some(hours) = value.strip_suffix('h') {
        hours.parse::<f64>().ok()? * 3600.0
    } else if absolute {
        value.parse().ok()?
    } else {
        return None;
    };
    let ms = (seconds * 1000.0).round();
    (0.0..u64::MAX as f64).contains(&ms).then_some(ms as u64)
}

fn read_line(paragraph: Node<'_, '_>, absolute: bool, line_only: bool) -> io::Result<LyricLine> {
    let interval = timing(paragraph, absolute)?;
    let mut text = String::new();
    let mut spans = Vec::new();
    for node in paragraph
        .descendants()
        .filter(|node| node.is_text() || node.has_tag_name("br"))
    {
        let ancestors =
            std::iter::successors(Some(node), Node::parent).take_while(|&node| node != paragraph);
        if ancestors.clone().nth(64).is_some() {
            return Err(invalid("TTML elements are nested too deeply"));
        }
        if ancestors.clone().any(|node| {
            matches!(
                node.attribute((METADATA, "role")),
                Some("x-bg" | "x-translation" | "x-roman" | "x-romanization")
            ) || node.attribute((STYLING, "ruby")) == Some("text")
        }) {
            continue;
        }
        if node.has_tag_name("br") {
            text.truncate(text.trim_end_matches(' ').len());
            if !text.is_empty() {
                text.push('\n');
            }
            continue;
        }
        let offset = text.len();
        for ch in node.text().unwrap_or_default().chars() {
            if ch.is_ascii_whitespace() {
                if !text.is_empty() && !text.ends_with([' ', '\n']) {
                    text.push(' ');
                }
            } else {
                text.push(ch);
            }
        }
        let fragment = text[offset..].trim();
        if line_only || fragment.is_empty() {
            continue;
        }
        let Some(span) = ancestors
            .filter(|node| node.has_tag_name("span"))
            .find(|node| {
                node.attribute("begin").is_some()
                    && (node.attribute("end").is_some() || node.attribute("dur").is_some())
            })
        else {
            continue;
        };
        let span = timing(span, absolute)?;
        if let (Some(start_ms), Some(end_ms)) = (span.start, span.end) {
            let end = text.trim_end().len();
            spans.push(TimedSpan {
                bytes: end - fragment.len()..end,
                start_ms,
                end_ms,
            });
        }
    }
    text.truncate(text.trim_end().len());
    let time_ms = interval
        .start
        .or_else(|| spans.iter().map(|span| span.start_ms).min())
        .or_else(|| interval.end.map(|_| 0));
    let right_align = paragraph
        .attribute((METADATA, "agent"))
        .is_some_and(|a| !a.is_empty() && !a.eq_ignore_ascii_case("v1") && !a.eq_ignore_ascii_case("v0"));
    Ok(LyricLine {
        text,
        time_ms,
        end_ms: interval.end,
        spans,
        right_align,
        background: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_word_intervals_and_utf8_ranges_are_preserved() {
        let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml"
            xmlns:m="http://www.w3.org/ns/ttml#metadata"
            xmlns:i="http://music.apple.com/lyric-ttml-internal" i:timing="Word">
            <body><div begin="00:10.000"><p begin="00:10.000" end="00:12.000">
              <span begin="00:10.000" end="00:10.300">Tiếng</span>
              <span begin="00:10.400" end="00:11.500">Việt &amp; nhạc</span>!
              <span m:role="x-bg">Hát bè</span>
            </p></div></body></tt>"#;
        let lyrics = parse(xml).unwrap();
        let line = &lyrics.lines[0];
        assert_eq!(line.text, "Tiếng Việt & nhạc!");
        assert_eq!(line.time_ms, Some(10_000));
        assert_eq!(&line.text[line.spans[1].bytes.clone()], "Việt & nhạc");
        assert_eq!(
            (line.spans[1].start_ms, line.spans[1].end_ms),
            (10_400, 11_500)
        );
        assert_eq!(lyrics.current(9_999), None);
        assert_eq!(lyrics.current(10_400), Some(0));
        assert_eq!(lyrics.current(12_000), None);
        assert!(lyrics.ended(12_000));
    }

    #[test]
    fn media_time_is_relative_and_untimed_spans_stay_untimed() {
        let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div begin="10s">
            <p begin="2s" dur="3s"><span begin="250ms" dur="500ms">Mưa</span> <span>rơi</span><br/>nhẹ.</p>
            <p begin="6s" end="7s">Chờ.</p></div></body></tt>"#;
        let lyrics = parse(xml).unwrap();
        let line = &lyrics.lines[0];
        assert_eq!(line.text, "Mưa rơi\nnhẹ.");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(
            (line.spans[0].start_ms, line.spans[0].end_ms),
            (12_250, 12_750)
        );
        assert_eq!(lyrics.current(15_000), None);
        assert_eq!(lyrics.current(16_000), Some(1));
    }

    #[test]
    fn rejects_invalid_timing_and_document_types() {
        for xml in [
            "<tt><body><div><p begin='2s' end='1s'>A</p></div></body></tt>",
            "<!DOCTYPE tt [<!ENTITY content 'A'>]><tt><body><p>&content;</p></body></tt>",
            "<tt><body><div timeContainer='seq'><p>A</p></div></body></tt>",
        ] {
            assert!(parse(xml).is_err());
        }
    }
}
