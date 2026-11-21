//! Saved music, playlists, and the indexes used by the library.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::audio::{LoadSpec, SourceSpec};
use crate::mechanics::{normalization_gain_db, Transition, CROSSFADE_MAX_S, LOUD_OFFSET_DB, QUIET_OFFSET_DB};
use crate::queue::{QueueSnapshot, Repeat};

pub type TrackId = u64;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Track {
    pub id: TrackId,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub album_artist: Option<String>,
    #[serde(default)]
    pub genre: Option<String>,
    #[serde(default)]
    pub year: Option<u32>,
    #[serde(default)]
    pub track_no: Option<u32>,
    #[serde(default)]
    pub disc_no: Option<u32>,
    pub duration_ms: u64,
    /// Measured or tagged loudness in dB; `None` until known.
    #[serde(default)]
    pub loudness_db: Option<f32>,
    #[serde(default)]
    pub peak_db: Option<f32>,
    #[serde(default)]
    pub art_url: Option<String>,
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub mtime: u64,
    #[serde(default)]
    pub lyrics: Option<String>,
    #[serde(default)]
    pub spotify_id: Option<String>,
    #[serde(default)]
    pub isrc: Option<String>,
}

impl Track {
    pub fn youtube_id(&self) -> Option<&str> {
        self.path.strip_prefix("yt:").filter(|id| crate::ytm::valid_video_id(id))
    }

    /// Where the cover comes from: a web address, a file's tags, or nowhere.
    pub fn art_source(&self) -> Option<&str> {
        match (&self.art_url, self.youtube_id().is_some() || self.demo_seed().is_some()) {
            (Some(url), _) => Some(url),
            (None, true) => None,
            (None, false) => Some(&self.path),
        }
    }

    pub fn demo_seed(&self) -> Option<u32> {
        self.path.strip_prefix("demo:").and_then(|s| s.parse().ok())
    }

    /// Songs with guest artists still belong to the main artist's album.
    pub fn album_owner(&self) -> String {
        match &self.album_artist {
            Some(a) => a.clone(),
            None => split_artists(&self.artist).into_iter().next().unwrap_or_else(|| self.artist.clone()),
        }
    }

    pub fn effective_album(&self) -> &str {
        let trimmed = self.album.trim();
        if trimmed.is_empty() {
            &self.title
        } else {
            trimmed
        }
    }

    pub fn album_key(&self) -> String {
        album_key(&self.album_owner(), self.effective_album())
    }

    pub fn cover_key(&self) -> String {
        self.art_url.as_deref().map_or_else(|| self.album_key(), cover_key)
    }

    // ponytail: file extension stands in for the codec; m4a counts as lossy even when it's ALAC.
    fn is_lossless(&self) -> bool {
        let ext = Path::new(&self.path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        matches!(ext.as_str(), "flac" | "wav" | "aiff" | "aif" | "caf")
    }

    pub fn gain_db(&self, level: NormLevel, normalize: bool) -> f32 {
        if !normalize {
            return 0.0;
        }
        normalization_gain_db(self.loudness_db, self.peak_db, level.offset_db(), level == NormLevel::Loud, self.is_lossless())
    }

    pub fn load_spec(&self, level: NormLevel, normalize: bool, transition: Transition) -> LoadSpec {
        let source = match self.demo_seed() {
            Some(seed) => SourceSpec::Demo { seed, duration_ms: self.duration_ms },
            None => SourceSpec::File(self.path.clone()),
        };
        LoadSpec { id: self.id, source, duration_ms: self.duration_ms, gain_db: self.gain_db(level, normalize), transition }
    }
}

pub fn cover_key(url: &str) -> String {
    format!("cover:{url}")
}

/// Picks the hand-over between two songs, in the order Spotify.dll 0x180d56f4c checks them.
pub fn transition(now: &Track, next: &Track, gapless: bool, crossfade_ms: Option<u64>) -> Transition {
    if gapless && consecutive_on_album(now, next) {
        return Transition::Gapless;
    }
    match crossfade_ms {
        Some(0) => return Transition::Gapless,
        Some(ms) if ms * 2 <= now.duration_ms && ms * 2 <= next.duration_ms => return Transition::Crossfade,
        _ => {}
    }
    if gapless {
        Transition::Gapless
    } else {
        Transition::None
    }
}

fn consecutive_on_album(now: &Track, next: &Track) -> bool {
    !now.album.trim().is_empty() && now.album_key() == next.album_key()
        && now.disc_no == next.disc_no
        && now.track_no.zip(next.track_no).is_some_and(|(a, b)| a.checked_add(1) == Some(b))
}

pub fn album_key(artist: &str, album: &str) -> String {
    format!("{}\u{1f}{}", artist, album)
}

pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[derive(Clone, Debug)]
pub struct Album {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub tracks: Vec<TrackId>,
    pub cover_path: Option<String>,
    pub added: u64,
}

#[derive(Clone, Debug)]
pub struct Artist {
    pub name: String,
    pub tracks: Vec<TrackId>,
    pub albums: Vec<String>,
    pub cover_key: String,
    pub cover_path: Option<String>,
}

#[derive(Default)]
pub struct Library {
    pub tracks: HashMap<TrackId, Arc<Track>>,
    /// Stable order: artist, album, disc, track number.
    pub order: Vec<TrackId>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
    pub album_index: HashMap<String, usize>,
    pub artist_index: HashMap<String, usize>,
    pub demo: bool,
}

impl Library {
    pub fn new(tracks: Vec<Track>) -> Self {
        let mut l = Self::default();
        l.tracks = tracks.into_iter().map(|t| (t.id, Arc::new(t))).collect();
        l.reindex();
        l
    }

    pub fn track(&self, id: TrackId) -> Option<Arc<Track>> {
        self.tracks.get(&id).cloned()
    }

    pub fn upsert(&mut self, t: Track) {
        self.tracks.insert(t.id, Arc::new(t));
    }

    pub fn reindex(&mut self) {
        let mut all: Vec<&Arc<Track>> = self.tracks.values().collect();
        all.sort_by(|a, b| {
            let ka = (fold(&a.artist), fold(&a.album), a.disc_no.unwrap_or(1), a.track_no.unwrap_or(9999), fold(&a.title));
            let kb = (fold(&b.artist), fold(&b.album), b.disc_no.unwrap_or(1), b.track_no.unwrap_or(9999), fold(&b.title));
            ka.cmp(&kb)
        });
        self.order = all.iter().map(|t| t.id).collect();

        let mut albums: BTreeMap<String, Album> = BTreeMap::new();
        let mut artists: BTreeMap<String, Artist> = BTreeMap::new();
        for t in &all {
            let key = t.album_key();
            let a = albums.entry(key.clone()).or_insert_with(|| Album {
                key: key.clone(),
                title: t.effective_album().to_owned(),
                artist: t.album_owner(),
                year: t.year,
                tracks: Vec::new(),
                cover_path: t.art_source().map(str::to_string),
                added: t.added,
            });
            a.tracks.push(t.id);
            a.added = a.added.max(t.added);
            for name in split_artists(&t.artist) {
                let e = artists.entry(name.clone()).or_insert_with(|| Artist {
                    name: name.clone(),
                    tracks: Vec::new(),
                    albums: Vec::new(),
                    cover_key: format!("artist:{}", name),
                    cover_path: t.art_source().map(str::to_string),
                });
                e.tracks.push(t.id);
                if !e.albums.contains(&key) {
                    e.albums.push(key.clone());
                }
            }
        }
        self.albums = albums.into_values().collect();
        self.albums.sort_by_key(|a| fold(&a.title));
        self.artists = artists.into_values().collect();
        self.artists.sort_by_key(|a| fold(&a.name));
        self.album_index = self.albums.iter().enumerate().map(|(i, a)| (a.key.clone(), i)).collect();
        self.artist_index = self.artists.iter().enumerate().map(|(i, a)| (a.name.clone(), i)).collect();
    }

