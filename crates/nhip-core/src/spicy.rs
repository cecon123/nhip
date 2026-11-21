//! Official Spicy Lyrics Developer API (`https://api.spicylyrics.org`) lyrics provider.
//!
//! Verified against official documentation (`https://developers.spicylyrics.org/llms-full.txt`):
//! - `GET /v1/lyrics/{id}` accepts a Spotify track ID (22 base62 chars), ISRC (12 chars),
//!   or Apple Music track ID (digits only).
//! - `GET /v1/tracks/search` resolves title/artist/album/duration to a canonical Spotify ID with
//!   confidence tiers (`exact`, `likely`, `version`, `other_artist`, `related`).
//! - Credential safety: NEVER embeds a secret key (`sl_sk_...`) in the distributed desktop binary.
//!   Users supply either a publishable client key (`sl_pk_...`, e.g. from an App Template with
//!   native "No origin header" enabled) or point `base_url` to a compliant backend proxy holding
//!   `sl_sk_...`.
//! - Caching: local cache strictly honors the 30-day maximum retention rule (`MAX_CACHE_TTL_SECS`).
//! - Attribution: preserves `source` and conditional `UploadAttribution` (`Uploader` + optional
//!   `Maker` strictly when `source == "spicy_lyrics"`), required by Terms of Service Section 6.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::library::Track;
use crate::lrc::{
    provider_label_for_source, LyricLine, Lyrics, LyricsAttribution, LyricsContributor,
    LyricsTrackIds, TimedSpan,
};

pub const DEFAULT_BASE_URL: &str = "https://api.spicylyrics.org";
/// Maximum permitted local cache retention per Spicy Lyrics platform terms (30 days).
pub const MAX_CACHE_TTL_SECS: u64 = 30 * 86_400;
/// Default local cache TTL (7 days, safely within the 30-day maximum).
pub const DEFAULT_CACHE_TTL_SECS: u64 = 7 * 86_400;
const MAX_RESPONSE_BYTES: u64 = 1_024 * 1_024;
const INLINE_RETRY_MAX_SECS: u64 = 2;
const DEFAULT_TIMEOUT_SECS: u64 = 5;

#[derive(Clone, Debug)]
pub struct SpicyApiConfig {
    pub enabled: bool,
    /// User-supplied API key (`sl_pk_...` or proxy token). Never hardcoded in the binary.
    pub api_key: Option<String>,
    /// Defaults to `https://api.spicylyrics.org`; can point to a self-hosted proxy.
    pub base_url: String,
    pub timeout: Duration,
    /// Local cache TTL in seconds, always clamped to `<= MAX_CACHE_TTL_SECS` (30 days).
    pub cache_ttl_secs: u64,
    /// Optional disk cache path (`None` disables disk persistence, e.g. in unit tests).
    pub cache_path: Option<PathBuf>,
}

impl Default for SpicyApiConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
            base_url: DEFAULT_BASE_URL.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            cache_ttl_secs: DEFAULT_CACHE_TTL_SECS,
            cache_path: None,
        }
    }
}

