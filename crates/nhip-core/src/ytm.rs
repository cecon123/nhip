//! Public YouTube Music metadata. Streaming audio has a separate acceptance check.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use ytmapi_rs::auth::noauth::NoAuthToken;
use ytmapi_rs::common::{AlbumID, ArtistChannelID, PlaylistID, VideoID, YoutubeID};
use ytmapi_rs::parse::{BasicSearchResultCommunityPlaylist, ParseFrom, PlaylistItem};
use ytmapi_rs::query::{GetArtistQuery, GetPlaylistTracksQuery, PostMethod, PostQuery, Query, SearchQuery};
use ytmapi_rs::{ProcessedResult, YtMusic};

pub use ytmapi_rs::common::AlbumType as ReleaseKind;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemId {
    Song(String),
    Album(String),
    Artist(String),
    Playlist(String),
}

impl ItemId {
    pub fn url(&self) -> String {
        let (path, key, id) = match self {
            Self::Song(id) => ("watch", "v", id),
            Self::Playlist(id) => ("playlist", "list", id),
            Self::Artist(id) => return format!("https://music.youtube.com/channel/{}", encode(id)),
            Self::Album(id) => return format!("https://music.youtube.com/browse/{}", encode(id)),
        };
        format!("https://music.youtube.com/{path}?{key}={}", encode(id))
    }
}

pub fn open_browser(id: &ItemId) {
    let url = id.url();
    #[cfg(windows)]
    let _ = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}

pub fn valid_video_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
}

pub fn from_url(text: &str) -> Option<ItemId> {
    let url = reqwest::Url::parse(text.trim()).ok()?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str()?,
            "music.youtube.com" | "www.youtube.com" | "youtube.com"
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let (kind, value) = match url.path() {
        "/playlist" => ("list", url.query_pairs().find(|(k, _)| k == "list")?.1),
        "/watch" => ("song", url.query_pairs().find(|(k, _)| k == "v")?.1),
        path if path.starts_with("/browse/MPREb_") => {
            ("album", path.strip_prefix("/browse/")?.into())
        }
        path if path.starts_with("/channel/UC") => {
            ("artist", path.strip_prefix("/channel/")?.into())
        }
        _ => return None,
    };
    if value.is_empty()
        || value.len() > 160
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return None;
    }
    let value = value.into_owned();
    match kind {
        "list" => Some(ItemId::Playlist(value)),
        "song" if value.len() == 11 => Some(ItemId::Song(value)),
        "album" => Some(ItemId::Album(value)),
        "artist" => Some(ItemId::Artist(value)),
        _ => None,
    }
}

fn encode(value: &str) -> String {
    percent_encoding::utf8_percent_encode(value, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub title: String,
    pub by: String,
    pub image: Option<String>,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default)]
    pub album: String,
}

impl Item {
    pub fn cover_key(&self) -> String {
        self.image
            .as_deref()
            .map_or_else(|| self.id.url(), crate::library::cover_key)
    }

    pub fn from_track(track: &crate::library::Track) -> Option<Self> {
        Some(Self {
            id: ItemId::Song(track.youtube_id()?.into()),
            title: track.title.clone(),
            by: track.artist.clone(),
            image: track.art_url.clone(),
            duration_ms: track.duration_ms,
            album: track.album.clone(),
        })
    }

    fn with_recording(mut self, album: String, duration: &str) -> Self {
        self.album = album;
        self.duration_ms = duration_ms(duration);
        self
    }

    pub fn track(&self) -> Option<crate::library::Track> {
        let ItemId::Song(video_id) = &self.id else {
            return None;
        };
        if !valid_video_id(video_id) {
            return None;
        }
        let path = format!("yt:{video_id}");
        Some(crate::library::Track {
            id: crate::library::fnv(&path),
            path,
            title: self.title.clone(),
            artist: self.by.clone(),
            album: self.album.clone(),
            art_url: self.image.as_deref().and_then(image_url),
            duration_ms: self.duration_ms,
            album_artist: None,
            genre: None,
            year: None,
            track_no: None,
            disc_no: None,
            loudness_db: None,
            peak_db: None,
            added: 0,
            mtime: 0,
            lyrics: None,
            spotify_id: None,
            isrc: None,
        })
    }
}

fn duration_ms(text: &str) -> u64 {
    fn seconds(text: &str) -> Option<u64> {
        let (head, tail) = text.trim().rsplit_once(':')?;
        let tail: u64 = tail.parse().ok().filter(|&part| part < 60)?;
        let minutes = match head.split_once(':') {
            Some((hours, minutes)) => {
                let minutes: u64 = minutes.parse().ok().filter(|&part| part < 60)?;
                hours
                    .parse::<u64>()
                    .ok()?
                    .checked_mul(60)?
                    .checked_add(minutes)?
            }
            None => head.parse().ok()?,
        };
        minutes.checked_mul(60)?.checked_add(tail)
    }
    seconds(text)
        .and_then(|value| value.checked_mul(1000))
        .unwrap_or(0)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Page {
    pub title: String,
    pub items: Vec<Item>,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub artist_image: Option<String>,
    #[serde(default)]
    pub year: Option<String>,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub sections: Vec<HomeSection>,
    #[serde(default)]
    pub release_kinds: HashMap<String, ReleaseKind>,
    #[serde(default)]
    pub track_plays: HashMap<String, String>,
    #[serde(skip)]
    pub continuation: Option<String>,
}

#[derive(Clone, Debug)]
pub enum ProfileLookup<T> {
    Ready(T),
    Loading,
    Unavailable,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HomeSection {
    pub title: String,
    pub items: Vec<Item>,
}

impl Page {
    pub fn release_kind(&self, id: &ItemId) -> Option<&ReleaseKind> {
        let ItemId::Album(id) = id else { return None };
        self.release_kinds.get(id)
    }

    pub fn is_single_or_ep(&self, id: &ItemId) -> bool {
        if let Some(kind) = self.release_kind(id) {
            return matches!(kind, ReleaseKind::Single | ReleaseKind::EP);
        }
        self.sections.iter().any(|section| {
            matches!(section.title.trim().to_lowercase().as_str(),
                "singles" | "singles & eps" | "singles and eps" | "eps" | "ep"
                | "đĩa đơn" | "đĩa đơn và ep" | "đĩa đơn và đĩa ep")
                && section.items.iter().any(|item| &item.id == id)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Home(Option<String>),
    Search(String),
    Open(ItemId),
    Related(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FollowedPlaylist {
    pub id: String,
    pub snapshot: Page,
    #[serde(default)]
    pub added: u64,
}

impl FollowedPlaylist {
    pub fn item(&self) -> Item {
        let id = match self.id.strip_prefix("album:") {
            Some(album_id) => ItemId::Album(album_id.to_string()),
            None => ItemId::Playlist(self.id.clone()),
        };
        Item {
            id,
            title: self.snapshot.title.clone(),
            by: self.snapshot.subtitle.clone().unwrap_or_default(),
            image: self.snapshot.image.clone().or_else(|| {
                self.snapshot
                    .items
                    .first()
                    .and_then(|item| item.image.clone())
            }),
            duration_ms: 0,
            album: String::new(),
        }
    }

    pub fn refresh(&mut self, mut page: Page) {
        if page.title.is_empty() {
            page.title = self.snapshot.title.clone();
        }
        if page.image.is_none() {
            page.image.clone_from(&self.snapshot.image);
        }
        self.snapshot = page;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Network,
    Unavailable,
    Format,
}

impl From<ytmapi_rs::Error> for Error {
    fn from(error: ytmapi_rs::Error) -> Self {
        use ytmapi_rs::error::ErrorKind;
        match error.into_kind() {
            ErrorKind::Web { .. } => Self::Network,
            ErrorKind::Io(error) if error.kind() != std::io::ErrorKind::InvalidData => {
                Self::Network
            }
            ErrorKind::OtherErrorCodeInResponse {
                code: 401 | 403 | 404 | 410,
                ..
            }
            | ErrorKind::ApiStatusFailed => Self::Unavailable,
            _ => Self::Format,
        }
    }
}

pub struct Catalog {
    request: mpsc::Sender<(u64, Request)>,
    response: mpsc::Receiver<(u64, Result<Page, Error>)>,
}

impl Catalog {
    pub fn start(wake: impl Fn() + Send + 'static) -> std::io::Result<Self> {
        let (request, requests) = mpsc::channel();
        let (responses, response) = mpsc::channel();
        std::thread::Builder::new()
            .name("youtube-music".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let mut api = None;
                while let Ok(mut job) = requests.recv() {
                    while let Ok(latest) = requests.try_recv() {
                        job = latest;
                    }
                    let (id, request) = job;
                    let result = match &runtime {
                        Ok(runtime) => runtime.block_on(async {
                            tokio::time::timeout(Duration::from_secs(20), async {
                                if api.is_none() {
                                    api = Some(connect().await?);
                                }
                                query(api.as_ref().ok_or(Error::Network)?, request).await
                            })
                            .await
                            .unwrap_or(Err(Error::Network))
                        }),
                        Err(_) => Err(Error::Network),
                    };
                    if matches!(&result, Err(Error::Network)) {
                        api = None;
                    }
                    if responses.send((id, result)).is_err() {
                        break;
                    }
                    wake();
                }
            })?;
        Ok(Self { request, response })
    }

    pub fn send(&self, id: u64, request: Request) -> Result<(), Error> {
        self.request.send((id, request)).map_err(|_| Error::Network)
    }

    pub fn poll(&self) -> impl Iterator<Item = (u64, Result<Page, Error>)> + '_ {
        self.response.try_iter()
    }
}

async fn connect() -> Result<YtMusic<NoAuthToken>, Error> {
    let client = reqwest::Client::builder()
        .use_native_tls()
        .timeout(Duration::from_secs(8))
        .http2_max_header_list_size(64 * 1024)
        .build()
        .map_err(|_| Error::Network)?;
    ytmapi_rs::YtMusicBuilder::new_with_client(ytmapi_rs::Client::new_from_reqwest_client(client))
        .build()
        .await
        .map_err(Error::from)
}

async fn query(api: &YtMusic<NoAuthToken>, request: Request) -> Result<Page, Error> {
    match request {
        Request::Home(token) => home(api, token.as_deref()).await,
        Request::Search(text) => search(api, &text).await,
        Request::Open(ItemId::Album(id)) => album(api, &id).await,
        Request::Open(ItemId::Artist(id)) => artist(api, &id).await,
        Request::Open(ItemId::Playlist(id)) => playlist(api, &id).await,
        Request::Related(id) => Ok(api.query(Radio(&id)).await?),
        Request::Open(ItemId::Song(id)) => {
            let song = watch_song(api, &id).await.ok_or(Error::Unavailable)?;
            Ok(Page {
                title: song.title.clone(),
                items: vec![song],
                ..Page::default()
            })
        }
    }
}

async fn lyrics(api: &YtMusic<NoAuthToken>, id: &str) -> Option<crate::lrc::Lyrics> {
    let id = api.get_lyrics_id(VideoID::from_raw(id)).await.ok()?;
    let text = api.get_lyrics(id).await.ok()?.lyrics;
    let mut lyrics = crate::lrc::Lyrics::parse(&text);
    lyrics.provider = Some("YouTube Music");
    (!lyrics.lines.is_empty()).then_some(lyrics)
}

const INNERTUBE_API_KEY: &str = "AIzaSyAO_FJ2SlqU8Q4STEHLGCilw_Y9_11qcW8";
const UA_WEB: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const UA_VISIONOS: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 15_7_3) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";

#[derive(Clone, Debug)]
pub struct ResolvedStream {
    pub itag: Option<u32>,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub content_length: Option<u64>,
    pub expires_at: Option<Instant>,
}

pub fn google_media_url(raw: &str) -> Option<reqwest::Url> {
    let url = reqwest::Url::parse(raw).ok()?;
    let host = url.host_str()?;
    let ok_host = host == "googlevideo.com" || host.ends_with(".googlevideo.com");
    (url.scheme() == "https"
        && ok_host
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none())
    .then_some(url)
}

pub fn stream_http_agent() -> &'static reqwest::blocking::Client {
    static AGENT: OnceLock<reqwest::blocking::Client> = OnceLock::new();
    AGENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .expect("stream HTTP client")
    })
}

fn visitor_cache() -> &'static Mutex<Option<String>> {
    static CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisitorIdResponse {
    response_context: Option<VisitorContext>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct VisitorContext {
    visitor_data: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerResponse {
    playability_status: Option<PlayerPlayability>,
    streaming_data: Option<PlayerStreamingData>,
}

#[derive(Deserialize)]
struct PlayerPlayability {
    status: Option<String>,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerStreamingData {
    expires_in_seconds: Option<String>,
    #[serde(default)]
    formats: Vec<PlayerFormat>,
    #[serde(default)]
    adaptive_formats: Vec<PlayerFormat>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlayerFormat {
    itag: Option<u32>,
    url: Option<String>,
    mime_type: Option<String>,
    content_length: Option<String>,
}

fn fetch_visitor_data(agent: &reqwest::blocking::Client) -> Result<String, String> {
    if let Ok(vd) = fetch_visitor_id_api(agent) {
        return Ok(vd);
    }
    fetch_visitor_data_html(agent)
}

fn fetch_visitor_id_api(agent: &reqwest::blocking::Client) -> Result<String, String> {
    use std::io::Read;
    let body = json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20260708.00.00",
                "hl": "en",
                "gl": "VN"
            }
        }
    });
    let payload = serde_json::to_vec(&body).map_err(|e| format!("visitor_id encode failed: {e}"))?;
    let resp = agent
        .post(format!("https://www.youtube.com/youtubei/v1/visitor_id?key={INNERTUBE_API_KEY}"))
        .header("Content-Type", "application/json")
        .header("User-Agent", UA_WEB)
        .header("Origin", "https://www.youtube.com")
        .header("Referer", "https://www.youtube.com/")
        .body(payload)
        .send()
        .map_err(|e| format!("visitor_id request failed: {e}"))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("visitor_id HTTP {status}"));
    }
    let mut bytes = Vec::new();
    resp.take(1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("visitor_id read error: {e}"))?;
    let parsed: VisitorIdResponse =
        serde_json::from_slice(&bytes).map_err(|e| format!("visitor_id JSON error: {e}"))?;
    parsed
        .response_context
        .and_then(|c| c.visitor_data)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "visitor_id response missing visitorData".to_string())
}