    pub fn album(&self, key: &str) -> Option<&Album> {
        self.album_index.get(key).map(|&i| &self.albums[i])
    }

    pub fn artist(&self, name: &str) -> Option<&Artist> {
        self.artist_index.get(name).map(|&i| &self.artists[i])
    }

}

/// Lower-cases and strips accents, so "son tung" finds "Sơn Tùng".
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        out.push(match c {
            'à' | 'á' | 'ạ' | 'ả' | 'ã' | 'â' | 'ầ' | 'ấ' | 'ậ' | 'ẩ' | 'ẫ' | 'ă' | 'ằ' | 'ắ' | 'ặ' | 'ẳ' | 'ẵ' | 'ä' | 'å' => 'a',
            'è' | 'é' | 'ẹ' | 'ẻ' | 'ẽ' | 'ê' | 'ề' | 'ế' | 'ệ' | 'ể' | 'ễ' | 'ë' => 'e',
            'ì' | 'í' | 'ị' | 'ỉ' | 'ĩ' | 'î' | 'ï' => 'i',
            'ò' | 'ó' | 'ọ' | 'ỏ' | 'õ' | 'ô' | 'ồ' | 'ố' | 'ộ' | 'ổ' | 'ỗ' | 'ơ' | 'ờ' | 'ớ' | 'ợ' | 'ở' | 'ỡ' | 'ö' | 'ø' => 'o',
            'ù' | 'ú' | 'ụ' | 'ủ' | 'ũ' | 'ư' | 'ừ' | 'ứ' | 'ự' | 'ử' | 'ữ' | 'û' | 'ü' => 'u',
            'ỳ' | 'ý' | 'ỵ' | 'ỷ' | 'ỹ' | 'ÿ' => 'y',
            'đ' => 'd',
            'ñ' => 'n',
            'ç' => 'c',
            c => c,
        });
    }
    out
}

/// Splits "A, B & C feat. D" into separate artists, leaving single names untouched.
pub fn split_artists(s: &str) -> Vec<String> {
    let mut text = s.to_string();
    for sep in [" feat. ", " ft. ", " featuring ", " & ", " x ", " X ", "; ", "/"] {
        text = text.replace(sep, ",");
    }
    let parts: Vec<String> = text.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        vec![s.to_string()]
    } else {
        parts
    }
}

// --------------------------------------------------------------------------------------------
// Saved data

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Default, Debug)]
pub enum Lang {
    #[default]
    Vi,
    En,
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize, Default, Debug)]
pub enum NormLevel {
    Quiet,
    #[default]
    Normal,
    Loud,
}

