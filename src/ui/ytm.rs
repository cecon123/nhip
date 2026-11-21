//! Unified Live YouTube Music & Search pages styled identically to Spotify Desktop.

use std::cmp::Ordering;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align2, Color32, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui, UiBuilder, Vec2,
};

use super::icons::{self, Icon};
use super::pages::{
    adaptive_hero_color, album_duration, draw_carousel_shelf,
    draw_full_bleed_gradient, draw_home_page, draw_home_skeleton, draw_search_skeleton,
    draw_shelf_card, draw_sticky_mini_topbar, draw_tracklist_controls, equalizer_glyph,
    format_play_count, hero_art_side, hero_height_for_width, link, paint_artist_links,
    paint_fitted_hero_title, real_artist_albums_from_ytm, table_header, wrapped_text,
    ytm_kind_label, CardData, Columns, HomeShelf,
};
use super::widgets::{
    cover, cover_shadow, focus_ring, hover_play_button, icon_button, icon_button_sized, mmss, pill,
    pill_width, round_button, skeleton_glass_rect, text, text_width, vertical_gradient,
};
use crate::app::{Action, App, HomeFilter, Route};
use crate::library::{fold, RecentItem, TrackSortColumn};
use crate::queue::ContextRef;
use crate::theme::{
    body, bold, hover_white, semi, ACCENT, CARD, CARD_HOVER, ELEVATED, HAIRLINE, MUTED, PANEL,
    RADIUS_CARD, RADIUS_MODAL, ROW_H, SUBTLE, TEXT,
};
use crate::ytm::{Catalog, Error, Item, ItemId, Page, ReleaseKind, Request};

#[derive(Default)]
pub struct Browser {
    worker: Option<Catalog>,
    request: Option<Request>,
    generation: u64,
    pending: bool,
    error: Option<Error>,
    append: bool,
    page: Option<Page>,
    related: bool,
    visible_route: Option<Route>,
    edit_at: Option<Instant>,
    filter: SearchFilter,
    instant_worker: Option<Catalog>,
    instant_query: String,
    instant_gen: u64,
    instant_sent_at: Option<Instant>,
    instant_pending: bool,
    instant_items: Vec<Item>,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum SearchFilter {
    #[default]
    All,
    Songs,
    Albums,
    Artists,
    Playlists,
}

impl SearchFilter {
    fn includes(self, id: &ItemId) -> bool {
        matches!(
            (self, id),
            (Self::All, _)
                | (Self::Songs, ItemId::Song(_))
                | (Self::Albums, ItemId::Album(_))
                | (Self::Artists, ItemId::Artist(_))
                | (Self::Playlists, ItemId::Playlist(_))
        )
    }
}

impl Browser {
    pub fn loading(&self) -> bool {
        self.request.is_none() || self.pending
    }

    pub fn search_changed(&mut self) {
        self.edit_at = Some(Instant::now());
        self.filter = SearchFilter::All;
    }

    pub fn hide(&mut self) {
        self.visible_route = None;
    }

    pub fn poll_instant_search(&mut self, ctx: &egui::Context, query: &str) -> (bool, Vec<Item>) {
        let q = query.trim();
        if q.is_empty() || crate::ytm::from_url(q).is_some() {
            return (false, Vec::new());
        }
        if let Some(worker) = &self.instant_worker {
            for (id, resp) in worker.poll() {
                if id == self.instant_gen {
                    self.instant_pending = false;
                    if let Ok(page) = resp {
                        self.instant_items = page.items;
                    }
                }
            }
        }
        if self.instant_query != q {
            self.instant_query = q.to_owned();
            self.instant_items.clear();
            self.instant_pending = true;
            self.instant_sent_at = Some(Instant::now());
            ctx.request_repaint_after(Duration::from_millis(200));
            return (true, Vec::new());
        }
        if let Some(at) = self.instant_sent_at {
            if at.elapsed() >= Duration::from_millis(180) {
                self.instant_sent_at = None;
                self.instant_gen = self.instant_gen.wrapping_add(1);
                if self.instant_worker.is_none() {
                    let wake = ctx.clone();
                    self.instant_worker = Catalog::start(move || wake.request_repaint()).ok();
                }
                self.instant_pending = self
                    .instant_worker
                    .as_ref()
                    .is_some_and(|w| w.send(self.instant_gen, Request::Search(q.to_owned())).is_ok());
            } else {
                ctx.request_repaint_after(Duration::from_millis(60));
            }
        }
        (self.instant_pending, self.instant_items.clone())
    }

    fn send(&mut self, ctx: &egui::Context, request: Request, append: bool) {
        self.request = Some(request.clone());
        self.generation = self.generation.wrapping_add(1);
        self.error = None;
        self.append = append;
        if !append {
            self.page = None;
        }
        if self.worker.is_none() {
            let ctx = ctx.clone();
            self.worker = Catalog::start(move || ctx.request_repaint()).ok();
        }
        self.pending = self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.send(self.generation, request).is_ok());
        self.error = (!self.pending).then_some(Error::Network);
    }

    fn receive(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        let mut refreshed = false;
        for (id, response) in worker.poll() {
            if id != self.generation {
                continue;
            }
            self.pending = false;
            match response {
                Ok(mut page) => {
                    if self.append {
                        if let Some(old) = &mut self.page {
                            let used = match &self.request {
                                Some(Request::Home(token)) => token.as_deref(),
                                _ => None,
                            };
                            let mut seen: std::collections::HashSet<_> =
                                old.items.iter().map(|item| item.id.clone()).collect();
                            old.items.extend(
                                page.items
                                    .drain(..)
                                    .filter(|item| seen.insert(item.id.clone())),
                            );
                            old.sections.extend(page.sections);
                            old.release_kinds.extend(page.release_kinds);
                            old.continuation =
                                page.continuation.filter(|next| Some(next.as_str()) != used);
                            continue;
                        }
                    }
                    self.page = Some(page);
                    refreshed = true;
                }
                Err(error) => self.error = Some(error),
            }
        }
        refreshed
    }
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    let mut browser = std::mem::take(&mut app.ytm);

    if app.route == Route::Search && app.lib.demo {
        draw_search_page(app, ui, &mut browser, None, false, None, true);
        app.ytm = browser;
        return;
    }

    let detail = match &app.route {
        Route::YouTubeMusicItem(item) => Some(item.clone()),
        _ => None,
    };
    let opened = browser.visible_route.as_ref() != Some(&app.route);
    if opened {
        browser.visible_route = Some(app.route.clone());
        browser.related = false;
    }
    let song_id = match detail.as_ref().map(|item| &item.id) {
        Some(ItemId::Song(id)) => Some(id.clone()),
        _ => None,
    };

    let request = if let Some(id) = &song_id {
        if browser.related {
            Some(Request::Related(id.clone()))
        } else {
            detail
                .as_ref()
                .filter(|item| item.by.is_empty())
                .map(|item| Request::Open(item.id.clone()))
        }
    } else if let Some(item) = &detail {
        Some(Request::Open(item.id.clone()))
    } else if app.route == Route::Home {
        Some(home_recommendation_seed(app).map_or(Request::Home(None), Request::Related))
    } else if app.search.trim().is_empty() || crate::ytm::from_url(&app.search).is_some() {
        None
    } else {
        Some(Request::Search(app.search.trim().into()))
    };

    let request_matches = |browser: &Browser| {
        browser.request == request
            || matches!(
                (&browser.request, &request),
                (Some(Request::Home(_)), Some(Request::Home(None)))
            )
    };

    if let Some(request) = &request {
        if (opened && detail.is_some())
            || (!request_matches(&browser)
                && (detail.is_some()
                    || browser
                        .edit_at
                        .is_none_or(|at| at.elapsed() >= Duration::from_millis(250))))
        {
            browser.send(ui.ctx(), request.clone(), false);
        }
    }

    if opened {
        if let Some(item) = detail
            .as_ref()
            .filter(|item| matches!(item.id, ItemId::Song(_)) && !item.by.is_empty())
        {
            app.actions.push(Action::SelectYouTubeSong(item.clone()));
        }
    }

    let matches_request = request_matches(&browser);
    let refreshed = browser.receive();
    if refreshed && matches_request && matches!(&request, Some(Request::Open(ItemId::Song(_)))) {
        if let Some(song) = browser.page.as_ref().and_then(|page| page.items.first()) {
            app.actions.push(Action::SelectYouTubeSong(song.clone()));
        }
    }

    let follow_id = match &request {
        Some(Request::Open(ItemId::Playlist(id))) => Some(id.clone()),
        Some(Request::Open(ItemId::Album(id))) => Some(format!("album:{id}")),
        _ => None,
    };
    if refreshed && matches_request {
        if let (Some(id), Some(page)) = (&follow_id, &browser.page) {
            if app.data.followed_ytm.iter().any(|p| p.id == *id) {
                app.update_followed_ytm(id, Some(page.clone()));
            }
        }
    }

    let active_page = browser
        .page
        .as_ref()
        .filter(|_| matches_request)
        .cloned();
    let active_error = browser.error.filter(|_| matches_request);
    let is_pending = browser.pending && matches_request;

    if app.route == Route::Home {
        draw_ytm_home(
            app,
            ui,
            &mut browser,
            active_page.as_ref(),
            is_pending,
            active_error,
        );
    } else if app.route == Route::Search {
        let retry = request.clone();
        let retried = draw_search_page(
            app,
            ui,
            &mut browser,
            active_page.as_ref(),
            is_pending,
            active_error,
            false,
        );
        if retried {
            if let Some(req) = retry {
                browser.send(ui.ctx(), req, false);
            }
        }
    } else if let Some(item) = detail {
        draw_ytm_entity(
            app,
            ui,
            &mut browser,
            &item,
            active_page.as_ref(),
            is_pending,
            active_error,
            follow_id.as_deref(),
            request.as_ref(),
        );
    }

    app.ytm = browser;
}

// ---------------------------------------------------------------------------------------------
// Live YTM Home (unified with Spotify Home)

/// Formats YouTube Music raw artist stats (e.g. `"77,688,295 views"`, `"684.9K subscribers"`, `"1.8M monthly audience"`)
/// into localized Spotify-style Vietnamese / English phrasing (`"77,7 Tr lượt nghe"`, `"1,8 Tr người nghe hằng tháng"`).
pub(super) fn format_artist_subtitle(lang: crate::library::Lang, raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_ascii_lowercase();
    let num_token = trimmed.split_whitespace().next().unwrap_or("");
    let parsed_count = parse_compact_count(num_token);
    if let Some(count) = parsed_count {
        let formatted = format_compact_count(lang, count);
        if lower.contains("monthly") || lower.contains("listener") || lower.contains("audience") {
            return match lang {
                crate::library::Lang::Vi => format!("{formatted} người nghe hằng tháng"),
                crate::library::Lang::En => format!("{formatted} monthly listeners"),
            };
        }
        if lower.contains("sub") {
            return match lang {
                crate::library::Lang::Vi => format!("{formatted} người theo dõi"),
                crate::library::Lang::En => format!("{formatted} followers"),
            };
        }
        if lower.contains("view") || lower.contains("play") {
            return match lang {
                crate::library::Lang::Vi => format!("{formatted} lượt nghe"),
                crate::library::Lang::En => format!("{formatted} plays"),
            };
        }
        // Bare count (e.g. `"748K"` from YTM artist card) -> localized follower/listener phrasing.
        if trimmed.split_whitespace().count() == 1 {
            return match lang {
                crate::library::Lang::Vi => format!("{formatted} người nghe hằng tháng"),
                crate::library::Lang::En => format!("{formatted} monthly listeners"),
            };
        }
    }
    trimmed.to_string()
}

fn format_grouped_u64(lang: crate::library::Lang, n: u64) -> String {
    let sep = match lang {
        crate::library::Lang::Vi => '.',
        crate::library::Lang::En => ',',
    };
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (idx, ch) in s.chars().enumerate() {
        if idx > 0 && (s.len() - idx) % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    out
}

fn format_track_plays(lang: crate::library::Lang, raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let num_token = trimmed.split_whitespace().next().unwrap_or(trimmed);
    if let Some(count) = parse_compact_count(num_token) {
        if num_token.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') && !num_token.ends_with(['K', 'M', 'B', 'k', 'm', 'b']) {
            return format_grouped_u64(lang, count.round() as u64);
        }
        return format_compact_count(lang, count);
    }
    trimmed.to_string()
}

fn parse_compact_count(token: &str) -> Option<f64> {
    let clean = token.trim();
    if clean.is_empty() {
        return None;
    }
    let (digits, mult) = match clean.chars().last()? {
        'B' | 'b' => (&clean[..clean.len() - 1], 1_000_000_000.0),
        'M' | 'm' => (&clean[..clean.len() - 1], 1_000_000.0),
        'K' | 'k' => (&clean[..clean.len() - 1], 1_000.0),
        _ => (clean, 1.0),
    };
    let normalized = if mult == 1.0 {
        digits.replace([',', '.'], "")
    } else {
        digits.replace(',', ".")
    };
    normalized.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).map(|v| v * mult)
}

fn format_compact_count(lang: crate::library::Lang, count: f64) -> String {
    let fmt_dec = |v: f64| -> String {
        let rounded = (v * 10.0).round() / 10.0;
        let s = if (rounded - rounded.round()).abs() < 0.05 {
            format!("{}", rounded.round() as u64)
        } else {
            format!("{rounded:.1}")
        };
        match lang {
            crate::library::Lang::Vi => s.replace('.', ","),
            crate::library::Lang::En => s,
        }
    };
    if count >= 1_000_000_000.0 {
        let unit = match lang { crate::library::Lang::Vi => " T", crate::library::Lang::En => "B" };
        format!("{}{unit}", fmt_dec(count / 1_000_000_000.0))
    } else if count >= 1_000_000.0 {
        let unit = match lang { crate::library::Lang::Vi => " Tr", crate::library::Lang::En => "M" };
        format!("{}{unit}", fmt_dec(count / 1_000_000.0))
    } else if count >= 10_000.0 {
        let unit = match lang { crate::library::Lang::Vi => " N", crate::library::Lang::En => "K" };
        format!("{}{unit}", fmt_dec(count / 1_000.0))
    } else {
        format_grouped_u64(lang, count.round() as u64)
    }
}

