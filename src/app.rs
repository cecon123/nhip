//! Application state and everything that happens between frames.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;

use crate::art::ArtCache;
use crate::audio::{AudioSettings, Engine, Event, LoadSpec, SourceSpec};
#[allow(unused_imports)]
pub use crate::library::{
    LibraryCreatorFilter, LibrarySortBy, LibraryViewMode, PlaylistFolder, TableSort, TrackSortColumn,
};
use crate::library::{self, Lang, Library, Playlist, PlaylistEntry, RecentItem, Track, TrackId, UserData};
use crate::lrc::Lyrics;
use crate::mechanics::{volume_key_step, Transition, SEEK_STEP_MS, VOLUME_STEPS};
use crate::queue::{ContextRef, Player, Previous};
use crate::theme;

pub const SCREENSHOT_FRAMES: u64 = 120;
pub const PLAYLIST_DESCRIPTION_MAX_CHARS: usize = 300;
pub const SEARCH_DEBOUNCE_MS: u64 = 200;
pub const LEFT_EXPANDED: f32 = 584.0;
pub const SIDEBAR_KEY_STEP: f32 = 10.0;
const MAX_ERROR_STREAK: u32 = 5;

#[derive(Clone, PartialEq, Debug)]
pub enum Route {
    Home,
    Search,
    Liked,
    Playlist(u64),
    Album(String),
    Artist(String),
    Profile,
    TasteProfile,
    Settings,
    Lyrics,
    YouTubeMusicItem(crate::ytm::Item),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum RightTab {
    NowPlaying,
    Queue,
    History,
}

impl RightTab {
    pub fn is_queue(self) -> bool {
        matches!(self, Self::Queue | Self::History)
    }
}
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum HomeFilter {
    #[default]
    All,
    Music,
    Albums,
    Artists,
    Playlists,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum LibraryFilter {
    #[default]
    All,
    Playlists,
    Albums,
    Artists,
}

#[derive(Clone, PartialEq, Debug)]
pub enum ConfirmAction {
    ClearQueue,
    DeletePlaylist(u64),
    Unpin(RecentItem),
    DeleteTtml(TrackId),
    DeleteFolder(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct EditPlaylistState {
    pub id: u64,
    pub name: String,
    pub description: String,
    pub image: Option<String>,
    pub focus: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum TtmlImportMode {
    #[default]
    Save,
    JustOnce,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TopPopover {
    Notifications,
    Account,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FocusRegion {
    Sidebar,
    #[default]
    Main,
    Right,
}

#[derive(Clone)]
pub enum Action {
    Navigate(Route),
    Back,
    Forward,
    PlayContext { context: ContextRef, name: String, tracks: Vec<TrackId>, start: Option<usize> },
    PlayItem(RecentItem),
    SelectYouTubeSong(crate::ytm::Item),
    PlayYouTubeList { name: String, items: Vec<crate::ytm::Item>, start: usize },
    LikeYouTubeSong(crate::ytm::Item),
    EnqueueYouTubeSong(crate::ytm::Item),
    AddYouTubeSong(u64, crate::ytm::Item),
    CreatePlaylistWithSong(crate::ytm::Item),
    TogglePlay,
    Next,
    Previous,
    SeekTo(u64),
    SeekBy(i64),
    ToggleShuffle,
    CycleRepeat,
    ToggleRemainingTime,
    ToggleLike(TrackId),
    ToggleArtistFollow(String),
    HideTrack(TrackId),
    RestoreHiddenTracks,
    Enqueue(TrackId),
    Dequeue(usize),
    MoveQueueItem(usize, usize),
    ClearQueue,
    RequestClearQueue,
    RequestConfirm(ConfirmAction),
    AcceptConfirm,
    CancelConfirm,
    JumpUpcoming(TrackId),
    PlayQueued(usize),
    AddToPlaylist(u64, TrackId),
    RemoveFromPlaylist(u64, TrackId),
    MoveInPlaylist(u64, TrackId, TrackId),
    CreatePlaylist(Option<TrackId>),
    DeletePlaylist(u64),
    BeginRename(u64),
    SaveRename(u64, String),
    BeginEditPlaylist(u64),
    PickPlaylistCover(u64),
    ClearPlaylistCover,
    SaveEditPlaylist,
    CloseEditPlaylist,
    TogglePin(RecentItem),
    ToggleFolderPin(u64),
    CreateFolder(Option<String>),
    RenameFolder(u64, String),
    DeleteFolder(u64),
    MovePlaylistToFolder(u64, Option<u64>),
    OpenFolder(Option<u64>),
    SetVolume(f32),
    ToggleMute,
    ToggleRight(RightTab),
    CloseRight,
    ToggleLyrics,
    ToggleLyricsFullscreen,
    ToggleLyricsCardExpand,
    ToggleLyricsCardCollapse,
    ToggleLyricsCinema,
    SetTtmlImportMode(TtmlImportMode),
    ImportLyrics(TrackId),
    ImportLyricsWithMode(TrackId, TtmlImportMode),
    ExportLyrics(TrackId),
    ResetLyrics(TrackId),
    RemoveImportedLyrics(TrackId),
    RequestDeleteTtml(TrackId),
    ConfirmDeleteTtmlStep(TrackId),
    ToggleLeft,
    ExpandLeft,
    SetLeftWidth(f32),
    SortLibrary(library::LibrarySort),
    SetLibrarySort(LibrarySortBy),
    SetLibraryView(LibraryViewMode),
    SetLibraryFilter(LibraryFilter),
    SetLibraryCreatorFilter(LibraryCreatorFilter),
    SetHomeFilter(HomeFilter),
    ScrollHomeToTop,
    SetSearchDropdown(bool),
    SearchInputChanged,
    SubmitSearch,
    SearchCategory(String),
    SelectRecentSearch(String),
    RemoveRecentSearch(String),
    ClearRecentSearches,
    ToggleCompactTracklist,
    SetCompactTracklist(bool),
    SetTableSort(TableSort),
    ToggleTableSort(TrackSortColumn),
    SetEntityFilter(String),
    ClearEntityFilter,
    ToggleNotifications,
    ToggleFriendActivity,
    ToggleAccountMenu,
    TogglePrivateSession,
    ToggleMiniplayer,
    ToggleSaveAlbum(String),
    OpenPreviewFeed(PreviewFeedState),
    PreviewFeedStep(isize),
    ClosePreviewFeed,
    ClosePopovers,
    SettingsChanged,
}

#[derive(Clone, Debug)]
pub struct PreviewTrack {
    pub title: String,
    pub artist: String,
    pub cover_key: String,
    pub cover_path: Option<String>,
    pub ytm_item: Option<crate::ytm::Item>,
    pub local_id: Option<TrackId>,
    pub duration_ms: u64,
    pub url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PreviewFeedState {
    pub entity_title: String,
    pub entity_subtitle: Option<String>,
    pub entity_avatar_key: String,
    pub entity_avatar_src: Option<String>,
    pub artist_name: String,
    pub items: Vec<PreviewTrack>,
    pub index: usize,
    pub scroll_anim: f32,
    pub last_wheel: Option<Instant>,
    pub tags: Vec<String>,
}

pub struct RenameState {
    pub id: u64,
    pub name: String,
    pub focus: bool,
}

pub struct App {
    pub lib: Library,
    pub data: UserData,
    pub engine: Engine,
    pub player: Player,
    pub art: ArtCache,
    pub route: Route,
    pub back: Vec<Route>,
    pub forward: Vec<Route>,
    pub actions: Vec<Action>,
    pub search: String,
    pub instant_query: String,
    pub search_dropdown_open: bool,
    pub search_has_focus: bool,
    search_debounce_at: Option<Instant>,
    pub focus_search: bool,
    pub lib_filter: LibraryFilter,
    pub lib_creator_filter: LibraryCreatorFilter,
    pub lib_query: String,
    pub lib_search: bool,
    pub focus_lib_search: bool,
    pub opened_folder: Option<u64>,
    pub rename_folder: Option<RenameState>,
    pub home_filter: HomeFilter,
    pub home_scroll_to_top: bool,
    pub scroll_to_top_seq: u64,
    pub table_sort: TableSort,
    pub entity_filter: String,
    pub entity_filter_open: bool,
    pub focus_entity_filter: bool,
    pub right_tab: RightTab,
    pub help_open: bool,
    pub rename: Option<RenameState>,
    pub edit_playlist: Option<EditPlaylistState>,
    pub confirm: Option<ConfirmAction>,
    pub seek_drag: Option<f32>,
    pub lyrics: Option<Lyrics>,
    pub lyrics_for: Option<TrackId>,
    pub lyrics_fullscreen: Option<bool>,
    pub lyrics_card_expanded: bool,
    pub lyrics_card_collapsed: bool,
    pub lyrics_cinema: bool,
    pub lyrics_nowbar_right: bool,
    pub lyrics_cinema_queue: bool,
    pub lyrics_split_nowbar: bool,
    pub lyrics_manager_open: bool,
    pub lyrics_manager_query: String,
    pub lyrics_ttml_mode: TtmlImportMode,
    pub lyrics_session_override: bool,
    pub lyrics_delete_confirm_for: Option<TrackId>,
    session_lyrics_overrides: HashMap<TrackId, String>,
    pub ytm: crate::ui::ytm::Browser,
    lyrics_imported: bool,
    lyrics_response: Option<Receiver<Option<Lyrics>>>,
    session_played_yt: HashSet<String>,
    last_yt_seed: Option<String>,
    last_prefetched_next: Option<TrackId>,
    autoplay_req_id: u64,
    autoplay_response: Option<(u64, bool, Receiver<Result<Vec<crate::ytm::Item>, crate::ytm::Error>>)>,
    pub top_popover: Option<TopPopover>,
    pub preview_feed: Option<PreviewFeedState>,
    pub friend_activity_open: bool,
    pub miniplayer_open: bool,
    pub unread_notifications: bool,
    pub focused_region: FocusRegion,
    pub toast: Option<(String, Instant)>,
    pub demo_playlist_ids: Vec<u64>,
    error_streak: u32,
    halt_after_errors: bool,
    pub(crate) dirty: bool,
    can_save_user: bool,
    last_save: Instant,
    library_dirty: bool,
    context: egui::Context,
    #[cfg(windows)]
    media: Option<crate::media::Media>,
    media_track: Option<TrackId>,
    media_playing: bool,
    pub(crate) screenshot: Option<PathBuf>,
    frame: u64,
    shot_requested: bool,
    shot_done: Arc<std::sync::atomic::AtomicBool>,
}

fn audio_settings(data: &UserData) -> AudioSettings {
    let s = &data.settings;
    AudioSettings {
        crossfade_ms: if s.crossfade { s.crossfade_seconds as u64 * 1000 } else { 0 },
        eq_enabled: s.eq_enabled,
        eq_gains: s.eq_gains,
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>, page: Option<String>) -> Self {
        let ctx = cc.egui_ctx.clone();
        theme::install_fonts(&ctx);
        theme::apply_style(&ctx);

        let demo_mode = screenshot.is_some();
        let loaded = if demo_mode { Ok(UserData::default()) } else { library::load_user() };
        let can_save_user = loaded.is_ok();
        let data = loaded.unwrap_or_default();
        let cached = if demo_mode { Vec::new() } else { library::load_library_cache() };
        let wake = ctx.clone();
        let engine = Engine::start(audio_settings(&data), move || wake.request_repaint());

        let mut app = Self {
            lib: Library::new(cached),
            player: Player::default(),
            engine,
            art: ArtCache::new(ctx.clone()),
            route: Route::Home,
            back: Vec::new(),
            forward: Vec::new(),
            actions: Vec::new(),
            search: String::new(),
            instant_query: String::new(),
            search_dropdown_open: false,
            search_has_focus: false,
            search_debounce_at: None,
            focus_search: false,
            lib_filter: LibraryFilter::All,
            lib_creator_filter: LibraryCreatorFilter::All,
            lib_query: String::new(),
            lib_search: false,
            focus_lib_search: false,
            opened_folder: None,
            rename_folder: None,
            home_filter: HomeFilter::All,
            home_scroll_to_top: false,
            scroll_to_top_seq: 0,
            table_sort: TableSort::default(),
            entity_filter: String::new(),
            entity_filter_open: false,
            focus_entity_filter: false,
            right_tab: RightTab::NowPlaying,
            help_open: false,
            rename: None,
            edit_playlist: None,
            confirm: None,
            seek_drag: None,
            lyrics: None,
            lyrics_for: None,
            lyrics_fullscreen: None,
            lyrics_card_expanded: false,
            lyrics_card_collapsed: false,
            lyrics_cinema: false,
            lyrics_nowbar_right: false,
            lyrics_cinema_queue: false,
            lyrics_split_nowbar: false,
            lyrics_manager_open: false,
            lyrics_manager_query: String::new(),
            lyrics_ttml_mode: TtmlImportMode::Save,
            lyrics_session_override: false,
            lyrics_delete_confirm_for: None,
            session_lyrics_overrides: HashMap::new(),
            ytm: Default::default(),
            lyrics_imported: false,
            lyrics_response: None,
            session_played_yt: HashSet::new(),
            last_yt_seed: None,
            last_prefetched_next: None,
            autoplay_req_id: 0,
            autoplay_response: None,
            top_popover: None,
            preview_feed: None,
            friend_activity_open: false,
            miniplayer_open: false,
            unread_notifications: false,
            focused_region: FocusRegion::Main,
            toast: None,
            demo_playlist_ids: Vec::new(),
            error_streak: 0,
            halt_after_errors: false,
            dirty: false,
            can_save_user,
            last_save: Instant::now(),
            library_dirty: false,
            context: ctx.clone(),
            #[cfg(windows)]
            media: None,
            media_track: None,
            media_playing: false,
            screenshot,
            frame: 0,
            shot_requested: false,
            shot_done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            data,
        };

        app.player.shuffle = app.data.settings.shuffle;
        app.player.repeat = app.data.settings.repeat;
        app.apply_volume();
        if !can_save_user {
            app.toast(app.t("Không thể đọc dữ liệu đã lưu. Tệp hiện có được bảo toàn.", "Could not read saved data. The existing file was preserved.").into());
        }

        if demo_mode {
            match page.as_deref() {
                Some("youtube-music") => {}
                Some("youtube-music-settings") => app.route = Route::Settings,
                Some("youtube-music-search") => {
                    app.route = Route::Search;
                    app.search = "Khóc Đấy".into();
                }
                Some("youtube-music-album") => {
                    app.route = Route::YouTubeMusicItem(app.ytm_item(crate::ytm::ItemId::Album("MPREb_LSeSvSkBOEM".into())));
                }
                page => {
                    app.enter_demo();
                    app.stage_for_picture(page);
                }
            }
        } else {
            app.restore_session();
            app.setup_media(cc);
        }
        app
    }

    #[cfg(windows)]
    fn setup_media(&mut self, cc: &eframe::CreationContext<'_>) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        #[link(name = "dwmapi")]
        extern "system" {
            fn DwmSetWindowAttribute(
                hwnd: isize,
                dwAttribute: u32,
                pvAttribute: *const i32,
                cbAttribute: u32,
            ) -> i32;
        }
        let Ok(handle) = cc.window_handle() else { return };
        if let RawWindowHandle::Win32(h) = handle.as_raw() {
            let hwnd = h.hwnd.get();
            // SAFETY: Enables native Windows 11 DWM rounded corners + OS drop shadow on our main frameless window.
            unsafe {
                let round: i32 = 2; // DWMWCP_ROUND
                let _ = DwmSetWindowAttribute(hwnd, 33, &round, 4);
            }
            let wake = self.context.clone();
            self.media = crate::media::Media::new(hwnd as *mut std::ffi::c_void, move || wake.request_repaint());
        }
    }

    #[cfg(not(windows))]
    fn setup_media(&mut self, _cc: &eframe::CreationContext<'_>) {}

    pub fn lang(&self) -> Lang {
        self.data.settings.lang
    }

    /// Picks the Vietnamese or English wording.
    pub fn t(&self, vi: &'static str, en: &'static str) -> &'static str {
        match self.lang() {
            Lang::Vi => vi,
            Lang::En => en,
        }
    }

    fn enter_demo(&mut self) {
        self.lib = Library::new(library::demo_tracks());
        self.lib.demo = true;
        let playlists = library::demo_playlists();
        self.demo_playlist_ids = playlists.iter().map(|p| p.id).collect();
        for p in playlists {
            if self.data.playlist(p.id).is_none() {
                self.data.playlists.push(p);
            }
        }
    }

    /// Fills the app with sample content for the check picture: nothing personal, nothing saved.
    fn stage_for_picture(&mut self, page: Option<&str>) {
        let ids = self.lib.order.clone();
        for &id in ids.iter().step_by(3) {
            self.data.toggle_like(id);
        }
        for p in self.data.playlists.clone() {
            self.data.touch_recent(RecentItem::Playlist(p.id));
        }
        for a in self.lib.albums.clone() {
            self.data.touch_recent(RecentItem::Album(a.key));
        }
        let album = self.lib.albums[0].clone();
        self.player.play_context(ContextRef::Album(album.key), album.title, album.tracks.clone(), Some(1));
        if let Some(id) = self.player.current {
            self.start_track(id, 74_000, false);
        }
        let other = self.lib.albums[2].tracks.clone();
        for &id in other.iter().take(2) {
            self.player.enqueue(id);
        }
        self.sync_next();

        if let Ok(offset) = std::env::var("NHIP_SCREENSHOT_SCROLL").unwrap_or_default().parse::<f32>() {
            self.context.data_mut(|d| d.insert_temp(egui::Id::NULL.with("npv-screenshot-scroll"), offset.max(0.0)));
        }
        // Screenshot-only fixture: never read or overwrite the user's library.
        if let Some(path) = std::env::var_os("NHIP_SCREENSHOT_TRACK") {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(mut track) = serde_json::from_slice::<Track>(&bytes) {
                    if let Some(id) = self.player.current {
                        track.id = id;
                        track.path = "demo:0".into();
                        self.lib.tracks.insert(id, Arc::new(track));
                        self.lib.demo = false;
                        self.start_track(id, 74_000, false);
                    }
                }
            }
        }

        if let Some(cov_path) = std::env::var_os("NHIP_SCREENSHOT_COVER") {
            if let Ok(bytes) = std::fs::read(cov_path) {
                if let Ok(decoded) = image::load_from_memory(&bytes) {
                    let rgba = crate::art::strip_letterbox_bars(decoded)
                        .resize_to_fill(544, 544, image::imageops::FilterType::CatmullRom)
                        .to_rgba8();
                    let cimg = egui::ColorImage::from_rgba_unmultiplied([544, 544], &rgba);
                    if let Some(t) = self.current_track() {
                        self.art.inject_image(&self.context, &t.cover_key(), cimg);
                    }
                }
            }
        }

        // `--page` lets a check picture show something other than the home page.
        let first_album = self.lib.albums[0].key.clone();
        let first_artist = self.lib.artists[0].name.clone();
        let first_playlist = self.data.playlists[0].id;
        match page {
            Some("album") => self.route = Route::Album(first_album),
            Some("album-scroll") => {
                self.route = Route::Album(first_album);
                self.context.data_mut(|d| {
                    d.insert_temp(egui::Id::NULL.with("album-screenshot-scroll"), 336.0_f32);
                });
            }
            Some("artist") => self.route = Route::Artist(first_artist),
            Some("preview") => {
                self.route = Route::Artist(first_artist.clone());
                let tracks: Vec<PreviewTrack> = self.lib.artists[0]
                    .tracks
                    .iter()
                    .filter_map(|&tid| {
                        let tr = self.lib.track(tid)?;
                        Some(PreviewTrack {
                            title: tr.title.clone(),
                            artist: tr.artist.clone(),
                            cover_key: tr.cover_key(),
                            cover_path: tr.art_source().map(str::to_string),
                            ytm_item: None,
                            local_id: Some(tid),
                            duration_ms: tr.duration_ms,
                            url: None,
                        })
                    })
                    .collect();
                if !tracks.is_empty() {
                    self.preview_feed = Some(PreviewFeedState {
                        entity_title: first_artist.clone(),
                        entity_subtitle: Some("657,5 N".into()),
                        entity_avatar_key: format!("ytm-artist:{first_artist}"),
                        entity_avatar_src: None,
                        artist_name: first_artist,
                        items: tracks,
                        index: 1,
                        scroll_anim: 1.0,
                        last_wheel: None,
                        tags: vec!["#indie việt".into(), "#thư giãn".into(), "#khám phá".into()],
                    });
                }
            }
            Some("playlist") => self.route = Route::Playlist(first_playlist),
            Some("liked") => self.route = Route::Liked,
            Some("profile") => self.route = Route::Profile,
            Some("taste") => self.route = Route::TasteProfile,
            Some("settings") => {
                self.route = Route::Settings;
                self.data.settings.eq_enabled = true;
                self.data.settings.eq_gains = crate::mechanics::EQ_PRESETS[2].gains;
                self.data.settings.eq_preset = 2;
                self.data.settings.crossfade = true;
            }
            Some("browse") => self.route = Route::Search,
            Some("search") => {
                self.route = Route::Search;
                self.search = "nang".to_string();
            }
            Some("queue") => self.right_tab = RightTab::Queue,
            Some("npv-scroll") => {
                self.context.data_mut(|d| {
                    d.insert_temp(egui::Id::NULL.with("npv-screenshot-scroll"), 320.0_f32);
                });
            }
            Some("search-dropdown-skeleton") => {
                self.search = "con mèo".to_string();
                self.focus_search = true;
                self.search_dropdown_open = true;
                self.context.data_mut(|d| {
                    d.insert_temp(egui::Id::NULL.with("force-search-skeleton"), true);
                });
            }
            Some(
                "lyrics"
                | "lyrics-hover"
                | "lyrics-scroll"
                | "lyrics-scroll-hover"
                | "lyrics-scroll-below"
                | "lyrics-card"
                | "lyrics-fullscreen"
                | "lyrics-cinema"
                | "lyrics-cinema-hover"
                | "lyrics-cinema-liked"
                | "lyrics-cinema-queue"
                | "cinema-slide-mid"
                | "spicy-popup"
                | "spicy-popup-scroll-idle"
                | "spicy-popup-f1"
                | "spicy-popup-f2"
                | "spicy-popup-hover"
                | "spicy-popup-sleep"
                | "spicy-popup-cover"
                | "spicy-popup-narrow"
                | "spicy-popup-bar",
            ) => {
                let is_popup_mode = page.is_some_and(|p| p.starts_with("spicy-popup"));
                if !matches!(page, Some("lyrics-card" | "cinema-slide-mid")) && !is_popup_mode {
                    self.route = Route::Lyrics;
                }
                if page == Some("cinema-slide-mid") {
                    self.context.data_mut(|d| {
                        d.insert_temp(egui::Id::NULL.with("cinema-slide-override"), 0.52_f32);
                    });
                }
                if is_popup_mode {
                    self.miniplayer_open = true;
                    // Overlay over Cinema View just like `7a34afb4.jpg` & `02e48b2e.jpg`
                    self.route = Route::Lyrics;
                    self.lyrics_cinema = true;
                    let stage_label: &'static str = match page {
                        Some("spicy-popup" | "spicy-popup-scroll-idle" | "spicy-popup-cover") => "idle",
                        Some("spicy-popup-f1") => "f1",
                        Some("spicy-popup-f2") => "f2",
                        Some("spicy-popup-sleep") => "sleep1",
                        _ => "hover",
                    };
                    self.context.data_mut(|d| {
                        d.insert_temp(egui::Id::NULL.with("spicy-popup-stage"), stage_label);
                        if page == Some("spicy-popup-scroll-idle") {
                            d.insert_temp(egui::Id::NULL.with("spicy-popup-show-scrollbar"), true);
                        }
                        if page == Some("spicy-popup-narrow") {
                            d.insert_temp(egui::Id::NULL.with("spicy-popup-size"), [260.0_f32, 195.0_f32]);
                        } else if page == Some("spicy-popup-bar") {
                            d.insert_temp(egui::Id::NULL.with("spicy-popup-size"), [340.0_f32, 118.0_f32]);
                            d.insert_temp(egui::Id::NULL.with("spicy-popup-force-hover"), true);
                        }
                    });
                    if page == Some("spicy-popup-cover") {
                        self.data.settings.miniplayer_cover_mode = true;
                    }
                    // Make current demo track unliked so outline Heart matches `02e48b2e.jpg`
                    if let Some(id) = self.player.current {
                        if self.data.is_liked(id) {
                            self.data.toggle_like(id);
                        }
                    }
                }
                if page == Some("lyrics-hover") {
                    self.context.data_mut(|d| {
                        d.insert_temp(egui::Id::NULL.with("lyrics-force-top-hover"), true);
                    });
                }
                if matches!(page, Some("lyrics-scroll" | "lyrics-scroll-hover" | "lyrics-scroll-below")) {
                    self.context.data_mut(|d| {
                        d.insert_temp(egui::Id::NULL.with("lyrics-force-manual-scroll"), true);
                        if page == Some("lyrics-scroll-hover") {
                            d.insert_temp(egui::Id::NULL.with("lyrics-force-sync-hover"), true);
                        }
                        if page == Some("lyrics-scroll-below") {
                            d.insert_temp(egui::Id::NULL.with("lyrics-force-sync-below"), true);
                        }
                    });
                }
                self.lyrics_cinema |= matches!(
                    page,
                    Some(
                        "lyrics-cinema"
                            | "lyrics-cinema-hover"
                            | "lyrics-cinema-liked"
                            | "lyrics-cinema-queue"
                    )
                );
                self.lyrics_cinema_queue = page == Some("lyrics-cinema-queue");
                if matches!(page, Some("lyrics-cinema-hover" | "lyrics-cinema-liked")) {
                    self.context.data_mut(|d| {
                        d.insert_temp(egui::Id::NULL.with("lyrics-force-cover-hover"), true);
                    });
                }
                if page == Some("lyrics-cinema-liked") {
                    if let Some(id) = self.player.current {
                        if !self.data.is_liked(id) {
                            self.data.toggle_like(id);
                        }
                    }
                }
                if page == Some("lyrics-fullscreen") { self.lyrics_fullscreen = Some(false); }
                if is_popup_mode {
                    // Position near end of lyrics BEFORE assigning `self.lyrics` (`start_track` resets `lyrics_for`!)
                    if let Some(id) = self.player.current {
                        self.start_track(id, 98_000, false);
                    }
                }
                self.lyrics_for = self.player.current;
                let mut parsed = crate::ttml::parse(r#"<tt xmlns="http://www.w3.org/ns/ttml"><head><metadata><songwriters><songwriter>Vũ Đinh Trọng Thắng</songwriter></songwriters></metadata></head><body><div>
                    <p begin="56s" end="61s">Nắng rơi trên mái nhà</p>
                    <p begin="61s" end="66s">Gió đùa qua tán lá</p>
                    <p begin="66s" end="71s">Ta qua con đường nhỏ</p>
                    <p begin="71s" end="78s"><span begin="0s" end="1s">Nghe</span>
                        <span begin="1s" end="1.8s">thành</span> <span begin="1.8s" end="2.5s">phố</span>
                        <span begin="2.5s" end="4.1s">thở</span> <span begin="4.1s" end="6s">chậm</span></p>
                    <p begin="78s" end="83s">Một ngày dài vừa khép</p>
                    <p begin="83s" end="89s">Còn ánh đèn bên sông</p>
                    <p begin="89s" end="96s">Ta đi qua mùa nước</p>
                    <p begin="96s" end="102s">Mang bình yên về nhà</p>
                </div></body></tt>"#).ok();
                if let Some(ref mut l) = parsed {
                    l.provider = Some("Apple Music");
                }
                self.lyrics = parsed;
                if std::env::var_os("NHIP_SCREENSHOT_TRACK").is_some() {
                    self.lyrics = Some(Lyrics::parse("[00:56.00] Ý thơ người ví em là ánh trăng lẻ loi\n[01:01.00] Giá băng và đớn đau chờ chút ánh sáng ấm hơi bình minh\n[01:11.00] Anh là nắng, anh là gió, anh là đông, xuân, hạ, thu của đời em\n[01:18.00] Thế gian đổi khác, em không còn anh\n[01:23.00] Thế nên đừng nhắc thêm về ký ức đã nứt ra làm đôi\n[01:29.00] Hết thật rồi, hết thật rồi, anh về đi"));
                }
                if let Some(lrc_path) = std::env::var_os("NHIP_SCREENSHOT_LRC") {
                    if let Ok(text) = std::fs::read_to_string(lrc_path) {
                        self.lyrics = Some(Lyrics::parse(&text));
                    }
                }

            }
            Some("help") => self.help_open = true,
            Some("collapsed") => self.data.settings.left_collapsed = true,
            _ => {}
        }
    }

    fn restore_session(&mut self) {
        let session = std::mem::take(&mut self.data.session);
        if let Some((id, position)) = self.player.restore(session, |id| self.lib.tracks.contains_key(&id)) {
            self.start_track(id, position, false);
        }
    }

    fn apply_volume(&self) {
        let s = &self.data.settings;
        self.engine.set_volume(if s.muted { 0.0 } else { s.volume });
    }

    pub fn track(&self, id: TrackId) -> Option<Arc<Track>> {
        self.lib.track(id)
    }

    pub fn current_track(&self) -> Option<Arc<Track>> {
        self.player.current.and_then(|id| self.lib.track(id))
    }

    pub fn is_playing(&self) -> bool {
        self.player.current.is_some() && !self.engine.is_paused()
    }

    pub fn position_ms(&self) -> u64 {
        if self.player.current.is_some() {
            self.engine.position_ms()
        } else {
            0
        }
    }

    pub fn duration_ms(&self) -> u64 {
        let d = self.engine.duration_ms();
        if d > 0 {
            d
        } else {
            self.current_track().map(|t| t.duration_ms).unwrap_or(0)
        }
    }

    fn can_play_track(track: &Track) -> bool {
        track.demo_seed().is_some() || track.youtube_id().is_some()
    }

    fn track_load_spec(&self, track: &Track, transition: Transition) -> LoadSpec {
        let s = &self.data.settings;
        match track.youtube_id() {
            Some(video_id) => LoadSpec {
                id: track.id,
                source: SourceSpec::YouTube { video_id: video_id.to_string() },
                duration_ms: track.duration_ms,
                gain_db: track.gain_db(s.level, s.normalize),
                transition,
            },
            None => track.load_spec(s.level, s.normalize, transition),
        }
    }

    fn load_spec(&self, id: TrackId) -> Option<LoadSpec> {
        self.lib
            .track(id)
            .filter(|track| Self::can_play_track(track))
            .map(|t| self.track_load_spec(&t, Transition::Gapless))
    }

    fn note_youtube_started(&mut self, id: TrackId) {
        if let Some(vid) = self.track(id).and_then(|t| t.youtube_id().map(str::to_owned)) {
            self.session_played_yt.insert(vid.clone());
            self.last_yt_seed = Some(vid);
        }
    }

    fn start_track(&mut self, id: TrackId, start_ms: u64, play: bool) {
        self.halt_after_errors = false;
        self.cancel_autoplay_request();
        let Some(spec) = self.load_spec(id) else {
            self.engine.stop();
            return;
        };
        self.note_youtube_started(id);
        self.engine.load(spec, start_ms, play);
        self.after_track_change();
    }

    fn seek(&mut self, ms: u64) {
        if let Some(id) = self.player.current {
            if self.engine.has_song() {
                self.engine.seek(id, ms);
                self.dirty = true;
            } else {
                self.start_track(id, ms, !self.engine.is_paused());
            }
        }
    }

    /// Everything that has to follow the current song changing.
    fn after_track_change(&mut self) {
        self.sync_next();
        self.maybe_prefetch_autoplay();
        self.dirty = true;
        self.lyrics = None;
        self.lyrics_for = None;
        self.lyrics_session_override = false;
        self.lyrics_delete_confirm_for = None;
        let _ = self.current_lyrics();
    }

    pub fn show_remaining_time(&self) -> bool {
        self.data.settings.show_remaining_time
    }

    pub fn max_left_width(&self, window_width: f32) -> f32 {
        if window_width.is_finite() && window_width >= 1024.0 && window_width < 10_000.0 {
            let right_w = if self.data.settings.show_right { self.data.settings.right_width } else { 0.0 };
            (window_width - 416.0 - theme::GAP * 3.0 - right_w).max(LEFT_EXPANDED)
        } else {
            theme::LEFT_MAX
        }
    }

    pub fn on_search_input_changed(&mut self) {
        self.search_dropdown_open = true;
        self.search_debounce_at = Some(Instant::now());
    }

    pub fn search_debounce_pending(&self) -> bool {
        self.search_debounce_at.is_some()
    }

    pub fn pump_search_debounce(&mut self) {
        if let Some(at) = self.search_debounce_at {
            if at.elapsed() >= Duration::from_millis(SEARCH_DEBOUNCE_MS) {
                self.flush_search_debounce();
            }
        }
    }

    pub fn flush_search_debounce(&mut self) {
        self.instant_query = self.search.clone();
        self.search_debounce_at = None;
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        self.instant_query.clear();
        self.search_debounce_at = None;
        self.ytm.search_changed();
    }

    fn sync_next(&mut self) {
        let s = &self.data.settings;
        let next = self.player.peek_next().and_then(|id| self.lib.track(id)).filter(|track| Self::can_play_track(track));
        let spec = next.as_ref().map(|next| {
            let how = self.current_track().map_or(Transition::Gapless, |now| s.transition(&now, next));
            self.track_load_spec(next, how)
        });
        self.engine.set_next(spec);

        // Proactively prefetch next track's artwork, YouTube audio stream URL, and synced lyrics once per next-track change.
        let next_id = next.as_ref().map(|t| t.id);
        if self.screenshot.is_none() && !self.lib.demo && next_id != self.last_prefetched_next {
            self.last_prefetched_next = next_id;
            if let Some(next_track) = next {
                self.art.prefetch(&next_track.cover_key(), next_track.art_source());
                if let Some(vid) = next_track.youtube_id() {
                    crate::ytm::prefetch_stream(vid);
                }
                let track_clone = (*next_track).clone();
                if track_clone.youtube_id().is_some() {
                    let _ = std::thread::Builder::new().name("lyrics-prefetch".into()).spawn(move || {
                        let _ = crate::ytm::online_lyrics(&track_clone);
                    });
                }
            }
        }
    }

    pub fn tracks_of(&self, item: &RecentItem) -> (ContextRef, String, Vec<TrackId>) {
        match item {
            RecentItem::Liked => (ContextRef::Liked, self.t("Bài hát đã thích", "Liked Songs").to_string(), self.data.liked_ids()),
            RecentItem::Playlist(id) => {
                let (name, tracks) = self
                    .data
                    .playlist(*id)
                    .map(|p| (p.name.clone(), p.track_ids()))
                    .unwrap_or_default();
                (ContextRef::Playlist(*id), name, tracks)
            }
            RecentItem::Album(key) => {
                let a = self.lib.album(key);
                (
                    ContextRef::Album(key.clone()),
                    a.map(|a| a.title.clone()).unwrap_or_default(),
                    a.map(|a| a.tracks.clone()).unwrap_or_default(),
                )
            }
            RecentItem::Artist(name) => {
                let tracks = self.lib.artist(name).map(|a| a.tracks.clone()).unwrap_or_default();
                (ContextRef::Artist(name.clone()), name.clone(), tracks)
            }
            RecentItem::FollowedYouTube(_) => (ContextRef::None, String::new(), Vec::new()),
        }
    }

    pub fn context_is_playing(&self, context: &ContextRef) -> bool {
        self.is_playing() && self.player.context_ref == *context
    }

    fn recent_for_context(context: &ContextRef) -> Option<RecentItem> {
        match context {
            ContextRef::Liked => Some(RecentItem::Liked),
            ContextRef::Playlist(id) => Some(RecentItem::Playlist(*id)),
            ContextRef::Album(k) => Some(RecentItem::Album(k.clone())),
            ContextRef::Artist(n) => Some(RecentItem::Artist(n.clone())),
            _ => None,
        }
    }

    fn play_context(&mut self, context: ContextRef, name: String, tracks: Vec<TrackId>, start: Option<usize>) {
        let tracks: Vec<TrackId> = tracks.into_iter().filter(|t| self.lib.tracks.contains_key(t)).collect();
        if let Some(item) = Self::recent_for_context(&context) {
            self.data.touch_recent(item);
        }
        if let Some(id) = self.player.play_context(context, name, tracks, start) {
            self.error_streak = 0;
            self.start_track(id, 0, true);
        }
    }

    fn navigate(&mut self, route: Route) {
        self.top_popover = None;
        self.search_dropdown_open = false;
        if route != self.route {
            if let Route::YouTubeMusicItem(item) = &route {
                if let crate::ytm::ItemId::Playlist(id) = &item.id {
                    if self.data.followed_ytm.iter().any(|list| &list.id == id) {
                        self.data.touch_recent(RecentItem::FollowedYouTube(id.clone()));
                        self.dirty = true;
                    }
                }
            }
            self.back.push(std::mem::replace(&mut self.route, route));
            self.forward.clear();
            self.entity_filter.clear();
            self.entity_filter_open = false;
            self.table_sort = TableSort::default();
        } else if route == Route::Home {
            self.home_scroll_to_top = true;
            self.scroll_to_top_seq = self.scroll_to_top_seq.wrapping_add(1);
        }
    }

    pub fn update_followed_ytm(&mut self, id: &str, snapshot: Option<crate::ytm::Page>) {
        if let Some(snapshot) = snapshot {
            if let Some(existing) = self.data.followed_ytm.iter_mut().find(|playlist| playlist.id == id) {
                existing.refresh(snapshot);
            } else {
                self.data.followed_ytm.push(crate::ytm::FollowedPlaylist {
                    id: id.into(), snapshot, added: chrono::Utc::now().timestamp() as u64,
                });
            }
        } else {
            self.data.followed_ytm.retain(|playlist| playlist.id != id);
            self.data.recent.retain(|item| !matches!(item, RecentItem::FollowedYouTube(saved) if saved == id));
        }
        self.dirty = true;
    }

    pub fn submit_search(&mut self) {
        if let Some(id) = crate::ytm::from_url(&self.search) {
            self.actions.push(Action::Navigate(Route::YouTubeMusicItem(self.ytm_item(id))));
        } else {
            let query = self.search.trim();
            if !query.is_empty() {
                self.data.recent_searches.retain(|item| item != query);
                self.data.recent_searches.insert(0, query.to_owned());
                self.data.recent_searches.truncate(12);
                self.dirty = true;
            }
        }
    }

    pub fn ytm_item(&self, id: crate::ytm::ItemId) -> crate::ytm::Item {
        use crate::ytm::ItemId;
        if let ItemId::Song(video_id) = &id {
            if let Some(item) = self.track(library::fnv(&format!("yt:{video_id}")))
                .filter(|track| !track.artist.is_empty())
                .and_then(|track| crate::ytm::Item::from_track(&track))
            {
                return item;
            }
        }
        let title = match &id {
            ItemId::Playlist(_) => self.t("Danh sách phát", "Playlist"),
            ItemId::Song(_) => self.t("Bài hát", "Song"),
            ItemId::Album(_) => "Album",
            ItemId::Artist(_) => self.t("Nghệ sĩ", "Artist"),
        };
        crate::ytm::Item {
            id, title: title.into(), by: String::new(), image: None, duration_ms: 0, album: String::new(),
        }
    }

    pub fn can_play_in_app(&self) -> bool {
        self.current_track().is_some_and(|track| Self::can_play_track(&track))
    }

    fn remember_youtube_song(&mut self, item: &crate::ytm::Item) -> Option<TrackId> {
        let mut track = item.track()?;
        let id = track.id;
        if let Some(old) = self.track(id) {
            if track.artist.is_empty() {
                track.title.clone_from(&old.title);
                track.artist.clone_from(&old.artist);
            }
            if track.album.is_empty() {
                track.album.clone_from(&old.album);
            }
            if track.duration_ms == 0 {
                track.duration_ms = old.duration_ms;
            }
            track.art_url = track.art_url.or_else(|| old.art_url.clone());
            if (old.title.as_str(), old.artist.as_str(), old.album.as_str(), old.duration_ms, &old.art_url)
                == (track.title.as_str(), track.artist.as_str(), track.album.as_str(), track.duration_ms, &track.art_url)
            {
                return Some(id);
            }
            if self.player.current == Some(id) && !self.lyrics_imported {
                self.lyrics_for = None;
            }
        }
        self.lib.upsert(track);
        self.lib.reindex();
        self.library_dirty = true;
        Some(id)
    }

    pub fn toast(&mut self, text: String) {
        self.toast = Some((text, Instant::now()));
    }

    pub fn liked_toast(&self, liked: bool) -> String {
        if liked {
            self.t("Đã thêm vào Bài hát đã thích", "Added to Liked Songs").into()
        } else {
            self.t("Đã xóa khỏi Bài hát đã thích", "Removed from Liked Songs").into()
        }
    }

    fn run_action(&mut self, action: Action) {
        match action {
            Action::SelectYouTubeSong(item) => {
                if let Some(id) = self.remember_youtube_song(&item) {
                    if self.player.current == Some(id) {
                        self.run_action(Action::TogglePlay);
                    } else {
                        self.error_streak = 0;
                        self.player.play_context(ContextRef::None, item.title, vec![id], Some(0));
                        self.start_track(id, 0, true);
                    }
                }
            }
            Action::PlayYouTubeList { name, items, start } => {
                let mut ids = Vec::new();
                let mut target_idx = None;
                for (i, it) in items.iter().enumerate() {
                    if let Some(id) = self.remember_youtube_song(it) {
                        if i == start {
                            target_idx = Some(ids.len());
                        }
                        ids.push(id);
                    }
                }
                if !ids.is_empty() {
                    let chosen_pos = target_idx.unwrap_or(0);
                    let chosen_id = ids[chosen_pos];
                    if self.player.current == Some(chosen_id) && self.player.context == ids {
                        self.run_action(Action::TogglePlay);
                    } else {
                        self.error_streak = 0;
                        self.play_context(ContextRef::None, name, ids, Some(chosen_pos));
                    }
                }
            }
            Action::LikeYouTubeSong(item) => {
                if let Some(id) = self.remember_youtube_song(&item) {
                    self.run_action(Action::ToggleLike(id));
                }
            }
            Action::EnqueueYouTubeSong(item) => {
                if let Some(id) = self.remember_youtube_song(&item) {
                    self.run_action(Action::Enqueue(id));
                }
            }
            Action::AddYouTubeSong(playlist, item) => {
                if let Some(id) = self.remember_youtube_song(&item) {
                    self.run_action(Action::AddToPlaylist(playlist, id));
                }
            }
            Action::CreatePlaylistWithSong(item) => {
                if let Some(id) = self.remember_youtube_song(&item) {
                    self.run_action(Action::CreatePlaylist(Some(id)));
                }
            }
            Action::Navigate(r) => self.navigate(r),
            Action::ToggleLyrics => {
                if self.route == Route::Lyrics {
                    if self.back.is_empty() {
                        self.navigate(Route::Home);
                    } else {
                        self.run_action(Action::Back);
                    }
                } else {
                    self.navigate(Route::Lyrics);
                }
            }
            Action::ToggleLyricsFullscreen => {
                if self.lyrics_fullscreen.is_some() {
                    self.exit_lyrics_fullscreen();
                } else if self.route == Route::Lyrics {
                    self.lyrics_fullscreen = Some(self.context.input(|i| i.viewport().fullscreen.unwrap_or(false)));
                    self.context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                }
            }
            Action::ImportLyrics(id) => {
                if let Some(track) = self.track(id) {
                    self.import_lyrics(&track);
                }
            }
            Action::RemoveImportedLyrics(id) => {
                if let Some(track) = self.track(id) {
                    self.remove_imported_lyrics(&track);
                }
            }
            Action::Back => {
                if let Some(r) = self.back.pop() {
                    self.forward.push(std::mem::replace(&mut self.route, r));
                }
            }
            Action::Forward => {
                if let Some(r) = self.forward.pop() {
                    self.back.push(std::mem::replace(&mut self.route, r));
                }
            }
            Action::PlayContext { context, name, tracks, start } => self.play_context(context, name, tracks, start),
            Action::PlayItem(item) => {
                if let RecentItem::FollowedYouTube(ref id) = item {
                    if let Some(followed) = self.data.followed_ytm.iter().find(|f| &f.id == id).cloned() {
                        let songs: Vec<crate::ytm::Item> = followed
                            .snapshot
                            .items
                            .into_iter()
                            .filter(|it| matches!(it.id, crate::ytm::ItemId::Song(_)))
                            .collect();
                        if !songs.is_empty() {
                            self.run_action(Action::PlayYouTubeList {
                                name: followed.snapshot.title,
                                items: songs,
                                start: 0,
                            });
                        }
                    }
                } else {
                    let (context, name, tracks) = self.tracks_of(&item);
                    if self.player.context_ref == context && self.player.current.is_some() {
                        self.run_action(Action::TogglePlay);
                    } else {
                        self.play_context(context, name, tracks, None);
                    }
                }
            }
            Action::TogglePlay => {
                self.halt_after_errors = false;
                if self.player.current.is_none() {
                    let tracks = self.lib.order.clone();
                    let name = self.t("Tất cả bài hát", "All songs").to_string();
                    self.play_context(ContextRef::All, name, tracks, None);
                } else if !self.engine.has_song() {
                    if let Some(id) = self.player.current {
                        self.start_track(id, 0, true);
                    }
                } else if self.engine.is_paused() {
                    self.engine.play();
                } else {
                    self.engine.pause();
                }
            }
            Action::Next => {
                if let Some(cur) = self.player.current {
                    let pos = self.position_ms();
                    let dur = self.duration_ms();
                    if dur >= 30_000 && pos < 10_000 {
                        self.data.record_fast_skip(cur);
                        self.dirty = true;
                    } else if pos >= 30_000 {
                        self.data.record_play_complete(cur);
                        self.dirty = true;
                    }
                }
                self.error_streak = 0;
                self.skip(false);
            }
            Action::HideTrack(id) => {
                self.data.hidden_tracks.insert(id);
                self.dirty = true;
                if self.player.current == Some(id) {
                    self.run_action(Action::Next);
                }
                self.toast(self.t("Bài hát đã ẩn. Có thể khôi phục trong Cài đặt.", "Song hidden. Restore hidden songs in Settings.").into());
            }
            Action::RestoreHiddenTracks => {
                self.data.hidden_tracks.clear();
                self.dirty = true;
                self.toast(self.t("Đã khôi phục các bài hát bị ẩn", "Hidden songs restored").into());
            }
            Action::Previous => {
                self.error_streak = 0;
                match self.player.previous(self.position_ms()) {
                    Previous::Restart if !self.engine.has_song() => {
                        if let Some(id) = self.player.current {
                            self.start_track(id, 0, true);
                        }
                    }
                    Previous::Restart => self.seek(0),
                    Previous::Track(id) => self.start_track(id, 0, true),
                }
            }
            Action::SeekTo(ms) => self.seek(ms),
            Action::SeekBy(delta) => {
                let target = (self.position_ms() as i64 + delta).clamp(0, self.duration_ms() as i64);
                self.seek(target as u64);
            }
            Action::ToggleShuffle => {
                self.player.toggle_shuffle();
                self.data.settings.shuffle = self.player.shuffle;
                self.sync_next();
                self.dirty = true;
            }
            Action::CycleRepeat => {
                self.player.cycle_repeat();
                self.data.settings.repeat = self.player.repeat;
                self.sync_next();
                self.dirty = true;
            }
            Action::ToggleRemainingTime => {
                self.data.settings.show_remaining_time = !self.data.settings.show_remaining_time;
                self.dirty = true;
            }
            Action::ToggleArtistFollow(artist) => {
                if !self.data.followed_artists.remove(&artist) { self.data.followed_artists.insert(artist); }
                self.dirty = true;
            }
            Action::ToggleLike(id) => {
                let liked = self.data.toggle_like(id);
                self.toast(self.liked_toast(liked));
                self.refresh_live_context(&ContextRef::Liked);
                self.dirty = true;
            }
            Action::Enqueue(id) => {
                self.player.enqueue(id);
                self.sync_next();
                self.dirty = true;
                self.toast(self.t("Đã thêm vào danh sách chờ", "Added to queue").to_string());
            }
            Action::Dequeue(i) => {
                self.player.dequeue(i);
                self.sync_next();
                self.dirty = true;
            }
            Action::MoveQueueItem(from, to) => {
                if self.player.move_queue_item(from, to) {
                    self.sync_next();
                    self.dirty = true;
                }
            }
            Action::ClearQueue => {
                self.player.clear_queue();
                self.sync_next();
                self.dirty = true;
            }
            Action::RequestClearQueue => {
                if !self.player.queue.is_empty() {
                    self.confirm = Some(ConfirmAction::ClearQueue);
                }
            }
            Action::RequestConfirm(action) => {
                self.confirm = Some(action);
            }
            Action::AcceptConfirm => {
                if let Some(pending) = self.confirm.take() {
                    match pending {
                        ConfirmAction::ClearQueue => self.run_action(Action::ClearQueue),
                        ConfirmAction::DeletePlaylist(id) => self.run_action(Action::DeletePlaylist(id)),
                        ConfirmAction::Unpin(item) => {
                            self.data.set_pinned(item, false);
                            self.dirty = true;
                        }
                        ConfirmAction::DeleteTtml(id) => self.run_action(Action::RemoveImportedLyrics(id)),
                        ConfirmAction::DeleteFolder(id) => self.run_action(Action::DeleteFolder(id)),
                    }
                }
            }
            Action::CancelConfirm => {
                self.confirm = None;
            }
            Action::JumpUpcoming(id) => {
                if self.player.jump_to_upcoming(id).is_some() {
                    self.start_track(id, 0, true);
                }
            }
            Action::PlayQueued(index) => {
                for _ in 0..index {
                    self.player.queue.pop_front();
                }
                self.skip(false);
            }
            Action::AddToPlaylist(pid, tid) => {
                let added = chrono::Utc::now().timestamp() as u64;
                if let Some(p) = self.data.playlist_mut(pid) {
                    if !p.tracks.iter().any(|e| e.id == tid) {
                        p.tracks.push(PlaylistEntry { id: tid, added });
                    }
                }
                self.refresh_live_context(&ContextRef::Playlist(pid));
                self.toast(self.t("Đã thêm vào danh sách phát", "Added to playlist").to_string());
                self.dirty = true;
            }
            Action::RemoveFromPlaylist(pid, tid) => {
                if let Some(p) = self.data.playlist_mut(pid) {
                    p.tracks.retain(|e| e.id != tid);
                }
                self.refresh_live_context(&ContextRef::Playlist(pid));
                self.dirty = true;
            }
            Action::MoveInPlaylist(pid, track, neighbor) => {
                if let Some(p) = self.data.playlist_mut(pid) {
                    let index = p.tracks.iter().position(|entry| entry.id == track);
                    let other = p.tracks.iter().position(|entry| entry.id == neighbor);
                    if let (Some(index), Some(other)) = (index, other) {
                        p.tracks.swap(index, other);
                    }
                }
                self.refresh_live_context(&ContextRef::Playlist(pid));
                self.dirty = true;
            }
            Action::CreatePlaylist(first) => {
                let n = self.data.playlists.len() + 1;
                let name = match self.lang() {
                    Lang::Vi => format!("Danh sách phát #{n}"),
                    Lang::En => format!("My Playlist #{n}"),
                };
                let mut p = Playlist::new(name.clone());
                p.folder_id = self.opened_folder;
                let added = p.created;
                p.tracks.extend(first.map(|id| PlaylistEntry { id, added }));
                let id = p.id;
                self.data.playlists.insert(0, p);
                self.data.touch_recent(RecentItem::Playlist(id));
                self.navigate(Route::Playlist(id));
                self.rename = Some(RenameState { id, name, focus: true });
                self.dirty = true;
            }
            Action::DeletePlaylist(id) => {
                self.data.playlists.retain(|p| p.id != id);
                self.data.recent.retain(|r| *r != RecentItem::Playlist(id));
                self.data.pinned_items.retain(|r| *r != RecentItem::Playlist(id));
                if self.route == Route::Playlist(id) {
                    self.navigate(Route::Home);
                }
                self.dirty = true;
            }
            Action::BeginRename(id) => {
                let name = self.data.playlist(id).map(|p| p.name.clone()).unwrap_or_default();
                self.rename = Some(RenameState { id, name, focus: true });
            }
            Action::SaveRename(id, name) => {
                let trimmed = name.trim();
                if !trimmed.is_empty() {
                    if let Some(p) = self.data.playlist_mut(id) {
                        p.name = trimmed.to_string();
                        self.dirty = true;
                    }
                }
                self.rename = None;
            }
            Action::BeginEditPlaylist(id) => {
                if let Some(p) = self.data.playlist(id) {
                    self.edit_playlist = Some(EditPlaylistState {
                        id,
                        name: p.name.clone(),
                        description: p.description.clone(),
                        image: p.image.clone(),
                        focus: true,
                        error: None,
                    });
                }
            }
            Action::PickPlaylistCover(id) => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Image", &["png", "jpg", "jpeg", "webp"])
                    .pick_file()
                else {
                    return;
                };
                let path_str = path.to_string_lossy().into_owned();
                if let Some(edit) = self.edit_playlist.as_mut().filter(|e| e.id == id) {
                    edit.image = Some(path_str);
                } else if let Some(p) = self.data.playlist_mut(id) {
                    p.image = Some(path_str);
                    self.dirty = true;
                }
            }
            Action::ClearPlaylistCover => {
                if let Some(edit) = self.edit_playlist.as_mut() {
                    edit.image = None;
                }
            }
            Action::SaveEditPlaylist => {
                if let Some(mut state) = self.edit_playlist.take() {
                    let trimmed = state.name.trim().to_string();
                    if trimmed.is_empty() {
                        state.error = Some(self.t("Tên danh sách phát là bắt buộc", "Playlist name is required").to_string());
                        self.edit_playlist = Some(state);
                    } else {
                        let desc: String = state.description.chars().take(PLAYLIST_DESCRIPTION_MAX_CHARS).collect();
                        if let Some(p) = self.data.playlist_mut(state.id) {
                            p.name = trimmed;
                            p.description = desc;
                            p.image = state.image;
                        }
                        self.dirty = true;
                    }
                }
            }
            Action::CloseEditPlaylist => {
                self.edit_playlist = None;
            }
            Action::TogglePin(item) => {
                let pinned = self.data.toggle_pin(item);
                self.dirty = true;
                let msg = if pinned {
                    self.t("Đã ghim vào Thư viện", "Pinned to Your Library")
                } else {
                    self.t("Đã bỏ ghim khỏi Thư viện", "Unpinned from Your Library")
                };
                self.toast(msg.to_string());
            }
            Action::ToggleFolderPin(id) => {
                self.data.toggle_folder_pin(id);
                self.dirty = true;
            }
            Action::CreateFolder(name) => {
                let n = self.data.folders.len() + 1;
                let default_name = match self.lang() {
                    Lang::Vi => format!("Thư mục mới #{n}"),
                    Lang::En => format!("New Folder #{n}"),
                };
                let prompt_rename = name.is_none();
                let folder_name = name.unwrap_or(default_name);
                let id = self.data.create_folder(folder_name.clone());
                if let Some(folder) = self.data.folder_mut(id) {
                    folder.parent_id = self.opened_folder;
                }
                if prompt_rename {
                    self.rename_folder = Some(RenameState { id, name: folder_name, focus: true });
                }
                self.dirty = true;
            }
            Action::RenameFolder(id, name) => {
                if self.data.rename_folder(id, name) {
                    self.dirty = true;
                }
                self.rename_folder = None;
            }
            Action::DeleteFolder(id) => {
                if self.data.delete_folder(id) {
                    if self.opened_folder == Some(id) {
                        self.opened_folder = None;
                    }
                    self.dirty = true;
                }
            }
            Action::MovePlaylistToFolder(pid, fid) => {
                if self.data.move_playlist_to_folder(pid, fid) {
                    self.dirty = true;
                }
            }
            Action::OpenFolder(fid) => {
                self.opened_folder = fid;
            }
            Action::SetVolume(v) => {
                self.data.settings.volume = v.clamp(0.0, 1.0);
                self.data.settings.muted = false;
                self.apply_volume();
                self.dirty = true;
            }
            Action::ToggleMute => {
                self.data.settings.muted = !self.data.settings.muted;
                self.apply_volume();
                self.dirty = true;
            }
            Action::ToggleRight(tab) => {
                let s = &mut self.data.settings;
                if self.friend_activity_open {
                    self.friend_activity_open = false;
                    s.show_right = true;
                    self.right_tab = tab;
                } else if s.show_right && (self.right_tab == tab || (self.right_tab.is_queue() && tab.is_queue())) {
                    s.show_right = false;
                } else {
                    s.show_right = true;
                    self.right_tab = tab;
                }
                self.dirty = true;
            }
            Action::CloseRight => {
                self.data.settings.show_right = false;
                self.friend_activity_open = false;
                self.dirty = true;
            }
            Action::ToggleLeft => {
                self.data.settings.left_collapsed = !self.data.settings.left_collapsed;
                self.dirty = true;
            }
            Action::ExpandLeft => {
                let window_w = self.context.input(|i| i.content_rect().width());
                let expanded_w = if window_w.is_finite() && window_w >= 1024.0 && window_w < 10_000.0 {
                    LEFT_EXPANDED.min(self.max_left_width(window_w))
                } else {
                    theme::LEFT_MAX
                };
                let settings = &mut self.data.settings;
                settings.left_collapsed = false;
                settings.left_width = if settings.left_width >= theme::LEFT_MAX {
                    theme::LEFT_DEFAULT
                } else {
                    expanded_w
                };
                self.dirty = true;
            }
            Action::SetLeftWidth(w) => {
                let window_w = self.context.input(|i| i.content_rect().width());
                let max_w = self.max_left_width(window_w);
                self.data.settings.left_width = w.clamp(theme::LEFT_MIN, max_w);
                self.dirty = true;
            }
            Action::SortLibrary(sort) => {
                self.data.settings.set_library_sort(sort.into());
                self.dirty = true;
            }
            Action::SetLibrarySort(sort) => {
                self.data.settings.set_library_sort(sort);
                self.dirty = true;
            }
            Action::SetLibraryView(view) => {
                self.data.settings.library_view = view;
                self.dirty = true;
            }
            Action::SetLibraryFilter(filter) => {
                self.lib_filter = filter;
                if filter == LibraryFilter::All {
                    self.lib_creator_filter = LibraryCreatorFilter::All;
                }
            }
            Action::SetLibraryCreatorFilter(filter) => {
                self.lib_creator_filter = filter;
            }
            Action::SetHomeFilter(filter) => {
                self.home_filter = filter;
            }
            Action::ScrollHomeToTop => {
                self.home_scroll_to_top = true;
                self.scroll_to_top_seq = self.scroll_to_top_seq.wrapping_add(1);
            }
            Action::SetSearchDropdown(open) => {
                self.search_dropdown_open = open;
            }
            Action::SearchInputChanged => {
                self.on_search_input_changed();
            }
            Action::SubmitSearch => {
                self.flush_search_debounce();
                self.search_dropdown_open = false;
                self.submit_search();
                if crate::ytm::from_url(&self.search).is_none() {
                    self.navigate(Route::Search);
                }
            }
            Action::SearchCategory(category) => {
                self.search = category;
                self.flush_search_debounce();
                self.search_dropdown_open = false;
                self.ytm.search_changed();
                self.submit_search();
                self.navigate(Route::Search);
            }
            Action::SelectRecentSearch(query) => {
                self.search = query;
                self.flush_search_debounce();
                self.search_dropdown_open = false;
                self.ytm.search_changed();
                self.submit_search();
                self.navigate(Route::Search);
            }
            Action::RemoveRecentSearch(query) => {
                self.data.recent_searches.retain(|item| item != &query);
                self.dirty = true;
            }
            Action::ClearRecentSearches => {
                self.data.recent_searches.clear();
                self.dirty = true;
            }
            Action::ToggleCompactTracklist => {
                self.data.settings.compact_tracklist = !self.data.settings.compact_tracklist;
                self.dirty = true;
            }
            Action::SetCompactTracklist(compact) => {
                self.data.settings.compact_tracklist = compact;
                self.dirty = true;
            }
            Action::SetTableSort(sort) => {
                self.table_sort = sort;
            }
            Action::ToggleTableSort(column) => {
                if self.table_sort.column == column {
                    if self.table_sort.ascending {
                        self.table_sort.ascending = false;
                    } else {
                        self.table_sort = TableSort::default();
                    }
                } else {
                    self.table_sort = TableSort { column, ascending: true };
                }
            }
            Action::SetEntityFilter(query) => {
                self.entity_filter = query;
            }
            Action::ClearEntityFilter => {
                self.entity_filter.clear();
                self.entity_filter_open = false;
            }
            Action::ToggleLyricsCardExpand => {
                self.lyrics_card_expanded = !self.lyrics_card_expanded;
                if self.lyrics_card_expanded {
                    self.lyrics_card_collapsed = false;
                }
            }
            Action::ToggleLyricsCardCollapse => {
                self.lyrics_card_collapsed = !self.lyrics_card_collapsed;
                if self.lyrics_card_collapsed {
                    self.lyrics_card_expanded = false;
                }
            }
            Action::ToggleLyricsCinema => {
                self.lyrics_cinema = !self.lyrics_cinema;
                if self.lyrics_cinema && self.route != Route::Lyrics {
                    self.navigate(Route::Lyrics);
                }
            }
            Action::SetTtmlImportMode(mode) => {
                self.lyrics_ttml_mode = mode;
            }
            Action::ImportLyricsWithMode(id, mode) => {
                if let Some(track) = self.track(id) {
                    self.import_lyrics_with_mode(&track, mode);
                }
            }
            Action::ExportLyrics(id) => {
                if let Some(track) = self.track(id) {
                    self.export_lyrics(&track);
                }
            }
            Action::ResetLyrics(id) => {
                self.reset_lyrics(id);
            }
            Action::RequestDeleteTtml(id) => {
                self.confirm = Some(ConfirmAction::DeleteTtml(id));
            }
            Action::ConfirmDeleteTtmlStep(id) => {
                if self.lyrics_delete_confirm_for == Some(id) {
                    self.lyrics_delete_confirm_for = None;
                    self.run_action(Action::RemoveImportedLyrics(id));
                } else {
                    self.lyrics_delete_confirm_for = Some(id);
                }
            }
            Action::ToggleNotifications => {
                self.top_popover = if self.top_popover == Some(TopPopover::Notifications) {
                    None
                } else {
                    self.unread_notifications = false;
                    Some(TopPopover::Notifications)
                };
            }
            Action::ToggleFriendActivity => {
                if self.data.settings.show_right && self.friend_activity_open {
                    self.friend_activity_open = false;
                    self.data.settings.show_right = false;
                } else {
                    self.friend_activity_open = true;
                    self.data.settings.show_right = true;
                }
                self.dirty = true;
            }
            Action::ToggleAccountMenu => {
                self.top_popover = if self.top_popover == Some(TopPopover::Account) {
                    None
                } else {
                    Some(TopPopover::Account)
                };
            }
            Action::TogglePrivateSession => {
                self.data.settings.private_session = !self.data.settings.private_session;
                self.dirty = true;
            }
            Action::ToggleMiniplayer => {
                self.miniplayer_open = !self.miniplayer_open;
            }
            Action::ToggleSaveAlbum(key) => {
                if !self.data.saved_albums.remove(&key) {
                    self.data.saved_albums.insert(key.clone());
                    self.data.touch_recent(RecentItem::Album(key));
                }
                self.dirty = true;
            }
            Action::OpenPreviewFeed(mut feed) => {
                if feed.items.is_empty() {
                    return;
                }
                feed.index = feed.index.min(feed.items.len() - 1);
                feed.scroll_anim = feed.index as f32;
                let ytm_items: Vec<crate::ytm::Item> = feed
                    .items
                    .iter()
                    .filter_map(|t| t.ytm_item.clone())
                    .collect();
                if !ytm_items.is_empty() {
                    self.run_action(Action::PlayYouTubeList {
                        name: feed.entity_title.clone(),
                        items: ytm_items,
                        start: feed.index,
                    });
                } else {
                    let local_ids: Vec<TrackId> = feed.items.iter().filter_map(|t| t.local_id).collect();
                    if !local_ids.is_empty() {
                        let start = feed.index.min(local_ids.len() - 1);
                        self.run_action(Action::PlayContext {
                            context: ContextRef::Artist(feed.artist_name.clone()),
                            name: feed.entity_title.clone(),
                            tracks: local_ids,
                            start: Some(start),
                        });
                    }
                }
                self.top_popover = None;
                self.preview_feed = Some(feed);
            }
            Action::PreviewFeedStep(delta) => {
                let Some(feed) = self.preview_feed.as_mut() else {
                    return;
                };
                if feed.items.is_empty() {
                    return;
                }
                let max_idx = feed.items.len().saturating_sub(1) as isize;
                let next = (feed.index as isize + delta).clamp(0, max_idx) as usize;
                if next == feed.index {
                    return;
                }
                feed.index = next;
                let entity_title = feed.entity_title.clone();
                let artist_name = feed.artist_name.clone();
                let ytm_items: Vec<crate::ytm::Item> = feed
                    .items
                    .iter()
                    .filter_map(|t| t.ytm_item.clone())
                    .collect();
                let local_ids: Vec<TrackId> = feed.items.iter().filter_map(|t| t.local_id).collect();
                let target_id = feed.items.get(next).and_then(|t| {
                    t.local_id.or_else(|| match t.ytm_item.as_ref().map(|it| &it.id) {
                        Some(crate::ytm::ItemId::Song(vid)) => Some(library::fnv(&format!("yt:{vid}"))),
                        _ => None,
                    })
                });
                let next_matches_preloaded =
                    delta == 1 && target_id.is_some() && self.player.upcoming(1).first().copied() == target_id;
                if next_matches_preloaded {
                    self.skip(false);
                } else if !ytm_items.is_empty() {
                    let start = next.min(ytm_items.len().saturating_sub(1));
                    self.run_action(Action::PlayYouTubeList {
                        name: entity_title,
                        items: ytm_items,
                        start,
                    });
                } else if !local_ids.is_empty() {
                    let start = next.min(local_ids.len().saturating_sub(1));
                    self.run_action(Action::PlayContext {
                        context: ContextRef::Artist(artist_name),
                        name: entity_title,
                        tracks: local_ids,
                        start: Some(start),
                    });
                }
            }
            Action::ClosePreviewFeed => {
                self.preview_feed = None;
            }
            Action::ClosePopovers => {
                self.top_popover = None;
                self.search_dropdown_open = false;
            }
            Action::SettingsChanged => {
                self.engine.apply_settings(audio_settings(&self.data));
                self.refresh_gains();
                self.sync_next();
                self.dirty = true;
            }
        }
    }

    /// Normalization changed, or a track's loudness became known.
    fn refresh_gains(&self) {
        let s = &self.data.settings;
        if let Some(t) = self.current_track() {
            self.engine.set_gain(t.id, t.gain_db(s.level, s.normalize));
        }
    }

    /// A list that is currently playing was edited: the queue follows it without interrupting.
    fn refresh_live_context(&mut self, context: &ContextRef) {
        if self.player.context_ref == *context {
            let item = Self::recent_for_context(context);
            if let Some(item) = item {
                let (_, _, tracks) = self.tracks_of(&item);
                self.player.update_context(tracks);
                self.sync_next();
            }
        }
    }

    fn skip(&mut self, natural: bool) {
        self.halt_after_errors = false;
        match self.player.advance(natural) {
            Some(id) => self.start_track(id, 0, true),
            None => {
                self.engine.stop();
                if natural && self.data.settings.autoplay {
                    self.request_autoplay();
                }
            }
        }
    }

    fn cancel_autoplay_request(&mut self) {
        self.autoplay_req_id = self.autoplay_req_id.wrapping_add(1);
        self.autoplay_response = None;
    }

    fn request_autoplay(&mut self) {
        self.request_autoplay_mode(false);
    }

    /// Proactively seeds upcoming radio/autoplay recommendations while playing when the live queue is running low (< 3 tracks left).
    fn maybe_prefetch_autoplay(&mut self) {
        if self.screenshot.is_some()
            || self.lib.demo
            || !self.data.settings.autoplay
            || self.player.repeat != crate::queue::Repeat::Off
            || self.autoplay_response.is_some()
        {
            return;
        }
        let upcoming_count = self.player.queue.len() + self.player.upcoming(3).len();
        if upcoming_count >= 3 {
            return;
        }
        self.request_autoplay_mode(true);
    }

    fn request_autoplay_mode(&mut self, append_only: bool) {
        if self.screenshot.is_some() {
            return;
        }
        let Some(seed) = self
            .current_track()
            .and_then(|t| t.youtube_id().map(str::to_owned))
            .or_else(|| self.last_yt_seed.clone())
        else {
            return;
        };
        self.autoplay_req_id = self.autoplay_req_id.wrapping_add(1);
        let req_id = self.autoplay_req_id;
        let (tx, rx) = mpsc::channel();
        let wake = self.context.clone();
        let _ = std::thread::Builder::new().name("autoplay".into()).spawn(move || {
            let res = crate::ytm::related_songs(&seed);
            let _ = tx.send(res);
            wake.request_repaint();
        });
        self.autoplay_response = Some((req_id, append_only, rx));
    }

    fn pump_autoplay(&mut self) {
        let Some((req_id, append_only, rx)) = self.autoplay_response.take() else { return };
        if req_id != self.autoplay_req_id || !self.data.settings.autoplay {
            return;
        }
        match rx.try_recv() {
            Ok(Ok(items)) => {
                if append_only && self.engine.has_song() {
                    let _ = self.handle_autoplay_append(items);
                } else {
                    let _ = self.handle_autoplay_items(items);
                }
            }
            Ok(Err(_)) | Err(mpsc::TryRecvError::Disconnected) => {}
            Err(mpsc::TryRecvError::Empty) => {
                self.autoplay_response = Some((req_id, append_only, rx));
            }
        }
    }

    fn rank_and_remember_autoplay(&mut self, items: Vec<crate::ytm::Item>) -> Vec<TrackId> {
        let mut scored: Vec<(i32, usize, TrackId)> = Vec::new();
        let mut seen = HashSet::new();
        for (idx, item) in items.into_iter().enumerate() {
            let crate::ytm::ItemId::Song(ref vid) = item.id else { continue };
            let candidate_id = library::fnv(&format!("yt:{vid}"));
            if self.session_played_yt.contains(vid)
                || self.data.hidden_tracks.contains(&candidate_id)
                || self.player.current == Some(candidate_id)
            {
                continue;
            }
            let score = self.data.score_recommendation(&self.lib, candidate_id, &item.by);
            if let Some(id) = self.remember_youtube_song(&item) {
                if seen.insert(id) {
                    scored.push((score, idx, id));
                }
            }
        }
        // Preserve YouTube Music radio relevance while gently boosting taste-matched artists and demoting skipped tracks.
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        scored.into_iter().map(|(_, _, id)| id).collect()
    }

    /// Appends proactive recommendations into the live upcoming queue without interrupting the current song.
    fn handle_autoplay_append(&mut self, items: Vec<crate::ytm::Item>) -> usize {
        let tracks = self.rank_and_remember_autoplay(items);
        if tracks.is_empty() {
            return 0;
        }
        let added = self.player.append_context(tracks);
        if added > 0 {
            if self.player.context_ref == ContextRef::None {
                self.player.context_ref = ContextRef::Autoplay;
                self.player.context_name = self.t("Tự động phát", "Autoplay").to_string();
            }
            self.sync_next();
            self.dirty = true;
        }
        added
    }

    fn handle_autoplay_items(&mut self, items: Vec<crate::ytm::Item>) -> bool {
        let tracks = self.rank_and_remember_autoplay(items);
        if tracks.is_empty() {
            return false;
        }
        let name = self.t("Tự động phát", "Autoplay").to_string();
        if let Some(first) = self.player.play_context(ContextRef::Autoplay, name, tracks, Some(0)) {
            self.error_streak = 0;
            self.start_track(first, 0, true);
            true
        } else {
            false
        }
    }

    fn pump_engine(&mut self) {
        while let Some(event) = self.engine.poll() {
            self.handle_engine_event(event);
        }
    }

    fn handle_engine_event(&mut self, event: Event) {
        match event {
            Event::Advanced(id) => {
                if let Some(prev) = self.player.current {
                    self.data.record_play_complete(prev);
                }
                self.error_streak = 0;
                // The song the engine already started is the one the player expected.
                if self.player.advance(true) != Some(id) {
                    self.player.current = Some(id);
                }
                self.note_youtube_started(id);
                self.after_track_change();
            }
            Event::Finished => {
                if let Some(prev) = self.player.current {
                    self.data.record_play_complete(prev);
                }
                if self.halt_after_errors {
                    self.halt_after_errors = false;
                    self.engine.stop();
                } else {
                    self.skip(true);
                }
            }
            Event::Error(id, message) => {
                let name = self.track(id).map(|t| t.title.clone()).unwrap_or_default();
                let prefix = self.t("Không phát được", "Can't play");
                self.toast(format!("{prefix} \"{name}\": {message}"));
                self.error_streak += 1;
                if self.error_streak >= MAX_ERROR_STREAK {
                    // Too many consecutive playback failures: stop cleanly without mutating playlists.
                    self.error_streak = 0;
                    self.halt_after_errors = true;
                    self.cancel_autoplay_request();
                    self.engine.stop();
                } else if self.player.current == Some(id) {
                    self.skip(false);
                } else if self.player.peek_next() == Some(id) {
                    let _ = self.player.advance(true);
                }
            }
            Event::Duration(id, ms) => {
                self.error_streak = 0;
                if let Some(t) = self.lib.tracks.get(&id) {
                    if t.duration_ms.abs_diff(ms) > 1000 && t.demo_seed().is_none() {
                        let mut updated = (**t).clone();
                        updated.duration_ms = ms;
                        self.lib.upsert(updated);
                        self.library_dirty = true;
                    }
                }
            }
        }
    }

    #[cfg(windows)]
    fn pump_media(&mut self) {
        use crate::media::MediaCommand;
        let mut commands = Vec::new();
        if let Some(m) = &self.media {
            while let Some(c) = m.poll() {
                commands.push(c);
            }
        }
        for c in commands {
            let action = match c {
                MediaCommand::Toggle => Some(Action::TogglePlay),
                MediaCommand::Play if self.engine.is_paused() => Some(Action::TogglePlay),
                MediaCommand::Pause if !self.engine.is_paused() => Some(Action::TogglePlay),
                MediaCommand::Next => Some(Action::Next),
                MediaCommand::Previous => Some(Action::Previous),
                MediaCommand::SeekBy(d) => Some(Action::SeekBy(d)),
                _ => None,
            };
            if let Some(a) = action {
                self.actions.push(a);
            }
        }
        let playing = self.is_playing();
        let current = self.player.current;
        let track = self.current_track();
        let duration = self.duration_ms();
        if let Some(m) = &mut self.media {
            if current != self.media_track {
                self.media_track = current;
                if let Some(t) = &track {
                    m.set_track(&t.title, &t.artist, &t.album, duration);
                }
            }
            if playing != self.media_playing {
                self.media_playing = playing;
                m.set_playing(playing);
            }
        }
    }

    #[cfg(not(windows))]
    fn pump_media(&mut self) {}

    fn save_if_due(&mut self) {
        if self.screenshot.is_some() || self.last_save.elapsed() < Duration::from_secs(5) {
            return;
        }
        self.last_save = Instant::now();
        if self.library_dirty {
            self.library_dirty = library::save_library_cache(&self.lib).is_err();
            if self.library_dirty {
                self.toast(self.t("Không thể lưu thư viện", "Could not save library").into());
                return;
            }
        }
        if self.can_save_user && (self.dirty || (!self.lib.demo && self.is_playing())) {
            self.dirty = self.store_user().is_err();
            if self.dirty {
                self.toast(self.t("Không thể ghi dữ liệu Nhịp", "Could not save Nhịp data").into());
            }
        }
    }

    fn store_user(&self) -> std::io::Result<()> {
        let mut data = self.data.clone();
        data.playlists.retain(|p| !self.demo_playlist_ids.contains(&p.id));
        if self.lib.demo {
            data.liked.clear();
            data.session = Default::default();
        } else {
            data.session = self.player.snapshot(self.position_ms());
        }
        library::save_user(&data)
    }

    fn import_lyrics(&mut self, track: &Track) {
        self.import_lyrics_with_mode(track, self.lyrics_ttml_mode);
    }

    fn import_lyrics_with_mode(&mut self, track: &Track, mode: TtmlImportMode) {
        let Some(path) = rfd::FileDialog::new().add_filter("TTML", &["ttml"]).pick_file() else { return };
        match crate::ttml::read_text(&path) {
            Ok(xml) => match self.apply_ttml_xml(track.id, &xml, mode) {
                Ok(()) => {
                    let msg = match mode {
                        TtmlImportMode::Save => self.t("Đã nhập lời TTML", "TTML lyrics imported"),
                        TtmlImportMode::JustOnce => self.t("Đã áp dụng lời TTML cho lần phát này", "TTML lyrics applied for this session"),
                    };
                    self.toast(msg.into());
                }
                Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                    self.toast(self.t("Tệp TTML không hợp lệ hoặc dùng mốc chưa hỗ trợ", "Invalid TTML or unsupported timing").into());
                }
                Err(_) => self.toast(self.t("Không thể đọc hoặc lưu tệp lời", "Could not read or save lyrics").into()),
            },
            Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
                self.toast(self.t("Tệp TTML không hợp lệ hoặc dùng mốc chưa hỗ trợ", "Invalid TTML or unsupported timing").into());
            }
            Err(_) => self.toast(self.t("Không thể đọc hoặc lưu tệp lời", "Could not read or save lyrics").into()),
        }
    }

    /// Parses `xml` as TTML and applies it either persistently (`Save`) or session-only (`JustOnce`).
    pub fn apply_ttml_xml(&mut self, track_id: TrackId, xml: &str, mode: TtmlImportMode) -> std::io::Result<()> {
        let lyrics = crate::ttml::parse(xml)?;
        let Some(track) = self.track(track_id) else {
            return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "Track not found"));
        };
        match mode {
            TtmlImportMode::Save => {
                let dest = library::imported_lyrics_file(&track);
                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&dest, xml.as_bytes())?;
                self.session_lyrics_overrides.remove(&track_id);
                if self.player.current == Some(track_id) {
                    self.lyrics_for = Some(track_id);
                    self.lyrics_imported = true;
                    self.lyrics_session_override = false;
                    self.lyrics = Some(lyrics);
                    self.lyrics_response = None;
                }
            }
            TtmlImportMode::JustOnce => {
                self.session_lyrics_overrides.insert(track_id, xml.to_string());
                if self.player.current == Some(track_id) {
                    self.lyrics_for = Some(track_id);
                    self.lyrics_imported = false;
                    self.lyrics_session_override = true;
                    self.lyrics = Some(lyrics);
                    self.lyrics_response = None;
                }
            }
        }
        Ok(())
    }

    /// Returns raw TTML XML for `track` if a session override or imported disk file exists.
    pub fn export_lyrics_xml(&self, track: &Track) -> Option<String> {
        if let Some(xml) = self.session_lyrics_overrides.get(&track.id) {
            return Some(xml.clone());
        }
        crate::ttml::read_text(&library::imported_lyrics_file(track)).ok()
    }

    fn export_lyrics(&mut self, track: &Track) {
        let Some(xml) = self.export_lyrics_xml(track) else {
            self.toast(self.t("Không có tệp TTML để xuất", "No TTML lyrics to export").into());
            return;
        };
        let suggested = format!("{}.ttml", track.title.replace(['\\', '/', ':', '*', '?', '"', '<', '>', '|'], "_"));
        let Some(path) = rfd::FileDialog::new()
            .add_filter("TTML", &["ttml"])
            .set_file_name(suggested)
            .save_file()
        else {
            return;
        };
        match std::fs::write(&path, xml.as_bytes()) {
            Ok(()) => self.toast(self.t("Đã xuất tệp TTML", "TTML lyrics exported").into()),
            Err(_) => self.toast(self.t("Không thể xuất tệp TTML", "Could not export TTML lyrics").into()),
        }
    }

    pub fn reset_lyrics(&mut self, track_id: TrackId) {
        self.session_lyrics_overrides.remove(&track_id);
        self.lyrics_delete_confirm_for = None;
        if self.player.current == Some(track_id) {
            self.lyrics_for = None;
            self.lyrics = None;
            self.lyrics_imported = false;
            self.lyrics_session_override = false;
            self.lyrics_response = None;
        }
        self.toast(self.t("Đã đặt lại lời bài hát", "Lyrics reset").into());
    }

    fn remove_imported_lyrics(&mut self, track: &Track) {
        self.session_lyrics_overrides.remove(&track.id);
        self.lyrics_delete_confirm_for = None;
        if let Err(err) = std::fs::remove_file(library::imported_lyrics_file(track)) {
            if err.kind() != std::io::ErrorKind::NotFound {
                self.toast(self.t("Không thể gỡ lời đã nhập", "Could not remove imported lyrics").into());
                return;
            }
        }
        if self.player.current == Some(track.id) {
            self.lyrics_for = None;
            self.lyrics_imported = false;
            self.lyrics_session_override = false;
        }
        self.toast(self.t("Đã gỡ lời đã nhập", "Imported lyrics removed").into());
    }

    pub fn lyrics_imported(&self) -> bool {
        (self.lyrics_imported || self.lyrics_session_override) && self.player.current == self.lyrics_for
    }

    /// Lyrics for the current song, loaded the first time they're asked for.
    pub fn current_lyrics(&mut self) -> Option<&Lyrics> {
        let id = self.player.current?;
        if self.lyrics_for != Some(id) {
            self.lyrics_for = Some(id);
            self.lyrics = None;
            self.lyrics_imported = false;
            self.lyrics_session_override = false;
            self.lyrics_response = None;
            if let Some(xml) = self.session_lyrics_overrides.get(&id) {
                self.lyrics = crate::ttml::parse(xml).ok();
                self.lyrics_session_override = self.lyrics.is_some();
            } else {
                let track = self.lib.track(id)?;
                self.lyrics = crate::ttml::read_text(&library::imported_lyrics_file(&track))
                    .and_then(|xml| crate::ttml::parse(&xml)).ok();
                self.lyrics_imported = self.lyrics.is_some();
                if self.lyrics.is_none() && track.demo_seed().is_none() && self.screenshot.is_none() {
                    let (tx, rx) = mpsc::channel();
                    let wake = self.context.clone();
                    let _ = std::thread::Builder::new().name("lyrics".into()).spawn(move || {
                        let lyrics = crate::ytm::online_lyrics(&track);
                        let _ = tx.send(lyrics);
                        wake.request_repaint();
                    });
                    self.lyrics_response = Some(rx);
                }
            }
        }
        if let Some(rx) = &self.lyrics_response {
            match rx.try_recv() {
                Ok(lyrics) => {
                    self.lyrics = lyrics;
                    self.lyrics_response = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.lyrics_response = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        self.lyrics.as_ref()
    }

    pub fn lyrics_loading(&self) -> bool {
        self.lyrics_response.is_some()
    }

    fn exit_lyrics_fullscreen(&mut self) {
        if let Some(was_fullscreen) = self.lyrics_fullscreen.take() {
            self.context.send_viewport_cmd(egui::ViewportCommand::Fullscreen(was_fullscreen));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, KeyboardShortcut as S, Modifiers as M};
        const ALT_SHIFT: M = M { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };
        const CTRL_SHIFT: M = M { alt: false, ctrl: true, shift: true, mac_cmd: false, command: false };
        const CTRL_ALT: M = M { alt: true, ctrl: true, shift: false, mac_cmd: false, command: false };

        let text_focused = ctx.egui_wants_keyboard_input();
        let press = |ctx: &egui::Context, m: M, k: Key| ctx.input_mut(|i| i.consume_shortcut(&S::new(m, k)));
        let mut out: Vec<Action> = Vec::new();

        if press(ctx, M::CTRL, Key::L) || press(ctx, M::CTRL, Key::K) {
            self.focus_search = true;
            self.search_dropdown_open = true;
        }
        if press(ctx, CTRL_ALT, Key::F) {
            self.data.settings.left_collapsed = false;
            self.lib_search = true;
            self.focus_lib_search = true;
            self.focused_region = FocusRegion::Sidebar;
            self.dirty = true;
        }
        if press(ctx, M::CTRL, Key::F) {
            self.entity_filter_open = true;
            self.focus_entity_filter = true;
        }
        if press(ctx, M::CTRL, Key::Slash) {
            self.help_open = !self.help_open;
        }
        if press(ctx, M::CTRL, Key::Comma) {
            out.push(Action::Navigate(Route::Settings));
        }
        if press(ctx, CTRL_SHIFT, Key::N) {
            out.push(Action::CreateFolder(None));
        }
        if press(ctx, M::CTRL, Key::N) {
            out.push(Action::CreatePlaylist(None));
        }
        if press(ctx, CTRL_ALT, Key::ArrowLeft) {
            out.push(Action::Back);
        }
        if press(ctx, CTRL_ALT, Key::ArrowRight) {
            out.push(Action::Forward);
        }
        if ctx.input_mut(|i| i.key_pressed(Key::Escape)) {
            if self.preview_feed.is_some() {
                self.preview_feed = None;
            } else if self.lyrics_fullscreen.is_some() {
                self.exit_lyrics_fullscreen();
            } else if self.lyrics_cinema {
                self.lyrics_cinema = false;
                self.lyrics_cinema_queue = false;
            } else if self.confirm.is_some() {
                self.confirm = None;
            } else if self.edit_playlist.is_some() {
                self.edit_playlist = None;
            } else if self.rename.is_some() {
                self.rename = None;
            } else if self.rename_folder.is_some() {
                self.rename_folder = None;
            } else if self.help_open {
                self.help_open = false;
            } else if self.lyrics_manager_open {
                self.lyrics_manager_open = false;
            } else if self.top_popover.is_some() {
                self.top_popover = None;
            } else if self.search_has_focus || self.search_dropdown_open {
                if !self.search.is_empty() {
                    self.clear_search();
                } else {
                    self.search_dropdown_open = false;
                }
            } else if self.route == Route::Lyrics {
                out.push(Action::ToggleLyrics);
            } else if self.entity_filter_open || !self.entity_filter.is_empty() {
                if !self.entity_filter.is_empty() {
                    self.entity_filter.clear();
                } else {
                    self.entity_filter_open = false;
                }
            } else if self.lib_search || !self.lib_query.is_empty() {
                if !self.lib_query.is_empty() {
                    self.lib_query.clear();
                } else {
                    self.lib_search = false;
                }
            } else if self.route == Route::Search && !self.search.is_empty() {
                self.clear_search();
            }
        }

        if !text_focused {
            if self.preview_feed.is_some() {
                if press(ctx, M::NONE, Key::ArrowDown) {
                    out.push(Action::PreviewFeedStep(1));
                }
                if press(ctx, M::NONE, Key::ArrowUp) {
                    out.push(Action::PreviewFeedStep(-1));
                }
            }
            if press(ctx, M::NONE, Key::Questionmark)
                || press(ctx, M::SHIFT, Key::Questionmark)
                || press(ctx, M::SHIFT, Key::Slash)
            {
                self.help_open = !self.help_open;
            }
            // Check multi-modifier ALT_SHIFT bindings before single-modifier ALT/SHIFT bindings.
            let window_w = ctx.input(|i| i.content_rect().width());
            let max_left = self.max_left_width(window_w);
            let mut resized = false;
            if press(ctx, ALT_SHIFT, Key::ArrowLeft) {
                let s = &mut self.data.settings;
                s.left_width = (s.left_width - SIDEBAR_KEY_STEP).max(theme::LEFT_MIN);
                resized = true;
            }
            if press(ctx, ALT_SHIFT, Key::ArrowRight) {
                let s = &mut self.data.settings;
                s.left_width = (s.left_width + SIDEBAR_KEY_STEP).min(max_left);
                resized = true;
            }
            if press(ctx, ALT_SHIFT, Key::ArrowDown) {
                let s = &mut self.data.settings;
                s.right_width = (s.right_width - SIDEBAR_KEY_STEP).max(theme::RIGHT_MIN);
                resized = true;
            }
            if press(ctx, ALT_SHIFT, Key::ArrowUp) {
                let s = &mut self.data.settings;
                s.right_width = (s.right_width + SIDEBAR_KEY_STEP).min(theme::RIGHT_MAX);
                resized = true;
            }
            self.dirty |= resized;
            if press(ctx, ALT_SHIFT, Key::B) {
                out.extend(self.player.current.map(Action::ToggleLike));
            }
            if press(ctx, ALT_SHIFT, Key::H) {
                out.push(Action::Navigate(Route::Home));
            }
            if press(ctx, ALT_SHIFT, Key::S) {
                out.push(Action::Navigate(Route::Liked));
            }
            if press(ctx, ALT_SHIFT, Key::Q) {
                out.push(Action::ToggleRight(RightTab::Queue));
            }
            if press(ctx, ALT_SHIFT, Key::J) {
                out.push(Action::ToggleRight(RightTab::NowPlaying));
            }
            if press(ctx, ALT_SHIFT, Key::R) {
                if self.data.settings.show_right {
                    out.push(Action::CloseRight);
                } else {
                    out.push(Action::ToggleRight(self.right_tab));
                }
            }
            if press(ctx, ALT_SHIFT, Key::L) {
                out.push(Action::ToggleLeft);
            }
            if press(ctx, ALT_SHIFT, Key::U) {
                out.push(Action::ToggleFriendActivity);
            }
            if press(ctx, ALT_SHIFT, Key::Y) {
                out.push(Action::ToggleRight(RightTab::History));
            }
            if press(ctx, ALT_SHIFT, Key::Num0) {
                self.lib_filter = LibraryFilter::All;
                self.lib_creator_filter = LibraryCreatorFilter::All;
                self.focused_region = FocusRegion::Sidebar;
            }
            if press(ctx, ALT_SHIFT, Key::Num1) {
                self.lib_filter = LibraryFilter::Playlists;
                self.focused_region = FocusRegion::Sidebar;
            }
            if press(ctx, ALT_SHIFT, Key::Num2) {
                self.focused_region = FocusRegion::Main;
            }
            if press(ctx, ALT_SHIFT, Key::Num3) {
                self.lib_filter = LibraryFilter::Artists;
                self.focused_region = FocusRegion::Sidebar;
            }
            if press(ctx, ALT_SHIFT, Key::Num4) {
                self.lib_filter = LibraryFilter::Albums;
                self.focused_region = FocusRegion::Sidebar;
            }
            if press(ctx, ALT_SHIFT, Key::Num5) {
                self.focused_region = FocusRegion::Right;
            }

            if press(ctx, M::NONE, Key::Space) {
                out.push(Action::TogglePlay);
            }
            if press(ctx, M::NONE, Key::M) || press(ctx, M::CTRL, Key::M) {
                out.push(Action::ToggleMute);
            }
            if press(ctx, M::CTRL, Key::ArrowRight) {
                out.push(Action::Next);
            }
            if press(ctx, M::CTRL, Key::ArrowLeft) {
                out.push(Action::Previous);
            }
            if press(ctx, M::SHIFT, Key::ArrowRight) {
                out.push(Action::SeekBy(SEEK_STEP_MS as i64));
            }
            if press(ctx, M::SHIFT, Key::ArrowLeft) {
                out.push(Action::SeekBy(-(SEEK_STEP_MS as i64)));
            }
            if press(ctx, M::CTRL, Key::ArrowUp) {
                out.push(Action::SetVolume(self.data.settings.volume + volume_key_step(VOLUME_STEPS)));
            }
            if press(ctx, M::CTRL, Key::ArrowDown) {
                out.push(Action::SetVolume(self.data.settings.volume - volume_key_step(VOLUME_STEPS)));
            }
            if press(ctx, M::CTRL, Key::S) {
                out.push(Action::ToggleShuffle);
            }
            if press(ctx, M::CTRL, Key::R) {
                out.push(Action::CycleRepeat);
            }
            if press(ctx, M::ALT, Key::ArrowLeft) {
                out.push(Action::Back);
            }
            if press(ctx, M::ALT, Key::ArrowRight) {
                out.push(Action::Forward);
            }
        }
        self.actions.extend(out);
    }

    fn finish_frame(&mut self, ctx: &egui::Context) {
        self.pump_search_debounce();
        for action in std::mem::take(&mut self.actions) {
            self.run_action(action);
        }
        if self.route != Route::Lyrics {
            self.exit_lyrics_fullscreen();
            self.lyrics_cinema = false;
            self.lyrics_cinema_queue = false;
        }
        if !matches!(self.route, Route::Home | Route::Search | Route::YouTubeMusicItem(_)) {
            self.ytm.hide();
        }
        if self.is_playing() && self.position_ms() > 0 {
            self.error_streak = 0;
            if let Some(id) = self.player.current {
                if self.data.listening_history.first() != Some(&id) {
                    self.data.record_listen(id);
                    self.dirty = true;
                }
            }
        }
        self.save_if_due();
        if let Some((_, at)) = &self.toast {
            if at.elapsed() > Duration::from_secs(3) {
                self.toast = None;
            }
        }
        let busy = self.is_playing() || self.toast.is_some() || self.seek_drag.is_some();
        let wait = if self.search_debounce_pending() {
            Duration::from_millis(50)
        } else if busy {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        };
        ctx.request_repaint_after(wait);
    }

    fn drive_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.screenshot.clone() else { return };
        if std::env::var_os("NHIP_SCREENSHOT_TRACK").is_some() && ctx.input(|i| i.time) < 12.0 {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        self.frame += 1;
        if matches!(self.route, Route::Home | Route::Search | Route::YouTubeMusicItem(_)) && !self.lib.demo
            && (self.ytm.loading() || self.art.loading() && ctx.input(|input| input.time) < 4.0)
            && ctx.input(|input| input.time) < 21.0
        {
            ctx.request_repaint_after(Duration::from_millis(150));
            return;
        }
        ctx.request_repaint();
        if self.frame >= SCREENSHOT_FRAMES && !self.shot_requested {
            self.shot_requested = true;
            let done = self.shot_done.clone();
            let wake = ctx.clone();
            ctx.request_screenshot(move |img| {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let [w, h] = img.size;
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                let _ = image::save_buffer(&path, &rgba, w as u32, h as u32, image::ColorType::Rgba8);
                done.store(true, std::sync::atomic::Ordering::SeqCst);
                wake.request_repaint();
            });
        }
        if self.shot_done.load(std::sync::atomic::Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        if self.screenshot.is_some() {
            input.events.retain(|event| !matches!(event, egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }));
            input.events.push(egui::Event::PointerGone);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.art.pump(&ctx);
        self.pump_engine();
        self.pump_autoplay();
        self.pump_media();
        self.shortcuts(&ctx);
        crate::ui::draw(self, ui);
        self.finish_frame(&ctx);
        self.drive_screenshot(&ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.screenshot.is_none() {
            let cached = !self.library_dirty || library::save_library_cache(&self.lib).is_ok();
            if self.can_save_user && cached {
                let _ = self.store_user();
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn test_app() -> App {
        let ctx = egui::Context::default();
        let data = UserData::default();
        let wake = ctx.clone();
        let engine = Engine::start_silent(audio_settings(&data), move || wake.request_repaint());
        let mut app = App {
            lib: Library::default(),
            player: Player::default(),
            engine,
            art: ArtCache::new(ctx.clone()),
            route: Route::Home,
            back: Vec::new(),
            forward: Vec::new(),
            actions: Vec::new(),
            search: String::new(),
            instant_query: String::new(),
            search_dropdown_open: false,
            search_has_focus: false,
            search_debounce_at: None,
            focus_search: false,
            lib_filter: LibraryFilter::All,
            lib_creator_filter: LibraryCreatorFilter::All,
            lib_query: String::new(),
            lib_search: false,
            focus_lib_search: false,
            opened_folder: None,
            rename_folder: None,
            home_filter: HomeFilter::All,
            home_scroll_to_top: false,
            scroll_to_top_seq: 0,
            table_sort: TableSort::default(),
            entity_filter: String::new(),
            entity_filter_open: false,
            focus_entity_filter: false,
            right_tab: RightTab::NowPlaying,
            help_open: false,
            rename: None,
            edit_playlist: None,
            confirm: None,
            seek_drag: None,
            lyrics: None,
            lyrics_for: None,
            lyrics_fullscreen: None,
            lyrics_card_expanded: false,
            lyrics_card_collapsed: false,
            lyrics_cinema: false,
            lyrics_nowbar_right: false,
            lyrics_cinema_queue: false,
            lyrics_split_nowbar: false,
            lyrics_manager_open: false,
            lyrics_manager_query: String::new(),
            lyrics_ttml_mode: TtmlImportMode::Save,
            lyrics_session_override: false,
            lyrics_delete_confirm_for: None,
            session_lyrics_overrides: HashMap::new(),
            ytm: Default::default(),
            lyrics_imported: false,
            lyrics_response: None,
            session_played_yt: HashSet::new(),
            last_yt_seed: None,
            last_prefetched_next: None,
            autoplay_req_id: 0,
            autoplay_response: None,
            top_popover: None,
            preview_feed: None,
            friend_activity_open: false,
            miniplayer_open: false,
            unread_notifications: false,
            focused_region: FocusRegion::Main,
            toast: None,
            demo_playlist_ids: Vec::new(),
            error_streak: 0,
            halt_after_errors: false,
            dirty: false,
            can_save_user: false,
            last_save: Instant::now(),
            library_dirty: false,
            context: ctx,
            #[cfg(windows)]
            media: None,
            media_track: None,
            media_playing: false,
            screenshot: Some(PathBuf::from("test")),
            frame: 0,
            shot_requested: false,
            shot_done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            data,
        };
        app.enter_demo();
        app
    }

    fn press_key(app: &mut App, ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key, text_focused: bool) {
        let mut text = String::new();
        if text_focused {
            let mut setup = egui::RawInput::default();
            setup.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1304.0, 724.0)));
            let mut out = ctx.run_ui(setup, |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let resp = ui.text_edit_singleline(&mut text);
                    resp.request_focus();
                });
            });
            out.textures_delta.clear();
        } else if let Some(focused_id) = ctx.memory(|m| m.focused()) {
            ctx.memory_mut(|m| m.surrender_focus(focused_id));
        }
        let mut raw = egui::RawInput::default();
        raw.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1304.0, 724.0)));
        raw.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        let mut out = ctx.run_ui(raw, |ui| {
            let ctx = ui.ctx().clone();
            if text_focused {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.text_edit_singleline(&mut text);
                });
                assert!(ctx.egui_wants_keyboard_input());
            }
            app.shortcuts(&ctx);
        });
        out.textures_delta.clear();
        for action in std::mem::take(&mut app.actions) {
            app.run_action(action);
        }
    }

    #[test]
    fn toggle_right_queue_and_now_playing_match_spotify() {
        let mut app = test_app();
        app.data.settings.show_right = true;
        app.right_tab = RightTab::NowPlaying;

        // Clicking Queue when NowPlaying is open switches to Queue.
        app.run_action(Action::ToggleRight(RightTab::Queue));
        assert!(app.data.settings.show_right);
        assert_eq!(app.right_tab, RightTab::Queue);

        // Clicking Queue when Queue is open closes the right panel.
        app.run_action(Action::ToggleRight(RightTab::Queue));
        assert!(!app.data.settings.show_right);

        // Clicking Queue when closed opens Queue.
        app.run_action(Action::ToggleRight(RightTab::Queue));
        assert!(app.data.settings.show_right);
        assert_eq!(app.right_tab, RightTab::Queue);

        // Clicking NowPlaying when Queue is open switches to NowPlaying.
        app.run_action(Action::ToggleRight(RightTab::NowPlaying));
        assert!(app.data.settings.show_right);
        assert_eq!(app.right_tab, RightTab::NowPlaying);

        // Clicking NowPlaying when NowPlaying is open closes the panel.
        app.run_action(Action::ToggleRight(RightTab::NowPlaying));
        assert!(!app.data.settings.show_right);

        // Clicking Queue when History tab is active inside Queue panel also closes it.
        app.data.settings.show_right = true;
        app.right_tab = RightTab::History;
        app.run_action(Action::ToggleRight(RightTab::Queue));
        assert!(!app.data.settings.show_right);
    }

    #[test]
    fn queue_move_and_clear_confirmation_flow() {
        let mut app = test_app();
        let ids = app.lib.order.clone();
        app.run_action(Action::Enqueue(ids[0]));
        app.run_action(Action::Enqueue(ids[1]));
        app.run_action(Action::Enqueue(ids[2]));

        app.run_action(Action::MoveQueueItem(0, 2));
        assert_eq!(app.player.queue.iter().copied().collect::<Vec<_>>(), vec![ids[1], ids[2], ids[0]]);

        app.run_action(Action::RequestClearQueue);
        assert_eq!(app.confirm, Some(ConfirmAction::ClearQueue));

        app.run_action(Action::CancelConfirm);
        assert!(app.confirm.is_none());
        assert_eq!(app.player.queue.len(), 3);

        app.run_action(Action::RequestClearQueue);
        app.run_action(Action::AcceptConfirm);
        assert!(app.confirm.is_none());
        assert!(app.player.queue.is_empty());
    }

    #[test]
    fn home_reclick_scrolls_to_top_and_table_sort_cycles_three_states() {
        let mut app = test_app();

        // Navigating to Home while already on Home triggers scroll-to-top.
        assert!(!app.home_scroll_to_top);
        app.run_action(Action::Navigate(Route::Home));
        assert!(app.home_scroll_to_top);
        assert_eq!(app.scroll_to_top_seq, 1);

        // Table sort cycles: ascending -> descending -> default (unsorted).
        app.run_action(Action::ToggleTableSort(TrackSortColumn::Title));
        assert_eq!(app.table_sort, TableSort { column: TrackSortColumn::Title, ascending: true });
        app.run_action(Action::ToggleTableSort(TrackSortColumn::Title));
        assert_eq!(app.table_sort, TableSort { column: TrackSortColumn::Title, ascending: false });
        app.run_action(Action::ToggleTableSort(TrackSortColumn::Title));
        assert_eq!(app.table_sort, TableSort::default());
    }

    #[test]
    fn edit_playlist_validates_name_and_clamps_description_to_300_chars() {
        let mut app = test_app();
        let pid = app.data.playlists[0].id;
        app.run_action(Action::BeginEditPlaylist(pid));
        assert!(app.edit_playlist.is_some());

        // Empty name keeps modal open with error.
        app.edit_playlist.as_mut().unwrap().name = "   ".into();
        app.run_action(Action::SaveEditPlaylist);
        assert!(app.edit_playlist.as_ref().unwrap().error.is_some());

        // Valid name + 350-char description saves cleanly and clamps to 300 chars.
        let edit = app.edit_playlist.as_mut().unwrap();
        edit.name = "  Late Night  ".into();
        edit.description = "a".repeat(350);
        edit.image = Some("cover.png".into());
        app.run_action(Action::SaveEditPlaylist);
        assert!(app.edit_playlist.is_none());

        let saved = app.data.playlist(pid).unwrap();
        assert_eq!(saved.name, "Late Night");
        assert_eq!(saved.description.chars().count(), 300);
        assert_eq!(saved.image.as_deref(), Some("cover.png"));
    }

    #[test]
    fn ttml_just_once_export_reset_and_card_toggles() {
        let mut app = test_app();
        let tid = app.lib.order[0];
        app.player.play_context(ContextRef::All, "All".into(), vec![tid], Some(0));

        let xml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="1s" end="2s">Hello world</p></div></body></tt>"#;
        app.apply_ttml_xml(tid, xml, TtmlImportMode::JustOnce).unwrap();
        assert!(app.lyrics_imported());
        assert!(app.lyrics_session_override);
        assert_eq!(app.export_lyrics_xml(&app.track(tid).unwrap()).as_deref(), Some(xml));

        app.run_action(Action::ResetLyrics(tid));
        assert!(!app.lyrics_session_override);
        assert!(app.lyrics.is_none());

        app.run_action(Action::ToggleLyricsCardExpand);
        assert!(app.lyrics_card_expanded);
        assert!(!app.lyrics_card_collapsed);
        app.run_action(Action::ToggleLyricsCardCollapse);
        assert!(app.lyrics_card_collapsed);
        assert!(!app.lyrics_card_expanded);

        // Two-step delete confirm.
        app.run_action(Action::ConfirmDeleteTtmlStep(tid));
        assert_eq!(app.lyrics_delete_confirm_for, Some(tid));
        app.run_action(Action::ConfirmDeleteTtmlStep(tid));
        assert_eq!(app.lyrics_delete_confirm_for, None);
    }

    #[test]
    fn popovers_friend_activity_and_search_category_work() {
        let mut app = test_app();
        app.unread_notifications = true;
        app.run_action(Action::ToggleNotifications);
        assert_eq!(app.top_popover, Some(TopPopover::Notifications));
        assert!(!app.unread_notifications);
        app.run_action(Action::ToggleAccountMenu);
        assert_eq!(app.top_popover, Some(TopPopover::Account));
        app.run_action(Action::ClosePopovers);
        assert!(app.top_popover.is_none());

        app.run_action(Action::ToggleFriendActivity);
        assert!(app.friend_activity_open);
        assert!(app.data.settings.show_right);
        app.run_action(Action::ToggleRight(RightTab::Queue));
        assert!(!app.friend_activity_open);
        assert_eq!(app.right_tab, RightTab::Queue);

        app.run_action(Action::SearchCategory("Jazz".into()));
        assert_eq!(app.route, Route::Search);
        assert_eq!(app.search, "Jazz");
        assert_eq!(app.instant_query, "Jazz");
        assert!(!app.search_dropdown_open);
        assert_eq!(app.data.recent_searches.first().map(String::as_str), Some("Jazz"));
    }

    #[test]
    fn keyboard_shortcuts_respect_focus_guards_and_spotify_bindings() {
        let mut app = test_app();
        let ctx = app.context.clone();
        let alt_shift = egui::Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };
        let ctrl_shift = egui::Modifiers { alt: false, ctrl: true, shift: true, mac_cmd: false, command: false };
        let ctrl_alt = egui::Modifiers { alt: true, ctrl: true, shift: false, mac_cmd: false, command: false };

        // '?' opens shortcuts modal when not in a text box, ignored when text-focused.
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Questionmark, true);
        assert!(!app.help_open);
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Questionmark, false);
        assert!(app.help_open);

        // Esc closes help modal first.
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape, false);
        assert!(!app.help_open);

        // 'M' toggles mute only when not text-focused.
        assert!(!app.data.settings.muted);
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::M, true);
        assert!(!app.data.settings.muted);
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::M, false);
        assert!(app.data.settings.muted);

        // Ctrl+K / Ctrl+L focus search & open dropdown even when text-focused.
        press_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::K, true);
        assert!(app.focus_search);
        assert!(app.search_dropdown_open);

        // Esc clears search query when search dropdown is open.
        app.search = "abc".into();
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape, false);
        assert!(app.search.is_empty());

        // Ctrl+Shift+N creates a folder; Ctrl+, opens Settings.
        let folders_before = app.data.folders.len();
        press_key(&mut app, &ctx, ctrl_shift, egui::Key::N, false);
        assert_eq!(app.data.folders.len(), folders_before + 1);
        app.rename_folder = None;

        press_key(&mut app, &ctx, egui::Modifiers::CTRL, egui::Key::Comma, false);
        assert_eq!(app.route, Route::Settings);

        // Ctrl+Alt+F opens library filter search.
        press_key(&mut app, &ctx, ctrl_alt, egui::Key::F, false);
        assert!(app.lib_search);
        assert!(app.focus_lib_search);

        // Alt+Shift+Left/Right resize left sidebar in 10px steps.
        let start_w = app.data.settings.left_width;
        press_key(&mut app, &ctx, alt_shift, egui::Key::ArrowRight, false);
        assert_eq!(app.data.settings.left_width, start_w + 10.0);
        press_key(&mut app, &ctx, alt_shift, egui::Key::ArrowLeft, false);
        assert_eq!(app.data.settings.left_width, start_w);

        // Alt+Shift+0..5 update filters & focus regions without demo-gating 3/4.
        app.lib.demo = false;
        press_key(&mut app, &ctx, alt_shift, egui::Key::Num4, false);
        assert_eq!(app.lib_filter, LibraryFilter::Albums);
        press_key(&mut app, &ctx, alt_shift, egui::Key::Num3, false);
        assert_eq!(app.lib_filter, LibraryFilter::Artists);
        press_key(&mut app, &ctx, alt_shift, egui::Key::Num2, false);
        assert_eq!(app.focused_region, FocusRegion::Main);
        press_key(&mut app, &ctx, alt_shift, egui::Key::Num5, false);
        assert_eq!(app.focused_region, FocusRegion::Right);
        press_key(&mut app, &ctx, alt_shift, egui::Key::Num0, false);
        assert_eq!(app.lib_filter, LibraryFilter::All);
        assert_eq!(app.focused_region, FocusRegion::Sidebar);

        // Alt+Shift+S / Q / J / R / L
        press_key(&mut app, &ctx, alt_shift, egui::Key::S, false);
        assert_eq!(app.route, Route::Liked);
        press_key(&mut app, &ctx, alt_shift, egui::Key::Q, false);
        assert_eq!(app.right_tab, RightTab::Queue);
        assert!(app.data.settings.show_right);
        press_key(&mut app, &ctx, alt_shift, egui::Key::J, false);
        assert_eq!(app.right_tab, RightTab::NowPlaying);
        press_key(&mut app, &ctx, alt_shift, egui::Key::R, false);
        assert!(!app.data.settings.show_right);
        press_key(&mut app, &ctx, alt_shift, egui::Key::R, false);
        assert!(app.data.settings.show_right);
        press_key(&mut app, &ctx, alt_shift, egui::Key::L, false);
        assert!(app.data.settings.left_collapsed);

        // Esc exits non-fullscreen Route::Lyrics.
        app.run_action(Action::Navigate(Route::Lyrics));
        assert_eq!(app.route, Route::Lyrics);
        press_key(&mut app, &ctx, egui::Modifiers::NONE, egui::Key::Escape, false);
        assert_ne!(app.route, Route::Lyrics);
    }

    #[test]
    fn youtube_tracks_build_youtube_load_spec_and_are_playable_in_app() {
        let mut app = test_app();
        let item = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("dQw4w9WgXcQ".into()),
            title: "Never Gonna Give You Up".into(),
            by: "Rick Astley".into(),
            image: None,
            duration_ms: 213_000,
            album: "Whenever You Need Somebody".into(),
        };
        let id = app.remember_youtube_song(&item).unwrap();
        app.player.current = Some(id);
        assert!(app.can_play_in_app());

        let spec = app.load_spec(id).unwrap();
        assert_eq!(spec.id, id);
        assert!(matches!(
            spec.source,
            SourceSpec::YouTube { ref video_id } if video_id == "dQw4w9WgXcQ"
        ));
    }

    #[test]
    fn select_youtube_song_plays_and_ignores_duplicate_selection() {
        let mut app = test_app();
        let item = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("poGyHfrJ_uo".into()),
            title: "Khóc Đấy".into(),
            by: "Thắng".into(),
            image: None,
            duration_ms: 0,
            album: String::new(),
        };
        app.run_action(Action::SelectYouTubeSong(item.clone()));
        app.engine.stop();
        let id = crate::library::fnv("yt:poGyHfrJ_uo");
        assert_eq!(app.player.current, Some(id));
        assert!(app.can_play_in_app());
        assert!(app.session_played_yt.contains("poGyHfrJ_uo"));

        // Second emission (e.g., after Open(Song) fetches album/duration) updates metadata without switching tracks.
        let enriched = crate::ytm::Item {
            duration_ms: 228_000,
            album: "Cái Thứ Hai".into(),
            ..item
        };
        app.run_action(Action::SelectYouTubeSong(enriched));
        assert_eq!(app.player.current, Some(id));
        assert_eq!(app.track(id).unwrap().duration_ms, 228_000);
        assert_eq!(app.track(id).unwrap().album, "Cái Thứ Hai");
    }

    #[test]
    fn hide_track_advances_without_removing_likes_and_can_be_restored() {
        let mut app = test_app();
        let first = app.lib.order[0];
        let next = app.lib.order[1];
        app.data.toggle_like(first);
        let saved_playlists = serde_json::to_value(&app.data.playlists).unwrap();
        app.player.play_context(ContextRef::All, "All".into(), vec![first, next], Some(0));
        app.run_action(Action::HideTrack(first));
        app.engine.stop();
        assert_eq!(app.player.current, Some(next));
        assert!(app.data.hidden_tracks.contains(&first));
        assert!(app.data.is_liked(first));
        assert!(app.lib.tracks.contains_key(&first));
        assert_eq!(serde_json::to_value(&app.data.playlists).unwrap(), saved_playlists);
        assert!(app.dirty);
        app.dirty = false;
        app.run_action(Action::RestoreHiddenTracks);
        assert!(app.data.hidden_tracks.is_empty());
        assert!(app.data.is_liked(first));
        assert!(app.dirty);
    }

    #[test]
    fn profile_pages_support_picture_staging_and_back_navigation() {
        for (page, route) in [("profile", Route::Profile), ("taste", Route::TasteProfile)] {
            let mut app = test_app();
            app.stage_for_picture(Some(page));
            assert_eq!(app.route, route);
            app.run_action(Action::Navigate(Route::Home));
            app.run_action(Action::Back);
            assert_eq!(app.route, route);
            app.engine.stop();
        }
    }

    #[test]
    fn autoplay_excludes_hidden_and_played_songs_and_stops_when_exhausted() {
        let mut app = test_app();
        let playlists_before = app.data.playlists.clone();
        let seed_item = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("seedVid1234".into()),
            title: "Seed".into(),
            by: "Artist".into(),
            image: None,
            duration_ms: 180_000,
            album: String::new(),
        };
        let seed_id = app.remember_youtube_song(&seed_item).unwrap();
        app.note_youtube_started(seed_id);
        let hidden_item = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("hiddenVid11".into()),
            ..seed_item.clone()
        };
        let hidden_id = crate::library::fnv("yt:hiddenVid11");
        app.run_action(Action::HideTrack(hidden_id));

        let related = vec![
            seed_item,
            hidden_item.clone(),
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("nextVid1111".into()),
                title: "Next 1".into(),
                by: "Artist".into(),
                image: None,
                duration_ms: 200_000,
                album: String::new(),
            },
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("nextVid2222".into()),
                title: "Next 2".into(),
                by: "Artist".into(),
                image: None,
                duration_ms: 210_000,
                album: String::new(),
            },
        ];

        assert!(app.handle_autoplay_items(related));
        app.engine.stop();
        assert_eq!(app.player.context_ref, ContextRef::Autoplay);
        assert_eq!(app.player.current, Some(crate::library::fnv("yt:nextVid1111")));
        assert_eq!(app.player.peek_next(), Some(crate::library::fnv("yt:nextVid2222")));
        assert!(app.track(hidden_id).is_none());

        // Hidden and already-played songs cannot restart autoplay.
        assert!(!app.handle_autoplay_items(vec![hidden_item, crate::ytm::Item {
            id: crate::ytm::ItemId::Song("nextVid1111".into()),
            title: "Next 1".into(),
            by: "Artist".into(),
            image: None,
            duration_ms: 200_000,
            album: String::new(),
        }]));
        assert_eq!(app.data.playlists.len(), playlists_before.len());
    }

    #[test]
    fn error_streak_skips_halts_at_limit_and_resets_on_playback() {
        let mut app = test_app();
        let playlists_before = app.data.playlists.clone();
        let tracks: Vec<TrackId> = app.lib.order.iter().copied().take(6).collect();
        assert_eq!(tracks.len(), 6);
        app.play_context(ContextRef::All, "All".into(), tracks.clone(), Some(0));

        // Three errors advance through tracks[0..3] cleanly; Duration resets streak to 0.
        app.handle_engine_event(Event::Error(tracks[0], "fail 1".into()));
        assert_eq!(app.player.current, Some(tracks[1]));
        assert_eq!(app.error_streak, 1);
        app.handle_engine_event(Event::Duration(tracks[1], 10_000));
        assert_eq!(app.error_streak, 0);

        // Preloaded next track error advances past tracks[2] then Finished moves to tracks[3].
        app.handle_engine_event(Event::Error(tracks[2], "next failed".into()));
        assert_eq!(app.error_streak, 1);
        app.handle_engine_event(Event::Finished);
        assert_eq!(app.player.current, Some(tracks[3]));
        app.handle_engine_event(Event::Duration(tracks[3], 10_000));
        assert_eq!(app.error_streak, 0);

        // Five consecutive errors halt playback at MAX_ERROR_STREAK without mutating playlists.
        for _ in 0..MAX_ERROR_STREAK {
            let cur = app.player.current.unwrap();
            app.handle_engine_event(Event::Error(cur, "boom".into()));
        }
        assert_eq!(app.error_streak, 0);
        assert!(app.halt_after_errors);
        assert!(app.toast.is_some());
        let stopped_at = app.player.current;
        app.handle_engine_event(Event::Finished);
        assert_eq!(app.player.current, stopped_at);
        assert!(!app.halt_after_errors);
        assert_eq!(app.data.playlists.len(), playlists_before.len());
        app.engine.stop();
    }

    #[test]
    fn toggle_play_on_youtube_track_stays_on_current_and_toggles_playing() {
        let mut app = test_app();
        // Verify in-app TogglePlay flips paused/playing state on a playable deck.
        let local_id = app.lib.order[0];
        app.play_context(ContextRef::All, "All".into(), vec![local_id], Some(0));
        for _ in 0..40 {
            if app.is_playing() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.is_playing());
        app.run_action(Action::TogglePlay);
        for _ in 0..40 {
            if !app.is_playing() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.is_playing());
        app.run_action(Action::TogglePlay);
        for _ in 0..40 {
            if app.is_playing() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(app.is_playing());
        app.engine.stop();

        // Selecting a YouTube song registers an in-app playable track, and re-selecting or TogglePlay preserves current without navigating away.
        let item = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("dQw4w9WgXcQ".into()),
            title: "Bút Chì Bạc".into(),
            by: "Thắng".into(),
            image: None,
            duration_ms: 176_000,
            album: "Cái Thứ Hai".into(),
        };
        let route_before = app.route.clone();
        app.run_action(Action::SelectYouTubeSong(item.clone()));
        let tid = crate::library::fnv("yt:dQw4w9WgXcQ");
        assert_eq!(app.player.current, Some(tid));
        assert!(app.can_play_in_app());

        app.run_action(Action::SelectYouTubeSong(item));
        assert_eq!(app.player.current, Some(tid));
        assert_eq!(app.route, route_before);

        app.run_action(Action::TogglePlay);
        assert_eq!(app.player.current, Some(tid));
        app.engine.stop();
    }

    #[test]
    fn play_youtube_list_populates_context_queue_at_start() {
        let mut app = test_app();
        let items = vec![
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("vidA1111111".into()),
                title: "Track 1".into(),
                by: "Thắng".into(),
                image: None,
                duration_ms: 150_000,
                album: "Cái Thứ Hai".into(),
            },
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("vidB2222222".into()),
                title: "Track 2".into(),
                by: "Thắng".into(),
                image: None,
                duration_ms: 160_000,
                album: "Cái Thứ Hai".into(),
            },
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("vidC3333333".into()),
                title: "Track 3".into(),
                by: "Thắng".into(),
                image: None,
                duration_ms: 170_000,
                album: "Cái Thứ Hai".into(),
            },
        ];
        app.run_action(Action::PlayYouTubeList {
            name: "Cái Thứ Hai".into(),
            items,
            start: 1,
        });
        assert_eq!(app.player.context_name, "Cái Thứ Hai");
        assert_eq!(app.player.current, Some(crate::library::fnv("yt:vidB2222222")));
        assert_eq!(app.player.peek_next(), Some(crate::library::fnv("yt:vidC3333333")));
        assert!(app.can_play_in_app());
        app.engine.stop();
    }

    #[test]
    fn proactive_autoplay_appends_to_live_queue_and_ranks_by_user_taste() {
        let mut app = test_app();
        let seed = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("seedVid0001".into()),
            title: "An Thần".into(),
            by: "Low G, Thắng".into(),
            image: None,
            duration_ms: 174_000,
            album: String::new(),
        };
        app.data.followed_artists.insert("Thắng".into());
        app.run_action(Action::SelectYouTubeSong(seed.clone()));
        let seed_id = crate::library::fnv("yt:seedVid0001");
        assert_eq!(app.player.current, Some(seed_id));
        assert_eq!(app.player.peek_next(), None);

        let radio = vec![
            seed,
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("randVid0001".into()),
                title: "Random Track".into(),
                by: "Other Artist".into(),
                image: None,
                duration_ms: 180_000,
                album: String::new(),
            },
            crate::ytm::Item {
                id: crate::ytm::ItemId::Song("thangVid001".into()),
                title: "Trước Khi Em Tồn Tại".into(),
                by: "Thắng".into(),
                image: None,
                duration_ms: 194_000,
                album: String::new(),
            },
        ];
        let added = app.handle_autoplay_append(radio);
        assert_eq!(added, 2);
        // Current track is never interrupted, and followed artist track ranks first in Up Next!
        assert_eq!(app.player.current, Some(seed_id));
        assert_eq!(app.player.peek_next(), Some(crate::library::fnv("yt:thangVid001")));
        assert_eq!(app.player.context_ref, ContextRef::Autoplay);
        app.engine.stop();
    }

    #[test]
    fn preview_feed_opens_steps_clamps_and_closes_cleanly() {
        let mut app = test_app();
        let make_track = |vid: &str, title: &str| PreviewTrack {
            title: title.into(),
            artist: "Low G".into(),
            cover_key: format!("yt:{vid}"),
            cover_path: None,
            ytm_item: Some(crate::ytm::Item {
                id: crate::ytm::ItemId::Song(vid.into()),
                title: title.into(),
                by: "Low G".into(),
                image: None,
                duration_ms: 180_000,
                album: String::new(),
            }),
            local_id: None,
            duration_ms: 180_000,
            url: Some(format!("https://music.youtube.com/watch?v={vid}")),
        };
        let feed = PreviewFeedState {
            entity_title: "Low G".into(),
            entity_subtitle: Some("2 Tr".into()),
            entity_avatar_key: "ytm-artist:low-g".into(),
            entity_avatar_src: None,
            artist_name: "Low G".into(),
            items: vec![
                make_track("prevVid0001", "In Love"),
                make_track("prevVid0002", "Có Em"),
                make_track("prevVid0003", "An Thần"),
            ],
            index: 0,
            scroll_anim: 0.0,
            last_wheel: None,
            tags: vec!["#v-pop".into(), "#hip hop".into()],
        };
        app.run_action(Action::OpenPreviewFeed(feed));
        assert_eq!(app.preview_feed.as_ref().map(|f| f.index), Some(0));
        assert_eq!(app.player.current, Some(crate::library::fnv("yt:prevVid0001")));

        // Stepping forward advances index and switches to next preloaded track.
        app.run_action(Action::PreviewFeedStep(1));
        assert_eq!(app.preview_feed.as_ref().map(|f| f.index), Some(1));
        assert_eq!(app.player.current, Some(crate::library::fnv("yt:prevVid0002")));

        // Stepping past bounds clamps cleanly to last item.
        app.run_action(Action::PreviewFeedStep(5));
        assert_eq!(app.preview_feed.as_ref().map(|f| f.index), Some(2));
        assert_eq!(app.player.current, Some(crate::library::fnv("yt:prevVid0003")));

        // Stepping backward returns cleanly and ClosePreviewFeed clears modal state.
        app.run_action(Action::PreviewFeedStep(-1));
        assert_eq!(app.preview_feed.as_ref().map(|f| f.index), Some(1));
        app.run_action(Action::ClosePreviewFeed);
        assert!(app.preview_feed.is_none());
        app.engine.stop();
    }
}