fn fetch_visitor_data_html(agent: &reqwest::blocking::Client) -> Result<String, String> {
    use std::io::Read;
    let resp = agent
        .get("https://www.youtube.com/")
        .header("User-Agent", UA_WEB)
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .map_err(|e| format!("youtube.com HTML request failed: {e}"))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("youtube.com HTML HTTP {status}"));
    }
    let mut html = String::new();
    resp.take(8 * 1024 * 1024)
        .read_to_string(&mut html)
        .map_err(|e| format!("youtube.com HTML read failed: {e}"))?;
    extract_visitor_data_from_html(&html)
        .ok_or_else(|| "VISITOR_DATA not found in youtube.com HTML".to_string())
}

fn extract_visitor_data_from_html(html: &str) -> Option<String> {
    let key = "\"VISITOR_DATA\"";
    let idx = html.find(key)?;
    let rest = html[idx + key.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    let val = &rest[..end];
    (!val.is_empty()).then(|| val.to_string())
}

fn parse_visionos_player(parsed: PlayerResponse) -> Result<ResolvedStream, (String, bool)> {
    let status = parsed
        .playability_status
        .as_ref()
        .and_then(|p| p.status.as_deref())
        .unwrap_or("UNKNOWN");
    if status != "OK" {
        let reason = parsed
            .playability_status
            .as_ref()
            .and_then(|p| p.reason.as_deref())
            .unwrap_or("no reason");
        return Err((format!("YouTube {status}: {reason}"), status == "LOGIN_REQUIRED"));
    }

    let sd = parsed
        .streaming_data
        .ok_or_else(|| ("YouTube response missing streamingData".to_string(), false))?;
    let expires_at = sd
        .expires_in_seconds
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|secs| Instant::now() + Duration::from_secs(secs.saturating_sub(30)));

    let mut candidates: Vec<ResolvedStream> = sd
        .adaptive_formats
        .into_iter()
        .chain(sd.formats)
        .filter_map(|f| {
            let url = f.url.filter(|u| google_media_url(u).is_some())?;
            let mime = f.mime_type.as_deref()?;
            if !mime.starts_with("audio/mp4;") || !mime.contains("\"mp4a.40.2\"") {
                return None;
            }
            Some(ResolvedStream {
                itag: f.itag,
                url,
                headers: vec![("user-agent".to_string(), UA_VISIONOS.to_string())],
                content_length: f.content_length.as_deref().and_then(|s| s.parse().ok()),
                expires_at,
            })
        })
        .collect();
    candidates.sort_by_key(|f| f.itag != Some(140));
    candidates
        .into_iter()
        .next()
        .ok_or_else(|| ("no playable AAC-LC stream".to_string(), false))
}

fn resolve_visionos_once(
    agent: &reqwest::blocking::Client,
    video_id: &str,
    visitor_data: &str,
) -> Result<ResolvedStream, (String, bool)> {
    use std::io::Read;
    let body = json!({
        "context": {
            "client": {
                "clientName": "VISIONOS",
                "clientVersion": "1.02",
                "hl": "en",
                "gl": "VN",
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "visitorData": visitor_data
            }
        },
        "videoId": video_id,
        "contentCheckOk": true,
        "racyCheckOk": true
    });
    let payload =
        serde_json::to_vec(&body).map_err(|e| (format!("player encode failed: {e}"), false))?;
    let resp = agent
        .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
        .header("Content-Type", "application/json")
        .header("User-Agent", UA_VISIONOS)
        .header("X-Goog-Api-Key", INNERTUBE_API_KEY)
        .header("X-YouTube-Client-Name", "101")
        .header("X-YouTube-Client-Version", "1.02")
        .header("X-Goog-Visitor-Id", visitor_data)
        .header("Origin", "https://www.youtube.com")
        .header("Referer", "https://www.youtube.com/")
        .body(payload)
        .send()
        .map_err(|e| (format!("player request failed: {e}"), false))?;

    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err((format!("player HTTP {status}"), false));
    }
    let mut bytes = Vec::new();
    resp.take(8 * 1024 * 1024)
        .read_to_end(&mut bytes)
        .map_err(|e| (format!("player read error: {e}"), false))?;
    let parsed: PlayerResponse =
        serde_json::from_slice(&bytes).map_err(|e| (format!("player JSON error: {e}"), false))?;
    parse_visionos_player(parsed)
}

const STREAM_CACHE_TTL: Duration = Duration::from_secs(20 * 60);

fn stream_cache() -> &'static Mutex<HashMap<String, (Instant, ResolvedStream)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, ResolvedStream)>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn cached_stream(video_id: &str) -> Option<ResolvedStream> {
    let mut cache = stream_cache().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, stream)) = cache.get(video_id) {
        if at.elapsed() < STREAM_CACHE_TTL {
            return Some(stream.clone());
        }
        cache.remove(video_id);
    }
    None
}

fn cache_stream(video_id: &str, stream: &ResolvedStream) {
    let mut cache = stream_cache().lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() > 64 {
        cache.retain(|_, (at, _)| at.elapsed() < STREAM_CACHE_TTL);
    }
    cache.insert(video_id.to_string(), (Instant::now(), stream.clone()));
}

fn evict_cached_stream(video_id: &str) {
    let mut cache = stream_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.remove(video_id);
}

/// Resolves an unauthenticated AAC-LC audio stream via `VISIONOS/1.02` without logging URLs/tokens.
/// Reuses a bounded 20-minute TTL cache so preloaded/replayed tracks avoid redundant Innertube calls.
pub fn resolve_stream(video_id: &str) -> Result<ResolvedStream, String> {
    resolve_stream_inner(video_id, false)
}

/// Bypasses and evicts any cached stream URL for `video_id` (used after HTTP 401/403/410 or seek recovery).
pub fn resolve_stream_fresh(video_id: &str) -> Result<ResolvedStream, String> {
    resolve_stream_inner(video_id, true)
}

fn resolve_stream_inner(video_id: &str, force_fresh: bool) -> Result<ResolvedStream, String> {
    if !valid_video_id(video_id) {
        return Err("invalid YouTube video id".to_string());
    }
    if force_fresh {
        evict_cached_stream(video_id);
    } else if let Some(hit) = cached_stream(video_id) {
        return Ok(hit);
    }
    let agent = stream_http_agent();
    let cached_vd = if force_fresh {
        None
    } else {
        visitor_cache()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    };
    let vd = match cached_vd {
        Some(vd) => vd,
        None => {
            let fresh = fetch_visitor_data(agent)?;
            *visitor_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some(fresh.clone());
            fresh
        }
    };
    let resolved = match resolve_visionos_once(agent, video_id, &vd) {
        Ok(stream) => Ok(stream),
        Err((_, true)) => {
            let fresh = fetch_visitor_data(agent)?;
            *visitor_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some(fresh.clone());
            resolve_visionos_once(agent, video_id, &fresh).map_err(|(err, _)| err)
        }
        Err((err, false)) => Err(err),
    }?;
    cache_stream(video_id, &resolved);
    Ok(resolved)
}

/// Warms the stream URL cache in the background for an upcoming video ID (no-op if cached or in-flight).
pub fn prefetch_stream(video_id: &str) {
    if !valid_video_id(video_id) || cached_stream(video_id).is_some() {
        return;
    }
    static IN_FLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    let inflight = IN_FLIGHT.get_or_init(Mutex::default);
    {
        let mut set = inflight.lock().unwrap_or_else(|e| e.into_inner());
        if !set.insert(video_id.to_string()) {
            return;
        }
    }
    let vid = video_id.to_string();
    let _ = std::thread::Builder::new().name("ytm-prefetch".into()).spawn(move || {
        let _ = resolve_stream(&vid);
        if let Ok(mut set) = inflight.lock() {
            set.remove(&vid);
        }
    });
}

pub fn related_songs(video_id: &str) -> Result<Vec<Item>, Error> {
    if !valid_video_id(video_id) {
        return Err(Error::Unavailable);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::Network)?;
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(12), async {
            let api = connect().await?;
            let page = api.query(Radio(video_id)).await?;
            Ok(page.items)
        })
        .await
        .unwrap_or(Err(Error::Network))
    })
}

pub fn online_lyrics(track: &crate::library::Track) -> Option<crate::lrc::Lyrics> {
    let cfg = crate::spicy::SpicyApiConfig::from_settings(&crate::library::Settings::default());
    online_lyrics_with_config(track, &cfg)
}

/// Lyrics Provider Manager:
/// 1. Spicy Lyrics API first when enabled and authenticated/configured.
/// 2. Existing fallback providers in their preserved order (LyricsPlus v2 -> YouTube Music / LRCLIB).
pub fn online_lyrics_with_config(
    track: &crate::library::Track,
    spicy_config: &crate::spicy::SpicyApiConfig,
) -> Option<crate::lrc::Lyrics> {
    if spicy_config.is_eligible() {
        if let Ok(lyrics) = crate::spicy::fetch_for_track(track, spicy_config) {
            if !lyrics.lines.is_empty() {
                return Some(lyrics);
            }
        }
    }
    online_lyrics_fallbacks(track)
}

/// Testable provider-order coordinator: tries Spicy Lyrics first when eligible, then invokes `fallback`.
pub fn resolve_lyrics_provider_chain<F>(
    track: &crate::library::Track,
    spicy_config: &crate::spicy::SpicyApiConfig,
    spicy_state: &crate::spicy::SpicyClientState,
    fallback: F,
) -> Option<crate::lrc::Lyrics>
where
    F: FnOnce(&crate::library::Track) -> Option<crate::lrc::Lyrics>,
{
    if spicy_config.is_eligible() {
        if let Ok(lyrics) = crate::spicy::fetch_for_track_with_state(track, spicy_config, spicy_state)
        {
            if !lyrics.lines.is_empty() {
                return Some(lyrics);
            }
        }
    }
    fallback(track)
}

