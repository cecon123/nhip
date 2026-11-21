//! Lyrics from LRCLIB (lrclib.net), a free and open lyrics database with timed lines. Lyrics are
//! fetched when a song is shown and kept only in memory.

use std::io::Read;
use std::time::Duration;

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde_json::Value;

const BASE: &str = "https://lrclib.net/api";

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Timed lyrics when there are any, plain text otherwise.
fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v[key].as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn pick(v: &Value) -> Option<&str> {
    text(v, "syncedLyrics").or_else(|| text(v, "plainLyrics"))
}

fn get(agent: &reqwest::blocking::Client, url: &str) -> Option<Value> {
    let resp = agent
        .get(url)
        .header("User-Agent", "Nhip/0.1 (desktop music player)")
        .send()
        .ok()?;
    if resp.status().as_u16() != 200 {
        return None;
    }
    let mut text = String::new();
    resp.take(256_000).read_to_string(&mut text).ok()?;
    serde_json::from_str(&text).ok()
}

fn matches_recording(v: &Value, title: &str, artist: &str, album: &str, seconds: f64) -> bool {
    let same = |key, expected: &str| {
        text(v, key).is_some_and(|s| s.to_lowercase() == expected.trim().to_lowercase())
    };
    same("trackName", title)
        && same("artistName", artist)
        && (album.trim().is_empty() || text(v, "albumName").is_none() || same("albumName", album))
        && v["duration"]
            .as_f64()
            .is_some_and(|duration| (duration - seconds).abs() <= 2.0)
}

fn strip_title_suffix(title: &str) -> String {
    let mut out = title.trim();
    for open in ['(', '[', '（', '【'] {
        if let Some(idx) = out.find(open) {
            let head = out[..idx].trim();
            if head.chars().count() >= 2 {
                out = head;
            }
        }
    }
    out.trim().to_owned()
}

fn matches_relaxed_recording(v: &Value, clean_title: &str, primary_artist: &str, seconds: f64) -> bool {
    let t_fold = crate::library::fold(clean_title);
    let a_fold = crate::library::fold(primary_artist);
    let Some(cand_t) = text(v, "trackName").map(crate::library::fold) else {
        return false;
    };
    let Some(cand_a) = text(v, "artistName").map(crate::library::fold) else {
        return false;
    };
    let title_ok = !t_fold.is_empty()
        && (cand_t == t_fold
            || cand_t.starts_with(&t_fold)
            || t_fold.starts_with(&cand_t));
    let artist_ok = !a_fold.is_empty() && (cand_a.contains(&a_fold) || a_fold.contains(&cand_a));
    let dur_ok = v["duration"]
        .as_f64()
        .is_some_and(|duration| (duration - seconds).abs() <= 5.0);
    title_ok && artist_ok && dur_ok
}

/// Search fallback keeps the timing tied to the recording selected in the player.
pub fn fetch(title: &str, artist: &str, album: &str, duration_ms: u64) -> Option<String> {
    if duration_ms == 0 {
        return None;
    }
    let agent = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .ok()?;
    let secs = duration_ms as f64 / 1000.0;
    let mut exact = format!(
        "{BASE}/get?track_name={}&artist_name={}&duration={secs:.0}",
        enc(title),
        enc(artist)
    );
    if !album.trim().is_empty() {
        exact.push_str(&format!("&album_name={}", enc(album)));
    }
    let found = get(&agent, &exact).filter(|v| matches_recording(v, title, artist, album, secs));
    if let Some(synced) = found.as_ref().and_then(|v| text(v, "syncedLyrics")) {
        return Some(synced.to_owned());
    }
    let fallback = found.as_ref().and_then(pick).map(str::to_owned);
    let search = format!(
        "{BASE}/search?track_name={}&artist_name={}",
        enc(title),
        enc(artist)
    );
    let result = get(&agent, &search);
    let candidates: Vec<_> = result
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| matches_recording(v, title, artist, album, secs))
        .collect();
    if let Some(synced) = candidates.iter().find_map(|v| text(v, "syncedLyrics")) {
        return Some(synced.to_owned());
    }

    // Relaxed fallback for YouTube Music titles with parenthetical subtitles or featured artists
    let clean_title = strip_title_suffix(title);
    let primary_artist = crate::library::split_artists(artist)
        .into_iter()
        .next()
        .unwrap_or_else(|| artist.trim().to_owned());
    let relaxed_query = if clean_title != title.trim() || primary_artist != artist.trim() {
        let url = format!(
            "{BASE}/search?track_name={}&artist_name={}",
            enc(&clean_title),
            enc(&primary_artist)
        );
        get(&agent, &url)
    } else {
        result.clone()
    };
    let relaxed_candidates: Vec<_> = relaxed_query
        .as_ref()
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| matches_relaxed_recording(v, &clean_title, &primary_artist, secs))
        .collect();
    if let Some(synced) = relaxed_candidates.iter().find_map(|v| text(v, "syncedLyrics")) {
        return Some(synced.to_owned());
    }

    fallback
        .or_else(|| candidates.into_iter().find_map(pick).map(str::to_owned))
        .or_else(|| relaxed_candidates.into_iter().find_map(pick).map(str::to_owned))
}