impl NormLevel {
    pub fn offset_db(self) -> f32 {
        match self {
            NormLevel::Quiet => QUIET_OFFSET_DB,
            NormLevel::Normal => 0.0,
            NormLevel::Loud => LOUD_OFFSET_DB,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryViewMode {
    CompactList,
    #[default]
    DefaultList,
    CompactGrid,
    DefaultGrid,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibrarySort {
    #[default]
    Recent,
    Added,
    Alphabetical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibrarySortBy {
    #[default]
    #[serde(alias = "Recent")]
    Recents,
    #[serde(alias = "Added")]
    RecentlyAdded,
    Alphabetical,
    Creator,
}

impl From<LibrarySort> for LibrarySortBy {
    fn from(sort: LibrarySort) -> Self {
        match sort {
            LibrarySort::Recent => Self::Recents,
            LibrarySort::Added => Self::RecentlyAdded,
            LibrarySort::Alphabetical => Self::Alphabetical,
        }
    }
}

impl From<LibrarySortBy> for LibrarySort {
    fn from(sort: LibrarySortBy) -> Self {
        match sort {
            LibrarySortBy::Recents => Self::Recent,
            LibrarySortBy::RecentlyAdded => Self::Added,
            LibrarySortBy::Alphabetical | LibrarySortBy::Creator => Self::Alphabetical,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LibraryCreatorFilter {
    #[default]
    All,
    ByYou,
    BySpotify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackSortColumn {
    #[default]
    Default,
    Title,
    Artist,
    Album,
    DateAdded,
    Duration,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSort {
    pub column: TrackSortColumn,
    pub ascending: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceStyle {
    /// Classic crisp dark Spotify chrome (`#000000` / `#121212`).
    #[default]
    Standard,
    /// Native Windows 11 Mica (`DWMSBT_MAINWINDOW`) matte wallpaper-tinted glass.
    #[serde(alias = "AmbientBlur")]
    Mica,
    /// Native Windows 11 Mica Alt (`DWMSBT_TABBEDWINDOW`) richer wallpaper-tinted glass.
    MicaAlt,
    /// Native Windows 11 Acrylic (`DWMSBT_TRANSIENTWINDOW`) / Win10 blur-behind frosted glass.
    Acrylic,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccentTheme {
    #[default]
    SpotifyGreen,
    OceanBlue,
    Violet,
    Rose,
    Amber,
    Mint,
}

impl AccentTheme {
    pub const ALL: &'static [AccentTheme] = &[
        AccentTheme::SpotifyGreen,
        AccentTheme::OceanBlue,
        AccentTheme::Violet,
        AccentTheme::Rose,
        AccentTheme::Amber,
        AccentTheme::Mint,
    ];

    pub fn rgb(self) -> [u8; 3] {
        match self {
            AccentTheme::SpotifyGreen => [0x1e, 0xd7, 0x60],
            AccentTheme::OceanBlue => [0x3b, 0x82, 0xf6],
            AccentTheme::Violet => [0xa8, 0x55, 0xf7],
            AccentTheme::Rose => [0xf4, 0x3f, 0x5e],
            AccentTheme::Amber => [0xf5, 0x9e, 0x0b],
            AccentTheme::Mint => [0x14, 0xb8, 0xa6],
        }
    }

    pub fn label(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (AccentTheme::SpotifyGreen, Lang::Vi) => "Xanh lá (Mặc định)",
            (AccentTheme::SpotifyGreen, Lang::En) => "Green (Default)",
            (AccentTheme::OceanBlue, Lang::Vi) => "Xanh dương",
            (AccentTheme::OceanBlue, Lang::En) => "Ocean Blue",
            (AccentTheme::Violet, Lang::Vi) => "Tím",
            (AccentTheme::Violet, Lang::En) => "Violet",
            (AccentTheme::Rose, Lang::Vi) => "Hồng đỏ",
            (AccentTheme::Rose, Lang::En) => "Rose",
            (AccentTheme::Amber, Lang::Vi) => "Hổ phách",
            (AccentTheme::Amber, Lang::En) => "Amber",
            (AccentTheme::Mint, Lang::Vi) => "Xanh ngọc",
            (AccentTheme::Mint, Lang::En) => "Mint",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub lang: Lang,
    pub volume: f32,
    pub muted: bool,
    pub shuffle: bool,
    pub repeat: Repeat,
    pub crossfade: bool,
    pub auto_mix: bool,
    pub crossfade_seconds: u32,
    pub gapless: bool,
    pub normalize: bool,
    pub level: NormLevel,
    pub eq_enabled: bool,
    pub eq_gains: [f32; 6],
    pub eq_preset: usize,
    pub left_width: f32,
    pub left_collapsed: bool,
    pub library_sort: LibrarySort,
    pub library_sort_by: Option<LibrarySortBy>,
    pub library_view: LibraryViewMode,
    pub right_width: f32,
    pub show_right: bool,
    pub lyrics_motion: bool,
    pub prefers_reduced_motion: bool,
    pub show_remaining_time: bool,
    pub compact_tracklist: bool,
    pub private_session: bool,
    pub autoplay: bool,
    pub miniplayer_size: Option<[f32; 2]>,
    pub miniplayer_cover_mode: bool,
    pub surface_style: SurfaceStyle,
    /// Backdrop tint opacity (`0.0..=0.95`) over Windows Acrylic / Mica.
    pub surface_opacity: f32,
    /// Translucent card/panel opacity (`0.10..=0.95`) over the window surface.
    pub panel_opacity: f32,
    /// Album-art ambient color tint strength (`0.0..=1.0`) blended over translucent surfaces.
    pub surface_tint_strength: f32,
    /// User-customized local profile display name.
    pub profile_name: String,
    /// Personalized accent color theme.
    pub accent_theme: AccentTheme,
    /// Whether Home recommendations and Heavy Rotation shelves personalize to local listening habits.
    pub personalize_home: bool,
    pub spicy_lyrics_enabled: bool,
    pub spicy_lyrics_api_key: String,
    pub spicy_lyrics_api_url: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lang: Lang::Vi,
            volume: 0.8,
            muted: false,
            shuffle: false,
            repeat: Repeat::Off,
            crossfade: false,
            auto_mix: false,
            crossfade_seconds: CROSSFADE_MAX_S / 2,
            gapless: true,
            normalize: true,
            level: NormLevel::Normal,
            eq_enabled: false,
            eq_gains: [0.0; 6],
            eq_preset: 0,
            left_width: crate::mechanics::LEFT_DEFAULT,
            left_collapsed: false,
            library_sort: LibrarySort::default(),
            library_sort_by: None,
            library_view: LibraryViewMode::default(),
            right_width: crate::mechanics::RIGHT_WIDTH,
            show_right: true,
            lyrics_motion: true,
            prefers_reduced_motion: false,
            show_remaining_time: false,
            compact_tracklist: false,
            private_session: false,
            autoplay: true,
            miniplayer_size: None,
            miniplayer_cover_mode: false,
            surface_style: SurfaceStyle::Standard,
            surface_opacity: 0.62,
            panel_opacity: 0.64,
            surface_tint_strength: 0.28,
            profile_name: "Nhịp".to_string(),
            accent_theme: AccentTheme::SpotifyGreen,
            personalize_home: true,
            spicy_lyrics_enabled: true,
            spicy_lyrics_api_key: String::new(),
            spicy_lyrics_api_url: String::new(),
        }
    }
}

impl Settings {
    pub fn effective_library_sort(&self) -> LibrarySortBy {
        self.library_sort_by.unwrap_or_else(|| self.library_sort.into())
    }

    pub fn set_library_sort(&mut self, sort: LibrarySortBy) {
        self.library_sort_by = Some(sort);
        self.library_sort = sort.into();
    }

    pub fn effective_lyrics_motion(&self) -> bool {
        self.lyrics_motion && !self.prefers_reduced_motion
    }

    pub fn transition(&self, now: &Track, next: &Track) -> Transition {
        if self.auto_mix {
            return if now.id == next.id || consecutive_on_album(now, next) { Transition::Gapless } else { Transition::AutoMix };
        }
        transition(now, next, self.gapless, self.crossfade.then_some(self.crossfade_seconds as u64 * 1000))
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Playlist {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub tracks: Vec<PlaylistEntry>,
    pub created: u64,
    #[serde(default)]
    pub image: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub folder_id: Option<u64>,
    #[serde(default)]
    pub creator: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PlaylistEntry {
    pub id: TrackId,
    pub added: u64,
}

impl Playlist {
    pub fn new(name: String) -> Self {
        let created = now();
        Self {
            id: fnv(&format!("{name}{created}{}", rand::random::<u32>())),
            name,
            description: String::new(),
            tracks: Vec::new(),
            created,
            image: None,
            pinned: false,
            folder_id: None,
            creator: None,
        }
    }

    pub fn track_ids(&self) -> Vec<TrackId> {
        self.tracks.iter().map(|e| e.id).collect()
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct PlaylistFolder {
    pub id: u64,
    pub name: String,
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub parent_id: Option<u64>,
}

impl PlaylistFolder {
    pub fn new(name: String) -> Self {
        let created = now();
        Self {
            id: fnv(&format!("folder:{name}:{created}:{}", rand::random::<u32>())),
            name,
            created,
            pinned: false,
            parent_id: None,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
pub enum RecentItem {
    Liked,
    Playlist(u64),
    Album(String),
    Artist(String),
    FollowedYouTube(String),
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
#[serde(default)]
pub struct Session {
    pub current: Option<TrackId>,
    pub position_ms: u64,
    pub context_name: String,
    pub context: Vec<TrackId>,
    pub queue: Option<QueueSnapshot>,
}

fn default_pinned_items() -> Vec<RecentItem> {
    vec![RecentItem::Liked]
}

fn deserialize_folders<'de, D>(deserializer: D) -> Result<Vec<PlaylistFolder>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let items: Vec<serde_json::Value> = Vec::deserialize(deserializer)?;
    let mut out = Vec::new();
    for item in items {
        // Legacy versions stored local music directory paths (`Vec<String>`) in `"folders"`;
        // ignore those strings gracefully so `user.json` upgrades without blocking saves.
        if item.is_string() {
            continue;
        }
        if let Ok(folder) = serde_json::from_value::<PlaylistFolder>(item) {
            out.push(folder);
        }
    }
    Ok(out)
}

pub const USER_SCHEMA_VERSION: u32 = 2;
const MAX_LISTENING_HISTORY: usize = 200;
const MAX_UNREFERENCED_YT_CACHE: usize = 1_500;

fn default_schema_version() -> u32 {
    USER_SCHEMA_VERSION
}

#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct UserData {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub playlists: Vec<Playlist>,
    #[serde(default, deserialize_with = "deserialize_folders")]
    pub folders: Vec<PlaylistFolder>,
    pub followed_ytm: Vec<crate::ytm::FollowedPlaylist>,
    pub followed_artists: std::collections::HashSet<String>,
    /// Cached real portrait/avatar URL per folded artist name so sidebar & profile never collide on a shared song cover.
    pub artist_avatars: std::collections::BTreeMap<String, String>,
    pub saved_albums: std::collections::HashSet<String>,
    pub liked: Vec<PlaylistEntry>,
    pub hidden_tracks: std::collections::HashSet<TrackId>,
    pub recent: Vec<RecentItem>,
    #[serde(default = "default_pinned_items")]
    pub pinned_items: Vec<RecentItem>,
    pub listening_history: Vec<TrackId>,
    /// Local privacy-preserving play counts (tracks heard >= 30s or naturally completed) for taste & recommendation ranking.
    pub play_counts: HashMap<TrackId, u32>,
    /// Local fast-skip counts (manually skipped < 10s in) used to down-rank disliked suggestions.
    pub skip_counts: HashMap<TrackId, u32>,
    pub recent_searches: Vec<String>,
    pub settings: Settings,
    pub session: Session,
}

impl Default for UserData {
    fn default() -> Self {
        Self {
            schema_version: USER_SCHEMA_VERSION,
            playlists: Vec::new(),
            folders: Vec::new(),
            followed_ytm: Vec::new(),
            followed_artists: std::collections::HashSet::new(),
            artist_avatars: std::collections::BTreeMap::new(),
            saved_albums: std::collections::HashSet::new(),
            liked: Vec::new(),
            hidden_tracks: std::collections::HashSet::new(),
            recent: Vec::new(),
            pinned_items: default_pinned_items(),
            listening_history: Vec::new(),
            play_counts: HashMap::new(),
            skip_counts: HashMap::new(),
            recent_searches: Vec::new(),
            settings: Settings::default(),
            session: Session::default(),
        }
    }
}

impl UserData {
    pub fn record_listen(&mut self, id: TrackId) {
        self.listening_history.retain(|&entry| entry != id);
        self.listening_history.insert(0, id);
        self.listening_history.truncate(MAX_LISTENING_HISTORY);
    }

    pub fn clear_taste_history(&mut self) {
        self.listening_history.clear();
        self.play_counts.clear();
        self.skip_counts.clear();
        self.recent_searches.clear();
    }

    pub fn total_completed_plays(&self) -> u64 {
        self.play_counts.values().map(|&v| v as u64).sum()
    }

    /// Returns the user's most played non-hidden tracks present in `lib`, ranked by plays minus fast skips.
    pub fn top_played_tracks(&self, lib: &Library, limit: usize) -> Vec<TrackId> {
        let mut ranked: Vec<(i64, u32, TrackId)> = self
            .play_counts
            .iter()
            .filter(|(&id, &plays)| {
                plays > 0 && !self.hidden_tracks.contains(&id) && lib.tracks.contains_key(&id)
            })
            .map(|(&id, &plays)| {
                let skips = self.skip_counts.get(&id).copied().unwrap_or(0);
                let score = (plays as i64) * 3 - (skips as i64) * 2;
                (score, plays, id)
            })
            .collect();
        ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)).then_with(|| a.2.cmp(&b.2)));
        ranked.into_iter().take(limit).map(|(_, _, id)| id).collect()
    }

    pub fn is_liked(&self, id: TrackId) -> bool {
        self.liked.iter().any(|e| e.id == id)
    }

    pub fn toggle_like(&mut self, id: TrackId) -> bool {
        if let Some(p) = self.liked.iter().position(|e| e.id == id) {
            self.liked.remove(p);
            false
        } else {
            self.liked.insert(0, PlaylistEntry { id, added: now() });
            true
        }
    }

    pub fn liked_ids(&self) -> Vec<TrackId> {
        self.liked.iter().map(|e| e.id).collect()
    }

    pub fn playlist(&self, id: u64) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn playlist_mut(&mut self, id: u64) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    pub fn is_pinned(&self, item: &RecentItem) -> bool {
        if self.pinned_items.contains(item) {
            return true;
        }
        matches!(item, RecentItem::Playlist(id) if self.playlist(*id).is_some_and(|p| p.pinned))
    }

    pub fn set_pinned(&mut self, item: RecentItem, pinned: bool) {
        if let RecentItem::Playlist(id) = &item {
            if let Some(p) = self.playlist_mut(*id) {
                p.pinned = pinned;
            }
        }
        if pinned {
            if !self.pinned_items.contains(&item) {
                self.pinned_items.push(item);
            }
        } else {
            self.pinned_items.retain(|existing| existing != &item);
        }
    }

    pub fn toggle_pin(&mut self, item: RecentItem) -> bool {
        let next = !self.is_pinned(&item);
        self.set_pinned(item, next);
        next
    }

    pub fn folder(&self, id: u64) -> Option<&PlaylistFolder> {
        self.folders.iter().find(|f| f.id == id)
    }

    pub fn folder_mut(&mut self, id: u64) -> Option<&mut PlaylistFolder> {
        self.folders.iter_mut().find(|f| f.id == id)
    }

    pub fn create_folder(&mut self, name: String) -> u64 {
        let folder = PlaylistFolder::new(name);
        let id = folder.id;
        self.folders.insert(0, folder);
        id
    }

    pub fn rename_folder(&mut self, id: u64, name: String) -> bool {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Some(folder) = self.folder_mut(id) {
            folder.name = trimmed.to_string();
            true
        } else {
            false
        }
    }

    pub fn delete_folder(&mut self, id: u64) -> bool {
        let Some(pos) = self.folders.iter().position(|f| f.id == id) else {
            return false;
        };
        let parent = self.folders[pos].parent_id;
        self.folders.remove(pos);
        for playlist in &mut self.playlists {
            if playlist.folder_id == Some(id) {
                playlist.folder_id = parent;
            }
        }
        for folder in &mut self.folders {
            if folder.parent_id == Some(id) {
                folder.parent_id = parent;
            }
        }
        true
    }

    pub fn move_playlist_to_folder(&mut self, playlist_id: u64, folder_id: Option<u64>) -> bool {
        if let Some(fid) = folder_id {
            if self.folder(fid).is_none() {
                return false;
            }
        }
        let Some(playlist) = self.playlist_mut(playlist_id) else {
            return false;
        };
        playlist.folder_id = folder_id;
        true
    }

    pub fn playlists_in_folder(&self, folder_id: Option<u64>) -> Vec<&Playlist> {
        self.playlists.iter().filter(|p| p.folder_id == folder_id).collect()
    }

    pub fn toggle_folder_pin(&mut self, id: u64) -> bool {
        if let Some(folder) = self.folder_mut(id) {
            folder.pinned = !folder.pinned;
            folder.pinned
        } else {
            false
        }
    }

    pub fn touch_recent(&mut self, item: RecentItem) {
        self.recent.retain(|r| *r != item);
        self.recent.insert(0, item);
        self.recent.truncate(40);
    }

    pub fn record_artist_avatar(&mut self, name: &str, url: &str) -> bool {
        let key = fold(name.trim());
        let clean = url.trim();
        if key.is_empty() || clean.is_empty() {
            return false;
        }
        if self.artist_avatars.get(&key).map(String::as_str) == Some(clean) {
            return false;
        }
        self.artist_avatars.insert(key, clean.to_string());
        true
    }

    pub fn artist_avatar(&self, name: &str) -> Option<&str> {
        self.artist_avatars.get(&fold(name.trim())).map(String::as_str)
    }

    pub fn record_play_complete(&mut self, id: TrackId) {
        let entry = self.play_counts.entry(id).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    pub fn record_fast_skip(&mut self, id: TrackId) {
        let entry = self.skip_counts.entry(id).or_insert(0);
        *entry = entry.saturating_add(1);
    }

    /// Computes a local taste affinity score for an artist from follows, likes, completed plays, recent listens, and fast skips.
    pub fn artist_affinity(&self, lib: &Library, artist_name: &str) -> i32 {
        let target = fold(artist_name.trim());
        if target.is_empty() {
            return 0;
        }
        let mut score = 0_i32;
        if self.followed_artists.iter().any(|a| fold(a) == target) {
            score += 6;
        }
        for entry in &self.liked {
            if self.hidden_tracks.contains(&entry.id) {
                continue;
            }
            if let Some(t) = lib.tracks.get(&entry.id) {
                if split_artists(&t.artist).iter().any(|a| fold(a) == target) {
                    score += 3;
                }
            }
        }
        for (&id, &plays) in &self.play_counts {
            if self.hidden_tracks.contains(&id) {
                continue;
            }
            if let Some(t) = lib.tracks.get(&id) {
                if split_artists(&t.artist).iter().any(|a| fold(a) == target) {
                    score += (plays.min(5) as i32) * 2;
                }
            }
        }
        for (&id, &skips) in &self.skip_counts {
            if let Some(t) = lib.tracks.get(&id) {
                if split_artists(&t.artist).iter().any(|a| fold(a) == target) {
                    score -= (skips.min(4) as i32) * 2;
                }
            }
        }
        for &id in self.listening_history.iter().take(20) {
            if let Some(t) = lib.tracks.get(&id) {
                if split_artists(&t.artist).iter().any(|a| fold(a) == target) {
                    score += 1;
                }
            }
        }
        score
    }

    /// Scores a candidate recommendation track using local listening habits (higher = better match).
    pub fn score_recommendation(&self, lib: &Library, id: TrackId, artist_str: &str) -> i32 {
        if self.hidden_tracks.contains(&id) {
            return i32::MIN / 2;
        }
        let mut score = 0_i32;
        for artist in split_artists(artist_str) {
            score += self.artist_affinity(lib, &artist);
        }
        if let Some(&plays) = self.play_counts.get(&id) {
            score += plays.min(3) as i32;
        }
        if let Some(&skips) = self.skip_counts.get(&id) {
            score -= (skips.min(5) as i32) * 4;
        }
        if self.listening_history.iter().take(8).any(|&h| h == id) {
            score -= 6;
        }
        score
    }
}

fn installed_data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Nhip")
}

fn portable_data_dir(exe: &Path) -> Option<PathBuf> {
    let parent = exe.parent()?;
    parent.join("nhip.portable").is_file().then(|| parent.join("data"))
}

pub fn data_dir() -> &'static Path {
    static DIRECTORY: OnceLock<PathBuf> = OnceLock::new();
    DIRECTORY.get_or_init(|| {
        std::env::current_exe().ok().and_then(|exe| portable_data_dir(&exe)).unwrap_or_else(installed_data_dir)
    })
}

fn user_file() -> PathBuf {
    data_dir().join("user.json")
}

fn library_file() -> PathBuf {
    data_dir().join("youtube-music-library.json")
}

fn user_backup_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(".bak");
    PathBuf::from(os)
}

fn read_user(path: &Path) -> io::Result<UserData> {
    serde_json::from_slice(&std::fs::read(path)?).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn load_user_at(path: &Path, legacy: &Path) -> io::Result<UserData> {
    match read_user(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            // If primary is absent but a rolling `.bak` exists (e.g. crash mid-replace), recover from `.bak`.
            let bak = user_backup_path(path);
            if let Ok(recovered) = read_user(&bak) {
                return Ok(recovered);
            }
            let mut data = UserData::default();
            if path != legacy {
                if let Ok(old) = read_user(legacy) {
                    data.settings = old.settings;
                }
            }
            Ok(data)
        }
        Err(err) if err.kind() == io::ErrorKind::InvalidData => {
            let bak = user_backup_path(path);
            if let Ok(recovered) = read_user(&bak) {
                // Quarantine the corrupt primary file so the user can inspect it without losing data.
                let quarantine = path.with_extension(format!("corrupt-{}", now()));
                let _ = std::fs::rename(path, quarantine);
                return Ok(recovered);
            }
            Err(err)
        }
        result => result,
    }
}

pub fn load_user() -> io::Result<UserData> {
    load_user_at(&user_file(), &installed_data_dir().join("user.json"))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension(format!("{:016x}.tmp", rand::random::<u64>()));
    let mut file = std::fs::File::create_new(&tmp)?;
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| {
        // Bounded retry on Windows where AV/search indexers transiently hold the target file.
        let mut last_err = None;
        for attempt in 0..5 {
            match std::fs::rename(&tmp, path) {
                Ok(()) => return Ok(()),
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied && attempt < 4 => {
                    last_err = Some(err);
                    std::thread::sleep(std::time::Duration::from_millis(15 * (attempt as u64 + 1)));
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_err.unwrap_or_else(|| io::Error::new(io::ErrorKind::PermissionDenied, "rename failed")))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn save_user_at(path: &Path, data: &UserData) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(data)?;
    // Keep a rolling `.bak` copy of the last valid on-disk file before replacing it.
    if let Ok(prev) = std::fs::read(path) {
        if serde_json::from_slice::<serde_json::Value>(&prev).is_ok() {
            let _ = write_atomic(&user_backup_path(path), &prev);
        }
    }
    write_atomic(path, &bytes)
}

pub fn save_user(data: &UserData) -> std::io::Result<()> {
    save_user_at(&user_file(), data)
}

pub fn export_user_backup(destination: &Path, data: &UserData) -> io::Result<()> {
    write_atomic(destination, &serde_json::to_vec_pretty(data)?)
}

pub fn load_library_cache() -> Vec<Track> {
    std::fs::read(library_file())
        .ok()
        .and_then(|b| serde_json::from_slice::<Vec<Track>>(&b).ok())
        .unwrap_or_default().into_iter().filter(|track| track.youtube_id().is_some()).collect()
}

pub fn save_library_cache(lib: &Library) -> std::io::Result<()> {
    let tracks: Vec<&Track> = lib.tracks.values().map(|t| t.as_ref()).filter(|t| t.youtube_id().is_some()).collect();
    write_atomic(&library_file(), &serde_json::to_vec(&tracks)?)
}

/// Saves cached YouTube Music tracks while pruning unbounded unreferenced entries (> `1,500`)
/// and unconditionally preserving every track referenced by likes, playlists, history, play counts, or active session.
pub fn save_library_cache_pruned(lib: &Library, data: &UserData) -> io::Result<()> {
    let tracks = select_cached_tracks(lib, data, MAX_UNREFERENCED_YT_CACHE);
    write_atomic(&library_file(), &serde_json::to_vec(&tracks)?)
}

fn select_cached_tracks<'a>(lib: &'a Library, data: &UserData, max_unreferenced: usize) -> Vec<&'a Track> {
    let mut keep_ids = std::collections::HashSet::new();
    for entry in &data.liked {
        keep_ids.insert(entry.id);
    }
    for pl in &data.playlists {
        for entry in &pl.tracks {
            keep_ids.insert(entry.id);
        }
    }
    for &id in &data.listening_history {
        keep_ids.insert(id);
    }
    for (&id, &plays) in &data.play_counts {
        if plays > 0 {
            keep_ids.insert(id);
        }
    }
    if let Some(cur) = data.session.current {
        keep_ids.insert(cur);
    }
    for &id in &data.session.context {
        keep_ids.insert(id);
    }

    let mut pinned = Vec::new();
    let mut transient = Vec::new();
    for track in lib.tracks.values().map(|t| t.as_ref()) {
        if track.youtube_id().is_none() {
            continue;
        }
        if keep_ids.contains(&track.id) {
            pinned.push(track);
        } else {
            transient.push(track);
        }
    }
    if transient.len() > max_unreferenced {
        transient.sort_by(|a, b| b.added.cmp(&a.added).then_with(|| a.id.cmp(&b.id)));
        transient.truncate(max_unreferenced);
    }
    pinned.extend(transient);
    pinned
}

#[derive(Clone, Debug, Default)]
pub struct LocalDbStats {
    pub user_bytes: u64,
    pub library_bytes: u64,
    pub has_backup: bool,
    pub cached_yt_tracks: usize,
    pub playlists_count: usize,
    pub liked_count: usize,
    pub history_count: usize,
    pub total_plays: u64,
}

pub fn local_db_stats(lib: &Library, data: &UserData) -> LocalDbStats {
    let uf = user_file();
    let lf = library_file();
    LocalDbStats {
        user_bytes: std::fs::metadata(&uf).map(|m| m.len()).unwrap_or(0),
        library_bytes: std::fs::metadata(&lf).map(|m| m.len()).unwrap_or(0),
        has_backup: user_backup_path(&uf).is_file(),
        cached_yt_tracks: lib.tracks.values().filter(|t| t.youtube_id().is_some()).count(),
        playlists_count: data.playlists.len() + data.followed_ytm.len(),
        liked_count: data.liked.len(),
        history_count: data.listening_history.len(),
        total_plays: data.total_completed_plays(),
    }
}

pub fn imported_lyrics_file(track: &Track) -> PathBuf {
    let name = match track.path.strip_prefix("yt:") {
        Some(id) if id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') => {
            // Windows filenames are case-insensitive, but video IDs are case-sensitive.
            let hex = b"0123456789abcdef";
            let key: String = id.bytes().flat_map(|b| [char::from(hex[(b >> 4) as usize]), char::from(hex[(b & 15) as usize])]).collect();
            format!("yt-{key}.ttml")
        }
        _ => format!("{:016x}.ttml", track.id),
    };
    data_dir().join("lyrics").join(name)
}

pub fn import_lyrics(source: &Path, destination: &Path) -> std::io::Result<crate::lrc::Lyrics> {
    let xml = crate::ttml::read_text(source)?;
    let lyrics = crate::ttml::parse(&xml)?;
    write_atomic(destination, xml.as_bytes())?;
    Ok(lyrics)
}

// --------------------------------------------------------------------------------------------
// Demo collection

const DEMO_SONGS: &[(&str, &str, &str, &str, u64)] = &[
    ("Hoàng Hôn Trên Sông", "Nhóm Gió Nam", "Mùa Nước Nổi", "Lo-fi", 184),
    ("Con Đường Ven Đê", "Nhóm Gió Nam", "Mùa Nước Nổi", "Lo-fi", 201),
    ("Đèn Khuya", "Nhóm Gió Nam", "Mùa Nước Nổi", "Lo-fi", 167),
    ("Mưa Đầu Mùa", "Linh Anh", "Phố Sau Mưa", "Pop", 213),
    ("Phố Sau Mưa", "Linh Anh", "Phố Sau Mưa", "Pop", 195),
    ("Những Ngày Nắng", "Linh Anh", "Phố Sau Mưa", "Pop", 228),
    ("Quán Góc Phố", "Linh Anh", "Phố Sau Mưa", "Pop", 174),
    ("Nhịp Đêm", "Trạm Số 9", "Tín Hiệu", "Điện tử", 242),
    ("Tín Hiệu", "Trạm Số 9", "Tín Hiệu", "Điện tử", 219),
    ("Đường Hầm Ánh Sáng", "Trạm Số 9", "Tín Hiệu", "Điện tử", 256),
    ("Sao Băng", "Trạm Số 9", "Tín Hiệu", "Điện tử", 198),
    ("Bến Cũ", "An Khê", "Miền Gió", "Acoustic", 206),
    ("Miền Gió", "An Khê", "Miền Gió", "Acoustic", 181),
    ("Chiều Cao Nguyên", "An Khê", "Miền Gió", "Acoustic", 233),
    ("Lá Thư Chưa Gửi", "An Khê & Linh Anh", "Miền Gió", "Acoustic", 190),
    ("Một Vòng Thành Phố", "Bốn Mùa Trio", "Bốn Mùa", "Jazz", 247),
    ("Cà Phê Sáng", "Bốn Mùa Trio", "Bốn Mùa", "Jazz", 222),
    ("Nắng Hạ", "Bốn Mùa Trio", "Bốn Mùa", "Jazz", 205),
];

pub fn demo_tracks() -> Vec<Track> {
    DEMO_SONGS
        .iter()
        .enumerate()
        .map(|(i, (title, artist, album, genre, secs))| {
            let path = format!("demo:{i}");
            let number = DEMO_SONGS[..i].iter().filter(|s| s.2 == *album).count() as u32 + 1;
            Track {
                id: fnv(&path),
                path,
                title: (*title).to_string(),
                artist: (*artist).to_string(),
                album: (*album).to_string(),
                album_artist: None,
                genre: Some((*genre).to_string()),
                year: Some(2024 + (i as u32 % 3)),
                track_no: Some(number),
                disc_no: None,
                duration_ms: secs * 1000,
                loudness_db: Some(-14.0),
                peak_db: None,
                art_url: None,
                added: now().saturating_sub(i as u64 * 86_400),
                mtime: 0,
                lyrics: None,
                spotify_id: None,
                isrc: None,
            }
        })
        .collect()
}

pub fn demo_playlists() -> Vec<Playlist> {
    let tracks = demo_tracks();
    let make = |name: &str, desc: &str, picks: &[usize]| Playlist {
        id: fnv(name),
        name: name.to_string(),
        description: desc.to_string(),
        tracks: picks.iter().map(|&i| PlaylistEntry { id: tracks[i].id, added: tracks[i].added }).collect(),
        created: now(),
        image: None,
        pinned: false,
        folder_id: None,
        creator: None,
    };
    vec![
        make("Tập Trung Làm Việc", "Nhạc nhẹ để giữ nhịp cả ngày", &[0, 1, 2, 11, 12, 15, 16, 17]),
        make("Đi Đường Xa", "Cho những chuyến đi dài", &[3, 4, 5, 7, 9, 13]),
        make("Đêm Muộn", "Khi thành phố đã ngủ", &[2, 7, 8, 10, 14]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_songs_survive_save_and_old_profiles_still_load() {
        let mut data: UserData = serde_json::from_str("{}").unwrap();
        assert!(data.hidden_tracks.is_empty());
        data.hidden_tracks.insert(42);
        data.toggle_like(42);
        let saved = serde_json::to_vec(&data).unwrap();
        let restored: UserData = serde_json::from_slice(&saved).unwrap();
        assert!(restored.hidden_tracks.contains(&42));
        assert!(restored.is_liked(42));
    }

    #[test]
    fn history_moves_replayed_songs_to_the_front_and_caps_size() {
        let mut data = UserData::default();
        for id in 0..205 {
            data.record_listen(id);
        }
        data.record_listen(17);
        assert_eq!(&data.listening_history[..3], &[17, 204, 203]);
        assert_eq!(data.listening_history.len(), MAX_LISTENING_HISTORY);
        assert_eq!(data.listening_history.iter().filter(|&&id| id == 17).count(), 1);
    }

    #[test]
    fn portable_data_follows_the_executable_and_takes_precedence() -> io::Result<()> {
        let dir = std::env::temp_dir().join(format!("nhip-portable-{}-{}", std::process::id(), rand::random::<u64>()));
        let old_exe = dir.join("old/nhip.exe");
        let new_exe = dir.join("new/nhip.exe");
        let old_marker = old_exe.with_file_name("nhip.portable");
        let legacy = dir.join("legacy/user.json");
        let mut installed = UserData::default();
        installed.settings.volume = 0.25;
        installed.session.current = Some(7);
        write_atomic(&legacy, &serde_json::to_vec(&installed)?)?;
        write_atomic(&old_marker, b"portable")?;
        let old_data = portable_data_dir(&old_exe).unwrap();
        let old_file = old_data.join("user.json");
        let mut user = load_user_at(&old_file, &legacy)?;
        assert_eq!(user.settings.volume, 0.25);
        assert_eq!(user.session.current, None);
        user.settings.volume = 0.5;
        write_atomic(&old_file, &serde_json::to_vec(&user)?)?;
        std::fs::rename(old_exe.parent().unwrap(), new_exe.parent().unwrap())?;
        let new_data = portable_data_dir(&new_exe).unwrap();
        let new_file = new_data.join("user.json");
        assert_eq!(load_user_at(&new_file, &legacy)?.settings.volume, 0.5);
        write_atomic(&new_file, b"corrupt")?;
        assert_eq!(load_user_at(&new_file, &legacy).unwrap_err().kind(), io::ErrorKind::InvalidData);
        std::fs::remove_file(new_file)?;
        std::fs::remove_file(new_exe.with_file_name("nhip.portable"))?;
        std::fs::remove_dir(new_data)?;
        std::fs::remove_dir(new_exe.parent().unwrap())?;
        std::fs::remove_file(&legacy)?;
        std::fs::remove_dir(legacy.parent().unwrap())?;
        std::fs::remove_dir(dir)
    }

    #[test]
    fn imported_lyrics_keep_case_sensitive_video_ids_distinct_on_windows() {
        let mut track = demo_tracks().remove(0);
        track.path = "yt:Abcdefghijk".into();
        let first = imported_lyrics_file(&track).to_string_lossy().to_ascii_lowercase();
        track.path = "yt:abcdefghijk".into();
        assert_ne!(first, imported_lyrics_file(&track).to_string_lossy().to_ascii_lowercase());
    }

    #[test]
    fn imported_ttml_reopens_and_an_invalid_replacement_preserves_it() -> std::io::Result<()> {
        let dir = std::env::temp_dir().join(format!("nhip-ttml-{}-{}", std::process::id(), rand::random::<u64>()));
        std::fs::create_dir(&dir)?;
        let source = dir.join("source.ttml");
        let destination = dir.join("saved.ttml");
        let xml = "<tt><body><div><p begin='1s' end='2s'>Mưa rơi</p></div></body></tt>";
        std::fs::write(&source, xml)?;
        assert_eq!(import_lyrics(&source, &destination)?.current(1_500), Some(0));
        std::fs::write(&source, xml.replace("Mưa rơi", "Nắng lên"))?;
        import_lyrics(&source, &destination)?;
        std::fs::write(&source, "<broken>")?;
        assert!(import_lyrics(&source, &destination).is_err());
        assert_eq!(crate::ttml::parse(&crate::ttml::read_text(&destination)?)?.lines[0].text, "Nắng lên");
        std::fs::remove_file(&destination)?;
        assert!(source.is_file());
        std::fs::remove_file(source)?;
        std::fs::remove_dir(dir)
    }

    #[test]
    fn fold_strips_vietnamese_marks() {
        assert_eq!(fold("Sơn Tùng"), "son tung");
        assert_eq!(fold("ĐÀ LẠT"), "da lat");
    }

    #[test]
    fn artists_split_on_separators() {
        assert_eq!(split_artists("A & B"), vec!["A", "B"]);
        assert_eq!(split_artists("Solo"), vec!["Solo"]);
    }

    #[test]
    fn albums_and_artists_are_grouped() {
        let lib = Library::new(demo_tracks());
        assert_eq!(lib.albums.len(), 5);
        assert!(lib.artist("Linh Anh").is_some());
        assert!(lib.artist("An Khê").unwrap().tracks.len() >= 4);
    }

    fn song(album: &str, number: u32, secs: u64) -> Track {
        let mut t = demo_tracks().remove(0);
        t.album = album.to_string();
        t.track_no = Some(number);
        t.duration_ms = secs * 1000;
        t
    }

    #[test]
    fn next_track_on_an_album_stays_gapless_even_with_crossfade() {
        assert_eq!(transition(&song("A", 1, 200), &song("A", 2, 200), true, Some(6000)), Transition::Gapless);
    }

    #[test]
    fn other_songs_crossfade_only_when_both_are_long_enough() {
        assert_eq!(transition(&song("A", 1, 200), &song("B", 1, 200), true, Some(6000)), Transition::Crossfade);
        assert_eq!(transition(&song("A", 1, 10), &song("B", 1, 200), true, Some(6000)), Transition::Gapless);
        assert_eq!(transition(&song("A", 1, 10), &song("B", 1, 200), false, Some(6000)), Transition::None);
    }

    #[test]
    fn crossfade_of_zero_is_gapless_and_no_settings_is_a_stop() {
        assert_eq!(transition(&song("A", 1, 200), &song("B", 1, 200), false, Some(0)), Transition::Gapless);
        assert_eq!(transition(&song("A", 1, 200), &song("B", 1, 200), false, None), Transition::None);
    }

    #[test]
    fn legacy_user_data_deserializes_with_backwards_compatible_defaults() {
        let legacy_json = r#"{
            "playlists": [{
                "id": 42,
                "name": "Chill",
                "tracks": [],
                "created": 100
            }],
            "settings": {
                "lang": "En",
                "library_sort": "Added"
            }
        }"#;
        let mut data: UserData = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(data.settings.lang, Lang::En);
        assert_eq!(data.settings.library_sort, LibrarySort::Added);
        assert_eq!(data.settings.effective_library_sort(), LibrarySortBy::RecentlyAdded);
        assert_eq!(data.settings.library_view, LibraryViewMode::DefaultList);
        assert!(!data.settings.show_remaining_time);
        assert!(!data.settings.compact_tracklist);
        assert!(!data.settings.prefers_reduced_motion);
        assert!(data.settings.effective_lyrics_motion());
        data.settings.prefers_reduced_motion = true;
        assert!(!data.settings.effective_lyrics_motion());
        assert!(!data.settings.private_session);
        assert!(data.settings.autoplay);
        assert!(data.folders.is_empty());
        assert!(data.followed_artists.is_empty());
        assert_eq!(data.pinned_items, vec![RecentItem::Liked]);
        assert!(!data.playlists[0].pinned);
        assert_eq!(data.playlists[0].folder_id, None);
        assert_eq!(data.playlists[0].creator, None);
    }

    #[test]
    fn toggle_pin_works_for_playlists_albums_artists_and_liked() {
        let mut d = UserData::default();
        let p = Playlist::new("Roadtrip".into());
        let pid = p.id;
        d.playlists.push(p);

        assert!(d.is_pinned(&RecentItem::Liked));
        assert!(!d.toggle_pin(RecentItem::Liked));
        assert!(!d.is_pinned(&RecentItem::Liked));

        assert!(d.toggle_pin(RecentItem::Playlist(pid)));
        assert!(d.is_pinned(&RecentItem::Playlist(pid)));
        assert!(d.playlist(pid).unwrap().pinned);

        assert!(d.toggle_pin(RecentItem::Album("k1".into())));
        assert!(d.toggle_pin(RecentItem::Artist("Linh Anh".into())));
        assert!(d.is_pinned(&RecentItem::Album("k1".into())));
        assert!(d.is_pinned(&RecentItem::Artist("Linh Anh".into())));

        assert!(!d.toggle_pin(RecentItem::Playlist(pid)));
        assert!(!d.playlist(pid).unwrap().pinned);
    }

    #[test]
    fn folders_create_rename_group_and_delete_preserving_playlists() {
        let mut d = UserData::default();
        let p1 = Playlist::new("One".into());
        let p2 = Playlist::new("Two".into());
        let (id1, id2) = (p1.id, p2.id);
        d.playlists.push(p1);
        d.playlists.push(p2);

        let fid = d.create_folder("Focus".into());
        assert_eq!(d.folder(fid).unwrap().name, "Focus");
        assert!(d.rename_folder(fid, "  Work Mixes  ".into()));
        assert_eq!(d.folder(fid).unwrap().name, "Work Mixes");
        assert!(!d.rename_folder(fid, "   ".into()));

        assert!(d.move_playlist_to_folder(id1, Some(fid)));
        assert!(!d.move_playlist_to_folder(id1, Some(999_999)));
        assert_eq!(d.playlists_in_folder(Some(fid)).len(), 1);
        assert_eq!(d.playlists_in_folder(None).len(), 1);
        assert_eq!(d.playlists_in_folder(Some(fid))[0].id, id1);

        assert!(d.toggle_folder_pin(fid));
        assert!(d.folder(fid).unwrap().pinned);

        assert!(d.delete_folder(fid));
        assert!(d.folder(fid).is_none());
        assert_eq!(d.playlist(id1).unwrap().folder_id, None);
        assert_eq!(d.playlist(id2).unwrap().folder_id, None);
        assert_eq!(d.playlists_in_folder(None).len(), 2);
    }

    #[test]
    fn empty_album_uses_single_title_and_saved_albums_round_trip() {
        let mut t = demo_tracks().into_iter().next().unwrap();
        t.title = "Khóc Đấy".to_string();
        t.artist = "Thắng".to_string();
        t.album = "   ".to_string();
        assert_eq!(t.effective_album(), "Khóc Đấy");
        let key = t.album_key();
        let lib = Library::new(vec![t]);
        assert_eq!(lib.albums.len(), 1);
        assert_eq!(lib.albums[0].title, "Khóc Đấy");
        assert_eq!(lib.albums[0].key, key);

        let mut d = UserData::default();
        d.saved_albums.insert(key.clone());
        let encoded = serde_json::to_string(&d).unwrap();
        let decoded: UserData = serde_json::from_str(&encoded).unwrap();
        assert!(decoded.saved_albums.contains(&key));
    }

    #[test]
    fn artist_avatars_and_taste_scoring_distinguish_collaborators_and_demote_fast_skips() {
        let mut t = demo_tracks().into_iter().next().unwrap();
        t.id = fnv("yt:anThan00001");
        t.path = "yt:anThan00001".into();
        t.title = "An Thần".into();
        t.artist = "Low G, Thắng".into();
        let lib = Library::new(vec![t.clone()]);

        let mut data = UserData::default();
        assert!(data.record_artist_avatar("Low G", "https://lh3.googleusercontent.com/lowg"));
        assert!(data.record_artist_avatar("Thắng", "https://lh3.googleusercontent.com/thang"));
        assert_ne!(data.artist_avatar("low g"), data.artist_avatar("Thắng"));

        data.followed_artists.insert("Thắng".into());
        data.record_play_complete(t.id);
        let favored = data.score_recommendation(&lib, fnv("yt:other000001"), "Thắng");
        let unknown = data.score_recommendation(&lib, fnv("yt:other000002"), "Unknown Artist");
        assert!(favored > unknown, "followed/played artist should outrank unknown artist ({favored} vs {unknown})");

        let skipped_id = fnv("yt:skipped0001");
        data.record_fast_skip(skipped_id);
        data.record_fast_skip(skipped_id);
        let skipped = data.score_recommendation(&lib, skipped_id, "Unknown Artist");
        assert!(skipped < unknown, "fast-skipped track should be demoted ({skipped} vs {unknown})");
    }

    #[test]
    fn legacy_user_json_with_string_folders_migrates_without_failing() {
        let legacy = r#"{
            "folders": ["C:\\Users\\demo\\Music"],
            "playlists": [],
            "liked": [],
            "recent_searches": ["thắng"],
            "settings": { "lang": "Vi", "volume": 0.42, "surface_style": "AmbientBlur", "spotify_client_id": "" }
        }"#;
        let parsed: UserData = serde_json::from_str(legacy).expect("legacy user.json must deserialize");
        assert!(parsed.folders.is_empty());
        assert_eq!(parsed.recent_searches, vec!["thắng"]);
        assert!((parsed.settings.volume - 0.42).abs() < 1e-4);
        assert_eq!(parsed.settings.surface_style, SurfaceStyle::Mica);
    }

    #[test]
    fn corrupt_user_json_recovers_automatically_from_rolling_backup() -> io::Result<()> {
        let dir = std::env::temp_dir().join(format!("nhip-bak-{}-{}", std::process::id(), rand::random::<u64>()));
        let user_path = dir.join("user.json");
        let mut v1 = UserData::default();
        v1.settings.profile_name = "Minh".into();
        v1.toggle_like(9001);
        save_user_at(&user_path, &v1)?;

        // Second save creates `user.json.bak` containing v1.
        let mut v2 = v1.clone();
        v2.settings.profile_name = "Minh Pro".into();
        save_user_at(&user_path, &v2)?;
        assert!(user_backup_path(&user_path).is_file());

        // Simulate corrupt primary `user.json`.
        write_atomic(&user_path, b"{truncated_corrupt_json")?;
        let recovered = load_user_at(&user_path, &user_path)?;
        assert_eq!(recovered.settings.profile_name, "Minh");
        assert!(recovered.is_liked(9001));

        let _ = std::fs::remove_dir_all(dir);
        Ok(())
    }

    #[test]
    fn pruned_library_cache_preserves_referenced_tracks_and_caps_unreferenced() {
        let mut tracks = Vec::new();
        for i in 0..10u64 {
            let vid = format!("vid{i:08}");
            let mut t = demo_tracks().remove(0);
            t.id = fnv(&format!("yt:{vid}"));
            t.path = format!("yt:{vid}");
            t.added = 100 + i;
            tracks.push(t);
        }
        let lib = Library::new(tracks.clone());
        let mut data = UserData::default();
        data.toggle_like(tracks[0].id); // oldest track is liked -> MUST stay!
        data.record_play_complete(tracks[1].id); // played -> MUST stay!

        let kept = select_cached_tracks(&lib, &data, 3);
        let kept_ids: std::collections::HashSet<TrackId> = kept.iter().map(|t| t.id).collect();
        assert_eq!(kept.len(), 5); // 2 referenced + 3 newest unreferenced
        assert!(kept_ids.contains(&tracks[0].id));
        assert!(kept_ids.contains(&tracks[1].id));
        assert!(kept_ids.contains(&tracks[9].id));
        assert!(kept_ids.contains(&tracks[8].id));
        assert!(kept_ids.contains(&tracks[7].id));
        assert!(!kept_ids.contains(&tracks[2].id));
    }
}