fn online_lyrics_fallbacks(track: &crate::library::Track) -> Option<crate::lrc::Lyrics> {
    static CACHE: OnceLock<Mutex<HashMap<String, crate::lrc::Lyrics>>> = OnceLock::new();
    let key = format!("{}|{}|{}|{}|{}", track.youtube_id().unwrap_or_default(), track.title, track.artist, track.album, track.duration_ms);
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(lyrics) = cache.lock().ok().and_then(|c| c.get(&key).cloned()) { return Some(lyrics); }
    let (tx, rx) = mpsc::channel::<(u8, Option<crate::lrc::Lyrics>)>();
    let word_track = track.clone();
    let word_tx = tx.clone();
    let _ = std::thread::Builder::new().name("lyrics-word-sync".into()).spawn(move || {
        let lyrics = crate::lrclib::fetch_word_synced(
            &word_track.title,
            &word_track.artist,
            &word_track.album,
            word_track.duration_ms,
        );
        let _ = word_tx.send((2, lyrics));
    });
    let youtube_track = track.clone();
    let youtube_tx = tx.clone();
    let _ = std::thread::Builder::new().name("lyrics-youtube".into()).spawn(move || {
        let _ = youtube_tx.send((1, youtube_lyrics(&youtube_track)));
    });
    let lrclib_track = track.clone();
    let _ = std::thread::Builder::new().name("lyrics-lrclib".into()).spawn(move || {
        let lyrics = crate::lrclib::fetch(&lrclib_track.title, &lrclib_track.artist, &lrclib_track.album, lrclib_track.duration_ms)
            .map(|text| {
                let mut l = crate::lrc::Lyrics::parse(&text);
                l.provider = Some("LRCLIB");
                l.source = Some("lrclib".to_string());
                l
            })
            .filter(|l| !l.lines.is_empty());
        let _ = tx.send((0, lyrics));
    });

    let deadline = Instant::now() + Duration::from_secs(8);
    let (mut word_synced, mut youtube, mut lrclib) = (None, None, None);
    let mut lp_done = false;
    for _ in 0..3 {
        let Ok((src, lyrics)) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) else { break };
        match src {
            2 => {
                word_synced = lyrics;
                lp_done = true;
                // Prioritize LyricsPlus v2: return immediately whenever LyricsPlus succeeds!
                if word_synced.as_ref().is_some_and(|l| l.synced) {
                    break;
                }
            }
            1 => youtube = lyrics,
            _ => lrclib = lyrics,
        }
        // Only short-circuit on fallback sources once LyricsPlus v2 has already finished with no match.
        if lp_done && (youtube.as_ref().is_some_and(|l| l.synced) || lrclib.as_ref().is_some_and(|l| l.synced)) {
            break;
        }
    }
    let result = if word_synced.as_ref().is_some_and(|l| l.synced) {
        word_synced
    } else {
        choose_lyrics(youtube, lrclib).or(word_synced)
    };
    if let Some(lyrics) = &result {
        if let Ok(mut cache) = cache.lock() {
            if cache.len() >= 64 { if let Some(key) = cache.keys().next().cloned() { cache.remove(&key); } }
            cache.insert(key, lyrics.clone());
        }
    }
    result
}

#[cfg(test)]
fn collect_lyrics(rx: mpsc::Receiver<(bool, Option<crate::lrc::Lyrics>)>, timeout: Duration)
    -> (Option<crate::lrc::Lyrics>, Option<crate::lrc::Lyrics>) {
    let deadline = Instant::now() + timeout;
    let (mut youtube, mut lrclib) = (None, None);
    for _ in 0..2 {
        let Ok((is_youtube, lyrics)) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) else { break };
        let synced = lyrics.as_ref().is_some_and(|l| l.synced);
        if is_youtube { youtube = lyrics; } else { lrclib = lyrics; }
        if synced { break; }
    }
    (youtube, lrclib)
}

fn youtube_lyrics(track: &crate::library::Track) -> Option<crate::lrc::Lyrics> {
    let mut recording = None;
    let youtube = track.youtube_id().and_then(|id| {
        let (youtube, details) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok()?
            .block_on(async {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(9);
                let api = tokio::time::timeout_at(deadline, connect())
                    .await
                    .ok()?
                    .ok()?;
                let (youtube, details) = futures_util::future::join(
                    tokio::time::timeout_at(deadline, lyrics(&api, id)),
                    tokio::time::timeout_at(deadline, async {
                        if track.duration_ms > 0 && !track.artist.is_empty() {
                            return None;
                        }
                        watch_song(&api, id).await
                    }),
                )
                .await;
                Some((youtube.ok().flatten(), details.ok().flatten()))
            })?;
        recording = details;
        youtube
    });
    if youtube.as_ref().is_some_and(|lyrics| lyrics.synced) {
        return youtube;
    }
    // Missing recording metadata must be hydrated before matching timed lyrics.
    if let Some(song) = recording {
        let lrclib = crate::lrclib::fetch(&song.title, &song.by, &track.album, song.duration_ms)
            .map(|text| {
                let mut l = crate::lrc::Lyrics::parse(&text);
                l.provider = Some("LRCLIB");
                l
            })
            .filter(|l| !l.lines.is_empty());
        return choose_lyrics(youtube, lrclib);
    }
    youtube
}

async fn watch_song(api: &YtMusic<NoAuthToken>, id: &str) -> Option<Item> {
    let tracks = api
        .get_watch_playlist_from_video_id(VideoID::from_raw(id))
        .await
        .ok()?;
    let exact = tracks
        .into_iter()
        .find(|track| track.video_id.get_raw() == id)?;
    if exact.title.is_empty() || exact.author.is_empty() {
        return None;
    }
    Some(
        item(
            ItemId::Song(id.to_owned()),
            exact.title,
            exact.author,
            &exact.thumbnails,
        )
        .with_recording(String::new(), &exact.duration),
    )
}

fn choose_lyrics(
    youtube: Option<crate::lrc::Lyrics>,
    lrclib: Option<crate::lrc::Lyrics>,
) -> Option<crate::lrc::Lyrics> {
    if lrclib.as_ref().is_some_and(|lyrics| lyrics.synced)
        && youtube.as_ref().is_none_or(|lyrics| !lyrics.synced)
    {
        lrclib
    } else {
        youtube.or(lrclib)
    }
}

fn item(
    id: ItemId,
    title: String,
    by: String,
    thumbnails: &[ytmapi_rs::common::Thumbnail],
) -> Item {
    let image = thumbnails
        .iter()
        .rev()
        .find_map(|thumb| image_url(&thumb.url));
    Item {
        id,
        title,
        by,
        image,
        duration_ms: 0,
        album: String::new(),
    }
}

fn image_url(url: &str) -> Option<String> {
    let url = reqwest::Url::parse(url).ok()?;
    let host = url.host_str()?;
    (url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && ["googleusercontent.com", "ggpht.com", "ytimg.com"]
            .iter()
            .any(|&domain| {
                host == domain
                    || host
                        .strip_suffix(domain)
                        .is_some_and(|prefix| prefix.ends_with('.'))
            }))
    .then(|| url.into())
}