const LYRICSPLUS_BASES: &[&str] = &[
    "https://lyricsplus.prjktla.my.id",
    "https://lyricsplus.binimum.org",
];

fn build_lyricsplus_line(
    syllables: &[&Value],
    fallback_text: Option<&str>,
    fallback_start: Option<u64>,
    fallback_end: Option<u64>,
    right_align: bool,
    background: bool,
) -> Option<crate::lrc::LyricLine> {
    let mut text = String::new();
    let mut spans = Vec::new();
    for syl in syllables {
        let Some(part) = syl["text"].as_str() else { continue };
        if part.is_empty() {
            continue;
        }
        let start_byte = text.len();
        text.push_str(part);
        let end_byte = text.len();
        if let Some(st) = syl["time"].as_u64() {
            let dur = syl["duration"].as_u64().unwrap_or(0);
            let en = st.saturating_add(dur.max(40));
            spans.push(crate::lrc::TimedSpan {
                bytes: start_byte..end_byte,
                start_ms: st,
                end_ms: en,
            });
        }
    }
    if text.trim().is_empty() {
        if let Some(fb) = fallback_text {
            text = fb.trim().to_string();
        }
    }
    // Trim leading/trailing whitespace (and outer parentheses on background vocals like "(Oh my love)").
    let mut lead_trim = text.len() - text.trim_start().len();
    let mut end_trim = text.trim_end().len();
    if background && end_trim > lead_trim {
        let slice = &text[lead_trim..end_trim];
        if slice.starts_with('(') && slice.ends_with(')') && slice.len() > 2 {
            lead_trim += 1;
            end_trim -= 1;
            let inner = &text[lead_trim..end_trim];
            let extra_lead = inner.len() - inner.trim_start().len();
            let extra_end = inner.trim_end().len();
            end_trim = lead_trim + extra_end;
            lead_trim += extra_lead;
        }
    }
    if lead_trim > 0 || end_trim < text.len() {
        text = text[lead_trim..end_trim].to_string();
        let max_len = text.len();
        for sp in &mut spans {
            let s = sp.bytes.start.saturating_sub(lead_trim).min(max_len);
            let e = sp.bytes.end.saturating_sub(lead_trim).min(max_len);
            sp.bytes = s..e;
        }
        spans.retain(|sp| sp.bytes.start < sp.bytes.end);
    }
    if text.trim().is_empty() {
        return None;
    }
    let effective_start = if background {
        spans.first().map(|s| s.start_ms).or(fallback_start)
    } else {
        fallback_start.or_else(|| spans.first().map(|s| s.start_ms))
    };
    let effective_end = if background {
        spans.last().map(|s| s.end_ms).or(fallback_end)
    } else {
        spans.last().map(|s| s.end_ms).or(fallback_end)
    };
    Some(crate::lrc::LyricLine {
        time_ms: effective_start,
        end_ms: effective_end,
        text,
        spans,
        right_align,
        background,
    })
}