fn home_recommendation_seed(app: &App) -> Option<String> {
    let mut best: Option<(i32, String)> = None;
    let mut seen = std::collections::HashSet::new();
    for (rank, id) in app
        .data
        .liked
        .iter()
        .map(|entry| entry.id)
        .chain(app.data.listening_history.iter().copied())
        .enumerate()
    {
        if app.data.hidden_tracks.contains(&id) || !seen.insert(id) {
            continue;
        }
        let Some(track) = app.track(id) else { continue };
        let Some(video) = track.youtube_id() else { continue };
        if !crate::ytm::valid_video_id(video) {
            continue;
        }
        let score = app.data.score_recommendation(&app.lib, id, &track.artist)
            + (12_i32.saturating_sub(rank as i32)).max(0);
        if best.as_ref().is_none_or(|(b, _)| score > *b) {
            best = Some((score, video.to_string()));
        }
    }
    best.map(|(_, vid)| vid)
}

fn home_item_visible(data: &crate::library::UserData, item: &Item, recommendations: bool) -> bool {
    let ItemId::Song(video) = &item.id else { return true };
    let id = crate::library::fnv(&format!("yt:{video}"));
    !data.hidden_tracks.contains(&id)
        && (!recommendations || !data.listening_history.contains(&id))
}

fn home_filter_includes(filter: HomeFilter, id: &ItemId) -> bool {
    match filter {
        HomeFilter::All | HomeFilter::Music => true,
        HomeFilter::Albums => matches!(id, ItemId::Album(_)),
        HomeFilter::Artists => matches!(id, ItemId::Artist(_)),
        HomeFilter::Playlists => matches!(id, ItemId::Playlist(_)),
    }
}

fn discography_filter_includes(filter: u8, kind: Option<&ReleaseKind>, single_ep_shelf: bool) -> bool {
    match filter {
        1 => matches!(kind, Some(ReleaseKind::Album)) || (kind.is_none() && !single_ep_shelf),
        2 => matches!(kind, Some(ReleaseKind::Single | ReleaseKind::EP)) || (kind.is_none() && single_ep_shelf),
        _ => true,
    }
}

fn search_results_side_by_side(inner_width: f32) -> bool {
    ((inner_width + 20.0) / (180.0 + 20.0)).floor() > 3.0
}