fn parse_search_top_card(json: &Value, release_kinds: &mut HashMap<String, ReleaseKind>) -> Option<Item> {
    let sections = json
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")?
        .as_array()?;
    let card = sections.iter().find_map(|s| s.get("musicCardShelfRenderer"))?;
    let title = card
        .pointer("/title/runs/0/text")
        .and_then(Value::as_str)?
        .trim();
    if title.is_empty() {
        return None;
    }
    let nav = card
        .pointer("/title/runs/0/navigationEndpoint")
        .or_else(|| card.get("onTap"))?;
    let image = card
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find_map(|t| t["url"].as_str().and_then(image_url));
    let sub_runs: Vec<&str> = card
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|r| r["text"].as_str())
        .collect();

    if let Some(bid) = nav.pointer("/browseEndpoint/browseId").and_then(Value::as_str) {
        if bid.starts_with("MPREb_") {
            if let Some(kind) = sub_runs.iter().copied().find_map(release_kind_from_label) {
                release_kinds.insert(bid.to_string(), kind);
            }
            let by = sub_runs
                .iter()
                .copied()
                .map(str::trim)
                .filter(|s| {
                    !s.is_empty()
                        && *s != "•"
                        && release_kind_from_label(s).is_none()
                        && !(s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Some(Item {
                id: ItemId::Album(bid.into()),
                title: title.into(),
                by,
                image,
                duration_ms: 0,
                album: String::new(),
            });
        }
        if bid.starts_with("UC") {
            return Some(Item {
                id: ItemId::Artist(bid.into()),
                title: title.into(),
                by: String::new(),
                image,
                duration_ms: 0,
                album: String::new(),
            });
        }
        if let Some(pl_id) = bid.strip_prefix("VL") {
            return Some(Item {
                id: ItemId::Playlist(pl_id.into()),
                title: title.into(),
                by: String::new(),
                image,
                duration_ms: 0,
                album: String::new(),
            });
        }
    }
    if let Some(watch) = nav.get("watchEndpoint").filter(|w| is_song(w)) {
        if let Some(vid) = watch["videoId"].as_str().filter(|v| valid_video_id(v)) {
            let by = sub_runs
                .iter()
                .copied()
                .map(str::trim)
                .filter(|s| !s.is_empty() && *s != "•" && release_kind_from_label(s).is_none())
                .next()
                .unwrap_or_default()
                .to_string();
            let dur = sub_runs
                .iter()
                .rev()
                .copied()
                .map(duration_ms)
                .find(|&ms| ms > 0)
                .unwrap_or(0);
            return Some(Item {
                id: ItemId::Song(vid.into()),
                title: title.into(),
                by,
                image,
                duration_ms: dur,
                album: String::new(),
            });
        }
    }
    None
}

async fn search(api: &YtMusic<NoAuthToken>, text: &str) -> Result<Page, Error> {
    let basic_req = SearchQuery::<ytmapi_rs::query::search::BasicSearch>::from(text);
    let (basic_res, songs_res, albums_res, artists_res) = futures_util::future::join4(
        api.json_query::<SearchQuery<'_, ytmapi_rs::query::search::BasicSearch>>(&basic_req),
        api.search_songs(text),
        api.search_albums(text),
        api.search_artists(text),
    )
    .await;

    let mut release_kinds = HashMap::new();
    let mut top_card = None;
    let basic_parsed = if let Ok(raw_str) = basic_res {
        if let Ok(raw_val) = ytmapi_rs::json::from_json::<Value>(raw_str.clone()) {
            top_card = parse_search_top_card(&raw_val, &mut release_kinds);
            ytmapi_rs::parse::SearchResults::parse_from(ProcessedResult {
                query: &basic_req,
                source: raw_val.to_string(),
                json: raw_str,
            })
            .ok()
        } else {
            None
        }
    } else {
        None
    };

    let songs = songs_res.ok().or_else(|| basic_parsed.as_ref().map(|r| r.songs.clone()));
    if songs.is_none() && albums_res.is_err() && artists_res.is_err() && top_card.is_none() {
        return Err(Error::Network);
    }

    let mut items: Vec<_> = songs
        .unwrap_or_default()
        .into_iter()
        .map(|song| {
            item(
                ItemId::Song(song.video_id.get_raw().into()),
                song.title,
                song.artist,
                &song.thumbnails,
            )
            .with_recording(
                song.album.map(|album| album.name).unwrap_or_default(),
                &song.duration,
            )
        })
        .collect();

    let mut album_items = Vec::new();
    if let Ok(albums) = albums_res {
        for al in albums {
            release_kinds.insert(al.album_id.get_raw().to_string(), al.album_type);
            album_items.push(item(
                ItemId::Album(al.album_id.get_raw().into()),
                al.title,
                al.artist,
                &al.thumbnails,
            ));
        }
    }
    if let Some(ref basic) = basic_parsed {
        for al in &basic.albums {
            release_kinds.insert(al.album_id.get_raw().to_string(), al.album_type.clone());
            album_items.push(item(
                ItemId::Album(al.album_id.get_raw().into()),
                al.title.clone(),
                al.artist.clone(),
                &al.thumbnails,
            ));
        }
    }

    let mut artist_items = Vec::new();
    if let Ok(artists) = artists_res {
        for ar in artists {
            artist_items.push(item(
                ItemId::Artist(ar.browse_id.get_raw().into()),
                ar.artist,
                String::new(),
                &ar.thumbnails,
            ));
        }
    }
    if let Some(ref basic) = basic_parsed {
        for ar in &basic.artists {
            artist_items.push(item(
                ItemId::Artist(ar.browse_id.get_raw().into()),
                ar.artist.clone(),
                String::new(),
                &ar.thumbnails,
            ));
        }
    }

    // Crown YTM's `musicCardShelfRenderer` Top Result or an exact Artist / Album match at `items[0]`.
    let q_fold = crate::library::fold(text.trim());
    if let Some(card_item) = top_card {
        items.insert(0, card_item);
    } else if !q_fold.is_empty() {
        if let Some(exact_artist) = artist_items
            .iter()
            .find(|a| crate::library::fold(&a.title) == q_fold)
            .cloned()
        {
            items.insert(0, exact_artist);
        } else if let Some(exact_album) = album_items
            .iter()
            .find(|al| {
                crate::library::fold(&al.title) == q_fold
                    && matches!(
                        al.id,
                        ItemId::Album(ref bid)
                            if release_kinds.get(bid) == Some(&ReleaseKind::Album)
                    )
            })
            .cloned()
        {
            items.insert(0, exact_album);
        }
    }

    items.extend(album_items);
    items.extend(artist_items);
    if let Some(basic) = basic_parsed {
        items.extend(basic.featured_playlists.into_iter().map(|list| {
            item(
                ItemId::Playlist(list.playlist_id.get_raw().into()),
                list.title,
                list.author,
                &list.thumbnails,
            )
        }));
        items.extend(
            basic
                .community_playlists
                .into_iter()
                .filter_map(|result| match result {
                    BasicSearchResultCommunityPlaylist::Playlist(list) => Some(item(
                        ItemId::Playlist(list.playlist_id.get_raw().into()),
                        list.title,
                        list.author,
                        &list.thumbnails,
                    )),
                    _ => None,
                }),
        );
    }
    let mut seen = HashSet::new();
    items.retain(|item| seen.insert(item.id.clone()));
    Ok(Page {
        title: text.into(),
        items,
        release_kinds,
        ..Page::default()
    })
}

async fn album(api: &YtMusic<NoAuthToken>, id: &str) -> Result<Page, Error> {
    let album = api.get_album(AlbumID::from_raw(id)).await?;
    let by = album
        .artists
        .into_iter()
        .map(|artist| artist.name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut track_plays = HashMap::new();
    let items = album
        .tracks
        .into_iter()
        .map(|song| {
            let vid = song.video_id.get_raw().to_string();
            let plays_clean = song.plays.trim();
            if !plays_clean.is_empty() {
                track_plays.insert(vid.clone(), plays_clean.to_string());
            }
            item(
                ItemId::Song(vid),
                song.title,
                by.clone(),
                &album.thumbnails,
            )
            .with_recording(album.title.clone(), &song.duration)
        })
        .collect();
    let year = {
        let y = album.year.trim();
        (!y.is_empty()).then(|| y.to_string())
    };
    let artist_image = album
        .artist_thumbnails
        .iter()
        .rev()
        .find_map(|thumb| image_url(&thumb.url));
    Ok(Page {
        title: album.title,
        items,
        year,
        artist_image,
        subtitle: Some(by),
        description: album.description,
        release_kinds: HashMap::from([(id.to_string(), album.category)]),
        track_plays,
        image: album
            .thumbnails
            .iter()
            .rev()
            .find_map(|thumb| image_url(&thumb.url)),
        ..Page::default()
    })
}

async fn artist(api: &YtMusic<NoAuthToken>, id: &str) -> Result<Page, Error> {
    if let Some(name) = id.strip_prefix("name:") {
        return Box::pin(artist_by_name(api, name)).await;
    }
    let request = GetArtistQuery::new(ArtistChannelID::from_raw(id));
    let response = api.json_query::<GetArtistQuery<'_>>(&request).await?;
    let raw: Value = ytmapi_rs::json::from_json(response.clone())?;
    let parsed = ytmapi_rs::parse::GetArtist::parse_from(ProcessedResult {
        query: &request,
        source: raw.to_string(),
        json: response,
    });

    let mut page = match parsed {
        Ok(artist) => {
            let name = artist.name.clone();
            let mut items = Vec::new();
            let mut seen = HashSet::new();
            let mut release_kinds = HashMap::new();

            if let Some(songs) = artist.top_releases.songs {
                for song in songs.results {
                    let vid = song.video_id.get_raw().to_string();
                    let iid = ItemId::Song(vid.clone());
                    if !seen.insert(iid.clone()) {
                        continue;
                    }
                    let by = if song.artists.is_empty() {
                        name.clone()
                    } else {
                        song.artists
                            .into_iter()
                            .map(|a| a.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    items.push(Item {
                        id: iid,
                        title: song.title,
                        by,
                        image: Some(format!("https://i.ytimg.com/vi/{vid}/hqdefault.jpg")),
                        duration_ms: 0,
                        album: song.album.name,
                    });
                }
            }

            for release in artist
                .top_releases
                .albums
                .into_iter()
                .flat_map(|a| a.results)
                .chain(artist.top_releases.singles.into_iter().flat_map(|s| s.results))
            {
                let id = release.album_id.get_raw().to_string();
                #[allow(deprecated)]
                if let Some(kind) = release.album_type {
                    release_kinds.insert(id.clone(), kind);
                }
                let iid = ItemId::Album(id);
                if !seen.insert(iid.clone()) {
                    continue;
                }
                items.push(item(iid, release.title, release.year, &release.thumbnails));
            }

            if let Some(related) = artist.top_releases.related {
                for rel in related.results {
                    let iid = ItemId::Artist(rel.browse_id.get_raw().into());
                    if !seen.insert(iid.clone()) {
                        continue;
                    }
                    items.push(Item {
                        id: iid,
                        title: rel.title,
                        by: rel.subscribers,
                        image: None,
                        duration_ms: 0,
                        album: String::new(),
                    });
                }
            }

            Page {
                title: artist.name,
                items,
                release_kinds,
                image: artist
                    .thumbnails
                    .iter()
                    .rev()
                    .find_map(|thumb| image_url(&thumb.url)),
                subtitle: artist.views.or(artist.subscribers),
                description: artist.description,
                ..Page::default()
            }
        }
        Err(_) => parse_artist_raw(&raw),
    };

    enrich_artist_from_raw(&mut page, &raw);
    append_artist_release_shelves(&mut page, &raw);
    if page.title.is_empty() && page.items.is_empty() {
        return Err(Error::Unavailable);
    }
    Ok(page)
}

fn parse_artist_raw(json: &Value) -> Page {
    let hdr = json
        .pointer("/header/musicImmersiveHeaderRenderer")
        .or_else(|| json.pointer("/header/musicVisualHeaderRenderer"));
    let title = hdr
        .and_then(|h| h.pointer("/title/runs/0/text"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let image = hdr
        .and_then(|h| h.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find_map(|t| t["url"].as_str().and_then(image_url));
    let description = hdr
        .and_then(|h| h.pointer("/description/runs/0/text"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let subtitle = hdr
        .and_then(|h| h.pointer("/subscriptionButton/subscribeButtonRenderer/subscriberCountText/runs/0/text"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    Page {
        title,
        image,
        subtitle,
        description,
        ..Page::default()
    }
}

fn enrich_artist_from_raw(page: &mut Page, json: &Value) {
    let contents = json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents");
    let mut seen: HashSet<_> = page.items.iter().map(|i| i.id.clone()).collect();
    for sec in contents.and_then(Value::as_array).into_iter().flatten() {
        if let Some(shelf) = sec.get("musicShelfRenderer") {
            for entry in shelf["contents"].as_array().into_iter().flatten() {
                let Some(row) = entry.get("musicResponsiveListItemRenderer") else { continue };
                let Some(parsed_song) = home_song(row) else { continue };
                if let ItemId::Song(ref vid) = parsed_song.id {
                    // Extract play count from flexColumns[2] if present
                    if let Some(plays) = row
                        .pointer("/flexColumns/2/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
                        .and_then(Value::as_str)
                        .filter(|s| s.chars().any(|c| c.is_ascii_digit()))
                    {
                        page.track_plays.insert(vid.clone(), plays.to_string());
                    }
                }
                if let Some(existing) = page.items.iter_mut().find(|i| i.id == parsed_song.id) {
                    if parsed_song.image.is_some() {
                        existing.image = parsed_song.image;
                    }
                    if parsed_song.duration_ms > 0 {
                        existing.duration_ms = parsed_song.duration_ms;
                    }
                    if existing.album.is_empty() && !parsed_song.album.is_empty() {
                        existing.album = parsed_song.album;
                    }
                } else if seen.insert(parsed_song.id.clone()) {
                    page.items.push(parsed_song);
                }
            }
        }
        if let Some(carousel) = sec.get("musicCarouselShelfRenderer") {
            for card in carousel["contents"].as_array().into_iter().flatten() {
                let card = &card["musicTwoRowItemRenderer"];
                let Some(bid) = card
                    .pointer("/navigationEndpoint/browseEndpoint/browseId")
                    .or_else(|| card.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId"))
                    .and_then(Value::as_str)
                else {
                    continue;
                };
                if !bid.starts_with("UC") {
                    continue;
                }
                let thumb = card
                    .pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .rev()
                    .find_map(|t| t["url"].as_str().and_then(image_url));
                let target_id = ItemId::Artist(bid.into());
                if let Some(existing) = page.items.iter_mut().find(|i| i.id == target_id) {
                    if existing.image.is_none() {
                        existing.image = thumb;
                    }
                } else if let Some(rel_title) = card.pointer("/title/runs/0/text").and_then(Value::as_str) {
                    if seen.insert(target_id.clone()) {
                        let sub: String = card
                            .pointer("/subtitle/runs")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|r| r["text"].as_str())
                            .collect();
                        page.items.push(Item {
                            id: target_id,
                            title: rel_title.into(),
                            by: sub,
                            image: thumb,
                            duration_ms: 0,
                            album: String::new(),
                        });
                    }
                }
            }
        }
    }
}

fn append_artist_release_shelves(page: &mut Page, json: &Value) {
    let mut seen: HashSet<_> = page.items.iter().map(|item| item.id.clone()).collect();
    let contents = json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents");
    for shelf in contents.and_then(Value::as_array).into_iter().flatten() {
        let shelf = &shelf["musicCarouselShelfRenderer"];
        let mut section = HomeSection {
            title: shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title/runs")
                .and_then(Value::as_array).into_iter().flatten()
                .filter_map(|run| run["text"].as_str()).collect(),
            items: Vec::new(),
        };
        let mut section_seen = HashSet::new();
        for card in shelf["contents"].as_array().into_iter().flatten() {
            let card = &card["musicTwoRowItemRenderer"];
            let Some(id) = card.pointer("/navigationEndpoint/browseEndpoint/browseId")
                .or_else(|| card.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId"))
                .and_then(Value::as_str).filter(|id| id.starts_with("MPREb_")) else { continue };
            let Some(title) = card.pointer("/title/runs/0/text").and_then(Value::as_str) else { continue };
            let id = ItemId::Album(id.into());
            if !section_seen.insert(id.clone()) {
                continue;
            }
            let subtitle: Vec<_> = card.pointer("/subtitle/runs")
                .and_then(Value::as_array).into_iter().flatten()
                .filter_map(|run| run["text"].as_str()).collect();
            let shelf_is_albums = matches!(
                section.title.trim().to_lowercase().as_str(),
                "albums" | "album" | "đĩa nhạc"
            );
            if let Some(kind) = subtitle
                .iter()
                .find_map(|label| release_kind_from_label(label))
                .or_else(|| shelf_is_albums.then_some(ReleaseKind::Album))
            {
                if let ItemId::Album(id) = &id {
                    page.release_kinds.insert(id.clone(), kind);
                }
            }
            let item = Item {
                id,
                title: title.into(),
                by: subtitle.iter().map(|text| text.trim())
                    .find(|text| text.len() == 4 && text.bytes().all(|b| b.is_ascii_digit()))
                    .unwrap_or_default().into(),
                image: card.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
                    .and_then(Value::as_array).into_iter().flatten().rev()
                    .find_map(|thumb| thumb["url"].as_str().and_then(image_url)),
                duration_ms: 0,
                album: String::new(),
            };
            if seen.insert(item.id.clone()) {
                page.items.push(item.clone());
            }
            section.items.push(item);
        }
        if !section.items.is_empty() {
            page.sections.push(section);
        }
    }
}

fn extract_artist_candidates_from_search_json(json: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(secs) = json
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array)
    else {
        return out;
    };
    for sec in secs {
        if let Some(card) = sec.get("musicCardShelfRenderer") {
            if let (Some(t), Some(bid)) = (
                card.pointer("/title/runs/0/text").and_then(Value::as_str),
                card.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
                    .and_then(Value::as_str),
            ) {
                if bid.starts_with("UC") {
                    out.push((t.to_string(), bid.to_string()));
                }
            }
            for run in card
                .pointer("/subtitle/runs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let (Some(t), Some(bid)) = (
                    run["text"].as_str(),
                    run.pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(Value::as_str),
                ) {
                    if bid.starts_with("UC") {
                        out.push((t.to_string(), bid.to_string()));
                    }
                }
            }
        }
        if let Some(shelf) = sec.get("musicShelfRenderer") {
            for row in shelf["contents"].as_array().into_iter().flatten() {
                let it = &row["musicResponsiveListItemRenderer"];
                if let (Some(t), Some(bid)) = (
                    it.pointer("/flexColumns/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
                        .and_then(Value::as_str),
                    it.pointer("/navigationEndpoint/browseEndpoint/browseId")
                        .and_then(Value::as_str),
                ) {
                    if bid.starts_with("UC") {
                        out.push((t.to_string(), bid.to_string()));
                    }
                }
                for col in it["flexColumns"].as_array().into_iter().flatten() {
                    for run in col
                        .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        if let (Some(t), Some(bid)) = (
                            run["text"].as_str(),
                            run.pointer("/navigationEndpoint/browseEndpoint/browseId")
                                .and_then(Value::as_str),
                        ) {
                            if bid.starts_with("UC") {
                                out.push((t.to_string(), bid.to_string()));
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

async fn artist_by_name(api: &YtMusic<NoAuthToken>, name: &str) -> Result<Page, Error> {
    let clean = name.trim();
    if clean.is_empty() {
        return Err(Error::Unavailable);
    }
    let basic_req = SearchQuery::<ytmapi_rs::query::search::BasicSearch>::from(clean);
    let (basic_res, artists_res) = futures_util::future::join(
        api.json_query::<SearchQuery<'_, ytmapi_rs::query::search::BasicSearch>>(&basic_req),
        api.search_artists(clean),
    )
    .await;

    let mut candidates: Vec<(String, String)> = Vec::new();
    if let Ok(artists) = artists_res {
        for a in artists {
            candidates.push((a.artist, a.browse_id.get_raw().to_string()));
        }
    }
    if let Ok(raw_str) = basic_res {
        if let Ok(raw_val) = ytmapi_rs::json::from_json::<Value>(raw_str) {
            candidates.extend(extract_artist_candidates_from_search_json(&raw_val));
        }
    }

    let norm = |s: &str| {
        crate::library::fold(s)
            .trim()
            .trim_end_matches('.')
            .trim()
            .to_string()
    };
    let wanted = norm(clean);
    let best_id = candidates
        .iter()
        .find(|(n, _)| norm(n) == wanted)
        .or_else(|| {
            candidates.iter().find(|(n, _)| {
                let cn = norm(n);
                !cn.is_empty() && (cn.contains(&wanted) || wanted.contains(&cn))
            })
        })
        .or_else(|| candidates.first())
        .map(|(_, id)| id.clone())
        .ok_or(Error::Unavailable)?;

    artist(api, &best_id).await
}

struct ArtistProfileCache {
    entries: Mutex<HashMap<String, (Option<Page>, Instant)>>,
    inflight: Mutex<HashSet<String>>,
    tx: mpsc::Sender<String>,
    wake_cb: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn artist_cache() -> &'static ArtistProfileCache {
    static CACHE: OnceLock<ArtistProfileCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<String>();
        let entries = Mutex::new(HashMap::new());
        let inflight = Mutex::new(HashSet::new());
        std::thread::Builder::new()
            .name("ytm-artist-cache".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let mut api = None;
                while let Ok(name) = rx.recv() {
                    let key = crate::library::fold(name.trim());
                    let result = match &runtime {
                        Ok(rt) => rt.block_on(async {
                            tokio::time::timeout(Duration::from_secs(12), async {
                                if api.is_none() {
                                    api = Some(connect().await?);
                                }
                                artist_by_name(api.as_ref().ok_or(Error::Network)?, &name).await
                            })
                            .await
                            .ok()
                            .and_then(Result::ok)
                        }),
                        Err(_) => None,
                    };
                    if result.is_none() {
                        api = None;
                    }
                    let cache = artist_cache();
                    if let Ok(mut map) = cache.entries.lock() {
                        map.insert(key.clone(), (result, Instant::now()));
                    }
                    if let Ok(mut set) = cache.inflight.lock() {
                        set.remove(&key);
                    }
                    if let Ok(guard) = cache.wake_cb.lock() {
                        if let Some(cb) = guard.as_ref() {
                            cb();
                        }
                    }
                }
            })
            .ok();
        ArtistProfileCache {
            entries,
            inflight,
            tx,
            wake_cb: Mutex::new(None),
        }
    })
}

pub fn artist_profile_state(
    name: &str,
    wake: impl Fn() + Send + 'static,
) -> ProfileLookup<Page> {
    let clean = name.trim();
    let key = crate::library::fold(clean);
    if key.is_empty() {
        return ProfileLookup::Unavailable;
    }
    let cache = artist_cache();
    if let Ok(map) = cache.entries.lock() {
        if let Some((entry, at)) = map.get(&key) {
            if let Some(page) = entry {
                return ProfileLookup::Ready(page.clone());
            }
            if at.elapsed() < Duration::from_secs(6) {
                return ProfileLookup::Unavailable;
            }
        }
    }
    if let Ok(mut guard) = cache.wake_cb.lock() {
        *guard = Some(Box::new(wake));
    }
    if let Ok(mut set) = cache.inflight.lock() {
        if set.insert(key) {
            let _ = cache.tx.send(clean.to_string());
        }
    }
    ProfileLookup::Loading
}

/// Returns cached YouTube Music artist profile (photo, views, bio, top songs & releases),
/// scheduling a background fetch on first request without disturbing the main page browser.
pub fn artist_profile_cached(name: &str, wake: impl Fn() + Send + 'static) -> Option<Page> {
    match artist_profile_state(name, wake) {
        ProfileLookup::Ready(page) => Some(page),
        _ => None,
    }
}

async fn album_by_title_artist(
    api: &YtMusic<NoAuthToken>,
    title: &str,
    artist_name: &str,
) -> Result<(String, Page), Error> {
    let query = format!("{} {}", title.trim(), artist_name.trim());
    let basic_req = SearchQuery::<ytmapi_rs::query::search::BasicSearch>::from(query.trim());
    let (albums_res, basic_res) = futures_util::future::join(
        api.search_albums(query.trim()),
        api.json_query::<SearchQuery<'_, ytmapi_rs::query::search::BasicSearch>>(&basic_req),
    )
    .await;

    let wanted_title = crate::library::fold(title.trim());
    let primary = crate::library::split_artists(artist_name)
        .into_iter()
        .next()
        .unwrap_or_else(|| artist_name.to_string());
    let wanted_artist = crate::library::fold(primary.trim());

    let mut chosen_id: Option<String> = None;
    if let Ok(albums) = albums_res {
        let matches_meta = |al: &ytmapi_rs::parse::SearchResultAlbum| {
            let t = crate::library::fold(&al.title);
            let a = crate::library::fold(&al.artist);
            (t == wanted_title || t.contains(&wanted_title) || wanted_title.contains(&t))
                && (wanted_artist.is_empty() || a.contains(&wanted_artist) || wanted_artist.contains(&a))
        };
        chosen_id = albums
            .iter()
            .find(|al| crate::library::fold(&al.title) == wanted_title && matches_meta(al) && al.album_type == ReleaseKind::Album)
            .or_else(|| albums.iter().find(|al| matches_meta(al) && al.album_type == ReleaseKind::Album))
            .or_else(|| albums.iter().find(|al| matches_meta(al)))
            .map(|al| al.album_id.get_raw().to_string());
    }
    if chosen_id.is_none() {
        if let Ok(raw_str) = basic_res {
            if let Ok(raw_val) = ytmapi_rs::json::from_json::<Value>(raw_str) {
                let mut kinds = HashMap::new();
                if let Some(Item { id: ItemId::Album(bid), .. }) = parse_search_top_card(&raw_val, &mut kinds) {
                    chosen_id = Some(bid);
                }
            }
        }
    }
    let id = chosen_id.ok_or(Error::Unavailable)?;
    let page = album(api, &id).await?;
    Ok((id, page))
}

struct AlbumProfileCache {
    entries: Mutex<HashMap<String, (Option<(String, Page)>, Instant)>>,
    inflight: Mutex<HashSet<String>>,
    tx: mpsc::Sender<(String, String)>,
    wake_cb: Mutex<Option<Box<dyn Fn() + Send>>>,
}

fn album_cache() -> &'static AlbumProfileCache {
    static CACHE: OnceLock<AlbumProfileCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<(String, String)>();
        let entries = Mutex::new(HashMap::new());
        let inflight = Mutex::new(HashSet::new());
        std::thread::Builder::new()
            .name("ytm-album-cache".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let mut api = None;
                while let Ok((title, artist_name)) = rx.recv() {
                    let key = format!(
                        "{}::{}",
                        crate::library::fold(&artist_name),
                        crate::library::fold(&title)
                    );
                    let result = match &runtime {
                        Ok(rt) => rt.block_on(async {
                            tokio::time::timeout(Duration::from_secs(12), async {
                                if api.is_none() {
                                    api = Some(connect().await?);
                                }
                                album_by_title_artist(
                                    api.as_ref().ok_or(Error::Network)?,
                                    &title,
                                    &artist_name,
                                )
                                .await
                            })
                            .await
                            .ok()
                            .and_then(Result::ok)
                        }),
                        Err(_) => None,
                    };
                    if result.is_none() {
                        api = None;
                    }
                    let cache = album_cache();
                    if let Ok(mut map) = cache.entries.lock() {
                        map.insert(key.clone(), (result, Instant::now()));
                    }
                    if let Ok(mut set) = cache.inflight.lock() {
                        set.remove(&key);
                    }
                    if let Ok(guard) = cache.wake_cb.lock() {
                        if let Some(cb) = guard.as_ref() {
                            cb();
                        }
                    }
                }
            })
            .ok();
        AlbumProfileCache {
            entries,
            inflight,
            tx,
            wake_cb: Mutex::new(None),
        }
    })
}

pub fn album_profile_state(
    title: &str,
    artist_name: &str,
    wake: impl Fn() + Send + 'static,
) -> ProfileLookup<(String, Page)> {
    let t = title.trim();
    if t.is_empty() {
        return ProfileLookup::Unavailable;
    }
    let key = format!(
        "{}::{}",
        crate::library::fold(artist_name.trim()),
        crate::library::fold(t)
    );
    let cache = album_cache();
    if let Ok(map) = cache.entries.lock() {
        if let Some((entry, at)) = map.get(&key) {
            if let Some(found) = entry {
                return ProfileLookup::Ready(found.clone());
            }
            if at.elapsed() < Duration::from_secs(6) {
                return ProfileLookup::Unavailable;
            }
        }
    }
    if let Ok(mut guard) = cache.wake_cb.lock() {
        *guard = Some(Box::new(wake));
    }
    if let Ok(mut set) = cache.inflight.lock() {
        if set.insert(key) {
            let _ = cache.tx.send((t.to_string(), artist_name.trim().to_string()));
        }
    }
    ProfileLookup::Loading
}

fn extract_mpreb_from_playlist_json(json: &Value) -> Option<String> {
    let rows = json
        .pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents")?
        .as_array()?;
    for row in rows {
        let cols = row
            .pointer("/musicResponsiveListItemRenderer/flexColumns")
            .and_then(Value::as_array);
        for col in cols.into_iter().flatten() {
            for run in col
                .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(bid) = run
                    .pointer("/navigationEndpoint/browseEndpoint/browseId")
                    .and_then(Value::as_str)
                    .filter(|s| s.starts_with("MPREb_"))
                {
                    return Some(bid.to_string());
                }
            }
        }
    }
    None
}

async fn playlist(api: &YtMusic<NoAuthToken>, id: &str) -> Result<Page, Error> {
    let id = id.strip_prefix("VL").unwrap_or(id);
    if id.starts_with("OLAK5uy_") {
        if let Ok(raw_str) = api.json_query::<PlaylistBrowse<'_>>(&PlaylistBrowse { id, token: None }).await {
            if let Ok(raw_val) = ytmapi_rs::json::from_json::<Value>(raw_str) {
                if let Some(mpreb) = extract_mpreb_from_playlist_json(&raw_val) {
                    if let Ok(album_page) = album(api, &mpreb).await {
                        return Ok(album_page);
                    }
                }
            }
        }
    }
    match typed_playlist(api, id).await {
        Err(Error::Format) => raw_playlist(api, id).await,
        result => result,
    }
}

async fn typed_playlist(api: &YtMusic<NoAuthToken>, id: &str) -> Result<Page, Error> {
    use futures_util::{pin_mut, StreamExt};
    let id = PlaylistID::from_raw(id);
    let info = api.get_playlist_details(&id).await?;
    if matches!(
        info.privacy,
        Some(ytmapi_rs::query::playlist::PrivacyStatus::Private)
    ) {
        return Err(Error::Unavailable);
    }
    let query = GetPlaylistTracksQuery::new(id);
    let pages = api.stream(&query);
    pin_mut!(pages);
    let mut items = Vec::new();
    while let Some(page) = pages.next().await {
        for entry in page? {
            if let PlaylistItem::Song(song) = entry {
                items.push(
                    item(
                        ItemId::Song(song.video_id.get_raw().into()),
                        song.title,
                        song.artists
                            .into_iter()
                            .map(|artist| artist.name)
                            .collect::<Vec<_>>()
                            .join(", "),
                        &song.thumbnails,
                    )
                    .with_recording(song.album.name, &song.duration),
                );
            }
        }
        if items.len() > 5_000 {
            return Err(Error::Format);
        }
    }
    Ok(Page {
        title: info.title,
        items,
        image: info
            .thumbnails
            .iter()
            .rev()
            .find_map(|thumb| image_url(&thumb.url)),
        ..Page::default()
    })
}

struct Home<'a>(Option<&'a str>);

struct PlaylistBrowse<'a> {
    id: &'a str,
    token: Option<&'a str>,
}

impl Query<NoAuthToken> for PlaylistBrowse<'_> {
    type Output = Page;
    type Method = PostMethod;
}

impl PostQuery for PlaylistBrowse<'_> {
    fn path(&self) -> &str {
        "browse"
    }

    fn header(&self) -> serde_json::Map<String, Value> {
        match self.token {
            Some(token) => serde_json::Map::from_iter([("continuation".into(), json!(token))]),
            None => {
                serde_json::Map::from_iter([("browseId".into(), json!(format!("VL{}", self.id)))])
            }
        }
    }

    fn params(&self) -> Vec<(&str, Cow<'_, str>)> {
        self.token
            .map(|token| {
                vec![
                    ("continuation", token.into()),
                    ("ctoken", token.into()),
                    ("type", "next".into()),
                ]
            })
            .unwrap_or_default()
    }
}

impl ParseFrom<PlaylistBrowse<'_>> for Page {
    fn parse_from(response: ProcessedResult<PlaylistBrowse<'_>>) -> ytmapi_rs::Result<Self> {
        let json = ytmapi_rs::json::from_json(response.json)?;
        parse_playlist(&json).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "missing public playlist shelf",
            )
            .into()
        })
    }
}

async fn raw_playlist(api: &YtMusic<NoAuthToken>, id: &str) -> Result<Page, Error> {
    let mut page = api.query(PlaylistBrowse { id, token: None }).await?;
    let mut used = HashSet::new();
    while let Some(token) = page.continuation.take() {
        if !used.insert(token.clone()) || used.len() > 50 {
            return Err(Error::Format);
        }
        let next = api
            .query(PlaylistBrowse {
                id,
                token: Some(&token),
            })
            .await?;
        page.items.extend(next.items);
        page.continuation = next.continuation;
        if page.items.len() > 5_000 {
            return Err(Error::Format);
        }
    }
    Ok(page)
}

fn parse_playlist(json: &Value) -> Option<Page> {
    let root = json.pointer("/contents/twoColumnBrowseResultsRenderer");
    let (rows, title, token) = if let Some(root) = root {
        let shelf = root.pointer(
            "/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer",
        )?;
        let title = root.pointer("/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer/title/runs/0/text")
            .and_then(Value::as_str).or_else(|| shelf.pointer("/title/runs/0/text")?.as_str());
        let token = shelf
            .pointer("/continuations/0/nextContinuationData/continuation")
            .and_then(Value::as_str);
        (shelf["contents"].as_array()?, title, token)
    } else if let Some(shelf) = json.pointer("/continuationContents/musicPlaylistShelfContinuation")
    {
        let token = shelf
            .pointer("/continuations/0/nextContinuationData/continuation")
            .and_then(Value::as_str);
        (shelf["contents"].as_array()?, None, token)
    } else {
        (
            json.pointer(
                "/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems",
            )?
            .as_array()?,
            None,
            None,
        )
    };
    let continuation = token
        .or_else(|| {
            rows.iter().find_map(|row| {
                row.pointer(
                    "/continuationItemRenderer/continuationEndpoint/continuationCommand/token",
                )?
                .as_str()
            })
        })
        .map(str::to_owned);
    let title = title.or_else(|| rows.iter().find_map(|row| {
        row.pointer("/musicResponsiveListItemRenderer/flexColumns/2/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")?.as_str()
    })).unwrap_or_default().to_owned();
    let items: Vec<Item> = rows
        .iter()
        .filter_map(|row| home_song(row.get("musicResponsiveListItemRenderer")?))
        .collect();
    let header = root.and_then(|r| {
        r.pointer("/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer")
    });
    let image = header
        .and_then(|h| h.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find_map(|t| t["url"].as_str().and_then(image_url))
        .or_else(|| items.first().and_then(|it| it.image.clone()));
    let subtitle = header
        .and_then(|h| h.pointer("/straplineTextOne/runs/0/text"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            items
                .first()
                .map(|it| it.by.clone())
                .filter(|s| !s.is_empty())
        });
    Some(Page {
        title,
        items,
        image,
        subtitle,
        continuation,
        ..Page::default()
    })
}

struct Radio<'a>(&'a str);

impl Query<NoAuthToken> for Radio<'_> {
    type Output = Page;
    type Method = PostMethod;
}

impl PostQuery for Radio<'_> {
    fn path(&self) -> &str {
        "next"
    }

    fn header(&self) -> serde_json::Map<String, Value> {
        serde_json::Map::from_iter([
            ("videoId".into(), json!(self.0)),
            ("playlistId".into(), json!(format!("RDAMVM{}", self.0))),
            ("isAudioOnly".into(), json!(true)),
            ("enablePersistentPlaylistPanel".into(), json!(true)),
        ])
    }

    fn params(&self) -> Vec<(&str, Cow<'_, str>)> {
        Vec::new()
    }
}

impl ParseFrom<Radio<'_>> for Page {
    fn parse_from(response: ProcessedResult<Radio<'_>>) -> ytmapi_rs::Result<Self> {
        let json: Value = ytmapi_rs::json::from_json(response.json)?;
        Ok(parse_radio(&json, response.query.0))
    }
}