fn parse_clock_ms(raw: &str) -> Option<u64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let parts: Vec<&str> = s.split(':').collect();
    match parts.as_slice() {
        [secs] => secs.parse::<f64>().ok().filter(|v| *v > 0.0).map(|v| (v * 1000.0).round() as u64),
        [mins, secs] => {
            let m = mins.parse::<u64>().ok()?;
            let sec = secs.parse::<f64>().ok()?;
            Some(m.saturating_mul(60_000).saturating_add((sec * 1000.0).round() as u64))
        }
        [hrs, mins, secs] => {
            let h = hrs.parse::<u64>().ok()?;
            let m = mins.parse::<u64>().ok()?;
            let sec = secs.parse::<f64>().ok()?;
            Some(
                h.saturating_mul(3_600_000)
                    .saturating_add(m.saturating_mul(60_000))
                    .saturating_add((sec * 1000.0).round() as u64),
            )
        }
        _ => None,
    }
}

/// Guards against LyricsPlus v2 fuzzy-matching a completely different song with the same title
/// (e.g., returning Changg's 3:39 "Hướng Dương" when playing Thắng's 3:28 "Hướng Dương").
pub fn lyricsplus_matches_duration(v: &Value, parsed: &crate::lrc::Lyrics, expected_duration_ms: u64) -> bool {
    if expected_duration_ms < 30_000 {
        return true;
    }
    if let Some(td_ms) = v
        .pointer("/metadata/totalDuration")
        .and_then(Value::as_str)
        .and_then(parse_clock_ms)
    {
        if td_ms.abs_diff(expected_duration_ms) > 6_500 {
            return false;
        }
    }
    if let Some(last_ms) = parsed
        .lines
        .iter()
        .filter_map(|l| l.end_ms.or(l.time_ms))
        .max()
    {
        if last_ms > expected_duration_ms.saturating_add(7_500) {
            return false;
        }
    }
    true
}

/// Parses a LyricsPlus v2 JSON payload (`type: "Word" | "Line"`, `lyrics: [{ time, duration, text, syllabus, element }]`)
/// into Nhịp's unified `Lyrics` with millisecond-accurate UTF-8 syllable `TimedSpan`s, opposite-singer alignment,
/// and separate background vocal sub-lines matching Spicy Lyrics.
pub fn parse_lyricsplus_json(v: &Value) -> Option<crate::lrc::Lyrics> {
    let arr = v["lyrics"].as_array()?;
    if arr.is_empty() {
        return None;
    }
    let is_word_synced = v["type"]
        .as_str()
        .is_some_and(|t| t.eq_ignore_ascii_case("word"));
    let agents = v.pointer("/metadata/agents").and_then(Value::as_object);
    let mut lines = Vec::with_capacity(arr.len());
    let mut any_timed = false;
    for entry in arr {
        let line_start = entry["time"].as_u64();
        let line_dur = entry["duration"].as_u64().unwrap_or(0);
        let line_end = line_start.map(|s| s.saturating_add(line_dur)).filter(|_| line_dur > 0);
        // Only right-align an individual opposite vocalist (e.g. `v2` person) on word-synced duet tracks;
        // NEVER right-align line-synced tracks or group choruses (`v1000` / `type: "group"`).
        let right_align = is_word_synced
            && entry
                .pointer("/element/singer")
                .and_then(Value::as_str)
                .is_some_and(|s| {
                    if s.is_empty()
                        || s.eq_ignore_ascii_case("v1")
                        || s.eq_ignore_ascii_case("v0")
                        || s.eq_ignore_ascii_case("v1000")
                    {
                        return false;
                    }
                    let is_group = agents
                        .and_then(|map| map.get(s))
                        .and_then(|a| a.get("type"))
                        .and_then(Value::as_str)
                        .is_some_and(|t| t.eq_ignore_ascii_case("group"));
                    !is_group
                });
        let line_is_bg = entry["element"]["isBackground"].as_bool().unwrap_or(false)
            || entry["isBackground"].as_bool().unwrap_or(false);

        if let Some(syllables) = entry["syllabus"].as_array().filter(|s| !s.is_empty()) {
            let mut main_syls: Vec<&Value> = Vec::new();
            let mut bg_syls: Vec<&Value> = Vec::new();
            for syl in syllables {
                if line_is_bg || syl["isBackground"].as_bool().unwrap_or(false) {
                    bg_syls.push(syl);
                } else {
                    main_syls.push(syl);
                }
            }
            if !main_syls.is_empty() {
                if let Some(line) = build_lyricsplus_line(&main_syls, None, line_start, line_end, right_align, false) {
                    if line.time_ms.is_some() {
                        any_timed = true;
                    }
                    lines.push(line);
                }
            }
            if !bg_syls.is_empty() {
                if let Some(bg_line) = build_lyricsplus_line(&bg_syls, None, line_start, line_end, right_align, true) {
                    if bg_line.time_ms.is_some() {
                        any_timed = true;
                    }
                    lines.push(bg_line);
                }
            }
        } else if let Some(line) = build_lyricsplus_line(
            &[],
            entry["text"].as_str(),
            line_start,
            line_end,
            right_align,
            line_is_bg,
        ) {
            if line.time_ms.is_some() {
                any_timed = true;
            }
            lines.push(line);
        }
    }
    if lines.is_empty() || !any_timed {
        return None;
    }
    let songwriter = v
        .pointer("/metadata/songWriters")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty());
    let provider: &'static str = match v
        .pointer("/metadata/source")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "apple" | "apple music" => "Apple Music",
        "musixmatch" | "musixmatch-word" => "Musixmatch",
        "spotify" => "Spotify",
        _ => "LyricsPlus",
    };
    Some(crate::lrc::Lyrics {
        lines,
        synced: true,
        songwriter,
        provider: Some(provider),
        source: Some(provider.to_ascii_lowercase().replace(' ', "_")),
        attribution: None,
        track_ids: None,
    })
}