impl SpicyApiConfig {
    /// Builds runtime config from user `Settings` + optional environment overrides
    /// (`SPICY_LYRICS_API_KEY`, `SPICY_LYRICS_API_URL`).
    pub fn from_settings(settings: &crate::library::Settings) -> Self {
        let env_key = std::env::var("SPICY_LYRICS_API_KEY")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let setting_key = {
            let trimmed = settings.spicy_lyrics_api_key.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        let env_url = std::env::var("SPICY_LYRICS_API_URL")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty());
        let setting_url = {
            let trimmed = settings.spicy_lyrics_api_url.trim().trim_end_matches('/');
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        };
        Self {
            enabled: settings.spicy_lyrics_enabled,
            api_key: setting_key.or(env_key),
            base_url: setting_url
                .or(env_url)
                .unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECS),
            cache_ttl_secs: DEFAULT_CACHE_TTL_SECS,
            cache_path: Some(crate::library::data_dir().join("spicy-lyrics-cache.json")),
        }
    }

    /// True when the Spicy Lyrics provider is eligible to run first.
    /// Direct calls to `api.spicylyrics.org` require a key; a custom proxy URL may authenticate server-side.
    pub fn is_eligible(&self) -> bool {
        if !self.enabled {
            return false;
        }
        let has_key = self
            .api_key
            .as_deref()
            .is_some_and(|k| !k.trim().is_empty());
        let is_custom_proxy = self.base_url.trim_end_matches('/') != DEFAULT_BASE_URL;
        has_key || is_custom_proxy
    }

    pub fn effective_ttl_secs(&self) -> u64 {
        self.cache_ttl_secs.min(MAX_CACHE_TTL_SECS)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpicyError {
    NotConfigured,
    Unauthorized(u16, String),
    Forbidden(u16, String),
    NotFound(String),
    RateLimited { retry_after_secs: u64, code: String },
    ServerError(u16, String),
    Network(String),
    Malformed(String),
    EmptyLyrics,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CachedEntry {
    fetched_at_unix: u64,
    lyrics: Lyrics,
}

type InFlightWaiter = Arc<(Mutex<Option<Result<Lyrics, SpicyError>>>, Condvar)>;

#[derive(Default)]
pub struct SpicyClientState {
    cache: Mutex<HashMap<String, CachedEntry>>,
    disk_loaded: Mutex<bool>,
    in_flight: Mutex<HashMap<String, InFlightWaiter>>,
    cooldown_until: Mutex<Option<Instant>>,
}

impl SpicyClientState {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_cooling_down(&self) -> Option<u64> {
        let guard = self.cooldown_until.lock().ok()?;
        let until = (*guard)?;
        let now = Instant::now();
        if until > now {
            Some(until.saturating_duration_since(now).as_secs().max(1))
        } else {
            None
        }
    }

    fn set_cooldown(&self, secs: u64) {
        if secs == 0 {
            return;
        }
        if let Ok(mut guard) = self.cooldown_until.lock() {
            let until = Instant::now() + Duration::from_secs(secs.min(3600));
            *guard = Some(match *guard {
                Some(prev) if prev > until => prev,
                _ => until,
            });
        }
    }

    fn ensure_disk_loaded(&self, path: Option<&PathBuf>, ttl_secs: u64) {
        let Some(path) = path else { return };
        let Ok(mut loaded) = self.disk_loaded.lock() else {
            return;
        };
        if *loaded {
            return;
        }
        *loaded = true;
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(mut map) = serde_json::from_str::<HashMap<String, CachedEntry>>(&raw) else {
            return;
        };
        let now = unix_now();
        map.retain(|_, entry| now.saturating_sub(entry.fetched_at_unix) <= ttl_secs);
        for entry in map.values_mut() {
            entry.lyrics.restore_provider_label();
        }
        if let Ok(mut cache) = self.cache.lock() {
            cache.extend(map);
        }
    }

    fn get_cached(&self, key: &str, ttl_secs: u64, now_unix: u64) -> Option<Lyrics> {
        let mut cache = self.cache.lock().ok()?;
        let valid = cache
            .get(key)
            .is_some_and(|e| now_unix.saturating_sub(e.fetched_at_unix) <= ttl_secs);
        if !valid {
            cache.remove(key);
            return None;
        }
        let mut lyrics = cache.get(key)?.lyrics.clone();
        lyrics.restore_provider_label();
        Some(lyrics)
    }

    fn put_cached(&self, key: String, lyrics: Lyrics, path: Option<&PathBuf>, ttl_secs: u64) {
        let now = unix_now();
        let entry = CachedEntry {
            fetched_at_unix: now,
            lyrics,
        };
        let snapshot = if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|_, e| now.saturating_sub(e.fetched_at_unix) <= ttl_secs);
            if cache.len() >= 256 {
                // Evict oldest entry to keep disk cache compact.
                if let Some(oldest_k) = cache
                    .iter()
                    .min_by_key(|(_, e)| e.fetched_at_unix)
                    .map(|(k, _)| k.clone())
                {
                    cache.remove(&oldest_k);
                }
            }
            cache.insert(key, entry);
            path.is_some().then(|| cache.clone())
        } else {
            None
        };
        if let (Some(path), Some(map)) = (path, snapshot) {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(json) = serde_json::to_string(&map) {
                let _ = std::fs::write(path, json);
            }
        }
    }
}