fn parse_radio(json: &Value, seed: &str) -> Page {
    let contents = json.pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents");
    let mut seen = HashSet::from([seed]);
    let items = contents
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let row = entry.get("playlistPanelVideoRenderer")?;
            let watch = row.pointer("/navigationEndpoint/watchEndpoint")?;
            if !is_song(watch) {
                return None;
            }
            let id = row["videoId"]
                .as_str()
                .or_else(|| watch["videoId"].as_str())?;
            let title = row.pointer("/title/runs/0/text")?.as_str()?;
            if !seen.insert(id) {
                return None;
            }
            let by = row
                .pointer("/shortBylineText/runs/0/text")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let image = row
                .pointer("/thumbnail/thumbnails")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .rev()
                .find_map(|thumb| thumb["url"].as_str().and_then(image_url));
            Some(Item {
                id: ItemId::Song(id.into()),
                title: title.into(),
                by: by.into(),
                image,
                duration_ms: row
                    .pointer("/lengthText/runs/0/text")
                    .and_then(Value::as_str)
                    .map_or(0, duration_ms),
                album: String::new(),
            })
        })
        .collect();
    Page {
        items,
        ..Page::default()
    }
}

impl Query<NoAuthToken> for Home<'_> {
    type Output = Page;
    type Method = PostMethod;
}