/// Fetches word-by-word / syllable-synced lyrics from open public LyricsPlus v2 aggregators (zero auth).
pub fn fetch_word_synced(title: &str, artist: &str, album: &str, duration_ms: u64) -> Option<crate::lrc::Lyrics> {
    let clean_title = strip_title_suffix(title);
    let full_artist = artist.trim();
    let primary_artist = crate::library::split_artists(artist)
        .into_iter()
        .next()
        .unwrap_or_else(|| full_artist.to_owned());
    if clean_title.is_empty() || primary_artist.is_empty() {
        return None;
    }
    let agent = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
        .ok()?;
    let dur_param = if duration_ms >= 30_000 {
        format!("&duration={}", (duration_ms as f64 / 1000.0).round() as u64)
    } else {
        String::new()
    };
    let album_param = if !album.trim().is_empty() {
        format!("&album={}", enc(album.trim()))
    } else {
        String::new()
    };
    // Include duration hint upfront so LyricsPlus matches the right recording when multiple artists share a title.
    let mut queries = Vec::with_capacity(3);
    queries.push(format!("/v2/lyrics/get?title={}&artist={}{album_param}{dur_param}", enc(&clean_title), enc(full_artist)));
    if primary_artist != full_artist {
        queries.push(format!("/v2/lyrics/get?title={}&artist={}{album_param}{dur_param}", enc(&clean_title), enc(&primary_artist)));
    }
    if !dur_param.is_empty() {
        queries.push(format!("/v2/lyrics/get?title={}&artist={}", enc(&clean_title), enc(full_artist)));
    }
    let mut best_line_only: Option<crate::lrc::Lyrics> = None;
    for base in LYRICSPLUS_BASES {
        for q in &queries {
            let url = format!("{base}{q}");
            if let Some(json) = get(&agent, &url) {
                if let Some(parsed) = parse_lyricsplus_json(&json) {
                    if !lyricsplus_matches_duration(&json, &parsed, duration_ms) {
                        continue;
                    }
                    let has_words = parsed.lines.iter().any(|l| !l.spans.is_empty());
                    if has_words {
                        return Some(parsed);
                    }
                    if best_line_only.is_none() {
                        best_line_only = Some(parsed);
                    }
                }
            }
        }
    }
    best_line_only
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timed_lyrics_win_over_plain() {
        let v = json!({ "plainLyrics": "dong", "syncedLyrics": "[00:01.00]dong" });
        assert_eq!(pick(&v).as_deref(), Some("[00:01.00]dong"));
    }

    #[test]
    fn empty_or_missing_lyrics_are_none() {
        assert_eq!(
            pick(&json!({ "plainLyrics": "dong" })).as_deref(),
            Some("dong")
        );
        assert_eq!(
            pick(&json!({ "syncedLyrics": "  ", "plainLyrics": null })),
            None
        );
    }

    #[test]
    fn parse_lyricsplus_json_preserves_vietnamese_syllable_byte_ranges_and_metadata() {
        let payload = json!({
            "type": "Word",
            "metadata": {
                "source": "Apple",
                "songWriters": ["MONO", "EmceeDan"]
            },
            "lyrics": [
                {
                    "time": 12400,
                    "duration": 3100,
                    "text": "Từ ngày gặp em đến bây giờ",
                    "syllabus": [
                        { "time": 12400, "duration": 420, "text": "Từ " },
                        { "time": 12820, "duration": 510, "text": "ngày " },
                        { "time": 13330, "duration": 400, "text": "gặp " },
                        { "time": 13730, "duration": 450, "text": "em " },
                        { "time": 14180, "duration": 480, "text": "đến " },
                        { "time": 14660, "duration": 420, "text": "bây " },
                        { "time": 15080, "duration": 420, "text": "giờ" }
                    ],
                    "element": { "singer": "v1" }
                }
            ]
        });
        let parsed = parse_lyricsplus_json(&payload).expect("valid LyricsPlus word-synced payload");
        assert!(parsed.synced);
        assert_eq!(parsed.provider, Some("Apple Music"));
        assert_eq!(parsed.songwriter.as_deref(), Some("MONO, EmceeDan"));
        assert_eq!(parsed.lines.len(), 1);
        let line = &parsed.lines[0];
        assert_eq!(line.time_ms, Some(12_400));
        assert_eq!(line.end_ms, Some(15_500));
        assert_eq!(line.spans.len(), 7);
        // Every syllable byte slice must be valid UTF-8 and match the Vietnamese text exactly.
        let reconstructed: String = line.spans.iter().map(|s| &line.text[s.bytes.start..s.bytes.end]).collect();
        assert_eq!(reconstructed, "Từ ngày gặp em đến bây giờ");
        assert_eq!(&line.text[line.spans[0].bytes.start..line.spans[0].bytes.end], "Từ ");
        assert_eq!(&line.text[line.spans[6].bytes.start..line.spans[6].bytes.end], "giờ");
    }

    #[test]
    fn parse_lyricsplus_json_splits_background_vocals_and_right_aligns_second_singer() {
        let payload = json!({
            "type": "Word",
            "metadata": { "source": "Apple" },
            "lyrics": [
                {
                    "time": 76687,
                    "duration": 3833,
                    "text": "I'm in love with you (Oh my love)",
                    "syllabus": [
                        { "time": 76687, "duration": 400, "text": "I'm in love " },
                        { "time": 77087, "duration": 500, "text": "with you " },
                        { "time": 78517, "duration": 410, "text": "(Oh ", "isBackground": true },
                        { "time": 78927, "duration": 458, "text": "my ", "isBackground": true },
                        { "time": 79385, "duration": 1135, "text": "love)", "isBackground": true }
                    ],
                    "element": { "singer": "v1" }
                },
                {
                    "time": 82000,
                    "duration": 2400,
                    "text": "Em vẫn luôn rạng rỡ",
                    "syllabus": [
                        { "time": 82000, "duration": 1000, "text": "Em vẫn " },
                        { "time": 83000, "duration": 1400, "text": "luôn rạng rỡ" }
                    ],
                    "element": { "singer": "v2" }
                }
            ]
        });
        let parsed = parse_lyricsplus_json(&payload).expect("valid dual-singer LyricsPlus payload");
        assert_eq!(parsed.lines.len(), 3);
        // Line 0: main vocal v1, stripped of trailing background vocal
        assert!(!parsed.lines[0].right_align && !parsed.lines[0].background);
        assert_eq!(parsed.lines[0].text, "I'm in love with you");
        // Line 1: companion background vocal, outer parens stripped and UTF-8 byte spans shifted accurately
        assert!(!parsed.lines[1].right_align && parsed.lines[1].background);
        assert_eq!(parsed.lines[1].text, "Oh my love");
        let bg_recon: String = parsed.lines[1]
            .spans
            .iter()
            .map(|s| &parsed.lines[1].text[s.bytes.start..s.bytes.end])
            .collect();
        assert_eq!(bg_recon, "Oh my love");
        // Line 2: opposite singer v2 is right-aligned
        assert!(parsed.lines[2].right_align && !parsed.lines[2].background);
        // display_current bridges brief inter-line breath pauses (80_900ms between line 0/1 end and line 2 start)
        // without dropping to None (preventing jump-to-top mid-song!).
        assert_eq!(parsed.display_current(80_900), Some(0));
        assert_eq!(parsed.display_current(10_000), None);
        assert!(parsed.is_line_singing(1, 79_000));
    }

    #[test]
    fn lyricsplus_rejects_wrong_duration_song_and_keeps_line_synced_left_aligned() {
        // Case 1: Wrong-song fuzzy match (Changg's 3:39 "Hướng Dương" returned for Thắng's 3:28 track)
        let wrong_song = json!({
            "type": "Word",
            "metadata": { "source": "Apple", "totalDuration": "3:39.213" },
            "lyrics": [
                {
                    "time": 15520,
                    "duration": 3458,
                    "text": "Em thích đứng dưới bầu trời",
                    "syllabus": [{ "time": 15520, "duration": 3458, "text": "Em thích đứng dưới bầu trời" }],
                    "element": { "singer": "v1" }
                }
            ]
        });
        let parsed_wrong = parse_lyricsplus_json(&wrong_song).unwrap();
        assert!(!lyricsplus_matches_duration(&wrong_song, &parsed_wrong, 208_000));
        assert!(lyricsplus_matches_duration(&wrong_song, &parsed_wrong, 219_000));

        // Case 2: Line-synced track with group chorus v1000 stays strictly left-aligned like Spotify!
        let line_synced = json!({
            "type": "Line",
            "metadata": {
                "source": "Apple",
                "totalDuration": "4:32.362",
                "agents": {
                    "v1": { "type": "person" },
                    "v1000": { "type": "group" },
                    "v2": { "type": "person" }
                }
            },
            "lyrics": [
                { "time": 20469, "duration": 2175, "text": "Con là con của đại ngàn bao la", "element": { "singer": "v1" } },
                { "time": 26663, "duration": 4200, "text": "Nhìn bầu trời xanh mắt em trong như giọt sương", "element": { "singer": "v1000" } },
                { "time": 31000, "duration": 3000, "text": "Ngày ngày cùng bạn bè ríu rít", "element": { "singer": "v2" } }
            ]
        });
        let parsed_line = parse_lyricsplus_json(&line_synced).unwrap();
        assert!(lyricsplus_matches_duration(&line_synced, &parsed_line, 272_000));
        assert!(parsed_line.lines.iter().all(|l| !l.right_align));
    }

    #[test]
    fn search_fallback_rejects_other_recordings() {
        let v = json!({ "trackName": "Một Bài", "artistName": "Nghệ Sĩ", "albumName": "Bản thu", "duration": 203 });
        assert!(matches_recording(
            &v,
            "Một bài",
            "Nghệ sĩ",
            "Bản thu",
            202.0
        ));
        assert!(matches_recording(&v, "Một bài", "Nghệ sĩ", "", 202.0));
        assert!(!matches_recording(
            &v,
            "Một bài",
            "Người khác",
            "Bản thu",
            202.0
        ));
        assert!(!matches_recording(&v, "Một bài", "Nghệ sĩ", "Live", 202.0));
        assert!(!matches_recording(
            &v,
            "Một bài",
            "Nghệ sĩ",
            "Bản thu",
            207.0
        ));
    }
}