fn global_state() -> &'static SpicyClientState {
    static STATE: OnceLock<SpicyClientState> = OnceLock::new();
    STATE.get_or_init(SpicyClientState::new)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn enc(s: &str) -> String {
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

pub fn is_valid_spotify_id(id: &str) -> bool {
    id.len() == 22 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

pub fn is_valid_isrc(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|b| b.is_ascii_alphanumeric())
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

fn cache_key_for_track(track: &Track) -> String {
    if let Some(id) = track
        .spotify_id
        .as_deref()
        .filter(|s| is_valid_spotify_id(s))
    {
        return format!("sp:{id}");
    }
    if let Some(isrc) = track.isrc.as_deref().filter(|s| is_valid_isrc(s)) {
        return format!("isrc:{}", isrc.to_ascii_uppercase());
    }
    format!(
        "meta:{}|{}|{}|{}",
        crate::library::fold(&track.title),
        crate::library::fold(&track.artist),
        crate::library::fold(&track.album),
        track.duration_ms / 1000
    )
}

/// Fetches lyrics for `track` from the Spicy Lyrics API using the shared global state.
pub fn fetch_for_track(track: &Track, config: &SpicyApiConfig) -> Result<Lyrics, SpicyError> {
    fetch_for_track_with_state(track, config, global_state())
}

/// Fetches lyrics for `track` using an explicit `SpicyClientState` (enables isolated unit/mock tests).
pub fn fetch_for_track_with_state(
    track: &Track,
    config: &SpicyApiConfig,
    state: &SpicyClientState,
) -> Result<Lyrics, SpicyError> {
    if !config.is_eligible() {
        return Err(SpicyError::NotConfigured);
    }
    let ttl_secs = config.effective_ttl_secs();
    state.ensure_disk_loaded(config.cache_path.as_ref(), ttl_secs);
    let key = cache_key_for_track(track);

    if ttl_secs > 0 {
        if let Some(cached) = state.get_cached(&key, ttl_secs, unix_now()) {
            return Ok(cached);
        }
    }
    if let Some(wait_secs) = state.is_cooling_down() {
        return Err(SpicyError::RateLimited {
            retry_after_secs: wait_secs,
            code: "cooldown_active".to_string(),
        });
    }

    // In-flight deduplication: concurrent callers for the same track share one HTTP fetch.
    let (slot, is_leader) = {
        let mut map = state
            .in_flight
            .lock()
            .map_err(|_| SpicyError::Network("in_flight lock poisoned".into()))?;
        if let Some(existing) = map.get(&key) {
            (Arc::clone(existing), false)
        } else {
            let fresh = Arc::new((Mutex::new(None), Condvar::new()));
            map.insert(key.clone(), Arc::clone(&fresh));
            (fresh, true)
        }
    };

    if !is_leader {
        let (lock, cvar) = &*slot;
        let mut guard = lock
            .lock()
            .map_err(|_| SpicyError::Network("dedup lock poisoned".into()))?;
        while guard.is_none() {
            guard = cvar
                .wait(guard)
                .map_err(|_| SpicyError::Network("dedup wait failed".into()))?;
        }
        return guard
            .clone()
            .unwrap_or(Err(SpicyError::Network("empty dedup result".into())));
    }

    let result = fetch_uncached(track, config, state);
    if let Ok(ref lyrics) = result {
        if ttl_secs > 0 {
            state.put_cached(
                key.clone(),
                lyrics.clone(),
                config.cache_path.as_ref(),
                ttl_secs,
            );
        }
    }
    if let Ok(mut map) = state.in_flight.lock() {
        map.remove(&key);
    }
    let (lock, cvar) = &*slot;
    if let Ok(mut guard) = lock.lock() {
        *guard = Some(result.clone());
        cvar.notify_all();
    }
    result
}

fn fetch_uncached(
    track: &Track,
    config: &SpicyApiConfig,
    state: &SpicyClientState,
) -> Result<Lyrics, SpicyError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(config.timeout)
        .build()
        .map_err(|e| SpicyError::Network(e.to_string()))?;

    let base = config.base_url.trim_end_matches('/');
    let lookup_id = if let Some(sp) = track
        .spotify_id
        .as_deref()
        .filter(|s| is_valid_spotify_id(s))
    {
        sp.to_string()
    } else if let Some(isrc) = track.isrc.as_deref().filter(|s| is_valid_isrc(s)) {
        isrc.to_string()
    } else {
        resolve_track_id(&client, base, track, config, state)?
    };

    let url = format!("{base}/v1/lyrics/{}", enc(&lookup_id));
    let json = send_json_with_retry(&client, &url, config, state)?;
    parse_spicy_lyrics_envelope(&json)
}

fn resolve_track_id(
    client: &reqwest::blocking::Client,
    base: &str,
    track: &Track,
    config: &SpicyApiConfig,
    state: &SpicyClientState,
) -> Result<String, SpicyError> {
    let clean_title = strip_title_suffix(&track.title);
    let title = if clean_title.is_empty() {
        track.title.trim()
    } else {
        clean_title.as_str()
    };
    if title.is_empty() {
        return Err(SpicyError::NotFound("empty track title".into()));
    }

    let mut url = format!("{base}/v1/tracks/search?title={}&limit=5", enc(title));
    let artist = track.artist.trim();
    if !artist.is_empty() {
        url.push_str(&format!("&artist={}", enc(artist)));
    }
    let album = track.album.trim();
    if !album.is_empty() {
        url.push_str(&format!("&album={}", enc(album)));
    }
    let expected_secs = if track.duration_ms > 0 {
        let secs = track.duration_ms as f64 / 1000.0;
        url.push_str(&format!("&duration={secs:.3}"));
        Some(secs)
    } else {
        None
    };

    let json = send_json_with_retry(client, &url, config, state)?;
    pick_search_result_id(&json, expected_secs)
        .ok_or_else(|| SpicyError::NotFound("no trusted search match".into()))
}

/// Selects the canonical Spotify track ID from `GET /v1/tracks/search` envelope.
/// Honors documented match tiers: accepts `exact` directly, or `likely` only when duration
/// agrees within `<= 4.0s`. Rejects `version`, `other_artist`, and `related` matches.
pub fn pick_search_result_id(
    envelope: &Value,
    expected_duration_secs: Option<f64>,
) -> Option<String> {
    let results = envelope.pointer("/Body/results")?.as_array()?;
    // Pass 1: prefer `exact` tier.
    for item in results {
        let tier = item["tier"].as_str().unwrap_or("");
        if tier == "exact" {
            if let Some(id) = item["id"].as_str().filter(|s| !s.is_empty()) {
                return Some(id.to_string());
            }
        }
    }
    // Pass 2: accept `likely` only when duration confirms it is the same recording cut.
    if let Some(expected) = expected_duration_secs {
        for item in results {
            let tier = item["tier"].as_str().unwrap_or("");
            let dur = item["duration"].as_f64();
            if tier == "likely" && dur.is_some_and(|d| (d - expected).abs() <= 4.0) {
                if let Some(id) = item["id"].as_str().filter(|s| !s.is_empty()) {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

fn parse_retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get("retry-after")
        .or_else(|| headers.get("ratelimit-reset"))
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
}

fn parse_error_code(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.pointer("/Body/error")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "unknown_error".to_string())
}

fn send_json_with_retry(
    client: &reqwest::blocking::Client,
    url: &str,
    config: &SpicyApiConfig,
    state: &SpicyClientState,
) -> Result<Value, SpicyError> {
    let mut attempt = 0u8;
    loop {
        let mut req = client
            .get(url)
            .header("User-Agent", "Nhip/0.1 (desktop music player)")
            .header("Accept", "application/json");
        if let Some(ref key) = config.api_key {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                req = req.header("Authorization", format!("Bearer {trimmed}"));
            }
        }

        let resp = req.send().map_err(|e| SpicyError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        let retry_after = parse_retry_after_secs(resp.headers());

        let mut text = String::new();
        resp.take(MAX_RESPONSE_BYTES)
            .read_to_string(&mut text)
            .map_err(|e| SpicyError::Network(e.to_string()))?;

        match status {
            200 => {
                return serde_json::from_str::<Value>(&text)
                    .map_err(|e| SpicyError::Malformed(e.to_string()));
            }
            401 => return Err(SpicyError::Unauthorized(status, parse_error_code(&text))),
            403 => return Err(SpicyError::Forbidden(status, parse_error_code(&text))),
            400 | 404 => return Err(SpicyError::NotFound(parse_error_code(&text))),
            429 => {
                let wait = retry_after.unwrap_or(60).max(1);
                if attempt == 0 && wait <= INLINE_RETRY_MAX_SECS {
                    attempt += 1;
                    std::thread::sleep(Duration::from_secs(wait));
                    continue;
                }
                state.set_cooldown(wait);
                return Err(SpicyError::RateLimited {
                    retry_after_secs: wait,
                    code: parse_error_code(&text),
                });
            }
            503 => {
                let wait = retry_after.unwrap_or(5);
                if attempt == 0 && wait <= INLINE_RETRY_MAX_SECS {
                    attempt += 1;
                    std::thread::sleep(Duration::from_secs(wait));
                    continue;
                }
                if retry_after.is_some() {
                    state.set_cooldown(wait);
                }
                return Err(SpicyError::ServerError(status, parse_error_code(&text)));
            }
            500..=599 => {
                return Err(SpicyError::ServerError(status, parse_error_code(&text)));
            }
            _ => {
                return Err(SpicyError::Malformed(format!("unexpected status {status}")));
            }
        }
    }
}

fn sec_to_ms(v: &Value) -> Option<u64> {
    let s = v.as_f64()?;
    if !s.is_finite() || s < 0.0 {
        return None;
    }
    Some((s * 1000.0).round() as u64)
}

fn parse_contributor(v: &Value) -> Option<LyricsContributor> {
    let id = v["id"].as_str()?.trim();
    let username = v["username"].as_str()?.trim();
    let url = v["url"].as_str()?.trim();
    if id.is_empty() || username.is_empty() || url.is_empty() {
        return None;
    }
    let avatar = v["avatar"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some(LyricsContributor {
        id: id.to_string(),
        username: username.to_string(),
        url: url.to_string(),
        avatar,
    })
}

fn parse_track_ids(body: &Value) -> Option<LyricsTrackIds> {
    let ids = body.get("ids")?.as_object()?;
    let pick = |k: &str| {
        ids.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let parsed = LyricsTrackIds {
        spotify: pick("spotify").or_else(|| {
            body["id"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        }),
        isrc: pick("isrc"),
        apple_music: pick("apple_music"),
        tidal: pick("tidal"),
        youtube: pick("youtube"),
    };
    if parsed == LyricsTrackIds::default() {
        None
    } else {
        Some(parsed)
    }
}

fn build_syllable_group_line(
    group: &Value,
    right_align: bool,
    background: bool,
) -> Option<LyricLine> {
    let syls = group["Syllables"].as_array()?;
    let mut text = String::new();
    let mut spans = Vec::with_capacity(syls.len());

    // Filter renderable syllables while keeping original IsPartOfWord boundaries accurate.
    let renderable: Vec<&Value> = syls
        .iter()
        .filter(|s| s["Text"].as_str().is_some_and(|t| !t.trim().is_empty()))
        .collect();
    if renderable.is_empty() {
        return None;
    }

    for (idx, syl) in renderable.iter().enumerate() {
        let raw = syl["Text"].as_str().unwrap_or("");
        let part_of_word = syl["IsPartOfWord"].as_bool().unwrap_or(false);
        let is_last = idx + 1 == renderable.len();

        let start_byte = text.len();
        text.push_str(raw);
        let end_byte = text.len();

        if let Some(st) = sec_to_ms(&syl["StartTime"]) {
            let en = sec_to_ms(&syl["EndTime"]).unwrap_or(st + 40).max(st);
            spans.push(TimedSpan {
                bytes: start_byte..end_byte,
                start_ms: st,
                end_ms: en,
            });
        }

        if !part_of_word && !is_last && !text.ends_with(' ') {
            text.push(' ');
        }
    }

    if text.trim().is_empty() {
        return None;
    }

    let start_ms = sec_to_ms(&group["StartTime"]).or_else(|| spans.first().map(|s| s.start_ms));
    let end_ms = sec_to_ms(&group["EndTime"]).or_else(|| spans.last().map(|s| s.end_ms));

    Some(LyricLine {
        time_ms: start_ms,
        end_ms,
        text,
        spans,
        right_align,
        background,
    })
}

/// Validates and normalizes a `GET /v1/lyrics/{id}` JSON `LyricsEnvelope` into `Lyrics`.
pub fn parse_spicy_lyrics_envelope(envelope: &Value) -> Result<Lyrics, SpicyError> {
    let body = envelope
        .get("Body")
        .filter(|v| v.is_object())
        .ok_or_else(|| SpicyError::Malformed("missing Body object".into()))?;

    let kind = body["Type"]
        .as_str()
        .ok_or_else(|| SpicyError::Malformed("missing Body.Type".into()))?;

    let source_raw = body["source"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("spicy_lyrics");

    let provider = provider_label_for_source(source_raw);
    let songwriter = body["SongWriters"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty());

    // Attribution rule: UploadAttribution belongs ONLY to `source == "spicy_lyrics"`.
    let attribution = if source_raw.eq_ignore_ascii_case("spicy_lyrics") {
        body.get("UploadAttribution").and_then(|ua| {
            let uploader = ua.get("Uploader").and_then(parse_contributor)?;
            let maker = ua.get("Maker").and_then(parse_contributor);
            Some(LyricsAttribution { uploader, maker })
        })
    } else {
        None
    };

    let track_ids = parse_track_ids(body);
    let mut lines = Vec::new();
    let synced;

    match kind {
        "Syllable" => {
            synced = true;
            let content = body["Content"]
                .as_array()
                .ok_or_else(|| SpicyError::Malformed("missing Syllable Content array".into()))?;
            for entry in content {
                let right_align = entry["OppositeAligned"].as_bool().unwrap_or(false);
                if let Some(lead) = entry.get("Lead") {
                    if let Some(line) = build_syllable_group_line(lead, right_align, false) {
                        lines.push(line);
                    }
                }
                if let Some(bgs) = entry["Background"].as_array() {
                    for bg in bgs {
                        if let Some(bg_line) = build_syllable_group_line(bg, right_align, true) {
                            lines.push(bg_line);
                        }
                    }
                }
            }
        }
        "Line" => {
            synced = true;
            let content = body["Content"]
                .as_array()
                .ok_or_else(|| SpicyError::Malformed("missing Line Content array".into()))?;
            for entry in content {
                let Some(text) = entry["Text"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let start_ms = sec_to_ms(&entry["StartTime"]);
                let end_ms = sec_to_ms(&entry["EndTime"]);
                let right_align = entry["OppositeAligned"].as_bool().unwrap_or(false);
                lines.push(LyricLine {
                    time_ms: start_ms,
                    end_ms,
                    text: text.to_string(),
                    spans: Vec::new(),
                    right_align,
                    background: false,
                });
            }
        }
        "Static" => {
            synced = false;
            let static_lines = body["Lines"]
                .as_array()
                .ok_or_else(|| SpicyError::Malformed("missing Static Lines array".into()))?;
            for entry in static_lines {
                let Some(text) = entry["Text"]
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                lines.push(LyricLine {
                    time_ms: None,
                    end_ms: None,
                    text: text.to_string(),
                    spans: Vec::new(),
                    right_align: false,
                    background: false,
                });
            }
        }
        other => {
            return Err(SpicyError::Malformed(format!("unsupported Type: {other}")));
        }
    }

    if lines.is_empty() {
        return Err(SpicyError::EmptyLyrics);
    }

    Ok(Lyrics {
        lines,
        synced,
        songwriter,
        provider: Some(provider),
        source: Some(source_raw.to_string()),
        attribution,
        track_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sample_track() -> Track {
        let mut t = crate::library::demo_tracks().remove(0);
        t.title = "Em Không".into();
        t.artist = "Vũ Thanh Vân".into();
        t.album = "Em Không".into();
        t.duration_ms = 213_500;
        t.spotify_id = None;
        t.isrc = None;
        t
    }

    fn spawn_mock_server<F>(handler: F) -> (String, std::thread::JoinHandle<()>)
    where
        F: Fn(&str, &str) -> (u16, Vec<(&'static str, String)>, String) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                if reader.read_line(&mut request_line).is_err() {
                    continue;
                }
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                let mut auth = String::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some((k, v)) = line.split_once(':') {
                        if k.eq_ignore_ascii_case("authorization") {
                            auth = v.trim().to_string();
                        }
                    }
                }
                let (status, headers, body) = handler(&path, &auth);
                let mut stream = stream;
                let mut head = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (k, v) in headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
                if path == "/__shutdown" {
                    break;
                }
            }
        });
        (url, handle)
    }

    #[test]
    fn normalizes_syllable_background_duets_and_community_attribution() {
        let payload = json!({
            "Status": 200,
            "Type": "object",
            "Body": {
                "id": "1QV6tiMFM6fSOKOGLMHYYg",
                "ids": {
                    "spotify": "1QV6tiMFM6fSOKOGLMHYYg",
                    "isrc": "USUM70824409",
                    "apple_music": "713369399"
                },
                "source": "spicy_lyrics",
                "Type": "Syllable",
                "StartTime": 1.2,
                "EndTime": 10.5,
                "SongWriters": ["Writer A", "Writer B"],
                "UploadAttribution": {
                    "Uploader": {
                        "id": "123",
                        "username": "spikerko",
                        "url": "https://spicylyrics.org/uid/123"
                    },
                    "Maker": {
                        "id": "456",
                        "username": "gc",
                        "url": "https://spicylyrics.org/uid/456"
                    }
                },
                "Content": [
                    {
                        "Type": "Vocal",
                        "OppositeAligned": false,
                        "Lead": {
                            "StartTime": 1.2,
                            "EndTime": 3.4,
                            "Syllables": [
                                { "Text": "Em", "StartTime": 1.2, "EndTime": 1.6, "IsPartOfWord": false },
                                { "Text": "khô", "StartTime": 1.7, "EndTime": 2.1, "IsPartOfWord": true },
                                { "Text": "ng", "StartTime": 2.1, "EndTime": 2.6, "IsPartOfWord": false }
                            ]
                        },
                        "Background": [
                            {
                                "StartTime": 2.7,
                                "EndTime": 3.5,
                                "Syllables": [
                                    { "Text": "oh", "StartTime": 2.7, "EndTime": 3.1, "IsPartOfWord": false },
                                    { "Text": "yeah", "StartTime": 3.1, "EndTime": 3.5, "IsPartOfWord": false }
                                ]
                            }
                        ]
                    },
                    {
                        "Type": "Vocal",
                        "OppositeAligned": true,
                        "Lead": {
                            "StartTime": 5.0,
                            "EndTime": 7.0,
                            "Syllables": [
                                { "Text": "Duets", "StartTime": 5.0, "EndTime": 7.0, "IsPartOfWord": false }
                            ]
                        }
                    }
                ]
            }
        });

        let parsed = parse_spicy_lyrics_envelope(&payload).expect("valid syllable envelope");
        assert!(parsed.synced);
        assert_eq!(parsed.provider, Some("Spicy Lyrics"));
        assert_eq!(parsed.source.as_deref(), Some("spicy_lyrics"));
        assert_eq!(parsed.songwriter.as_deref(), Some("Writer A, Writer B"));
        let attr = parsed.attribution.as_ref().expect("community attribution");
        assert_eq!(attr.uploader.username, "spikerko");
        assert_eq!(attr.maker.as_ref().unwrap().username, "gc");
        assert_eq!(parsed.lines.len(), 3);
        // IsPartOfWord merges "khô" + "ng" without inserting a space!
        assert_eq!(parsed.lines[0].text, "Em không");
        assert_eq!(parsed.lines[0].spans.len(), 3);
        assert_eq!(
            &parsed.lines[0].text[parsed.lines[0].spans[1].bytes.clone()],
            "khô"
        );
        assert_eq!(
            &parsed.lines[0].text[parsed.lines[0].spans[2].bytes.clone()],
            "ng"
        );
        assert!(parsed.lines[1].background);
        assert_eq!(parsed.lines[1].text, "oh yeah");
        assert!(parsed.lines[2].right_align && !parsed.lines[2].background);
    }

    #[test]
    fn commercial_sources_omit_community_attribution_and_support_line_and_static() {
        let apple_line = json!({
            "Status": 200,
            "Type": "object",
            "Body": {
                "id": "7KNmIjcmGJIBrhP2s5Vioe",
                "ids": { "spotify": "7KNmIjcmGJIBrhP2s5Vioe" },
                "source": "apple_music",
                "Type": "Line",
                "Content": [
                    { "Type": "Vocal", "Text": "First line", "StartTime": 4.5, "EndTime": 8.0, "OppositeAligned": false }
                ]
            }
        });
        let parsed = parse_spicy_lyrics_envelope(&apple_line).unwrap();
        assert!(parsed.synced);
        assert_eq!(parsed.provider, Some("Apple Music"));
        assert!(parsed.attribution.is_none());
        assert_eq!(parsed.lines[0].time_ms, Some(4_500));

        let static_resp = json!({
            "Status": 200,
            "Type": "object",
            "Body": {
                "id": "7KNmIjcmGJIBrhP2s5Vioe",
                "ids": { "spotify": "7KNmIjcmGJIBrhP2s5Vioe" },
                "source": "spotify",
                "Type": "Static",
                "Lines": [{ "Text": "Plain text" }]
            }
        });
        let parsed_static = parse_spicy_lyrics_envelope(&static_resp).unwrap();
        assert!(!parsed_static.synced);
        assert_eq!(parsed_static.provider, Some("Spotify"));
        assert_eq!(parsed_static.lines[0].text, "Plain text");
    }

    #[test]
    fn search_tier_selection_accepts_exact_and_rejects_wrong_versions() {
        let version_only = json!({
            "Body": {
                "results": [
                    { "id": "wrongLiveId12345678901", "tier": "version", "duration": 213.5 },
                    { "id": "coverId123456789012345", "tier": "other_artist", "duration": 213.5 }
                ]
            }
        });
        assert_eq!(pick_search_result_id(&version_only, Some(213.5)), None);

        let exact = json!({
            "Body": {
                "results": [
                    { "id": "4cOdK2wGLETKBW3PvgPWqT", "tier": "exact", "duration": 213.573 }
                ]
            }
        });
        assert_eq!(
            pick_search_result_id(&exact, Some(213.5)),
            Some("4cOdK2wGLETKBW3PvgPWqT".into())
        );

        let likely_mismatch = json!({
            "Body": { "results": [{ "id": "id1", "tier": "likely", "duration": 240.0 }] }
        });
        assert_eq!(pick_search_result_id(&likely_mismatch, Some(213.5)), None);
        assert_eq!(
            pick_search_result_id(&likely_mismatch, Some(238.0)),
            Some("id1".into())
        );
    }

    #[test]
    fn mock_http_covers_auth_search_cache_dedup_and_error_codes() {
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_srv = Arc::clone(&hits);
        let (base_url, _srv) = spawn_mock_server(move |path, auth| {
            hits_srv.fetch_add(1, Ordering::SeqCst);
            if auth != "Bearer sl_pk_test_key" {
                return (
                    401,
                    vec![],
                    json!({ "Status": 401, "Type": "error", "Body": { "error": "key_not_found", "message": "bad key" } }).to_string(),
                );
            }
            if path.starts_with("/v1/tracks/search") {
                return (
                    200,
                    vec![],
                    json!({
                        "Status": 200,
                        "Type": "object",
                        "Body": {
                            "results": [{
                                "id": "4cOdK2wGLETKBW3PvgPWqT",
                                "tier": "exact",
                                "duration": 213.5
                            }]
                        }
                    })
                    .to_string(),
                );
            }
            match path {
                "/v1/lyrics/4cOdK2wGLETKBW3PvgPWqT" => (
                    200,
                    vec![],
                    json!({
                        "Status": 200,
                        "Type": "object",
                        "Body": {
                            "id": "4cOdK2wGLETKBW3PvgPWqT",
                            "ids": { "spotify": "4cOdK2wGLETKBW3PvgPWqT" },
                            "source": "spicy_lyrics",
                            "Type": "Line",
                            "Content": [
                                { "Type": "Vocal", "Text": "Synced from Spicy", "StartTime": 1.0, "EndTime": 4.0 }
                            ]
                        }
                    })
                    .to_string(),
                ),
                "/v1/lyrics/1111111111111111111403" => (
                    403,
                    vec![],
                    json!({ "Status": 403, "Type": "error", "Body": { "error": "origin_not_allowed", "message": "forbidden" } }).to_string(),
                ),
                "/v1/lyrics/1111111111111111111404" => (
                    404,
                    vec![],
                    json!({ "Status": 404, "Type": "error", "Body": { "error": "lyrics_not_found", "message": "missing" } }).to_string(),
                ),
                "/v1/lyrics/1111111111111111111429" => (
                    429,
                    vec![("Retry-After", "12".into())],
                    json!({ "Status": 429, "Type": "error", "Body": { "error": "rate_limited", "message": "slow down" } }).to_string(),
                ),
                "/v1/lyrics/1111111111111111111502" => (
                    502,
                    vec![],
                    json!({ "Status": 502, "Type": "error", "Body": { "error": "upstream_error", "message": "bad gw" } }).to_string(),
                ),
                "/v1/lyrics/1111111111111111111bad" => (200, vec![], "{not json".into()),
                "/v1/lyrics/1111111111111111111emp" => (
                    200,
                    vec![],
                    json!({ "Status": 200, "Type": "object", "Body": { "id": "x", "source": "spotify", "Type": "Line", "Content": [] } }).to_string(),
                ),
                _ => (404, vec![], "{}".into()),
            }
        });

        let state = SpicyClientState::new();
        let cfg = SpicyApiConfig {
            enabled: true,
            api_key: Some("sl_pk_test_key".into()),
            base_url: base_url.clone(),
            timeout: Duration::from_secs(3),
            cache_ttl_secs: DEFAULT_CACHE_TTL_SECS,
            cache_path: None,
        };

        // 1. Search -> Lyrics + cache hit on second call (zero extra network requests!)
        let t = sample_track();
        let res = fetch_for_track_with_state(&t, &cfg, &state).expect("search + fetch succeeds");
        assert_eq!(res.lines[0].text, "Synced from Spicy");
        let after_first = hits.load(Ordering::SeqCst);
        assert_eq!(after_first, 2);
        let res2 = fetch_for_track_with_state(&t, &cfg, &state).expect("cache hit");
        assert_eq!(res2.lines[0].text, "Synced from Spicy");
        assert_eq!(
            hits.load(Ordering::SeqCst),
            after_first,
            "cached response must not hit HTTP again"
        );

        // 2. 401 unauthorized
        let bad_cfg = SpicyApiConfig {
            api_key: Some("wrong".into()),
            ..cfg.clone()
        };
        let mut t401 = sample_track();
        t401.spotify_id = Some("4cOdK2wGLETKBW3PvgPWqT".into());
        assert!(matches!(
            fetch_for_track_with_state(&t401, &bad_cfg, &SpicyClientState::new()),
            Err(SpicyError::Unauthorized(401, _))
        ));

        // 3. 403 / 404 / 502 / malformed / empty
        for (id, check) in [
            ("1111111111111111111403", "403"),
            ("1111111111111111111404", "404"),
            ("1111111111111111111502", "502"),
            ("1111111111111111111bad", "bad"),
            ("1111111111111111111emp", "emp"),
        ] {
            let mut tr = sample_track();
            tr.spotify_id = Some(id.into());
            let err = fetch_for_track_with_state(&tr, &cfg, &SpicyClientState::new()).unwrap_err();
            match check {
                "403" => assert!(matches!(err, SpicyError::Forbidden(403, _)), "{err:?}"),
                "404" => assert!(matches!(err, SpicyError::NotFound(_)), "{err:?}"),
                "502" => assert!(matches!(err, SpicyError::ServerError(502, _)), "{err:?}"),
                "bad" => assert!(matches!(err, SpicyError::Malformed(_)), "{err:?}"),
                "emp" => assert!(matches!(err, SpicyError::EmptyLyrics), "{err:?}"),
                _ => unreachable!(),
            }
        }

        // 4. 429 Retry-After triggers cooldown so immediate follow-up doesn't hit server
        let rl_state = SpicyClientState::new();
        let mut t429 = sample_track();
        t429.spotify_id = Some("1111111111111111111429".into());
        let before_429 = hits.load(Ordering::SeqCst);
        let err1 = fetch_for_track_with_state(&t429, &cfg, &rl_state).unwrap_err();
        assert!(matches!(
            err1,
            SpicyError::RateLimited {
                retry_after_secs: 12,
                ..
            }
        ));
        assert_eq!(hits.load(Ordering::SeqCst), before_429 + 1);
        let err2 = fetch_for_track_with_state(&t429, &cfg, &rl_state).unwrap_err();
        assert!(matches!(err2, SpicyError::RateLimited { .. }));
        assert_eq!(
            hits.load(Ordering::SeqCst),
            before_429 + 1,
            "cooldown must prevent hammering API after Retry-After"
        );
    }

    #[test]
    fn cache_evicts_entries_older_than_30_day_limit() {
        let state = SpicyClientState::new();
        let lyrics = Lyrics::parse("[00:01.00]Hello");
        state.cache.lock().unwrap().insert(
            "k".into(),
            CachedEntry {
                fetched_at_unix: 1000,
                lyrics,
            },
        );
        // Within 30 days -> hit
        assert!(state
            .get_cached("k", MAX_CACHE_TTL_SECS, 1000 + MAX_CACHE_TTL_SECS)
            .is_some());
        // Past 30 days -> evicted
        assert!(state
            .get_cached("k", MAX_CACHE_TTL_SECS, 1000 + MAX_CACHE_TTL_SECS + 1)
            .is_none());
    }
}