impl PostQuery for Home<'_> {
    fn path(&self) -> &str {
        "browse"
    }

    fn header(&self) -> serde_json::Map<String, Value> {
        let mut body = serde_json::Map::from_iter([(
            "context".into(),
            json!({"client": {"clientName": "WEB_REMIX", "clientVersion": format!("1.{}.01.00", chrono::Utc::now().format("%Y%m%d")), "hl": "vi", "gl": "VN"}}),
        )]);
        match self.0 {
            Some(token) => {
                body.insert("continuation".into(), json!(token));
            }
            None => {
                body.insert("browseId".into(), json!("FEmusic_home"));
            }
        }
        body
    }

    fn params(&self) -> Vec<(&str, Cow<'_, str>)> {
        self.0
            .map(|token| {
                vec![
                    ("continuation", token.into()),
                    ("ctoken", token.into()),
                    ("type", "next".into()),
                ]
            })
            .unwrap_or_default()
    }
}

impl ParseFrom<Home<'_>> for Page {
    fn parse_from(response: ProcessedResult<Home<'_>>) -> ytmapi_rs::Result<Self> {
        let json: Value = ytmapi_rs::json::from_json(response.json)?;
        Ok(parse_home(&json))
    }
}

async fn home(api: &YtMusic<NoAuthToken>, token: Option<&str>) -> Result<Page, Error> {
    let page = api.query(Home(token)).await?;
    if page.items.is_empty() && page.continuation.is_none() && token.is_none() {
        return Err(Error::Format);
    }
    Ok(page)
}