fn draw_ytm_home(
    app: &mut App,
    ui: &mut Ui,
    browser: &mut Browser,
    page: Option<&Page>,
    pending: bool,
    error: Option<Error>,
) {
    let recommendations = matches!(browser.request, Some(Request::Related(_)));
    let mut quick: Vec<CardData> = super::pages::item_of(app, &RecentItem::Liked)
        .map(|item| CardData::from_local(&item)).into_iter().collect();
    let mut seen_keys = std::collections::HashSet::new();

    for followed in &app.data.followed_ytm {
        if quick.len() >= 8 {
            break;
        }
        let it = followed.item();
        if seen_keys.insert(it.id.url()) {
            quick.push(CardData::from_ytm(app, &it, page));
        }
    }
    for &id in &app.data.listening_history {
        if quick.len() >= 8 {
            break;
        }
        if let Some(track) = app.track(id) {
            if let Some(it) = Item::from_track(&track) {
                if seen_keys.insert(it.id.url()) {
                    quick.push(CardData::from_ytm(app, &it, page));
                }
            }
        }
    }
    if let Some(p) = page {
        for it in p.items.iter().filter(|item| home_item_visible(&app.data, item, recommendations)) {
            if quick.len() >= 8 {
                break;
            }
            if seen_keys.insert(it.id.url()) {
                quick.push(CardData::from_ytm(app, it, page));
            }
        }
    }

    let history_cards: Vec<CardData> = app
        .data
        .listening_history
        .iter()
        .take(12)
        .filter_map(|&id| app.track(id))
        .filter_map(|track| Item::from_track(&track))
        .map(|it| CardData::from_ytm(app, &it, page))
        .collect();

    let followed_cards: Vec<CardData> = app
        .data
        .followed_ytm
        .iter()
        .map(|f| CardData::from_ytm(app, &f.item(), page))
        .collect();

    let mut followed_artist_names: Vec<String> = app.data.followed_artists.iter().cloned().collect();
    followed_artist_names.sort_by(|a, b| {
        app.data
            .artist_affinity(&app.lib, b)
            .cmp(&app.data.artist_affinity(&app.lib, a))
            .then_with(|| fold(a).cmp(&fold(b)))
    });
    let artist_cards: Vec<CardData> = followed_artist_names
        .iter()
        .take(12)
        .filter_map(|name| super::pages::item_of(app, &RecentItem::Artist(name.clone())))
        .map(|item| CardData::from_local(&item))
        .collect();

    let mut shelves = Vec::new();
    let filter = app.home_filter;
    if matches!(filter, HomeFilter::All | HomeFilter::Music) && !history_cards.is_empty() {
        shelves.push(HomeShelf {
            id: "ytm-recent".into(),
            title: app.t("Nghe gần đây", "Recently played").to_string(),
            cards: history_cards,
        });
    }
    if matches!(filter, HomeFilter::All | HomeFilter::Artists) && !artist_cards.is_empty() {
        shelves.push(HomeShelf {
            id: "ytm-favorite-artists".into(),
            title: app.t("Nghệ sĩ bạn theo dõi", "Artists you follow").to_string(),
            cards: artist_cards,
        });
    }
    if matches!(filter, HomeFilter::All | HomeFilter::Playlists) && !followed_cards.is_empty() {
        shelves.push(HomeShelf {
            id: "ytm-followed".into(),
            title: app.t("Playlist đang theo dõi", "Followed playlists").to_string(),
            cards: followed_cards,
        });
    }
    if let Some(page) = page {
        for (i, section) in page.sections.iter().enumerate() {
            let cards = section.items.iter()
                .filter(|item| home_filter_includes(filter, &item.id))
                .filter(|item| home_item_visible(&app.data, item, false))
                .map(|item| CardData::from_ytm(app, item, Some(page)))
                .collect();
            shelves.push(HomeShelf {
                id: format!("ytm-section:{i}:{}", section.title),
                title: section.title.clone(),
                cards,
            });
        }
        if page.sections.is_empty() {
            shelves.push(HomeShelf {
                id: "ytm-home".into(),
                title: if recommendations {
                    app.t("Dựa trên nhạc bạn nghe và thích", "Based on your listening").into()
                } else {
                    "YouTube Music".into()
                },
                cards: page.items.iter()
                    .filter(|item| home_filter_includes(filter, &item.id))
                    .filter(|item| home_item_visible(&app.data, item, recommendations))
                    .map(|item| CardData::from_ytm(app, item, Some(page)))
                    .collect(),
            });
        }
    }

    if page.is_none() && pending {
        draw_home_skeleton(ui);
        return;
    }
    if page.is_none() && error.is_some() {
        ui.add_space(24.0);
        if let Some(err) = error {
            let ctx = ui.ctx().clone();
            let retry = browser.request.clone().unwrap_or(Request::Home(None));
            draw_error_banner(app, ui, 32.0, err, || {
                browser.send(&ctx, retry, false);
            });
        }
    }

    let cont = page.and_then(|p| p.continuation.clone());
    let load_more_label = app.t("Tải thêm", "Load more");
    let load_more = cont.as_ref().map(|_| (pending, load_more_label));
    if draw_home_page(app, ui, "music-home", &quick, &shelves, load_more) {
        if let Some(token) = cont {
            browser.send(ui.ctx(), Request::Home(Some(token)), true);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Search Page ("Browse all" 16:9 grid + Active 2:3 Top Result / Songs split + Carousels)

pub(super) fn browse_all_grid_layout(grid_width: f32) -> (usize, f32) {
    let min_col = if grid_width < 520.0 {
        180.0
    } else if grid_width < 1280.0 {
        254.0
    } else {
        300.0
    };
    let gap = 16.0;
    let cols = (((grid_width + gap) / (min_col + gap)).floor() as usize).max(1);
    (cols, min_col)
}

struct BrowseCategory {
    vi: &'static str,
    en: &'static str,
    query: &'static str,
    color: Color32,
}

const BROWSE_CATEGORIES: &[BrowseCategory] = &[
    BrowseCategory { vi: "Nhạc", en: "Music", query: "Nhạc Việt", color: Color32::from_rgb(0xdc, 0x14, 0x8c) },
    BrowseCategory { vi: "Bảng xếp hạng", en: "Charts", query: "Top Hits", color: Color32::from_rgb(0x8d, 0x67, 0xab) },
    BrowseCategory { vi: "Mới phát hành", en: "New Releases", query: "Nhạc mới", color: Color32::from_rgb(0xe8, 0x11, 0x5b) },
    BrowseCategory { vi: "V-Pop", en: "V-Pop", query: "V-Pop", color: Color32::from_rgb(0x14, 0x8a, 0x08) },
    BrowseCategory { vi: "Pop", en: "Pop", query: "Pop", color: Color32::from_rgb(0x47, 0x7d, 0x95) },
    BrowseCategory { vi: "Hip-Hop", en: "Hip-Hop", query: "Hip-Hop", color: Color32::from_rgb(0xbc, 0x59, 0x00) },
    BrowseCategory { vi: "K-Pop", en: "K-Pop", query: "K-Pop", color: Color32::from_rgb(0xe6, 0x1e, 0x32) },
    BrowseCategory { vi: "Thư giãn", en: "Chill", query: "Chill", color: Color32::from_rgb(0xd8, 0x40, 0x00) },
    BrowseCategory { vi: "Tâm trạng", en: "Mood", query: "Nhạc buồn", color: Color32::from_rgb(0xe1, 0x11, 0x8c) },
    BrowseCategory { vi: "Indie", en: "Indie", query: "Indie", color: Color32::from_rgb(0xe9, 0x14, 0x29) },
    BrowseCategory { vi: "Rock", en: "Rock", query: "Rock", color: Color32::from_rgb(0x00, 0x64, 0x50) },
    BrowseCategory { vi: "Jazz", en: "Jazz", query: "Jazz", color: Color32::from_rgb(0x1e, 0x32, 0x64) },
    BrowseCategory { vi: "Lo-Fi", en: "Lo-Fi", query: "Lo-fi", color: Color32::from_rgb(0x50, 0x37, 0x50) },
    BrowseCategory { vi: "Tập trung", en: "Focus", query: "Focus", color: Color32::from_rgb(0x53, 0x7a, 0xa1) },
    BrowseCategory { vi: "Acoustic", en: "Acoustic", query: "Acoustic", color: Color32::from_rgb(0xa5, 0x67, 0x52) },
    BrowseCategory { vi: "Điện tử", en: "Electronic", query: "Electronic", color: Color32::from_rgb(0x0d, 0x73, 0xec) },
];

fn draw_search_page(
    app: &mut App,
    ui: &mut Ui,
    browser: &mut Browser,
    page: Option<&Page>,
    pending: bool,
    error: Option<Error>,
    demo_mode: bool,
) -> bool {
    let mut retry_clicked = false;
    let query = app.search.trim().to_string();

    if query.is_empty() {
        draw_browse_all(app, ui);
        return false;
    }

    if crate::ytm::from_url(&query).is_some() {
        app.submit_search();
        return false;
    }

    // Build unified SearchResults (either Demo local search or Live YTM results).
    let demo_items: Vec<Item> = if demo_mode {
        build_demo_search_items(app, &query)
    } else {
        page.map(|p| p.items.clone()).unwrap_or_default()
    };

    let scroll_salt = format!("search:{query}");
    ScrollArea::vertical().id_salt(scroll_salt).auto_shrink([false, false]).show_viewport(ui, |ui, vp| {
        let w = ui.available_width();
        let side = if w < 640.0 { 16.0 } else { 24.0 };
        let inner_w = (w - side * 2.0).max(200.0);
        let scroll_y = vp.min.y.max(0.0);
        let origin = ui.cursor().min;
        let chips_h = 56.0;

        ui.add_space(chips_h);
        let body_clip = Rect::from_min_max(
            Pos2::new(origin.x, origin.y + scroll_y + chips_h),
            Pos2::new(origin.x + w, origin.y + vp.max.y + 8.0),
        );

        ui.scope_builder(UiBuilder::new().id_salt("search-results-body"), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body_clip));

            if pending && demo_items.is_empty() {
                draw_search_skeleton(ui, side);
                return;
            }
            if let Some(err) = error {
                ui.add_space(16.0);
                if draw_error_banner(app, ui, side, err, || {}) {
                    retry_clicked = true;
                }
                return;
            }

            let filter = browser.filter;
            let filtered: Vec<&Item> = demo_items.iter().filter(|it| filter.includes(&it.id)).collect();
            if filtered.is_empty() {
                ui.add_space(24.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
                text(
                    ui.painter(),
                    Pos2::new(r.left() + side, r.center().y),
                    &format!("{} \"{query}\"", app.t("Không tìm thấy kết quả cho", "No results found for")),
                    bold(18.0),
                    TEXT,
                    inner_w,
                    Align2::LEFT_CENTER,
                );
                return;
            }

            if filter == SearchFilter::All {
                let songs: Vec<&Item> = demo_items.iter().filter(|it| matches!(it.id, ItemId::Song(_))).collect();
                let artists: Vec<CardData> = demo_items.iter().filter(|it| matches!(it.id, ItemId::Artist(_))).map(|it| CardData::from_ytm(app, it, page)).collect();
                let albums: Vec<CardData> = demo_items.iter().filter(|it| matches!(it.id, ItemId::Album(_))).map(|it| CardData::from_ytm(app, it, page)).collect();
                let playlists: Vec<CardData> = demo_items.iter().filter(|it| matches!(it.id, ItemId::Playlist(_))).map(|it| CardData::from_ytm(app, it, page)).collect();

                if let Some(top) = demo_items.first() {
                    if search_results_side_by_side(inner_w) && !songs.is_empty() {
                        let gap = 20.0;
                        let top_w = ((inner_w - gap) * 0.40).clamp(200.0, 420.0);
                        let songs_w = (inner_w - gap - top_w).max(200.0);
                        let (split_r, _) = ui.allocate_exact_size(Vec2::new(w, 268.0), Sense::hover());
                        let top_head = Rect::from_min_size(Pos2::new(split_r.left() + side, split_r.top()), Vec2::new(top_w, 36.0));
                        text(ui.painter(), top_head.left_center(), app.t("Kết quả hàng đầu", "Top result"), bold(24.0), TEXT, top_w, Align2::LEFT_CENTER);
                        let top_card_r = Rect::from_min_size(Pos2::new(top_head.left(), top_head.bottom() + 6.0), Vec2::new(top_w, 224.0));
                        draw_top_result_card(app, ui, top_card_r, top, demo_mode, page);

                        let songs_head = Rect::from_min_size(Pos2::new(top_card_r.right() + gap, split_r.top()), Vec2::new(songs_w, 36.0));
                        text(ui.painter(), songs_head.left_center(), app.t("Bài hát", "Songs"), bold(24.0), TEXT, songs_w, Align2::LEFT_CENTER);
                        let songs_box = Rect::from_min_size(Pos2::new(songs_head.left(), songs_head.bottom() + 6.0), Vec2::new(songs_w, 224.0));
                        let mut songs_ui = ui.new_child(UiBuilder::new().id_salt("search-top-songs").max_rect(songs_box));
                        for (i, song) in songs.iter().take(4).enumerate() {
                            let row_r = Rect::from_min_size(Pos2::new(songs_box.left(), songs_box.top() + i as f32 * ROW_H), Vec2::new(songs_w, ROW_H));
                            draw_song_row(app, &mut songs_ui, row_r, i, song, false, demo_mode, page, None);
                        }
                    } else {
                        let (h1, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
                        text(ui.painter(), Pos2::new(h1.left() + side, h1.center().y), app.t("Kết quả hàng đầu", "Top result"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
                        let (cr, _) = ui.allocate_exact_size(Vec2::new(w, 224.0), Sense::hover());
                        draw_top_result_card(app, ui, Rect::from_min_size(Pos2::new(cr.left() + side, cr.top()), Vec2::new(inner_w, 224.0)), top, demo_mode, page);
                        ui.add_space(16.0);
                        if !songs.is_empty() {
                            let (h2, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
                            text(ui.painter(), Pos2::new(h2.left() + side, h2.center().y), app.t("Bài hát", "Songs"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
                            for (i, song) in songs.iter().take(4).enumerate() {
                                let (rr, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
                                let row_r = Rect::from_min_size(Pos2::new(rr.left() + side - 8.0, rr.top()), Vec2::new(inner_w + 16.0, ROW_H));
                                draw_song_row(app, ui, row_r, i, song, false, demo_mode, page, None);
                            }
                        }
                    }
                    ui.add_space(20.0);
                }

                if !artists.is_empty() {
                    draw_carousel_shelf(app, ui, "search-artists", app.t("Nghệ sĩ", "Artists"), &artists, side);
                }
                if !albums.is_empty() {
                    draw_carousel_shelf(app, ui, "search-albums", "Album", &albums, side);
                }
                if !playlists.is_empty() {
                    draw_carousel_shelf(app, ui, "search-playlists", "Playlist", &playlists, side);
                }
            } else if filter == SearchFilter::Songs {
                let header_h = 36.0;
                let (hr, _) = ui.allocate_exact_size(Vec2::new(w, header_h), Sense::hover());
                let cols = Columns::for_width(w, false, false);
                table_header(app, ui, hr, &cols, false, false);
                for (i, item) in filtered.iter().enumerate() {
                    let (rr, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
                    let row_r = Rect::from_min_size(Pos2::new(rr.left() + 16.0, rr.top()), Vec2::new(w - 32.0, ROW_H));
                    if ui.is_rect_visible(row_r) {
                        draw_song_row(app, ui, row_r, i, item, true, demo_mode, page, None);
                    }
                }
            } else {
                let cards: Vec<CardData> = filtered.iter().map(|it| CardData::from_ytm(app, it, page)).collect();
                let gap = 12.0;
                let cols = (((inner_w + gap) / (164.0 + gap)).floor() as usize).clamp(2, 8);
                let card_w = (inner_w - (cols as f32 - 1.0) * gap) / cols as f32;
                let card_h = card_w + 62.0;
                let rows = cards.len().div_ceil(cols);
                let (grid, _) = ui.allocate_exact_size(Vec2::new(w, rows as f32 * (card_h + gap)), Sense::hover());
                for (i, card) in cards.iter().enumerate() {
                    let (c, r) = (i % cols, i / cols);
                    let rect = Rect::from_min_size(
                        Pos2::new(grid.left() + side + c as f32 * (card_w + gap), grid.top() + r as f32 * (card_h + gap)),
                        Vec2::new(card_w, card_h),
                    );
                    if ui.is_rect_visible(rect) {
                        draw_shelf_card(app, ui, rect, card, ("search-grid", i));
                    }
                }
            }
            ui.add_space(24.0);
        });

        // Sticky Search Filter Chips Bar.
        let sticky_r = Rect::from_min_size(Pos2::new(origin.x, origin.y + scroll_y), Vec2::new(w, chips_h));
        ui.painter().rect_filled(sticky_r, 0.0, PANEL);
        if scroll_y > 8.0 {
            ui.painter().hline(sticky_r.x_range(), sticky_r.bottom(), Stroke::new(1.0, HAIRLINE));
        }
        let mut chips_ui = ui.new_child(UiBuilder::new().id_salt("search-sticky-chips").max_rect(sticky_r));
        let mut cx = sticky_r.left() + side;
        let cy = sticky_r.center().y - 16.0;
        for (filter, label) in [
            (SearchFilter::All, app.t("Tất cả", "All")),
            (SearchFilter::Songs, app.t("Bài hát", "Songs")),
            (SearchFilter::Albums, "Album"),
            (SearchFilter::Artists, app.t("Nghệ sĩ", "Artists")),
            (SearchFilter::Playlists, "Playlist"),
        ] {
            let pw = pill_width(&chips_ui, label);
            if cx + pw > sticky_r.right() - side + 4.0 {
                break;
            }
            let pr = Rect::from_min_size(Pos2::new(cx, cy), Vec2::new(pw, 32.0));
            if pill(&mut chips_ui, pr, ("search-chip", label), label, browser.filter == filter).clicked() {
                browser.filter = filter;
            }
            cx += pw + 8.0;
        }
    });

    retry_clicked
}

fn draw_browse_all(app: &mut App, ui: &mut Ui) {
    ScrollArea::vertical().id_salt("search-browse-all").auto_shrink([false, false]).show(ui, |ui| {
        let w = ui.available_width();
        let side = if w < 640.0 { 16.0 } else { 24.0 };
        let inner_w = (w - side * 2.0).max(200.0);

        ui.add_space(16.0);

        // Recent searches removable chips.
        if !app.data.recent_searches.is_empty() {
            let (head_r, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
            let clear_lbl = app.t("Xóa tất cả", "Clear all");
            let clear_w = text_width(ui, clear_lbl, semi(13.0)) + 8.0;
            text(ui.painter(), Pos2::new(head_r.left() + side, head_r.center().y), app.t("Tìm kiếm gần đây", "Recent searches"), bold(24.0), TEXT, inner_w - clear_w - 12.0, Align2::LEFT_CENTER);
            let clear_rect = Rect::from_min_size(Pos2::new(head_r.right() - side - clear_w, head_r.center().y - 12.0), Vec2::new(clear_w, 24.0));
            let c_resp = ui.interact(clear_rect, ui.scope_id().with("clear-recent-searches"), Sense::click());
            text(ui.painter(), clear_rect.right_center(), clear_lbl, semi(13.0), if c_resp.hovered() { TEXT } else { SUBTLE }, clear_w, Align2::RIGHT_CENTER);
            if c_resp.clicked() {
                app.actions.push(Action::ClearRecentSearches);
            }

            ui.add_space(8.0);
            let mut x = side;
            let mut row_items: Vec<(String, f32)> = Vec::new();
            let recent = app.data.recent_searches.clone();
            let mut flush_row = |ui: &mut Ui, items: &[(String, f32)]| {
                let (rr, _) = ui.allocate_exact_size(Vec2::new(w, 38.0), Sense::hover());
                let mut rx = rr.left() + side;
                for (q, cw) in items {
                    let chip_r = Rect::from_min_size(Pos2::new(rx, rr.top()), Vec2::new(*cw, 32.0));
                    let resp = ui.interact(chip_r, ui.scope_id().with(("recent-q", q)), Sense::click());
                    ui.painter().rect_filled(chip_r, 16.0, if resp.hovered() { CARD_HOVER } else { ELEVATED });
                    icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(chip_r.left() + 16.0, chip_r.center().y), Vec2::splat(14.0)), Icon::Search, SUBTLE);
                    text(ui.painter(), Pos2::new(chip_r.left() + 30.0, chip_r.center().y), q, body(13.0), TEXT, cw - 54.0, Align2::LEFT_CENTER);
                    let x_btn = Rect::from_center_size(Pos2::new(chip_r.right() - 15.0, chip_r.center().y), Vec2::splat(20.0));
                    let x_resp = icon_button(ui, x_btn, ("rm-recent-q", q), Icon::Close, SUBTLE, app.t("Xóa", "Remove"));
                    if x_resp.clicked() {
                        app.actions.push(Action::RemoveRecentSearch(q.clone()));
                    } else if resp.clicked() {
                        app.actions.push(Action::SelectRecentSearch(q.clone()));
                    }
                    rx += cw + 8.0;
                }
            };
            for q in recent {
                let cw = (text_width(ui, &q, body(13.0)) + 64.0).clamp(90.0, inner_w);
                if x + cw > w - side && !row_items.is_empty() {
                    flush_row(ui, &row_items);
                    row_items.clear();
                    x = side;
                }
                row_items.push((q, cw));
                x += cw + 8.0;
            }
            if !row_items.is_empty() {
                flush_row(ui, &row_items);
            }
            ui.add_space(16.0);
        }

        // Browse All 16:9 Category Grid.
        let (h_r, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
        text(ui.painter(), Pos2::new(h_r.left() + side, h_r.center().y), app.t("Duyệt tìm tất cả", "Browse all"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
        ui.add_space(8.0);

        let (cols, _) = browse_all_grid_layout(inner_w);
        let gap = 16.0;
        let tile_w = (inner_w - (cols as f32 - 1.0) * gap) / cols as f32;
        let tile_h = (tile_w * 9.0 / 16.0).clamp(108.0, 184.0);
        let rows = BROWSE_CATEGORIES.len().div_ceil(cols);
        let total_h = rows as f32 * tile_h + (rows.saturating_sub(1)) as f32 * gap;
        let (grid, _) = ui.allocate_exact_size(Vec2::new(w, total_h), Sense::hover());
        let title_font_sz = if inner_w <= 520.0 { 16.0 } else { 22.0 };

        for (i, cat) in BROWSE_CATEGORIES.iter().enumerate() {
            let (c, r) = (i % cols, i / cols);
            let rect = Rect::from_min_size(
                Pos2::new(grid.left() + side + c as f32 * (tile_w + gap), grid.top() + r as f32 * (tile_h + gap)),
                Vec2::new(tile_w, tile_h),
            );
            if !ui.is_rect_visible(rect) {
                continue;
            }
            let resp = ui.interact(rect, ui.scope_id().with(("browse-cat", i)), Sense::click());
            let fill = if resp.hovered() { cat.color.lerp_to_gamma(TEXT, 0.08) } else { cat.color };
            ui.painter().rect_filled(rect, RADIUS_CARD, fill);

            // Bottom-right 25deg tilted cover artwork clipped cleanly inside the 8px rounded card.
            let tex = app.art.get(ui.ctx(), &format!("cat:{}", cat.query), None);
            draw_tilted_art(ui, rect, &tex);

            let label = app.t(cat.vi, cat.en);
            wrapped_text(
                ui.painter(),
                Pos2::new(rect.left() + 16.0, rect.top() + 16.0),
                label,
                bold(title_font_sz),
                TEXT,
                (tile_w * 0.64).max(60.0),
                3,
                Align2::LEFT_TOP,
            );

            if resp.clicked() {
                app.actions.push(Action::SearchCategory(cat.query.to_string()));
            }
        }
        ui.add_space(28.0);
    });
}

fn draw_tilted_art(ui: &Ui, card: Rect, tex: &egui::TextureHandle) {
    let side = (card.height() * 0.68).clamp(54.0, 112.0);
    let center = Pos2::new(card.right() - side * 0.30, card.bottom() - side * 0.28);
    let angle = 25.0_f32.to_radians();
    let (sin, cos) = angle.sin_cos();
    let rotate = |dx: f32, dy: f32| Pos2::new(center.x + dx * cos - dy * sin, center.y + dx * sin + dy * cos);
    let h = side * 0.5;
    let corners = [rotate(-h, -h), rotate(h, -h), rotate(h, h), rotate(-h, h)];
    let shadow_corners: Vec<Pos2> = corners.iter().map(|p| *p + Vec2::new(0.0, 3.0)).collect();

    let clipped = ui.painter().with_clip_rect(card);
    let mut sh = egui::epaint::Mesh::default();
    for p in shadow_corners {
        sh.colored_vertex(p, Color32::from_black_alpha(75));
    }
    sh.add_triangle(0, 1, 2);
    sh.add_triangle(0, 2, 3);
    clipped.add(egui::Shape::mesh(sh));

    let mut mesh = egui::epaint::Mesh::with_texture(tex.id());
    let uvs = [Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0)];
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex { pos, uv, color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    clipped.add(egui::Shape::mesh(mesh));
}

fn build_demo_search_items(app: &App, query: &str) -> Vec<Item> {
    let q = fold(query);
    let mut out = Vec::new();
    for &id in &app.lib.order {
        if let Some(t) = app.lib.track(id) {
            let genre = t.genre.as_deref().unwrap_or("");
            if fold(&t.title).contains(&q) || fold(&t.artist).contains(&q) || fold(&t.album).contains(&q) || fold(genre).contains(&q) {
                out.push(Item {
                    id: ItemId::Song(format!("demo_song_{:011}", t.id % 100_000_000_000)),
                    title: t.title.clone(),
                    by: t.artist.clone(),
                    image: None,
                    duration_ms: t.duration_ms,
                    album: t.album.clone(),
                });
            }
        }
    }
    for a in &app.lib.artists {
        if fold(&a.name).contains(&q) {
            out.push(Item {
                id: ItemId::Artist(a.name.clone()),
                title: a.name.clone(),
                by: String::new(),
                image: a.cover_path.clone(),
                duration_ms: 0,
                album: String::new(),
            });
        }
    }
    for al in &app.lib.albums {
        if fold(&al.title).contains(&q) || fold(&al.artist).contains(&q) {
            out.push(Item {
                id: ItemId::Album(al.key.clone()),
                title: al.title.clone(),
                by: al.artist.clone(),
                image: al.cover_path.clone(),
                duration_ms: 0,
                album: String::new(),
            });
        }
    }
    for p in &app.data.playlists {
        if fold(&p.name).contains(&q) || fold(&p.description).contains(&q) {
            out.push(Item {
                id: ItemId::Playlist(p.id.to_string()),
                title: p.name.clone(),
                by: app.t("Bạn", "You").to_string(),
                image: p.image.clone(),
                duration_ms: 0,
                album: String::new(),
            });
        }
    }
    out
}

fn demo_navigate_or_play(app: &mut App, item: &Item, play: bool) {
    match &item.id {
        ItemId::Artist(name) => app.actions.push(Action::Navigate(Route::Artist(name.clone()))),
        ItemId::Album(key) => {
            if play {
                app.actions.push(Action::PlayItem(RecentItem::Album(key.clone())));
            } else {
                app.actions.push(Action::Navigate(Route::Album(key.clone())));
            }
        }
        ItemId::Playlist(id_str) => {
            if let Ok(pid) = id_str.parse::<u64>() {
                if play {
                    app.actions.push(Action::PlayItem(RecentItem::Playlist(pid)));
                } else {
                    app.actions.push(Action::Navigate(Route::Playlist(pid)));
                }
            }
        }
        ItemId::Song(_) => {
            if let Some(tid) = app.lib.order.iter().copied().find(|&id| app.lib.track(id).is_some_and(|t| t.title == item.title)) {
                if play {
                    app.actions.push(Action::PlayContext {
                        context: ContextRef::All,
                        name: item.title.clone(),
                        tracks: vec![tid],
                        start: Some(0),
                    });
                } else if let Some(t) = app.lib.track(tid) {
                    app.actions.push(Action::Navigate(Route::Album(t.album_key())));
                }
            }
        }
    }
}

fn draw_top_result_card(app: &mut App, ui: &mut Ui, r: Rect, item: &Item, demo_mode: bool, page: Option<&Page>) {
    let resp = ui.interact(r, ui.scope_id().with("search-top-result"), Sense::click());
    let hovered = ui.rect_contains_pointer(r) || resp.has_focus();
    focus_ring(ui, &resp, 4.0);
    ui.painter().rect_filled(r, RADIUS_CARD, if hovered { CARD_HOVER } else { CARD });

    let cover_r = Rect::from_min_size(r.min + Vec2::splat(20.0), Vec2::splat(92.0));
    let sc = ui.ctx().animate_value_with_time(
        ui.scope_id().with("top-res-cov-sc"),
        if hovered { 1.03 } else { 1.0 },
        0.18,
    );
    let draw_cov = Rect::from_center_size(cover_r.center(), cover_r.size() * sc);
    let round = if matches!(item.id, ItemId::Artist(_)) { draw_cov.width() * 0.5 } else { 6.0 };
    let key = top_result_cover_key(app, item, demo_mode);
    let tex = app.art.get(ui.ctx(), &key, item.image.as_deref());
    cover_shadow(ui.painter(), draw_cov, round);
    cover(ui, draw_cov, &tex, round);

    let room = (r.width() - 96.0).max(60.0);
    text(ui.painter(), Pos2::new(r.left() + 20.0, cover_r.bottom() + 20.0), &item.title, bold(26.0), TEXT, room, Align2::LEFT_TOP);

    let mut sx = r.left() + 20.0;
    let sy = r.bottom() - 30.0;
    let kind_lbl = ytm_kind_label(app, item, page);
    let badge_w = text_width(ui, kind_lbl, semi(12.0)) + 16.0;
    let badge_r = Rect::from_min_size(Pos2::new(sx, sy - 11.0), Vec2::new(badge_w, 22.0));
    ui.painter().rect_filled(badge_r, 11.0, hover_white(24));
    text(ui.painter(), badge_r.center(), kind_lbl, semi(12.0), TEXT, badge_w, Align2::CENTER_CENTER);
    sx = badge_r.right() + 8.0;

    let mut artist_clicked = false;
    if !item.by.is_empty() {
        let dot = text(ui.painter(), Pos2::new(sx, sy), "•", body(13.5), SUBTLE, 14.0, Align2::LEFT_CENTER);
        let rem = (r.right() - dot.right() - 76.0).max(20.0);
        artist_clicked = paint_artist_links(app, ui, Pos2::new(dot.right() + 6.0, sy), &item.by, rem, "top-res-artist");
    }

    let play_r = Rect::from_center_size(r.right_bottom() - Vec2::splat(44.0), Vec2::splat(48.0));
    let is_song = matches!(item.id, ItemId::Song(_));
    let is_playing = match &item.id {
        ItemId::Song(vid) => app.player.current == Some(crate::library::fnv(&format!("yt:{vid}"))) && app.is_playing(),
        _ => false,
    };
    let tip = if is_playing { app.t("Tạm dừng", "Pause") } else { app.t("Phát", "Play") };
    let play_clicked = hover_play_button(ui, play_r, "search-top-play-btn", is_playing, hovered || is_playing, tip).clicked();
    if play_clicked {
        if demo_mode {
            demo_navigate_or_play(app, item, true);
        } else if is_song {
            app.actions.push(Action::SelectYouTubeSong(item.clone()));
        } else {
            navigate(app, item);
        }
    } else if resp.clicked() && !artist_clicked {
        if demo_mode {
            demo_navigate_or_play(app, item, false);
        } else if is_song {
            app.actions.push(Action::SelectYouTubeSong(item.clone()));
        } else {
            navigate(app, item);
        }
    }
    if !demo_mode {
        resp.context_menu(|ui| item_menu(app, ui, item));
    }
}

fn top_result_cover_key(app: &App, item: &Item, demo_mode: bool) -> String {
    if demo_mode {
        if let ItemId::Album(k) = &item.id {
            return k.clone();
        }
        if let ItemId::Artist(n) = &item.id {
            if let Some(a) = app.lib.artist(n) {
                return a.cover_key.clone();
            }
        }
        if let Some(t) = app.lib.order.iter().filter_map(|id| app.lib.track(*id)).find(|t| t.title == item.title) {
            return t.cover_key();
        }
    }
    item.cover_key()
}

// ---------------------------------------------------------------------------------------------
// YTM Entity Pages (Album, Playlist, Artist, Song) with Full-Bleed Header & Spotify Action Bar

pub(super) fn draw_ytm_artist_page(app: &mut App, ui: &mut Ui, name: &str, page: &Page) {
    let mut browser = std::mem::take(&mut app.ytm);
    let item = Item {
        id: ItemId::Artist(format!("name:{name}")),
        title: if page.title.is_empty() { name.to_string() } else { page.title.clone() },
        by: page.subtitle.clone().unwrap_or_default(),
        image: page.image.clone(),
        duration_ms: 0,
        album: String::new(),
    };
    draw_ytm_entity(app, ui, &mut browser, &item, Some(page), false, None, None, None);
    app.ytm = browser;
}

pub(super) fn draw_ytm_album_page(
    app: &mut App,
    ui: &mut Ui,
    browse_id: &str,
    fallback_title: &str,
    fallback_artist: &str,
    page: &Page,
) {
    let mut browser = std::mem::take(&mut app.ytm);
    let item = Item {
        id: ItemId::Album(browse_id.to_string()),
        title: if page.title.is_empty() {
            fallback_title.to_string()
        } else {
            page.title.clone()
        },
        by: page
            .subtitle
            .clone()
            .unwrap_or_else(|| fallback_artist.to_string()),
        image: page.image.clone(),
        duration_ms: 0,
        album: String::new(),
    };
    let follow_id = format!("album:{browse_id}");
    draw_ytm_entity(
        app,
        ui,
        &mut browser,
        &item,
        Some(page),
        false,
        None,
        Some(follow_id.as_str()),
        None,
    );
    app.ytm = browser;
}

#[allow(clippy::too_many_arguments)]
fn draw_ytm_entity(
    app: &mut App,
    ui: &mut Ui,
    browser: &mut Browser,
    item: &Item,
    page: Option<&Page>,
    pending: bool,
    error: Option<Error>,
    follow_id: Option<&str>,
    request: Option<&Request>,
) {
    let title = page
        .filter(|p| !p.title.is_empty())
        .map_or(item.title.as_str(), |p| p.title.as_str())
        .to_string();
    let scroll_id = match request {
        Some(Request::Open(id)) => id.url(),
        Some(Request::Related(id)) => format!("related:{id}"),
        _ => item.id.url(),
    };
    let is_artist = matches!(item.id, ItemId::Artist(_));
    let is_song = matches!(item.id, ItemId::Song(_));
    let source = page.and_then(|p| p.image.as_deref()).or(item.image.as_deref());
    // Persist real artist portrait URLs so Left Sidebar & Profile never fall back to shared track covers.
    if is_artist {
        if let Some(url) = source {
            if app.data.record_artist_avatar(&title, url) {
                app.dirty = true;
            }
        }
        if let Some(p) = page {
            for rel in &p.items {
                if matches!(rel.id, ItemId::Artist(_)) {
                    if let Some(url) = rel.image.as_deref() {
                        if app.data.record_artist_avatar(&rel.title, url) {
                            app.dirty = true;
                        }
                    }
                }
            }
        }
    } else if let Some(p) = page {
        if let Some(art_img) = p.artist_image.as_deref() {
            let by = p.subtitle.as_deref().unwrap_or(&item.by);
            if let Some(primary) = crate::library::split_artists(by).into_iter().next() {
                if app.data.record_artist_avatar(&primary, art_img) {
                    app.dirty = true;
                }
            }
        }
    }
    let cover_key = source.map_or_else(|| item.id.url(), crate::art::cover_key);
    let hero_color = adaptive_hero_color(app, &cover_key, false);

    let items_raw: Vec<Item> = if let Some(p) = page {
        p.items.clone()
    } else if let Some(snap) = follow_id.and_then(|id| app.data.followed_ytm.iter().find(|l| l.id == id)).map(|l| l.snapshot.clone()) {
        snap.items
    } else {
        Vec::new()
    };

    let filtered_items = if !is_artist && !is_song {
        filter_and_sort_ytm_items(app, &items_raw)
    } else {
        items_raw.clone()
    };
    let row_h = if !is_artist && !is_song && app.data.settings.compact_tracklist { 32.0 } else { ROW_H };

    let see_more_id = ui.scope_id().with(("ytm-artist-see-more", &item.id));
    let disco_chip_id = ui.scope_id().with(("ytm-artist-disco-chip", &item.id));
    let disco_view_id = ui.scope_id().with(("ytm-artist-disco-view", &item.id));
    let about_id = ui.scope_id().with(("ytm-artist-about", &item.id));
    let mut see_more = ui.ctx().data(|d| d.get_temp::<bool>(see_more_id)).unwrap_or(false);
    let mut disco_chip = ui.ctx().data(|d| d.get_temp::<u8>(disco_chip_id)).unwrap_or(0);
    let mut disco_list_mode = ui.ctx().data(|d| d.get_temp::<bool>(disco_view_id)).unwrap_or(false);
    let mut about_open = ui.ctx().data(|d| d.get_temp::<bool>(about_id)).unwrap_or(false);

    let mut retry_clicked = false;
    let mut follow_toggle: Option<(String, Option<Page>)> = None;

    ScrollArea::vertical().id_salt(scroll_id).auto_shrink([false, false]).show_viewport(ui, |ui, vp| {
        let w = ui.available_width();
        let side = if w < 640.0 { 16.0 } else { 24.0 };
        let inner_w = (w - side * 2.0).max(200.0);
        let scroll_y = vp.min.y.max(0.0);
        let origin = ui.cursor().min;
        let hero_h = if is_artist && source.is_some() {
            (w * 0.42).clamp(260.0, 340.0)
        } else {
            hero_height_for_width(w)
        };
        let actions_h = 84.0;
        let header_h = if !is_artist && !is_song { 36.0 } else { 0.0 };

        // Edge-to-edge full-bleed hero gradient flowing cleanly below the artist banner without a hard seam.
        let grad_top = if is_artist && source.is_some() {
            hero_color.linear_multiply(0.55)
        } else {
            hero_color
        };
        draw_full_bleed_gradient(
            ui.painter(),
            Rect::from_min_size(origin, Vec2::new(w, hero_h + 200.0)),
            grad_top,
            PANEL,
        );

        let mini_h = 64.0;
        let mini_fade = ((scroll_y - (hero_h + 16.0)) / 24.0).clamp(0.0, 1.0);
        let header_rest_y = hero_h + actions_h;
        let header_stuck = header_h > 0.0 && scroll_y + mini_h > header_rest_y;
        let top_guard = if header_stuck {
            scroll_y + mini_h + header_h
        } else if mini_fade > 0.05 {
            scroll_y + mini_h
        } else {
            0.0
        };
        let body_clip = Rect::from_min_max(
            Pos2::new(origin.x, origin.y + top_guard),
            Pos2::new(origin.x + w, origin.y + vp.max.y + 8.0),
        );

        let cols = Columns::for_width(w, matches!(item.id, ItemId::Playlist(_)), false);

        ui.scope_builder(UiBuilder::new().id_salt("ytm-entity-body"), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body_clip));

            // Hero Header.
            let (hero_r, _) = ui.allocate_exact_size(Vec2::new(w, hero_h), Sense::hover());
            let tex = app.art.get(ui.ctx(), &cover_key, source);
            if is_artist && source.is_some() {
                let size = tex.size();
                let mut uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                if size[0] > 0 && size[1] > 0 && hero_r.height() > 1.0 {
                    let img_aspect = size[0] as f32 / size[1] as f32;
                    let box_aspect = hero_r.width() / hero_r.height();
                    if box_aspect > img_aspect {
                        let visible_h = (img_aspect / box_aspect).clamp(0.15, 1.0);
                        let top_uv = (1.0 - visible_h) * 0.25;
                        uv = Rect::from_min_max(Pos2::new(0.0, top_uv), Pos2::new(1.0, (top_uv + visible_h).min(1.0)));
                    } else {
                        let visible_w = (box_aspect / img_aspect).clamp(0.15, 1.0);
                        let left_uv = (1.0 - visible_w) * 0.5;
                        uv = Rect::from_min_max(Pos2::new(left_uv, 0.0), Pos2::new(left_uv + visible_w, 1.0));
                    }
                }
                ui.painter().image(tex.id(), hero_r, uv, Color32::WHITE);
                // Smooth multi-stop cinematic scrim: subtle top edge, deeper bottom vignette seamlessly matching the action bar tint.
                let mid_y = hero_r.top() + hero_r.height() * 0.42;
                vertical_gradient(
                    ui.painter(),
                    Rect::from_min_max(hero_r.min, Pos2::new(hero_r.right(), mid_y)),
                    Color32::from_black_alpha(24),
                    Color32::from_black_alpha(48),
                );
                vertical_gradient(
                    ui.painter(),
                    Rect::from_min_max(Pos2::new(hero_r.left(), mid_y), hero_r.max),
                    Color32::from_black_alpha(48),
                    Color32::from_black_alpha(210),
                );
                // Spotify Desktop 1.3.9 Artist Hero hierarchy:
                // 1) Huge ExtraBold Artist Name -> 2) Verified badge BELOW name -> 3) Localized listener count BELOW badge.
                let tx = hero_r.left() + side;
                let room = (hero_r.width() - side * 2.0).max(80.0);
                let meta_y = hero_r.bottom() - 22.0;
                let badge_y = meta_y - 28.0;
                let title_bottom = badge_y - 18.0;
                let title_avail_h = (title_bottom - (hero_r.top() + 28.0)).clamp(48.0, 108.0);
                let title_cy = title_bottom - title_avail_h * 0.44;
                paint_fitted_hero_title(
                    ui.painter(),
                    Pos2::new(tx, title_cy),
                    &title,
                    room,
                    title_avail_h,
                );
                let badge = Rect::from_center_size(Pos2::new(tx + 10.0, badge_y), Vec2::splat(20.0));
                icons::paint(ui.painter(), badge, Icon::VerifiedBadge, Color32::from_rgb(0x84, 0xf5, 0x9f));
                text(
                    ui.painter(),
                    Pos2::new(tx + 26.0, badge_y),
                    app.t("Nghệ sĩ đã xác minh", "Verified Artist"),
                    bold(14.0),
                    TEXT,
                    room - 26.0,
                    Align2::LEFT_CENTER,
                );
                if let Some(raw_sub) = page.and_then(|p| p.subtitle.as_deref()).filter(|s| !s.is_empty()) {
                    let sub_fmt = format_artist_subtitle(app.lang(), raw_sub);
                    text(ui.painter(), Pos2::new(tx, meta_y), &sub_fmt, body(15.0), TEXT, room, Align2::LEFT_CENTER);
                }
            } else {
                let art_side = hero_art_side(w);
                let cov_r = Rect::from_min_size(Pos2::new(hero_r.left() + side, hero_r.bottom() - art_side - 18.0), Vec2::splat(art_side));
                let cov_hov = ui.rect_contains_pointer(cov_r);
                let cov_sc = ui.ctx().animate_value_with_time(
                    ui.scope_id().with(("ytm-hero-cov-sc", &cover_key)),
                    if cov_hov { 1.028 } else { 1.0 },
                    0.18,
                );
                let cov_draw = Rect::from_center_size(cov_r.center(), cov_r.size() * cov_sc);
                let rad = if is_artist { cov_draw.width() * 0.5 } else { 6.0 };
                cover_shadow(ui.painter(), cov_draw, rad);
                cover(ui, cov_draw, &tex, rad);

                let tx = cov_r.right() + 24.0;
                let room = (hero_r.right() - tx - side).max(80.0);
                let top_y = cov_r.top() + (art_side * 0.06).clamp(4.0, 12.0) + 6.0;
                let meta_y = cov_r.bottom() - 10.0;
                let title_top = top_y + 12.0;
                let title_bottom = meta_y - 14.0;
                let title_avail_h = (title_bottom - title_top).max(36.0);
                let is_ytm_album = matches!(item.id, ItemId::Album(_))
                    || matches!(&item.id, ItemId::Playlist(pid) if pid.starts_with("OLAK5uy_"));
                if is_artist {
                    let badge = Rect::from_center_size(Pos2::new(tx + 10.0, top_y), Vec2::splat(20.0));
                    icons::paint(ui.painter(), badge, Icon::VerifiedBadge, Color32::from_rgb(0x3d, 0x91, 0xf4));
                    text(ui.painter(), Pos2::new(tx + 26.0, top_y), app.t("Nghệ sĩ đã xác minh", "Verified Artist"), semi(13.5), TEXT, room - 26.0, Align2::LEFT_CENTER);
                } else {
                    let explicit_kind = page.and_then(|p| p.release_kind(&item.id));
                    let kind_txt = match explicit_kind {
                        Some(ReleaseKind::Album) => "Album",
                        Some(ReleaseKind::EP) => "EP",
                        Some(ReleaseKind::Single) => app.t("Đĩa đơn", "Single"),
                        _ if is_ytm_album && !items_raw.is_empty() && items_raw.len() <= 3 => app.t("Đĩa đơn", "Single"),
                        _ if is_ytm_album && !items_raw.is_empty() && items_raw.len() <= 6 => "EP",
                        _ if is_ytm_album => "Album",
                        _ => ytm_kind_label(app, item, page),
                    };
                    text(ui.painter(), Pos2::new(tx, top_y), kind_txt, semi(13.5), TEXT, room, Align2::LEFT_CENTER);
                }
                paint_fitted_hero_title(ui.painter(), Pos2::new(tx, (title_top + title_bottom) * 0.5), &title, room, title_avail_h);

                let total_ms: u64 = items_raw.iter().map(|it| it.duration_ms).sum();
                let mut mx = tx;
                if is_artist {
                    if let Some(sub) = page.and_then(|p| p.subtitle.as_deref()).filter(|s| !s.is_empty()) {
                        text(ui.painter(), Pos2::new(mx, meta_y), sub, body(13.5), SUBTLE, room, Align2::LEFT_CENTER);
                    }
                } else {
                    let by = page.and_then(|p| p.subtitle.as_deref()).unwrap_or(&item.by);
                    if !by.is_empty() {
                        let primary_artist = crate::library::split_artists(by)
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| by.to_string());
                        if is_ytm_album {
                            let av_r = Rect::from_center_size(Pos2::new(mx + 12.0, meta_y), Vec2::splat(24.0));
                            let art_img = page.and_then(|p| p.artist_image.as_deref()).or(source);
                            let av_key = page
                                .and_then(|p| p.artist_image.as_deref())
                                .map_or_else(|| cover_key.clone(), crate::art::cover_key);
                            let av_tex = app.art.get(ui.ctx(), &av_key, art_img);
                            cover(ui, av_r, &av_tex, 12.0);
                            mx = av_r.right() + 8.0;
                        }
                        let by_r = text(
                            ui.painter(),
                            Pos2::new(mx, meta_y),
                            by,
                            bold(14.0),
                            TEXT,
                            (hero_r.right() - side - mx).max(40.0),
                            Align2::LEFT_CENTER,
                        );
                        if !primary_artist.trim().is_empty() && link(ui, by_r, "ytm-hero-artist-link") {
                            app.actions.push(Action::Navigate(Route::Artist(primary_artist)));
                        }
                        mx = by_r.right() + 6.0;
                    }
                    if let Some(yr) = page.and_then(|p| p.year.as_deref()).filter(|s| !s.is_empty()) {
                        if mx > tx {
                            let d_r = text(ui.painter(), Pos2::new(mx, meta_y), "•", body(13.5), SUBTLE, 14.0, Align2::LEFT_CENTER);
                            mx = d_r.right() + 6.0;
                        }
                        let yr_r = text(ui.painter(), Pos2::new(mx, meta_y), yr, body(13.5), SUBTLE, 64.0, Align2::LEFT_CENTER);
                        mx = yr_r.right() + 6.0;
                    }
                    if !items_raw.is_empty() {
                        if mx > tx {
                            let d_r = text(ui.painter(), Pos2::new(mx, meta_y), "•", body(13.5), SUBTLE, 14.0, Align2::LEFT_CENTER);
                            mx = d_r.right() + 6.0;
                        }
                        let dur_part = if total_ms > 0 {
                            format!(", {}", album_duration(app.lang(), total_ms))
                        } else {
                            String::new()
                        };
                        let tail = format!("{} {}{dur_part}", items_raw.len(), app.t("bài hát", "songs"));
                        text(ui.painter(), Pos2::new(mx, meta_y), &tail, body(13.5), SUBTLE, (hero_r.right() - side - mx).max(20.0), Align2::LEFT_CENTER);
                    }
                }
            }

            // Action Bar.
            let (act_r, _) = ui.allocate_exact_size(Vec2::new(w, actions_h), Sense::hover());
            let cy = act_r.center().y;
            let play = Rect::from_center_size(Pos2::new(act_r.left() + side + 28.0, cy), Vec2::splat(56.0));
            let songs_list: Vec<Item> = items_raw.iter().filter(|it| matches!(it.id, ItemId::Song(_))).cloned().collect();
            let list_is_current = app.player.current.is_some()
                && !songs_list.is_empty()
                && songs_list.iter().any(|s| match &s.id {
                    ItemId::Song(vid) => app.player.current == Some(crate::library::fnv(&format!("yt:{vid}"))),
                    _ => false,
                });
            let list_playing = list_is_current && app.is_playing();
            let (p_ic, p_tip) = if list_playing {
                (Icon::Pause, app.t("Tạm dừng", "Pause"))
            } else {
                (Icon::Play, app.t("Phát", "Play"))
            };
            if round_button(ui, play, "ytm-page-play", p_ic, ACCENT, Color32::BLACK, p_tip).clicked() {
                if list_is_current {
                    app.actions.push(Action::TogglePlay);
                } else if !songs_list.is_empty() {
                    app.actions.push(Action::PlayYouTubeList {
                        name: title.clone(),
                        items: songs_list.clone(),
                        start: 0,
                    });
                } else if matches!(item.id, ItemId::Song(_)) {
                    app.actions.push(Action::SelectYouTubeSong(item.clone()));
                }
            }
            let is_ytm_album = matches!(item.id, ItemId::Album(_));
            let mut ax = play.right() + 20.0;
            // Spotify Desktop vertical 34x44 preview pill on both Artist and Album pages.
            let show_preview_pill = is_ytm_album || (is_artist && !songs_list.is_empty());
            if show_preview_pill {
                let prev_r = Rect::from_center_size(Pos2::new(ax + 17.0, cy), Vec2::new(34.0, 44.0));
                let prev_tip = if is_artist {
                    app.t(
                        "Cuộn qua để xem thử các bản nhạc của nghệ sĩ này",
                        "Scroll to preview this artist's tracks",
                    )
                } else {
                    app.t("Xem thử các bản nhạc trong album này", "Preview tracks from this album")
                };
                let prev_resp = ui
                    .interact(prev_r, ui.scope_id().with("ytm-preview-pill"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(prev_tip);
                let prev_hov = prev_resp.hovered();
                let prev_sc = if app.data.settings.prefers_reduced_motion {
                    1.0
                } else {
                    ui.ctx().animate_value_with_time(
                        ui.scope_id().with("ytm-album-prev-sc"),
                        if prev_hov { 1.05 } else { 1.0 },
                        0.16,
                    )
                };
                let draw_prev = Rect::from_center_size(prev_r.center(), prev_r.size() * prev_sc);
                let preview_src = if is_artist {
                    songs_list.first().and_then(|s| s.image.as_deref()).or(source)
                } else {
                    source
                };
                let preview_key = preview_src.map_or_else(|| cover_key.clone(), crate::art::cover_key);
                let prev_tex = app.art.get(ui.ctx(), &preview_key, preview_src);
                ui.painter().rect_filled(draw_prev, 8.0, Color32::from_black_alpha(150));
                cover(ui, draw_prev.shrink(3.0), &prev_tex, 5.0);
                if prev_hov {
                    ui.painter().rect_filled(draw_prev.shrink(3.0), 5.0, Color32::from_black_alpha(95));
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(draw_prev.center() + Vec2::new(1.0, 0.0), Vec2::splat(14.0)),
                        Icon::Play,
                        TEXT,
                    );
                }
                ui.painter().rect_stroke(
                    draw_prev,
                    8.0,
                    Stroke::new(1.5, if prev_hov { TEXT } else { Color32::from_white_alpha(165) }),
                    egui::StrokeKind::Inside,
                );
                if prev_resp.clicked() && !songs_list.is_empty() {
                    let preview_items: Vec<crate::app::PreviewTrack> = songs_list
                        .iter()
                        .take(20)
                        .map(|s| {
                            let c_src = s.image.clone().or_else(|| source.map(str::to_string));
                            let c_key = c_src.as_deref().map_or_else(|| cover_key.clone(), crate::art::cover_key);
                            let url = match &s.id {
                                ItemId::Song(vid) => Some(format!("https://music.youtube.com/watch?v={vid}")),
                                _ => None,
                            };
                            crate::app::PreviewTrack {
                                title: s.title.clone(),
                                artist: if s.by.trim().is_empty() { title.clone() } else { s.by.clone() },
                                cover_key: c_key,
                                cover_path: c_src,
                                ytm_item: Some(s.clone()),
                                local_id: None,
                                duration_ms: s.duration_ms,
                                url,
                            }
                        })
                        .collect();
                    let short_stat = page
                        .and_then(|p| p.subtitle.as_deref())
                        .and_then(parse_compact_count)
                        .map(|n| format_compact_count(app.lang(), n));
                    let primary_artist = if is_artist {
                        title.clone()
                    } else {
                        page.and_then(|p| p.subtitle.as_deref())
                            .unwrap_or(&item.by)
                            .split('•')
                            .next()
                            .unwrap_or(&title)
                            .trim()
                            .to_string()
                    };
                    let mut tags = Vec::new();
                    if app.lang() == crate::library::Lang::Vi {
                        tags.push("#nhạc việt".to_string());
                        tags.push(if is_artist { "#nghệ sĩ".to_string() } else { "#album".to_string() });
                        tags.push("#khám phá".to_string());
                    } else {
                        tags.push("#pop".to_string());
                        tags.push(if is_artist { "#artist".to_string() } else { "#album".to_string() });
                        tags.push("#discover".to_string());
                    }
                    app.actions.push(Action::OpenPreviewFeed(crate::app::PreviewFeedState {
                        entity_title: title.clone(),
                        entity_subtitle: short_stat,
                        entity_avatar_key: cover_key.clone(),
                        entity_avatar_src: source.map(str::to_string),
                        artist_name: primary_artist,
                        items: preview_items,
                        index: 0,
                        scroll_anim: 0.0,
                        last_wheel: None,
                        tags,
                    }));
                }
                ax = prev_r.right() + 22.0;
            } else {
                ax += 6.0;
            }
            let shuf_r = Rect::from_center_size(Pos2::new(ax, cy), Vec2::splat(32.0));
            if icon_button(ui, shuf_r, "ytm-shuffle", Icon::Shuffle, if app.player.shuffle { ACCENT } else { SUBTLE }, app.t("Trộn bài", "Shuffle")).clicked() {
                app.actions.push(Action::ToggleShuffle);
            }
            ax = shuf_r.right() + 22.0;

            if is_artist {
                let followed = app.data.followed_artists.contains(&title);
                let lbl = if followed {
                    app.t("Đang theo dõi", "Following")
                } else {
                    app.t("Theo dõi", "Follow")
                };
                // Spotify Desktop 1.3.9 Follow / Following pill: always white bold text with subdued gray border (brightens to white on hover).
                let fw = text_width(ui, lbl, bold(14.0)) + 32.0;
                let fr = Rect::from_center_size(Pos2::new(ax + fw * 0.5, cy), Vec2::new(fw, 32.0));
                let f_resp = ui
                    .interact(fr, ui.scope_id().with("ytm-artist-follow-btn"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let stroke_col = if f_resp.hovered() { TEXT } else { MUTED };
                let f_draw = crate::theme::hover_scale_rect(
                    fr,
                    f_resp.hovered() && !app.data.settings.prefers_reduced_motion,
                    f_resp.is_pointer_button_down_on(),
                );
                ui.painter().rect_stroke(f_draw, 16.0, Stroke::new(1.0, stroke_col), egui::StrokeKind::Inside);
                text(
                    ui.painter(),
                    f_draw.center(),
                    lbl,
                    bold(14.0),
                    TEXT,
                    fw,
                    Align2::CENTER_CENTER,
                );
                if f_resp.clicked() {
                    app.actions.push(Action::ToggleArtistFollow(title.clone()));
                }
                ax = fr.right() + 20.0;
            } else if let Some(id) = follow_id {
                let followed = app.data.followed_ytm.iter().any(|l| l.id == id);
                if is_ytm_album {
                    let save_r = Rect::from_center_size(Pos2::new(ax, cy), Vec2::splat(32.0));
                    let (s_ic, s_col, s_tip) = if followed {
                        (Icon::Saved, ACCENT, app.t("Xóa khỏi Thư viện", "Remove from Your Library"))
                    } else {
                        (Icon::Save, SUBTLE, app.t("Lưu vào Thư viện", "Save to Your Library"))
                    };
                    if icon_button_sized(ui, save_r, "ytm-album-save-btn", s_ic, s_col, s_tip, 24.0).clicked() {
                        if followed {
                            follow_toggle = Some((id.to_string(), None));
                        } else if let Some(p) = page {
                            let mut snap = p.clone();
                            if snap.title.is_empty() {
                                snap.title = title.clone();
                            }
                            follow_toggle = Some((id.to_string(), Some(snap)));
                        }
                    }
                    ax = save_r.right() + 22.0;
                } else {
                    let lbl = if followed { app.t("Đang theo dõi", "Following") } else { app.t("Theo dõi", "Follow") };
                    let fw = text_width(ui, lbl, semi(13.0)) + 28.0;
                    let fr = Rect::from_center_size(Pos2::new(ax + fw * 0.5, cy), Vec2::new(fw, 32.0));
                    let f_resp = ui.interact(fr, ui.scope_id().with("ytm-follow-btn"), Sense::click());
                    ui.painter().rect_stroke(fr, 16.0, Stroke::new(1.0, if followed || f_resp.hovered() { TEXT } else { MUTED }), egui::StrokeKind::Inside);
                    text(ui.painter(), fr.center(), lbl, semi(13.0), TEXT, fw, Align2::CENTER_CENTER);
                    if f_resp.clicked() {
                        if followed {
                            follow_toggle = Some((id.to_string(), None));
                        } else if let Some(p) = page {
                            let mut snap = p.clone();
                            if snap.title.is_empty() {
                                snap.title = item.title.clone();
                            }
                            follow_toggle = Some((id.to_string(), Some(snap)));
                        }
                    }
                    ax = fr.right() + 20.0;
                }
            }

            let dots_r = Rect::from_center_size(Pos2::new(ax, cy), Vec2::splat(32.0));
            let dots_resp = icon_button(ui, dots_r, "ytm-page-dots", Icon::Dots, SUBTLE, app.t("Tùy chọn khác", "More options"));
            egui::Popup::menu(&dots_resp).show(|ui| {
                ui.set_min_width(210.0);
                if ui.button(app.t("Mở trên YouTube Music", "Open on YouTube Music")).clicked() {
                    crate::ytm::open_browser(&item.id);
                    ui.close();
                }
                if ui.button(app.t("Sao chép liên kết", "Copy link")).clicked() {
                    ui.ctx().copy_text(item.id.url());
                    ui.close();
                }
                if !items_raw.is_empty() {
                    ui.separator();
                    if ui.button(app.t("Thêm tất cả vào danh sách chờ", "Add all to queue")).clicked() {
                        for it in &items_raw {
                            if matches!(it.id, ItemId::Song(_)) {
                                app.actions.push(Action::EnqueueYouTubeSong(it.clone()));
                            }
                        }
                        ui.close();
                    }
                }
            });

            if !is_artist && !is_song {
                draw_tracklist_controls(app, ui, act_r.right() - side, cy, dots_r.right() + 16.0, matches!(item.id, ItemId::Album(_)));
            }

            if pending && items_raw.is_empty() {
                ui.add_space(12.0);
                for r_idx in 0..6 {
                    let (rr, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
                    let rx = rr.left() + side + 36.0;
                    let avail = (rr.right() - side - rx - 80.0).max(80.0);
                    let w1 = [0.46, 0.38, 0.54, 0.35, 0.49, 0.41][r_idx];
                    let w2 = [0.22, 0.27, 0.19, 0.25, 0.21, 0.24][r_idx];
                    skeleton_glass_rect(ui, Rect::from_min_size(Pos2::new(rx, rr.top() + 12.0), Vec2::new(avail * w1, 13.0)), 6.0);
                    skeleton_glass_rect(ui, Rect::from_min_size(Pos2::new(rx, rr.top() + 31.0), Vec2::new(avail * w2, 10.0)), 5.0);
                }
            } else if let Some(err) = error {
                if draw_error_banner(app, ui, side, err, || {}) {
                    retry_clicked = true;
                }
            }

            if is_song {
                let (tab_r, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
                let mut tx = tab_r.left() + side;
                // Default to Related Songs / Track context instead of a bare empty Lyrics tab.
                for (show_lyrics, lbl) in [(false, app.t("Bài hát tương tự", "Recommended")), (true, app.t("Lời bài hát", "Lyrics"))] {
                    let pw = pill_width(ui, lbl);
                    let pr = Rect::from_min_size(Pos2::new(tx, tab_r.center().y - 16.0), Vec2::new(pw, 32.0));
                    if pill(ui, pr, ("song-tab", show_lyrics), lbl, browser.related == show_lyrics).clicked() {
                        browser.related = show_lyrics;
                    }
                    tx += pw + 8.0;
                }
                ui.add_space(8.0);
                if !browser.related {
                    let rel_songs: Vec<Item> = items_raw.iter().filter(|it| matches!(it.id, ItemId::Song(_))).cloned().collect();
                    if rel_songs.is_empty() && !pending {
                        let (lr, _) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::hover());
                        text(ui.painter(), Pos2::new(lr.left() + side, lr.center().y), app.t("Nhấn nút Phát để bắt đầu bài hát và tạo danh sách đề xuất.", "Press Play to start this song and load recommendations."), body(14.0), SUBTLE, inner_w, Align2::LEFT_CENTER);
                    } else {
                        for (i, rel) in rel_songs.iter().enumerate() {
                            let (rr, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
                            let row_r = Rect::from_min_size(Pos2::new(rr.left() + side - 8.0, rr.top()), Vec2::new(w - (side - 8.0) * 2.0, ROW_H));
                            draw_song_row(app, ui, row_r, i, rel, false, false, page, Some((title.as_str(), &rel_songs)));
                        }
                    }
                } else {
                    let track_id = if let ItemId::Song(vid) = &item.id {
                        Some(crate::library::fnv(&format!("yt:{vid}")))
                    } else {
                        None
                    };
                    if app.player.current == track_id {
                        if let Some(lyrics) = app.current_lyrics() {
                            for line in &lyrics.lines {
                                let (lr, _) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::hover());
                                text(ui.painter(), Pos2::new(lr.left() + side, lr.center().y), &line.text, body(16.0), TEXT, inner_w, Align2::LEFT_CENTER);
                            }
                        } else if app.lyrics_loading() {
                            ui.horizontal(|ui| { ui.add_space(side); ui.spinner(); });
                        } else {
                            let (lr, _) = ui.allocate_exact_size(Vec2::new(w, 28.0), Sense::hover());
                            text(ui.painter(), Pos2::new(lr.left() + side, lr.center().y), app.t("Chưa có lời bài hát", "Lyrics unavailable"), body(14.0), SUBTLE, inner_w, Align2::LEFT_CENTER);
                        }
                    } else if let Some(first) = items_raw.first().or(Some(item)) {
                        ui.horizontal(|ui| {
                            ui.add_space(side);
                            if ui.button(app.t("Phát để xem lời bài hát", "Play to view lyrics")).clicked() {
                                app.actions.push(Action::SelectYouTubeSong(first.clone()));
                            }
                        });
                    }
                }
            } else if is_artist {
                let songs: Vec<&Item> = items_raw.iter().filter(|it| matches!(it.id, ItemId::Song(_))).collect();
                let releases: Vec<CardData> = items_raw.iter().filter(|it| matches!(it.id, ItemId::Album(_))).map(|it| CardData::from_ytm(app, it, page)).collect();
                let playlists: Vec<CardData> = items_raw.iter().filter(|it| matches!(it.id, ItemId::Playlist(_))).map(|it| CardData::from_ytm(app, it, page)).collect();
                let related_artists: Vec<CardData> = items_raw.iter().filter(|it| matches!(it.id, ItemId::Artist(_))).map(|it| CardData::from_ytm(app, it, page)).collect();

                if !songs.is_empty() {
                    let (hr, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
                    text(ui.painter(), Pos2::new(hr.left() + side, hr.center().y), app.t("Phổ biến", "Popular"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
                    let max_n = if see_more { 10 } else { 5 };
                    for (i, s) in songs.iter().take(max_n).enumerate() {
                        let (rr, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
                        let row_r = Rect::from_min_size(Pos2::new(rr.left() + side - 8.0, rr.top()), Vec2::new(w - (side - 8.0) * 2.0, ROW_H));
                        if ui.is_rect_visible(row_r) {
                            draw_song_row(app, ui, row_r, i, s, true, false, page, Some((title.as_str(), &songs_list)));
                        }
                    }
                    if songs.len() > 5 {
                        let (mr, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
                        let lbl = if see_more { app.t("Ẩn bớt", "Show less") } else { app.t("Xem thêm", "See more") };
                        let lw = text_width(ui, lbl, semi(13.0)) + 8.0;
                        let lr = Rect::from_min_size(Pos2::new(mr.left() + side + 12.0, mr.center().y - 12.0), Vec2::new(lw, 24.0));
                        let m_resp = ui.interact(lr, ui.scope_id().with("ytm-see-more"), Sense::click());
                        text(ui.painter(), lr.left_center(), lbl, semi(13.0), if m_resp.hovered() { TEXT } else { SUBTLE }, lw, Align2::LEFT_CENTER);
                        if m_resp.clicked() {
                            see_more = !see_more;
                        }
                    }
                    ui.add_space(16.0);
                }

                if !releases.is_empty() {
                    let (dh, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
                    text(ui.painter(), Pos2::new(dh.left() + side, dh.center().y), app.t("Danh sách đĩa nhạc", "Discography"), bold(24.0), TEXT, inner_w - 80.0, Align2::LEFT_CENTER);
                    let lb = Rect::from_center_size(Pos2::new(dh.right() - side - 44.0, dh.center().y), Vec2::splat(28.0));
                    let gb = Rect::from_center_size(Pos2::new(dh.right() - side - 14.0, dh.center().y), Vec2::splat(28.0));
                    if icon_button(ui, lb, "ytm-disco-list", Icon::Sort, if disco_list_mode { TEXT } else { SUBTLE }, app.t("Xem danh sách", "List view")).clicked() {
                        disco_list_mode = true;
                    }
                    if icon_button(ui, gb, "ytm-disco-grid", Icon::Grid, if !disco_list_mode { TEXT } else { SUBTLE }, app.t("Xem lưới", "Grid view")).clicked() {
                        disco_list_mode = false;
                    }
                    let (cr, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
                    let mut cx = cr.left() + side;
                    for (idx, lbl) in [(0u8, app.t("Phát hành phổ biến", "Popular releases")), (1u8, "Album"), (2u8, app.t("Đĩa đơn và EP", "Singles and EPs"))] {
                        let cw = pill_width(ui, lbl);
                        let pr = Rect::from_min_size(Pos2::new(cx, cr.center().y - 16.0), Vec2::new(cw, 32.0));
                        if pill(ui, pr, ("ytm-disco-chip", idx), lbl, disco_chip == idx).clicked() {
                            disco_chip = idx;
                        }
                        cx += cw + 8.0;
                    }
                    let releases: Vec<CardData> = releases.into_iter().filter(|card| {
                        let kind = card.ytm_item.as_ref().and_then(|item| page.and_then(|p| p.release_kind(&item.id)));
                        let single_ep_shelf = card.ytm_item.as_ref().is_some_and(|item| page.is_some_and(|p| p.is_single_or_ep(&item.id)));
                        discography_filter_includes(disco_chip, kind, single_ep_shelf)
                    }).collect();
                    if disco_list_mode {
                        for (i, release) in releases.iter().enumerate() {
                            let (slot, _) = ui.allocate_exact_size(Vec2::new(w, 96.0), Sense::hover());
                            let r = Rect::from_min_size(Pos2::new(slot.left() + side, slot.top()), Vec2::new(inner_w, 96.0));
                            let response = ui.interact(r, ui.scope_id().with(("ytm-release-row", i)), Sense::click());
                            let active = ui.rect_contains_pointer(r) || response.has_focus();
                            if active {
                                ui.painter().rect_filled(r, 4.0, hover_white(26));
                            }
                            let art = Rect::from_min_size(r.min + Vec2::splat(8.0), Vec2::splat(80.0));
                            super::pages::paint_card_cover(app, ui, art, release, 4.0);
                            let tx = art.right() + 16.0;
                            let room = (r.right() - tx - 64.0).max(20.0);
                            text(ui.painter(), Pos2::new(tx, r.center().y - 12.0), &release.title, bold(20.0), TEXT, room, Align2::LEFT_CENTER);
                            text(ui.painter(), Pos2::new(tx, r.center().y + 16.0), &release.sub, body(13.0), SUBTLE, room, Align2::LEFT_CENTER);
                            focus_ring(ui, &response, 4.0);
                            if response.clicked() {
                                app.actions.push(Action::Navigate(release.route.clone()));
                            }
                            if let Some(item) = &release.ytm_item {
                                response.context_menu(|ui| item_menu(app, ui, item));
                            }
                            let play = Rect::from_center_size(Pos2::new(r.right() - 30.0, r.center().y), Vec2::splat(44.0));
                            super::pages::card_play_button(app, ui, play, release, ("ytm-release-play", i), active);
                        }
                    } else {
                        draw_carousel_shelf(app, ui, "ytm-artist-releases", "", &releases, side);
                    }
                }
                if !playlists.is_empty() {
                    draw_carousel_shelf(app, ui, "ytm-artist-playlists", "Playlist", &playlists, side);
                }
                if !related_artists.is_empty() {
                    draw_carousel_shelf(app, ui, "ytm-artist-related", app.t("Người hâm mộ cũng thích", "Fans also like"), &related_artists, side);
                }
                // Spotify-style full-bleed photo About card (tall photographic card with bottom vignette, listener count & 3-line bio).
                let (ah, _) = ui.allocate_exact_size(Vec2::new(w, 38.0), Sense::hover());
                text(ui.painter(), Pos2::new(ah.left() + side, ah.center().y), app.t("Giới thiệu", "About"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
                let about_w = inner_w.min(680.0);
                let about_h = if about_w >= 460.0 { 296.0 } else { 220.0 };
                let (aslot, _) = ui.allocate_exact_size(Vec2::new(w, about_h + 16.0), Sense::hover());
                let card_r = Rect::from_min_size(Pos2::new(aslot.left() + side, aslot.top() + 8.0), Vec2::new(about_w, about_h));
                let ab_resp = ui
                    .interact(card_r, ui.scope_id().with("ytm-about-card"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let ab_sc = if app.data.settings.prefers_reduced_motion {
                    1.0
                } else {
                    ui.ctx().animate_value_with_time(
                        ui.scope_id().with("ytm-about-scale"),
                        if ab_resp.hovered() { 1.015 } else { 1.0 },
                        0.18,
                    )
                };
                let draw_r = Rect::from_center_size(card_r.center(), card_r.size() * ab_sc);
                ui.painter().rect_filled(draw_r, RADIUS_CARD, CARD);
                cover(ui, draw_r, &tex, RADIUS_CARD);
                vertical_gradient(
                    ui.painter(),
                    draw_r,
                    Color32::from_black_alpha(30),
                    Color32::from_black_alpha(225),
                );
                let sub_txt = page
                    .and_then(|p| p.subtitle.as_deref())
                    .filter(|s| !s.is_empty())
                    .map(|s| format_artist_subtitle(app.lang(), s))
                    .unwrap_or_else(|| app.t("Nghệ sĩ trên YouTube Music", "Artist on YouTube Music").to_string());
                let desc_opt = page.and_then(|p| p.description.as_deref()).filter(|s| !s.is_empty());
                let text_pad = 24.0;
                let text_w = (draw_r.width() - text_pad * 2.0).max(80.0);
                let stat_y = if desc_opt.is_some() { draw_r.bottom() - 88.0 } else { draw_r.bottom() - 36.0 };
                text(ui.painter(), Pos2::new(draw_r.left() + text_pad, stat_y), &sub_txt, bold(16.0), TEXT, text_w, Align2::LEFT_TOP);
                if let Some(desc) = desc_opt {
                    wrapped_text(
                        ui.painter(),
                        Pos2::new(draw_r.left() + text_pad, stat_y + 26.0),
                        desc,
                        body(14.0),
                        Color32::from_white_alpha(225),
                        text_w,
                        3,
                        Align2::LEFT_TOP,
                    );
                }
                if ab_resp.clicked() {
                    about_open = true;
                }
            } else {
                // Album or Playlist tracklist.
                ui.add_space(header_h + 8.0);
                let compact = app.data.settings.compact_tracklist;
                let is_ytm_album = matches!(item.id, ItemId::Album(_))
                    || matches!(&item.id, ItemId::Playlist(pid) if pid.starts_with("OLAK5uy_"));
                for (i, song) in filtered_items.iter().enumerate() {
                    let (rr, _) = ui.allocate_exact_size(Vec2::new(w, row_h), Sense::hover());
                    let row_r = Rect::from_min_size(Pos2::new(rr.left() + 16.0, rr.top()), Vec2::new(w - 32.0, row_h));
                    if ui.is_rect_visible(row_r) {
                        draw_ytm_table_row(
                            app,
                            ui,
                            row_r,
                            i,
                            song,
                            &cols,
                            compact,
                            is_ytm_album,
                            page,
                            &title,
                            &songs_list,
                        );
                    }
                }
                if is_ytm_album && !filtered_items.is_empty() {
                    let by = page.and_then(|p| p.subtitle.as_deref()).unwrap_or(&item.by);
                    let yr = page.and_then(|p| p.year.as_deref()).filter(|s| !s.is_empty()).unwrap_or("2025");
                    let rel_lbl = match app.lang() {
                        crate::library::Lang::Vi => format!("Phát hành năm {yr}"),
                        crate::library::Lang::En => format!("Released {yr}"),
                    };
                    let (fr, _) = ui.allocate_exact_size(Vec2::new(w, 72.0), Sense::hover());
                    text(ui.painter(), Pos2::new(fr.left() + side, fr.top() + 16.0), &rel_lbl, body(13.5), SUBTLE, inner_w, Align2::LEFT_TOP);
                    text(ui.painter(), Pos2::new(fr.left() + side, fr.top() + 36.0), &format!("© {yr} {by}"), body(11.5), SUBTLE, inner_w, Align2::LEFT_TOP);
                    text(ui.painter(), Pos2::new(fr.left() + side, fr.top() + 52.0), &format!("℗ {yr} {by}"), body(11.5), SUBTLE, inner_w, Align2::LEFT_TOP);

                    let more_albums = real_artist_albums_from_ytm(app, by, &title, ui.ctx());
                    if !more_albums.is_empty() {
                        ui.add_space(12.0);
                        let primary = crate::library::split_artists(by)
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| by.to_string());
                        let heading = match app.lang() {
                            crate::library::Lang::Vi => format!("Album khác của {primary}"),
                            crate::library::Lang::En => format!("More by {primary}"),
                        };
                        draw_carousel_shelf(app, ui, "ytm-more-by-artist", &heading, &more_albums, side);
                    }
                }
            }
            ui.add_space(24.0);
        });

        // Sticky Mini-Topbar.
        if mini_fade > 0.01 {
            draw_sticky_mini_topbar(
                app,
                ui,
                Rect::from_min_size(Pos2::new(origin.x, origin.y + scroll_y), Vec2::new(w, mini_h)),
                &title,
                hero_color,
                mini_fade,
                &ContextRef::None,
                &[],
                side,
            );
        }

        // Sticky 36px Column Header for Album / Playlist.
        if header_h > 0.0 {
            let header_y = if header_stuck { origin.y + scroll_y + mini_h } else { origin.y + header_rest_y };
            let header_rect = Rect::from_min_size(Pos2::new(origin.x, header_y), Vec2::new(w, header_h));
            if header_stuck {
                ui.painter().rect_filled(header_rect, 0.0, PANEL);
                ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, HAIRLINE));
            }
            table_header(app, ui, header_rect, &cols, false, matches!(item.id, ItemId::Album(_)));
        }
    });

    if let Some((id, snap)) = follow_toggle {
        app.update_followed_ytm(&id, snap);
    }
    if retry_clicked {
        if let Some(req) = request {
            browser.send(ui.ctx(), req.clone(), false);
        }
    }

    if about_open {
        let full = ui.ctx().content_rect();
        let layer = egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("ytm-about-layer"));
        let mut top = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(full).id_salt("ytm-about"));
        top.set_clip_rect(full);
        top.painter().rect_filled(full, 0.0, Color32::from_black_alpha(165));
        if top.interact(full, top.scope_id().with("scrim"), Sense::click()).clicked() {
            about_open = false;
        }
        let modal = Rect::from_center_size(full.center(), Vec2::new(500.0f32.min(full.width() - 40.0), 320.0));
        top.interact(modal, top.scope_id().with("panel"), Sense::click());
        top.painter().rect_filled(modal, RADIUS_MODAL, ELEVATED);
        text(top.painter(), Pos2::new(modal.left() + 24.0, modal.top() + 24.0), &title, bold(24.0), TEXT, modal.width() - 72.0, Align2::LEFT_TOP);
        let modal_body = page
            .and_then(|p| p.description.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| app.t("Thông tin nghệ sĩ được lấy trực tiếp từ YouTube Music.", "Artist details streamed from YouTube Music."));
        wrapped_text(top.painter(), Pos2::new(modal.left() + 24.0, modal.top() + 72.0), modal_body, body(14.0), SUBTLE, modal.width() - 48.0, 10, Align2::LEFT_TOP);
        let close = Rect::from_center_size(Pos2::new(modal.right() - 28.0, modal.top() + 28.0), Vec2::splat(32.0));
        if icon_button(&mut top, close, "ytm-about-close", Icon::Close, TEXT, app.t("Đóng", "Close")).clicked() {
            about_open = false;
        }
    }

    ui.ctx().data_mut(|d| {
        d.insert_temp(see_more_id, see_more);
        d.insert_temp(disco_chip_id, disco_chip);
        d.insert_temp(disco_view_id, disco_list_mode);
        d.insert_temp(about_id, about_open);
    });
}

fn filter_and_sort_ytm_items(app: &App, items: &[Item]) -> Vec<Item> {
    let q = fold(app.entity_filter.trim());
    let mut out: Vec<Item> = items
        .iter()
        .filter(|it| {
            q.is_empty() || fold(&it.title).contains(&q) || fold(&it.by).contains(&q) || fold(&it.album).contains(&q)
        })
        .cloned()
        .collect();
    let sort = app.table_sort;
    if sort.column != TrackSortColumn::Default {
        out.sort_by(|a, b| {
            let ord = match sort.column {
                TrackSortColumn::Default => Ordering::Equal,
                TrackSortColumn::Title => fold(&a.title).cmp(&fold(&b.title)),
                TrackSortColumn::Artist => fold(&a.by).cmp(&fold(&b.by)),
                TrackSortColumn::Album => fold(&a.album).cmp(&fold(&b.album)),
                TrackSortColumn::DateAdded => Ordering::Equal,
                TrackSortColumn::Duration => a.duration_ms.cmp(&b.duration_ms),
            };
            if sort.ascending { ord } else { ord.reverse() }
        });
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn draw_ytm_table_row(
    app: &mut App,
    ui: &mut Ui,
    r: Rect,
    i: usize,
    item: &Item,
    cols: &Columns,
    compact: bool,
    is_album: bool,
    page: Option<&Page>,
    list_title: &str,
    songs_list: &[Item],
) {
    let resp = ui.interact(r, ui.scope_id().with(("ytm-trow", i, &item.id)), Sense::click());
    let hovered = ui.rect_contains_pointer(r) || resp.has_focus();
    focus_ring(ui, &resp, 4.0);
    if hovered {
        ui.painter().rect_filled(r, 4.0, hover_white(26));
    }
    let song_id = match &item.id {
        ItemId::Song(vid) => Some(crate::library::fnv(&format!("yt:{vid}"))),
        _ => None,
    };
    let is_current = song_id.is_some() && app.player.current == song_id;
    let is_playing = is_current && app.is_playing();
    let title_col = if is_current { ACCENT } else { TEXT };
    let y = r.center().y;
    let base = r.left();
    let mut sub_clicked = false;

    let play_row_in_list = |app: &mut App| {
        if let Some(pos) = songs_list.iter().position(|s| s.id == item.id) {
            app.actions.push(Action::PlayYouTubeList {
                name: list_title.to_string(),
                items: songs_list.to_vec(),
                start: pos,
            });
        } else {
            app.actions.push(Action::SelectYouTubeSong(item.clone()));
        }
    };

    let ind = Rect::from_center_size(Pos2::new(base + 22.0, y), Vec2::splat(20.0));
    if hovered {
        let ic = if is_playing { Icon::Pause } else { Icon::Play };
        let tip = if is_playing { app.t("Tạm dừng", "Pause") } else { app.t("Phát", "Play") };
        if icon_button(ui, ind, ("ytm-trow-play", i), ic, TEXT, tip).clicked() {
            sub_clicked = true;
            if is_current {
                app.actions.push(Action::TogglePlay);
            } else {
                play_row_in_list(app);
            }
        }
    } else if is_playing {
        equalizer_glyph(ui, ind, ui.input(|inp| inp.time));
    } else {
        text(ui.painter(), ind.center(), &(i + 1).to_string(), body(14.5), if is_current { ACCENT } else { SUBTLE }, 30.0, Align2::CENTER_CENTER);
    }

    let plays_reserve = if is_album && r.width() >= 600.0 { 112.0 } else { 0.0 };
    if compact {
        let tx = base + cols.index;
        let room = (cols.title - 12.0 - plays_reserve).max(40.0);
        let title_w = (room * 0.58).max(40.0);
        text(ui.painter(), Pos2::new(tx, y), &item.title, body(14.0), title_col, title_w, Align2::LEFT_CENTER);
        if paint_artist_links(app, ui, Pos2::new(tx + title_w + 10.0, y), &item.by, (room - title_w - 10.0).max(30.0), ("ytm-trow-artist-c", i)) {
            sub_clicked = true;
        }
    } else {
        let tx = base + cols.index;
        let room = (cols.title - 16.0 - plays_reserve).max(40.0);
        text(ui.painter(), Pos2::new(tx, y - 10.0), &item.title, body(16.0), title_col, room, Align2::LEFT_CENTER);
        if paint_artist_links(app, ui, Pos2::new(tx, y + 11.0), &item.by, room, ("ytm-trow-artist", i)) {
            sub_clicked = true;
        }
    }

    if is_album && r.width() >= 600.0 {
        let real_plays = match &item.id {
            ItemId::Song(vid) => page.and_then(|p| p.track_plays.get(vid)).cloned(),
            _ => None,
        };
        let plays_lbl = real_plays.unwrap_or_else(|| format_play_count(app.lang(), song_id.unwrap_or(i as u64 + 1)));
        text(
            ui.painter(),
            Pos2::new(base + cols.right - 134.0, y),
            &plays_lbl,
            body(14.0),
            SUBTLE,
            106.0,
            Align2::RIGHT_CENTER,
        );
    }

    if let Some(a) = cols.album {
        let album_str = if item.album.is_empty() { &item.by } else { &item.album };
        let al_pos = Pos2::new(base + cols.index + cols.title, y);
        let al_r = text(ui.painter(), al_pos, album_str, body(13.5), SUBTLE, a - 16.0, Align2::LEFT_CENTER);
        if !item.album.trim().is_empty() && link(ui, al_r, ("ytm-trow-album", i)) {
            sub_clicked = true;
            let owner = crate::library::split_artists(&item.by).into_iter().next().unwrap_or_else(|| item.by.clone());
            app.actions.push(Action::Navigate(Route::Album(crate::library::album_key(&owner, &item.album))));
        }
    }

    let right = base + cols.right;
    if let Some(sid) = song_id {
        let saved = app.data.is_liked(sid);
        if saved || hovered {
            let save_r = Rect::from_center_size(Pos2::new(right - 82.0, y), Vec2::splat(24.0));
            let (ic, col, tip) = if saved {
                (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
            } else {
                (Icon::Save, SUBTLE, app.t("Thêm vào Bài hát đã thích", "Save to Liked Songs"))
            };
            if icon_button(ui, save_r, ("ytm-trow-save", i), ic, col, tip).clicked() {
                sub_clicked = true;
                app.actions.push(Action::LikeYouTubeSong(item.clone()));
            }
        }
    }
    if item.duration_ms > 0 {
        text(ui.painter(), Pos2::new(right - 38.0, y), &mmss(item.duration_ms), body(14.0), SUBTLE, 50.0, Align2::RIGHT_CENTER);
    }

    resp.context_menu(|ui| item_menu(app, ui, item));
    if hovered {
        let dots_r = Rect::from_center_size(Pos2::new(right - 14.0, y), Vec2::splat(24.0));
        let d = icon_button(ui, dots_r, ("ytm-trow-dots", i), Icon::Dots, TEXT, app.t("Tùy chọn khác", "More options"));
        if d.clicked() {
            sub_clicked = true;
        }
        egui::Popup::menu(&d).show(|ui| item_menu(app, ui, item));
    }
    if (resp.clicked() || resp.double_clicked()) && !sub_clicked {
        if matches!(item.id, ItemId::Song(_)) {
            if is_current && resp.clicked() && !resp.double_clicked() {
                app.actions.push(Action::TogglePlay);
            } else {
                play_row_in_list(app);
            }
        } else {
            navigate(app, item);
        }
    }
}

/// Song row for Search "Songs" block and Artist "Popular" tracks: 40x40 cover with hover scrim play,
/// play count column (when available on Artist pages), hover-reveal Save/Saved + duration + `...` menu.
#[allow(clippy::too_many_arguments)]
fn draw_song_row(
    app: &mut App,
    ui: &mut Ui,
    rect: Rect,
    i: usize,
    item: &Item,
    indexed: bool,
    demo_mode: bool,
    page: Option<&Page>,
    queue_list: Option<(&str, &[Item])>,
) {
    let row = ui.interact(rect, ui.scope_id().with(("song-row", i, &item.id)), Sense::click());
    let hovered = ui.rect_contains_pointer(rect) || row.has_focus();
    focus_ring(ui, &row, 4.0);
    if hovered {
        ui.painter().rect_filled(rect, 4.0, hover_white(26));
    }

    let song_vid = match &item.id {
        ItemId::Song(id) => Some(id.as_str()),
        _ => None,
    };
    let song_id = song_vid.map(|id| crate::library::fnv(&format!("yt:{id}")));
    let is_current = song_id.is_some() && app.player.current == song_id;
    let is_playing = is_current && app.is_playing();
    let tint = if is_current { ACCENT } else { TEXT };
    let mut sub_clicked = false;

    let trigger_play = |app: &mut App| {
        if demo_mode {
            demo_navigate_or_play(app, item, true);
        } else if is_current {
            app.actions.push(Action::TogglePlay);
        } else if let Some((list_name, list_items)) = queue_list {
            if let Some(pos) = list_items.iter().position(|it| it.id == item.id) {
                app.actions.push(Action::PlayYouTubeList {
                    name: list_name.to_string(),
                    items: list_items.to_vec(),
                    start: pos,
                });
                return;
            }
            app.actions.push(Action::SelectYouTubeSong(item.clone()));
        } else {
            app.actions.push(Action::SelectYouTubeSong(item.clone()));
        }
    };

    let mut x = rect.left() + 10.0;
    if indexed {
        let idx_r = Rect::from_center_size(Pos2::new(x + 12.0, rect.center().y), Vec2::splat(20.0));
        if hovered {
            let ic = if is_playing { Icon::Pause } else { Icon::Play };
            let tip = if is_playing { app.t("Tạm dừng", "Pause") } else { app.t("Phát", "Play") };
            if icon_button(ui, idx_r, ("srow-idx-play", i), ic, TEXT, tip).clicked() {
                sub_clicked = true;
                trigger_play(app);
            }
        } else if is_playing {
            equalizer_glyph(ui, idx_r, ui.input(|inp| inp.time));
        } else {
            text(ui.painter(), idx_r.center(), &(i + 1).to_string(), body(14.5), if is_current { ACCENT } else { SUBTLE }, 24.0, Align2::CENTER_CENTER);
        }
        x += 32.0;
    }

    let art_r = Rect::from_center_size(Pos2::new(x + 20.0, rect.center().y), Vec2::splat(40.0));
    let key = top_result_cover_key(app, item, demo_mode);
    let tex = app.art.get(ui.ctx(), &key, item.image.as_deref());
    let round = if matches!(item.id, ItemId::Artist(_)) { 20.0 } else { 4.0 };
    cover(ui, art_r, &tex, round);

    if hovered && matches!(item.id, ItemId::Song(_)) {
        ui.painter().rect_filled(art_r, round, Color32::from_black_alpha(155));
        let tip = if is_playing { app.t("Tạm dừng", "Pause") } else { app.t("Phát", "Play") };
        if icon_button(ui, art_r, ("srow-art-play", i), if is_playing { Icon::Pause } else { Icon::Play }, TEXT, tip).clicked() {
            sub_clicked = true;
            trigger_play(app);
        }
    }

    let raw_plays = song_vid.and_then(|vid| page.and_then(|p| p.track_plays.get(vid))).map(String::as_str);
    let show_plays = indexed && rect.width() >= 460.0 && raw_plays.is_some();
    let right_reserve = if show_plays { 228.0 } else { 108.0 };
    let title_x = art_r.right() + 12.0;
    let room = (rect.right() - title_x - right_reserve).max(20.0);
    text(ui.painter(), Pos2::new(title_x, rect.center().y - 9.0), &item.title, body(16.0), tint, room, Align2::LEFT_CENTER);
    let sub = if item.by.is_empty() { ytm_kind_label(app, item, None).to_string() } else { item.by.clone() };
    if !item.by.is_empty() {
        if paint_artist_links(app, ui, Pos2::new(title_x, rect.center().y + 11.0), &sub, room, ("srow-artist", i)) {
            sub_clicked = true;
        }
    } else {
        text(ui.painter(), Pos2::new(title_x, rect.center().y + 11.0), &sub, body(13.5), SUBTLE, room, Align2::LEFT_CENTER);
    }

    if show_plays {
        if let Some(plays) = raw_plays {
            let formatted = format_track_plays(app.lang(), plays);
            text(
                ui.painter(),
                Pos2::new(rect.right() - 122.0, rect.center().y),
                &formatted,
                body(14.0),
                SUBTLE,
                110.0,
                Align2::RIGHT_CENTER,
            );
        }
    }

    if let Some(sid) = song_id {
        let saved = if demo_mode {
            app.lib.order.iter().copied().find(|&id| app.lib.track(id).is_some_and(|t| t.title == item.title)).is_some_and(|id| app.data.is_liked(id))
        } else {
            app.data.is_liked(sid)
        };
        if saved || hovered {
            let save_r = Rect::from_center_size(Pos2::new(rect.right() - 86.0, rect.center().y), Vec2::splat(24.0));
            let (ic, col, tip) = if saved {
                (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
            } else {
                (Icon::Save, SUBTLE, app.t("Thêm vào Bài hát đã thích", "Save to Liked Songs"))
            };
            if icon_button(ui, save_r, ("srow-save", i), ic, col, tip).clicked() {
                sub_clicked = true;
                if demo_mode {
                    if let Some(tid) = app.lib.order.iter().copied().find(|&id| app.lib.track(id).is_some_and(|t| t.title == item.title)) {
                        app.actions.push(Action::ToggleLike(tid));
                    }
                } else {
                    app.actions.push(Action::LikeYouTubeSong(item.clone()));
                }
            }
        }
    }

    let dur_ms = if item.duration_ms > 0 {
        item.duration_ms
    } else {
        song_id.and_then(|sid| app.lib.track(sid)).map_or(0, |t| t.duration_ms)
    };
    if dur_ms > 0 {
        text(ui.painter(), Pos2::new(rect.right() - 40.0, rect.center().y), &mmss(dur_ms), body(14.0), SUBTLE, 50.0, Align2::RIGHT_CENTER);
    }

    if !demo_mode {
        row.context_menu(|ui| item_menu(app, ui, item));
        if hovered {
            let dots = Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y), Vec2::splat(24.0));
            let btn = icon_button(ui, dots, ("srow-dots", i), Icon::Dots, TEXT, app.t("Tùy chọn khác", "More options"));
            if btn.clicked() {
                sub_clicked = true;
            }
            egui::Popup::menu(&btn).show(|ui| item_menu(app, ui, item));
        }
    }

    if (row.clicked() || row.double_clicked()) && !sub_clicked {
        if demo_mode {
            demo_navigate_or_play(app, item, true);
        } else if matches!(item.id, ItemId::Song(_)) {
            trigger_play(app);
        } else {
            navigate(app, item);
        }
    }
}

fn draw_error_banner(app: &App, ui: &mut Ui, side: f32, error: Error, on_retry: impl FnOnce()) -> bool {
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.add_space(side);
        let label = match error {
            Error::Network => app.t("Không kết nối được YouTube Music", "Could not connect to YouTube Music"),
            Error::Unavailable => app.t("Nội dung hiện không truy cập được", "This content is unavailable"),
            Error::Format => app.t("Không đọc được phản hồi từ YouTube Music", "Could not read the YouTube Music response"),
        };
        ui.label(RichText::new(label).color(SUBTLE));
        if ui.button(app.t("Thử lại", "Retry")).clicked() {
            on_retry();
            clicked = true;
        }
    });
    clicked
}

fn navigate(app: &mut App, item: &Item) {
    app.actions.push(Action::Navigate(Route::YouTubeMusicItem(item.clone())));
}

pub(super) fn item_menu(app: &mut App, ui: &mut Ui, item: &Item) {
    ui.set_min_width(198.0);
    if let ItemId::Song(id) = &item.id {
        ui.menu_button(app.t("Thêm vào danh sách phát", "Add to playlist"), |ui| {
            if ui.button(app.t("Danh sách phát mới", "New playlist")).clicked() {
                app.actions.push(Action::CreatePlaylistWithSong(item.clone()));
                ui.close();
            }
            for playlist in &app.data.playlists {
                if ui.button(&playlist.name).clicked() {
                    app.actions.push(Action::AddYouTubeSong(playlist.id, item.clone()));
                    ui.close();
                }
            }
        });
        let liked = app.data.is_liked(crate::library::fnv(&format!("yt:{id}")));
        let label = if liked {
            app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs")
        } else {
            app.t("Thêm vào Bài hát đã thích", "Save to Liked Songs")
        };
        if ui.button(label).clicked() {
            app.actions.push(Action::LikeYouTubeSong(item.clone()));
            ui.close();
        }
        if ui.button(app.t("Thêm vào danh sách chờ", "Add to queue")).clicked() {
            app.actions.push(Action::EnqueueYouTubeSong(item.clone()));
            ui.close();
        }
        ui.separator();
    }
    if ui.button(app.t("Sao chép liên kết", "Copy link")).clicked() {
        ui.ctx().copy_text(item.id.url());
        ui.close();
    }
    if ui.button(app.t("Mở trên YouTube Music", "Open on YouTube Music")).clicked() {
        crate::ytm::open_browser(&item.id);
        ui.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discography_groups_singles_and_eps_without_guessing_from_track_count() {
        assert!(discography_filter_includes(1, Some(&ReleaseKind::Album), false));
        assert!(!discography_filter_includes(1, Some(&ReleaseKind::EP), true));
        assert!(discography_filter_includes(2, Some(&ReleaseKind::Single), true));
        assert!(discography_filter_includes(2, Some(&ReleaseKind::EP), true));
        assert!(!discography_filter_includes(2, Some(&ReleaseKind::Album), false));
        assert!(!discography_filter_includes(2, None, false));
        assert!(discography_filter_includes(0, None, false));
        assert!(discography_filter_includes(2, None, true));
        assert!(!discography_filter_includes(1, None, true));
    }

    #[test]
    fn search_stacks_top_result_until_four_columns_fit() {
        assert!(!search_results_side_by_side(602.0));
        assert!(!search_results_side_by_side(779.0));
        assert!(search_results_side_by_side(780.0));
    }

    #[test]
    fn recommendations_exclude_hidden_and_previously_played_songs() {
        let item: Item = serde_json::from_value(serde_json::json!({
            "id": {"Song": "poGyHfrJ_uo"}, "title": "Song", "by": "Artist", "image": null
        })).unwrap();
        let id = crate::library::fnv("yt:poGyHfrJ_uo");
        let mut data = crate::library::UserData::default();
        assert!(home_item_visible(&data, &item, true));
        data.listening_history.push(id);
        assert!(!home_item_visible(&data, &item, true));
        assert!(home_item_visible(&data, &item, false));
        data.hidden_tracks.insert(id);
        assert!(!home_item_visible(&data, &item, false));
        assert!(!home_item_visible(&data, &item, true));
    }

    #[test]
    fn browse_all_breakpoints_match_spotify_spec() {
        assert_eq!(browse_all_grid_layout(400.0), (2, 180.0));
        assert_eq!(browse_all_grid_layout(800.0), (3, 254.0));
        assert_eq!(browse_all_grid_layout(1400.0), (4, 300.0));
    }

    #[test]
    fn format_artist_subtitle_and_play_counts_localize_vietnamese_and_english() {
        assert_eq!(format_artist_subtitle(crate::library::Lang::Vi, "77,688,295 views"), "77,7 Tr lượt nghe");
        assert_eq!(format_artist_subtitle(crate::library::Lang::En, "77,688,295 views"), "77.7M plays");
        assert_eq!(format_artist_subtitle(crate::library::Lang::Vi, "1.8M monthly audience"), "1,8 Tr người nghe hằng tháng");
        assert_eq!(format_artist_subtitle(crate::library::Lang::Vi, "684.9K subscribers"), "684,9 N người theo dõi");
        assert_eq!(format_track_plays(crate::library::Lang::Vi, "63M plays"), "63 Tr");
        assert_eq!(format_track_plays(crate::library::Lang::Vi, "27,289,268"), "27.289.268");
    }
}