fn parse_home(json: &Value) -> Page {
    let contents = json.pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer")
        .or_else(|| json.pointer("/continuationContents/sectionListContinuation"));
    let mut page = Page::default();
    let Some(contents) = contents else {
        return page;
    };
    let mut seen = HashSet::new();
    for shelf in contents["contents"].as_array().into_iter().flatten() {
        let shelf = &shelf["musicCarouselShelfRenderer"];
        let mut section = HomeSection {
            title: shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title/runs")
                .and_then(Value::as_array)
                .into_iter().flatten()
                .filter_map(|run| run["text"].as_str())
                .collect(),
            items: Vec::new(),
        };
        let mut section_seen = HashSet::new();
        for card in shelf["contents"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(song) = card
                .get("musicResponsiveListItemRenderer")
                .and_then(home_song)
            {
                if section_seen.insert(song.id.clone()) {
                    section.items.push(song.clone());
                }
                if seen.insert(song.id.clone()) {
                    page.items.push(song);
                }
                continue;
            }
            let Some(card) = card.get("musicTwoRowItemRenderer") else {
                continue;
            };
            let endpoint = &card["navigationEndpoint"];
            let id = match endpoint
                .pointer("/browseEndpoint/browseId")
                .and_then(Value::as_str)
            {
                Some(id) if id.starts_with("MPREb_") => ItemId::Album(id.into()),
                Some(id) if id.starts_with("UC") => ItemId::Artist(id.into()),
                Some(id) if id.starts_with("VL") => ItemId::Playlist(id[2..].into()),
                _ => continue,
            };
            let Some(title) = card.pointer("/title/runs/0/text").and_then(Value::as_str) else {
                continue;
            };
            if !section_seen.insert(id.clone()) {
                continue;
            }
            if let ItemId::Album(id) = &id {
                if let Some(kind) = card.pointer("/subtitle/runs")
                    .and_then(Value::as_array)
                    .into_iter().flatten()
                    .filter_map(|run| run["text"].as_str())
                    .find_map(release_kind_from_label)
                {
                    page.release_kinds.insert(id.clone(), kind);
                }
            }
            let by = card
                .pointer("/subtitle/runs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|run| run["text"].as_str())
                .collect::<String>();
            let by = if matches!(id, ItemId::Album(_)) {
                by.split_once(" • ")
                    .filter(|(kind, _)| release_kind_from_label(kind).is_some())
                    .map_or_else(|| by.clone(), |(_, by)| by.to_string())
            } else {
                by
            };
            let image = card
                .pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .rev()
                .find_map(|thumb| thumb["url"].as_str().and_then(image_url));
            let item = Item {
                id,
                title: title.into(),
                by,
                image,
                duration_ms: 0,
                album: String::new(),
            };
            if seen.insert(item.id.clone()) {
                page.items.push(item.clone());
            }
            section.items.push(item);
        }
        if !section.items.is_empty() {
            if section.title.is_empty() {
                section.title = "YouTube Music".into();
            }
            page.sections.push(section);
        }
    }
    page.continuation = contents
        .pointer("/continuations/0/nextContinuationData/continuation")
        .and_then(Value::as_str)
        .map(str::to_owned);
    page
}

fn release_kind_from_label(label: &str) -> Option<ReleaseKind> {
    match label.trim().to_lowercase().as_str() {
        "album" | "đĩa nhạc" => Some(ReleaseKind::Album),
        "single" | "đĩa đơn" => Some(ReleaseKind::Single),
        "ep" | "đĩa ep" => Some(ReleaseKind::EP),
        _ => None,
    }
}

fn home_song(card: &Value) -> Option<Item> {
    let watch = card.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint")?;
    if !is_song(watch) {
        return None;
    }
    let id = watch["videoId"]
        .as_str()
        .or_else(|| card.pointer("/playlistItemData/videoId")?.as_str())?;
    let columns = &card["flexColumns"];
    let title = columns
        .pointer("/0/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")?
        .as_str()?;
    let by = columns
        .pointer("/1/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let album = columns
        .as_array()
        .into_iter()
        .flatten()
        .skip(2)
        .find_map(|col| {
            let run = col.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs/0")?;
            let bid = run
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)?;
            bid.starts_with("MPREb_")
                .then(|| run["text"].as_str().unwrap_or_default().to_string())
        })
        .unwrap_or_default();
    let image = card
        .pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .find_map(|thumb| thumb["url"].as_str().and_then(image_url));
    let mut dur_ms = card
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text/runs/0/text")
        .and_then(Value::as_str)
        .map_or(0, duration_ms);
    if dur_ms == 0 {
        dur_ms = columns
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|col| {
                col.pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|run| run["text"].as_str())
            .map(duration_ms)
            .find(|&ms| ms > 0)
            .unwrap_or(0);
    }
    Some(Item {
        id: ItemId::Song(id.into()),
        title: title.into(),
        by: by.into(),
        image,
        duration_ms: dur_ms,
        album,
    })
}

fn is_song(watch: &Value) -> bool {
    watch
        .pointer("/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(Value::as_str)
        == Some("MUSIC_VIDEO_TYPE_ATV")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_lyrics_return_without_waiting_for_the_other_source() {
        let (tx, rx) = mpsc::channel();
        tx.send((false, Some(crate::lrc::Lyrics::parse("[00:01.00] Timed")))).unwrap();
        // Keep the sender alive: waiting for a second reply would time out and lose this result.
        let started = Instant::now();
        let (_, timed) = collect_lyrics(rx, Duration::from_secs(1));
        assert!(timed.unwrap().synced);
        assert!(started.elapsed() < Duration::from_millis(250));
        drop(tx);
    }

    #[test]
    fn plain_lyrics_survive_a_disconnected_or_missing_second_source() {
        let (tx, rx) = mpsc::channel();
        tx.send((true, Some(crate::lrc::Lyrics::parse("Plain")))).unwrap();
        drop(tx);
        let (youtube, lrclib) = collect_lyrics(rx, Duration::from_millis(10));
        assert_eq!(choose_lyrics(youtube, lrclib).unwrap().lines[0].text, "Plain");
    }

    #[test]
    #[ignore = "queries the reference artist profile on public YouTube Music"]
    fn spicy_reference_artist_live() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let started = Instant::now();
        let profile = rt.block_on(async { let api = connect().await.unwrap(); artist_by_name(&api, "Mer").await })
            .expect("Mer artist profile");
        assert!(profile.image.is_some(), "reference artist portrait is missing");
        assert!(profile.items.iter().any(|item| crate::library::fold(&item.title).contains("giac mo tung rat tho")
            || crate::library::fold(&item.title).contains("feel luu ky")), "namesake artist did not contain the reference recording");
        eprintln!("Artist profile: title={}, image={}, description={}, elapsed={:?}", profile.title,
            profile.image.is_some(), profile.description.is_some(), started.elapsed());
        std::fs::write(std::env::temp_dir().join("nhip-reference-artist.json"), serde_json::to_vec_pretty(&profile).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "downloads a public reference track for screenshot QA"]
    fn spicy_reference_track_live() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let page = rt.block_on(async { let api = connect().await.unwrap(); search(&api, "Giấc mơ từng rất thơ Mer").await.unwrap() });
        let track = page.items.iter().filter_map(Item::track).find(|t| crate::library::fold(&t.title).contains("giac mo tung rat tho")).expect("reference recording");
        std::fs::write(std::env::temp_dir().join("nhip-reference-track.json"), serde_json::to_vec_pretty(&track).unwrap()).unwrap();
        let started = Instant::now();
        let lyrics = online_lyrics(&track).expect("reference lyrics");
        eprintln!("Reference lyrics: {} lines, synced={}, first={:?}", lyrics.lines.len(), lyrics.synced, started.elapsed());
        let cached = Instant::now();
        assert!(online_lyrics(&track).is_some());
        eprintln!("Cached lookup: {:?}", cached.elapsed());
    }

    #[test]
    fn spicy_provider_runs_first_and_falls_back_cleanly_on_failure_or_unconfigured() {
        use std::io::{BufRead, BufReader, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicBool, Ordering};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut req_line = String::new();
                if reader.read_line(&mut req_line).is_err() {
                    continue;
                }
                let path = req_line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                }
                let (status, body) = if path == "/v1/lyrics/4cOdK2wGLETKBW3PvgPWqT" {
                    (
                        200,
                        r#"{"Status":200,"Type":"object","Body":{"id":"4cOdK2wGLETKBW3PvgPWqT","ids":{"spotify":"4cOdK2wGLETKBW3PvgPWqT"},"source":"spicy_lyrics","Type":"Line","Content":[{"Type":"Vocal","Text":"From Spicy First","StartTime":1.0,"EndTime":3.0}]}}"#,
                    )
                } else {
                    (
                        404,
                        r#"{"Status":404,"Type":"error","Body":{"error":"lyrics_not_found","message":"missing"}}"#,
                    )
                };
                let resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let mut s = stream;
                let _ = s.write_all(resp.as_bytes());
            }
        });

        let mut track = crate::library::demo_tracks().remove(0);
        track.spotify_id = Some("4cOdK2wGLETKBW3PvgPWqT".into());
        let cfg = crate::spicy::SpicyApiConfig {
            enabled: true,
            api_key: Some("sl_pk_demo".into()),
            base_url,
            timeout: Duration::from_secs(2),
            cache_ttl_secs: 60,
            cache_path: None,
        };
        let state = crate::spicy::SpicyClientState::new();

        // 1. Spicy succeeds -> fallback is NEVER called.
        let fallback_called = AtomicBool::new(false);
        let winner = resolve_lyrics_provider_chain(&track, &cfg, &state, |_| {
            fallback_called.store(true, Ordering::SeqCst);
            Some(crate::lrc::Lyrics::parse("[00:01.00]Fallback"))
        })
        .expect("spicy lyrics");
        assert_eq!(winner.lines[0].text, "From Spicy First");
        assert!(!fallback_called.load(Ordering::SeqCst));

        // 2. Spicy 404 -> fallback IS invoked and returned.
        let mut missing = track.clone();
        missing.spotify_id = Some("9999999999999999999999".into());
        let fallback_res = resolve_lyrics_provider_chain(&missing, &cfg, &state, |_| {
            Some(crate::lrc::Lyrics::parse("[00:01.00]From Fallback"))
        })
        .expect("fallback lyrics");
        assert_eq!(fallback_res.lines[0].text, "From Fallback");

        // 3. Spicy disabled -> skips Spicy and goes straight to fallback.
        let disabled_cfg = crate::spicy::SpicyApiConfig { enabled: false, ..cfg };
        let disabled_res = resolve_lyrics_provider_chain(&track, &disabled_cfg, &state, |_| {
            Some(crate::lrc::Lyrics::parse("[00:01.00]Fallback When Disabled"))
        })
        .expect("disabled fallback");
        assert_eq!(disabled_res.lines[0].text, "Fallback When Disabled");
    }

    #[test]
    fn timed_online_lyrics_win_and_youtube_wins_a_tie() {
        let lyrics = |text| Some(crate::lrc::Lyrics::parse(text));
        assert_eq!(
            choose_lyrics(lyrics("YTM"), lyrics("[00:01.00] LRCLIB"))
                .unwrap()
                .lines[0]
                .text,
            "LRCLIB"
        );
        assert_eq!(
            choose_lyrics(lyrics("[00:01.00] YTM"), lyrics("[00:01.00] LRCLIB"))
                .unwrap()
                .lines[0]
                .text,
            "YTM"
        );
        assert_eq!(
            choose_lyrics(lyrics("YTM"), lyrics("LRCLIB"))
                .unwrap()
                .lines[0]
                .text,
            "YTM"
        );
    }

    #[test]
    fn public_urls_use_validated_ids_and_fixed_origin() {
        let list =
            from_url("https://music.youtube.com/playlist?list=PL_test-1&si=ignored").unwrap();
        assert_eq!(list, ItemId::Playlist("PL_test-1".into()));
        assert_eq!(from_url(&list.url()), Some(list));
        assert!(from_url("https://music.youtube.com.evil.example/playlist?list=PL123").is_none());
        assert!(from_url("https://music.youtube.com/watch?v=poGyHfrJ_uo%26list%3Dx").is_none());
    }

    #[test]
    fn saved_catalog_track_keeps_the_exact_video_id() {
        let item = Item {
            id: ItemId::Song("poGyHfrJ_uo".into()),
            title: "Khóc Đấy".into(),
            by: "Thắng".into(),
            image: None,
            duration_ms: 0,
            album: String::new(),
        }
        .with_recording("Cái Thứ Hai".into(), "3:48");
        let track = item.track().unwrap();
        assert_eq!(track.id, crate::library::fnv("yt:poGyHfrJ_uo"));
        let encoded = serde_json::to_vec(&track).unwrap();
        let restored: crate::library::Track = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(restored.youtube_id(), Some("poGyHfrJ_uo"));
        assert_eq!(restored.duration_ms, 228_000);
        assert_eq!(restored.album, "Cái Thứ Hai");
        assert!(Item {
            id: ItemId::Song("poGyHfrJ/uo".into()),
            ..item
        }
        .track()
        .is_none());
    }

    #[test]
    fn parses_clock_duration_and_reads_legacy_items() {
        assert_eq!(duration_ms("1:02:03"), 3_723_000);
        assert_eq!(duration_ms("3:65"), 0);
        assert_eq!(duration_ms("3 phút"), 0);
        let item: Item = serde_json::from_value(json!({
            "id": {"Song": "poGyHfrJ_uo"}, "title": "Khóc Đấy", "by": "Thắng", "image": null,
        }))
        .unwrap();
        assert_eq!(item.duration_ms, 0);
        assert!(item.album.is_empty());
    }

    #[test]
    fn a_playlist_without_its_shelf_is_not_an_empty_snapshot() {
        let removed = json!({"contents": {"messageRenderer": {"text": "Unavailable"}}});
        assert!(parse_playlist(&removed).is_none());
        let empty = json!({"contents": {"twoColumnBrowseResultsRenderer": {
            "secondaryContents": {"sectionListRenderer": {"contents": [{"musicPlaylistShelfRenderer": {"contents": []}}]}}
        }}});
        assert!(parse_playlist(&empty).unwrap().items.is_empty());
    }

    #[test]
    fn home_page_keeps_known_public_items_and_the_continuation() {
        let card = |id, title| {
            json!({"musicTwoRowItemRenderer": {
                "title": {"runs": [{"text": title}]}, "navigationEndpoint": {"browseEndpoint": {"browseId": id}}
            }})
        };
        let page = parse_home(
            &json!({"continuationContents": {"sectionListContinuation": {
                "contents": [{"musicCarouselShelfRenderer": {"contents": [
                    card("MPREb_123", "Album"), card("MPREb_123", "Duplicate"), card("MPSP123", "Podcast"), card("VLPL123", "Playlist")
                ]}}], "continuations": [{"nextContinuationData": {"continuation": "next"}}]
            }}}),
        );
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[1].id, ItemId::Playlist("PL123".into()));
        assert_eq!(page.continuation.as_deref(), Some("next"));
    }

    #[test]
    fn home_preserves_provider_groups_and_explicit_release_kinds() {
        let card = |id, kind| json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": "Release"}]},
            "subtitle": {"runs": [{"text": kind}, {"text": " • "}, {"text": "Artist"}]},
            "navigationEndpoint": {"browseEndpoint": {"browseId": id}}
        }});
        let shelf = |title, cards| json!({"musicCarouselShelfRenderer": {
            "header": {"musicCarouselShelfBasicHeaderRenderer": {"title": {"runs": [{"text": title}]}}},
            "contents": cards
        }});
        let page = parse_home(&json!({"continuationContents": {"sectionListContinuation": {
            "contents": [
                shelf("New releases", vec![card("MPREb_ep", "EP"), card("MPREb_single", "Đĩa đơn")]),
                shelf("Listen again", vec![card("MPREb_ep", "EP")])
            ]
        }}}));
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.sections.len(), 2);
        assert_eq!(page.sections[0].title, "New releases");
        assert_eq!(page.sections[1].title, "Listen again");
        assert_eq!(page.sections[1].items[0].id, page.items[0].id);
        assert_eq!(page.items[0].by, "Artist");
        assert_eq!(page.release_kind(&page.items[0].id), Some(&ReleaseKind::EP));
        assert_eq!(page.release_kind(&page.items[1].id), Some(&ReleaseKind::Single));
    }

    #[test]
    fn artist_keeps_singles_eps_and_year_only_releases_from_provider_shelves() {
        let card = |id, kind| json!({"musicTwoRowItemRenderer": {
            "title": {"runs": [{"text": "Release"}]},
            "subtitle": {"runs": [{"text": kind}, {"text": " • "}, {"text": "2026"}]},
            "navigationEndpoint": {"browseEndpoint": {"browseId": id}}
        }});
        let shelf = |title, cards| json!({"musicCarouselShelfRenderer": {
            "header": {"musicCarouselShelfBasicHeaderRenderer": {"title": {"runs": [{"text": title}]}}},
            "contents": cards
        }});
        let mut ep = card("MPREb_ep", "EP");
        ep["musicTwoRowItemRenderer"]["title"]["runs"][0]["navigationEndpoint"] =
            json!({"browseEndpoint": {"browseId": "MPREb_ep"}});
        ep["musicTwoRowItemRenderer"].as_object_mut().unwrap().remove("navigationEndpoint");
        let raw = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{
            "tabRenderer": {"content": {"sectionListRenderer": {"contents": [
                shelf("Albums", vec![card("MPREb_album", "Album")]),
                shelf("Singles & EPs", vec![
                    card("MPREb_single", "Single"), ep, card("MPREb_unknown", "2026"),
                    card("MPREb_single", "Single"), card("UC_artist", "Artist")
                ])
            ]}}}
        }]}}});
        let mut page = Page::default();
        append_artist_release_shelves(&mut page, &raw);
        assert_eq!(page.items.len(), 4);
        assert_eq!(page.sections[1].items.len(), 3);
        assert_eq!(page.release_kind(&ItemId::Album("MPREb_single".into())), Some(&ReleaseKind::Single));
        assert_eq!(page.release_kind(&ItemId::Album("MPREb_ep".into())), Some(&ReleaseKind::EP));
        assert_eq!(page.release_kind(&ItemId::Album("MPREb_unknown".into())), None);
        assert!(page.is_single_or_ep(&ItemId::Album("MPREb_unknown".into())));
        assert!(!page.is_single_or_ep(&ItemId::Album("MPREb_album".into())));
        assert_eq!(page.items[1].by, "2026");
    }

    #[test]
    fn saved_pages_without_release_metadata_remain_readable() {
        let page: Page = serde_json::from_value(json!({"title": "Old page", "items": []})).unwrap();
        assert!(page.sections.is_empty());
        assert!(page.release_kinds.is_empty());
        assert_eq!(release_kind_from_label("Live album edition"), None);
        assert_eq!(release_kind_from_label(" EP "), Some(ReleaseKind::EP));
    }

    #[test]
    fn extracts_visitor_data_from_ytcfg_html() {
        let html = r#"<script>ytcfg.set({"INNERTUBE_CONTEXT":{"client":{}},"VISITOR_DATA":"CgtTest123%3D%3D","STS":20767});</script>"#;
        assert_eq!(
            extract_visitor_data_from_html(html).as_deref(),
            Some("CgtTest123%3D%3D")
        );
    }

    #[test]
    fn parses_visionos_player_response_for_aac() {
        let parsed: PlayerResponse = serde_json::from_value(json!({
            "playabilityStatus": {"status": "OK"},
            "streamingData": {
                "expiresInSeconds": "21540",
                "adaptiveFormats": [
                    {"itag": 139, "url": "https://rr1---sn-example.googlevideo.com/139", "mimeType": "audio/mp4; codecs=\"mp4a.40.5\"", "contentLength": "1000"},
                    {"itag": 140, "url": "https://rr1---sn-example.googlevideo.com/140", "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "contentLength": "3754221"},
                    {"itag": 251, "url": "https://rr1---sn-example.googlevideo.com/251", "mimeType": "audio/webm; codecs=\"opus\"", "contentLength": "3958950"}
                ]
            }
        }))
        .unwrap();
        let stream = parse_visionos_player(parsed).unwrap();
        assert_eq!(stream.itag, Some(140));
        assert_eq!(stream.content_length, Some(3_754_221));
        assert!(stream.expires_at.is_some());
    }

    #[test]
    fn validates_google_media_url() {
        assert!(google_media_url("https://rr2---sn-42u.googlevideo.com/videoplayback?itag=140").is_some());
        assert!(google_media_url("http://rr2.googlevideo.com/videoplayback").is_none());
        assert!(google_media_url("https://user@googlevideo.com/videoplayback").is_none());
        assert!(google_media_url("https://evil.example.com/videoplayback").is_none());
    }

    #[test]
    #[ignore = "makes public YouTube Music requests"]
    fn public_catalog_and_lyrics_live() {
        let track = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let api = connect().await.expect("anonymous catalog session");
                let home = home(&api, None).await.expect("Vietnamese Home");
                assert!(!home.items.is_empty());
                eprintln!(
                    "Home: {} items, continuation={}",
                    home.items.len(),
                    home.continuation.is_some()
                );
                let search = search(&api, "Khóc Đấy Thắng").await.expect("search");
                eprintln!("Search: {} items", search.items.len());
                let found = search
                    .items
                    .iter()
                    .find(|item| item.id == ItemId::Song("poGyHfrJ_uo".into()))
                    .expect("exact Songs result");
                assert!(found.duration_ms > 0);
                let recording = query(&api, Request::Open(found.id.clone()))
                    .await
                    .expect("recording metadata by videoId");
                let song = recording.items.first().expect("watch response song");
                assert_eq!(song.id, found.id);
                assert_eq!(song.title, found.title);
                assert_eq!(song.by, found.by);
                assert_eq!(song.duration_ms, found.duration_ms);
                let album = album(&api, "MPREb_LSeSvSkBOEM")
                    .await
                    .expect("open the acceptance album by browse ID");
                eprintln!("Album: {} songs", album.items.len());
                assert_eq!(album.items.len(), 12);
                let artist = artist(&api, "UCoIioUVO1BW8e0PqHDNiJOg")
                    .await
                    .expect("open Thắng by artist channel ID");
                eprintln!("Artist: {} songs", artist.items.len());
                assert!(!artist.items.is_empty());
                let list = playlist(&api, "OLAK5uy_mnPjQ5TpcHvMJa2J4k15hmzXxSe1kBLmw")
                    .await
                    .expect("open the acceptance album as a public playlist");
                eprintln!("Playlist: {} songs", list.items.len());
                assert_eq!(list.items.len(), 12);
                let related = api.query(Radio("poGyHfrJ_uo")).await.expect("public radio");
                assert!(!related.items.is_empty());
                assert!(related
                    .items
                    .iter()
                    .all(|item| matches!(&item.id, ItemId::Song(id) if id != "poGyHfrJ_uo")));
                eprintln!("Related songs: {}", related.items.len());
                let lyrics = lyrics(&api, "poGyHfrJ_uo")
                    .await
                    .expect("lyrics for the exact videoId");
                eprintln!(
                    "Lyrics: {} lines, timed={}",
                    lyrics.lines.len(),
                    lyrics.synced
                );
                found.track().expect("catalog track for the lyrics lookup")
            });
        let chosen = online_lyrics(&track).expect("lyrics for the catalog track");
        eprintln!(
            "Combined lyrics lookup: {} lines, timed={}",
            chosen.lines.len(),
            chosen.synced
        );
    }

    #[test]
    #[ignore = "verifies live album search, OLAK5uy album upgrade, and artist resolution"]
    fn user_reported_albums_and_artists_live() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let api = connect().await.expect("connect");

                // 1. Searching "gieo" must crown the 11-track 2022 Album `Gieo` at items[0]
                let gieo_s = search(&api, "gieo").await.expect("search gieo");
                let top_gieo = gieo_s.items.first().expect("gieo top result");
                assert!(
                    matches!(top_gieo.id, ItemId::Album(_)) && crate::library::fold(&top_gieo.title) == "gieo",
                    "expected Album Gieo at items[0], got {top_gieo:?}"
                );
                if let ItemId::Album(ref bid) = top_gieo.id {
                    let gieo_al = album(&api, bid).await.expect("open Gieo album");
                    assert_eq!(gieo_al.items.len(), 11, "Gieo must have 11 tracks");
                    assert_eq!(gieo_al.year.as_deref(), Some("2022"));
                }

                // 2. Searching "cái thứ hai" must crown the 12-track Album `Cái Thứ Hai` at items[0]
                let cth_s = search(&api, "cái thứ hai").await.expect("search cái thứ hai");
                let top_cth = cth_s.items.first().expect("cái thứ hai top result");
                assert!(
                    matches!(top_cth.id, ItemId::Album(_)) && crate::library::fold(&top_cth.title) == "cai thu hai",
                    "expected Album Cái Thứ Hai at items[0], got {top_cth:?}"
                );

                // 3. Opening OLAK5uy_nS2wPjpziYVHsH5yz02bMoocFzYbaRUrg upgrades to full 11-track Gieo Album
                let olak = playlist(&api, "OLAK5uy_nS2wPjpziYVHsH5yz02bMoocFzYbaRUrg")
                    .await
                    .expect("open OLAK5uy Gieo");
                assert_eq!(olak.title, "Gieo");
                assert_eq!(olak.items.len(), 11);
                assert_eq!(olak.year.as_deref(), Some("2022"));
                assert!(olak.image.is_some());

                // 4. Clicking artists ("Ngọt", "Thắng", "SUNi", "dangthicong") resolves live YTM profiles
                for artist_name in ["Ngọt", "Thắng", "SUNi", "dangthicong"] {
                    let ar = artist_by_name(&api, artist_name)
                        .await
                        .unwrap_or_else(|e| panic!("artist_by_name({artist_name}) failed: {e:?}"));
                    assert!(
                        ar.items.iter().any(|it| matches!(it.id, ItemId::Song(_))),
                        "artist {artist_name} missing popular songs: {ar:?}"
                    );
                }

                // 5. Ngọt's profile exposes real studio Albums (distinct from Singles & EPs)
                let ngot = artist_by_name(&api, "Ngọt").await.unwrap();
                let real_albums: Vec<_> = ngot
                    .items
                    .iter()
                    .filter(|it| {
                        matches!(it.id, ItemId::Album(_))
                            && ngot.release_kind(&it.id) == Some(&ReleaseKind::Album)
                    })
                    .collect();
                assert!(
                    real_albums.len() >= 2,
                    "expected >=2 studio Albums for Ngọt, got {real_albums:?}"
                );
            });
    }
}
