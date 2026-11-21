//! The main area: home, search, playlists, albums, artists, liked songs, and shared Spotify layouts.

use std::cmp::Ordering;

use chrono::{Datelike, Local, TimeZone, Utc};
use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align2, Color32, ColorImage, Id, Painter, Pos2, Rect, ScrollArea, Sense,
    Stroke, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2,
};

use super::icons::{self, Icon};
use super::sidebar::route_of;
use super::widgets::*;
use crate::app::{Action, App, ConfirmAction, HomeFilter, Route};
use crate::library::{fold, split_artists, Lang, RecentItem, TableSort, TrackId, TrackSortColumn};
use crate::queue::ContextRef;
use crate::theme::*;

#[derive(Clone)]
pub struct Item {
    pub kind: RecentItem,
    pub title: String,
    pub sub: String,
    pub cover_key: String,
    pub cover_path: Option<String>,
    pub circle: bool,
    pub route: Route,
}

#[derive(Clone)]
pub(super) struct CardData {
    pub title: String,
    pub sub: String,
    pub cover_key: String,
    pub cover_path: Option<String>,
    pub circle: bool,
    pub liked_tile: bool,
    pub route: Route,
    pub play_kind: Option<RecentItem>,
    pub ytm_item: Option<crate::ytm::Item>,
}

impl CardData {
    pub(super) fn from_local(item: &Item) -> Self {
        Self {
            title: item.title.clone(),
            sub: item.sub.clone(),
            cover_key: item.cover_key.clone(),
            cover_path: item.cover_path.clone(),
            circle: item.circle,
            liked_tile: item.kind == RecentItem::Liked,
            route: item.route.clone(),
            play_kind: Some(item.kind.clone()),
            ytm_item: None,
        }
    }

    pub(super) fn from_ytm(app: &App, item: &crate::ytm::Item, page: Option<&crate::ytm::Page>) -> Self {
        let kind_label = ytm_kind_label(app, item, page);
        let sub = if item.by.is_empty() {
            kind_label.to_string()
        } else {
            format!("{kind_label} • {}", item.by)
        };
        Self {
            title: item.title.clone(),
            sub,
            cover_key: item.cover_key(),
            cover_path: item.image.clone(),
            circle: matches!(item.id, crate::ytm::ItemId::Artist(_)),
            liked_tile: false,
            route: Route::YouTubeMusicItem(item.clone()),
            play_kind: None,
            ytm_item: Some(item.clone()),
        }
    }
}

pub(super) struct HomeShelf {
    pub id: String,
    pub title: String,
    pub cards: Vec<CardData>,
}

pub(super) fn ytm_kind_label(app: &App, item: &crate::ytm::Item, page: Option<&crate::ytm::Page>) -> &'static str {
    match item.id {
        crate::ytm::ItemId::Song(_) => app.t("Bài hát", "Song"),
        crate::ytm::ItemId::Album(_) => match page.and_then(|p| p.release_kind(&item.id)) {
            Some(crate::ytm::ReleaseKind::Single) => app.t("Đĩa đơn", "Single"),
            Some(crate::ytm::ReleaseKind::EP) => "EP",
            None if page.is_some_and(|p| p.is_single_or_ep(&item.id)) => app.t("Đĩa đơn / EP", "Single / EP"),
            _ => "Album",
        },
        crate::ytm::ItemId::Artist(_) => app.t("Nghệ sĩ", "Artist"),
        crate::ytm::ItemId::Playlist(_) => "Playlist",
    }
}

pub fn item_of(app: &App, kind: &RecentItem) -> Option<Item> {
    let route = route_of(app, kind)?;
    match kind {
        RecentItem::Liked => Some(Item {
            kind: kind.clone(),
            title: app.t("Bài hát đã thích", "Liked Songs").to_string(),
            sub: format!(
                "{} • {} {}",
                app.t("Danh sách phát", "Playlist"),
                app.data.liked.iter().filter(|e| app.lib.tracks.contains_key(&e.id)).count(),
                app.t("bài hát", "songs")
            ),
            cover_key: "liked".into(),
            cover_path: None,
            circle: false,
            route,
        }),
        RecentItem::Playlist(id) => {
            let p = app.data.playlist(*id)?;
            let first = p.tracks.iter().find_map(|e| app.lib.track(e.id));
            let path = p
                .image
                .clone()
                .or_else(|| first.as_ref().and_then(|t| t.art_source().map(str::to_string)));
            Some(Item {
                kind: kind.clone(),
                title: p.name.clone(),
                sub: format!(
                    "{} • {} {}",
                    app.t("Danh sách phát", "Playlist"),
                    p.tracks.iter().filter(|e| app.lib.tracks.contains_key(&e.id)).count(),
                    app.t("bài", "songs")
                ),
                cover_key: if p.image.is_some() {
                    format!("pl:{id}")
                } else {
                    first.map(|t| t.cover_key()).unwrap_or_else(|| format!("pl:{id}"))
                },
                cover_path: path,
                circle: false,
                route,
            })
        }
        RecentItem::Album(key) => {
            let a = app.lib.album(key)?;
            Some(Item {
                kind: kind.clone(),
                title: a.title.clone(),
                sub: format!("{} • {}", app.t("Album", "Album"), a.artist),
                cover_key: a.key.clone(),
                cover_path: a.cover_path.clone(),
                circle: false,
                route,
            })
        }
        RecentItem::Artist(name) => {
            if let Some(av) = app.data.artist_avatar(name) {
                Some(Item {
                    kind: kind.clone(),
                    title: name.clone(),
                    sub: app.t("Nghệ sĩ", "Artist").to_string(),
                    cover_key: crate::art::cover_key(av),
                    cover_path: Some(av.to_string()),
                    circle: true,
                    route,
                })
            } else if let Some(a) = app.lib.artist(name) {
                // In live YTM mode, never stamp a shared collab song's cover art onto an artist;
                // let sidebar/profile hydrate the artist's real portrait via `artist_profile_cached`.
                let has_local_file = a.tracks.iter().any(|id| {
                    app.lib.track(*id).is_some_and(|t| !t.path.starts_with("yt:") && !t.path.starts_with("demo:"))
                });
                let use_track_art = app.lib.demo || has_local_file;
                Some(Item {
                    kind: kind.clone(),
                    title: a.name.clone(),
                    sub: app.t("Nghệ sĩ", "Artist").to_string(),
                    cover_key: if use_track_art { a.cover_key.clone() } else { format!("ytm-artist:{}", fold(&a.name)) },
                    cover_path: if use_track_art { a.cover_path.clone() } else { None },
                    circle: true,
                    route,
                })
            } else if app.data.followed_artists.contains(name) {
                Some(Item {
                    kind: kind.clone(),
                    title: name.clone(),
                    sub: app.t("Nghệ sĩ", "Artist").to_string(),
                    cover_key: format!("ytm-artist:{}", fold(name)),
                    cover_path: None,
                    circle: true,
                    route,
                })
            } else {
                None
            }
        }
        RecentItem::FollowedYouTube(id) => {
            let Route::YouTubeMusicItem(yt_item) = &route else {
                return None;
            };
            let is_album = id.starts_with("album:");
            let sub = if is_album {
                if yt_item.by.is_empty() {
                    "Album".to_string()
                } else {
                    format!("Album • {}", yt_item.by)
                }
            } else {
                app.t("Playlist đang theo dõi", "Followed playlist").into()
            };
            Some(Item {
                kind: kind.clone(),
                title: yt_item.title.clone(),
                sub,
                cover_key: yt_item.cover_key(),
                cover_path: yt_item.image.clone(),
                circle: false,
                route,
            })
        }
    }
}

pub fn paint_item_cover(app: &mut App, ui: &Ui, rect: Rect, item: &Item) {
    if item.kind == RecentItem::Liked {
        paint_liked_tile(ui, rect, 6.0);
        return;
    }
    let tex = app.art.get(ui.ctx(), &item.cover_key, item.cover_path.as_deref());
    cover(ui, rect, &tex, if item.circle { rect.width() * 0.5 } else { 6.0 });
}

pub(super) fn paint_liked_tile(ui: &Ui, rect: Rect, radius: f32) {
    let tex = liked_tile_texture(ui.ctx());
    cover(ui, rect, &tex, radius);
    let heart_size = (rect.width() * 0.42).clamp(14.0, 96.0);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(heart_size)),
        Icon::HeartFilled,
        TEXT,
    );
}

fn liked_tile_texture(ctx: &egui::Context) -> TextureHandle {
    let id = Id::NULL.with("liked-songs-tile-grad-tex");
    if let Some(tex) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return tex;
    }
    const N: usize = 64;
    let c0 = Color32::from_rgb(0x45, 0x0a, 0xf5);
    let c1 = Color32::from_rgb(0xc4, 0xef, 0xd9);
    let mut pixels = Vec::with_capacity(N * N);
    for y in 0..N {
        for x in 0..N {
            let t = ((x + y) as f32 / (2.0 * (N - 1) as f32)).clamp(0.0, 1.0);
            pixels.push(c0.lerp_to_gamma(c1, t));
        }
    }
    let tex = ctx.load_texture(
        "liked-songs-tile-grad",
        ColorImage::new([N, N], pixels),
        TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

pub(super) fn paint_card_cover(app: &mut App, ui: &Ui, rect: Rect, card: &CardData, radius: f32) {
    if card.liked_tile {
        paint_liked_tile(ui, rect, radius);
        return;
    }
    let tex = app.art.get(ui.ctx(), &card.cover_key, card.cover_path.as_deref());
    let r = if card.circle { rect.width() * 0.5 } else { radius };
    cover(ui, rect, &tex, r);
}

pub(super) fn adaptive_hero_color(app: &App, cover_key: &str, liked: bool) -> Color32 {
    if liked {
        return Color32::from_rgb(0x50, 0x38, 0xa0);
    }
    let raw = app.art.lyric_colors(cover_key)[0];
    normalize_hero_color(raw)
}

fn normalize_hero_color(raw: Color32) -> Color32 {
    let r = raw.r() as f32;
    let g = raw.g() as f32;
    let b = raw.b() as f32;
    let lum = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0;
    let factor = if lum > 0.55 {
        0.48 / lum
    } else if lum < 0.10 {
        1.4
    } else {
        0.78
    };
    Color32::from_rgb(
        (r * factor).clamp(20.0, 210.0) as u8,
        (g * factor).clamp(20.0, 210.0) as u8,
        (b * factor).clamp(20.0, 210.0) as u8,
    )
}

/// Full-bleed smooth gradient wash from `top_color` into `PANEL` (`#121212`) with rounded top corners.
pub(super) fn draw_full_bleed_gradient(p: &Painter, rect: Rect, top_color: Color32, target: Color32) {
    if rect.height() <= 0.0 || rect.width() <= 0.0 {
        return;
    }
    let r = (RADIUS as f32).min(rect.height() * 0.5);
    let top_strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), r * 2.0));
    p.rect_filled(top_strip, RADIUS, top_color);
    let mid_color = top_color.lerp_to_gamma(target, 0.52);
    let mid_y = rect.top() + rect.height() * 0.55;
    let upper = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + r), Pos2::new(rect.right(), mid_y));
    let lower = Rect::from_min_max(Pos2::new(rect.left(), mid_y), rect.max);
    if upper.height() > 0.0 {
        vertical_gradient(p, upper, top_color, mid_color);
    }
    if lower.height() > 0.0 {
        vertical_gradient(p, lower, mid_color, target);
    }
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    match app.route.clone() {
        Route::Home if app.lib.demo => home(app, ui),
        Route::Home | Route::Search | Route::YouTubeMusicItem(_) => super::ytm::draw(app, ui),
        Route::Settings => super::settings::draw(app, ui),
        Route::Profile | Route::TasteProfile => super::profile::draw(app, ui),
        Route::Lyrics => super::lyrics::draw(app, ui),
        Route::Liked => liked(app, ui),
        Route::Playlist(id) => playlist(app, ui, id),
        Route::Album(key) => album(app, ui, &key),
        Route::Artist(name) => artist(app, ui, &name),
    }
}

// ---------------------------------------------------------------------------------------------
// Home (shared by Demo & Live YouTube Music)

pub(super) fn home_content_padding(width: f32) -> f32 {
    if width < 600.0 { 16.0 } else { 40.0 }
}

pub(super) fn quick_grid_layout(width: f32) -> (usize, f32, f32) {
    if width < 600.0 {
        (2, 48.0, 8.0)
    } else if width < 815.0 {
        (3, 48.0, 8.0)
    } else if width < 1141.0 {
        (4, 48.0, 8.0)
    } else if width < 1599.0 {
        (4, 64.0, 12.0)
    } else {
        (4, 80.0, 12.0)
    }
}

fn home(app: &mut App, ui: &mut Ui) {
    let mut quick_items: Vec<RecentItem> = app.data.recent.iter().take(8).cloned().collect();
    if !quick_items.contains(&RecentItem::Liked) {
        quick_items.insert(0, RecentItem::Liked);
    }
    quick_items.truncate(8);
    for p in app.data.playlists.iter().map(|p| RecentItem::Playlist(p.id)) {
        if quick_items.len() >= 8 {
            break;
        }
        if !quick_items.contains(&p) {
            quick_items.push(p);
        }
    }
    for a in app.lib.albums.iter().map(|a| RecentItem::Album(a.key.clone())) {
        if quick_items.len() >= 8 {
            break;
        }
        if !quick_items.contains(&a) {
            quick_items.push(a);
        }
    }

    let quick: Vec<CardData> = quick_items
        .iter()
        .filter_map(|k| item_of(app, k))
        .map(|it| CardData::from_local(&it))
        .collect();
    let recent: Vec<CardData> = app
        .data
        .recent
        .iter()
        .filter_map(|k| item_of(app, k))
        .take(12)
        .map(|it| CardData::from_local(&it))
        .collect();
    let albums: Vec<CardData> = app
        .lib
        .albums
        .iter()
        .filter_map(|a| item_of(app, &RecentItem::Album(a.key.clone())))
        .take(12)
        .map(|it| CardData::from_local(&it))
        .collect();
    let artists: Vec<CardData> = app
        .lib
        .artists
        .iter()
        .filter_map(|a| item_of(app, &RecentItem::Artist(a.name.clone())))
        .take(12)
        .map(|it| CardData::from_local(&it))
        .collect();
    let playlists: Vec<CardData> = app
        .data
        .playlists
        .iter()
        .filter_map(|p| item_of(app, &RecentItem::Playlist(p.id)))
        .take(12)
        .map(|it| CardData::from_local(&it))
        .collect();

    let mut shelves = Vec::new();
    let filter = app.home_filter;
    if matches!(filter, HomeFilter::All | HomeFilter::Music) {
        shelves.push(HomeShelf {
            id: "home-recent".into(),
            title: app.t("Phát gần đây", "Recently played").to_string(),
            cards: recent,
        });
    }
    if matches!(filter, HomeFilter::All | HomeFilter::Music | HomeFilter::Albums) {
        shelves.push(HomeShelf {
            id: "home-albums".into(),
            title: app.t("Album của bạn", "Your albums").to_string(),
            cards: albums,
        });
    }
    if matches!(filter, HomeFilter::All | HomeFilter::Artists) {
        shelves.push(HomeShelf {
            id: "home-artists".into(),
            title: app.t("Nghệ sĩ của bạn", "Your artists").to_string(),
            cards: artists,
        });
    }
    if matches!(filter, HomeFilter::All | HomeFilter::Music | HomeFilter::Playlists) {
        shelves.push(HomeShelf {
            id: "home-playlists".into(),
            title: app.t("Danh sách phát của bạn", "Your playlists").to_string(),
            cards: playlists,
        });
    }

    draw_home_page(app, ui, "home", &quick, &shelves, None);
}

pub(super) fn draw_home_page(
    app: &mut App,
    ui: &mut Ui,
    scroll_id: &str,
    quick: &[CardData],
    shelves: &[HomeShelf],
    load_more: Option<(bool, &str)>,
) -> bool {
    let mut load_more_clicked = false;
    let scroll_to_top = app.home_scroll_to_top;
    if scroll_to_top {
        app.home_scroll_to_top = false;
    }

    let hover_state_id = Id::NULL.with("home-ambient-hover");
    let color_state_id = Id::NULL.with("home-ambient-color");
    let prev_hover: Option<(String, bool)> = ui.ctx().data(|d| d.get_temp(hover_state_id));
    let target_color = if let Some((key, liked)) = &prev_hover {
        adaptive_hero_color(app, key, *liked)
    } else if let Some(first) = quick.first() {
        adaptive_hero_color(app, &first.cover_key, first.liked_tile)
    } else {
        Color32::from_rgb(0x2a, 0x2a, 0x2a)
    };

    let current_color = animate_ambient_color(ui, color_state_id, target_color, app.data.settings.effective_lyrics_motion());

    let mut scroll = ScrollArea::vertical().id_salt(scroll_id).auto_shrink([false, false]);
    if scroll_to_top {
        scroll = scroll.vertical_scroll_offset(0.0);
    }

    scroll.show_viewport(ui, |ui, vp| {
        let w = ui.available_width();
        let side = home_content_padding(w);
        let inner_w = (w - side * 2.0).max(180.0);
        let scroll_y = vp.min.y.max(0.0);
        let origin = ui.cursor().min;

        // Paint 256px full-bleed adaptive wash at top of scroll content.
        let wash_rect = Rect::from_min_size(origin, Vec2::new(w, 256.0));
        // Spotify dwp-home-header.css overlays the ambient cover color with #0009.
        let fade_bottom = if app.data.settings.surface_style == crate::library::SurfaceStyle::Standard {
            PANEL
        } else {
            Color32::TRANSPARENT
        };
        draw_full_bleed_gradient(ui.painter(), wash_rect, current_color.lerp_to_gamma(Color32::BLACK, 0.6), fade_bottom);

        let chips_h = 64.0;
        let show_quick = matches!(app.home_filter, HomeFilter::All | HomeFilter::Music) && !quick.is_empty();
        let (cols, tile_h, tile_gap) = quick_grid_layout(inner_w);
        let quick_rows = if show_quick { quick.len().div_ceil(cols) } else { 0 };
        let quick_h = if quick_rows > 0 {
            quick_rows as f32 * tile_h + (quick_rows.saturating_sub(1)) as f32 * tile_gap + 24.0
        } else {
            0.0
        };

        let body_top = origin.y + chips_h + 4.0;
        let sticky_bottom = origin.y + scroll_y + chips_h;
        let mut next_hover: Option<(String, bool)> = None;

        // Reserve space for sticky chips row + render scrollable body clipped below sticky chips bar.
        ui.add_space(chips_h + 4.0);
        let body_clip = Rect::from_min_max(
            Pos2::new(origin.x, sticky_bottom.max(body_top)),
            Pos2::new(origin.x + w, origin.y + vp.max.y + 8.0),
        );

        ui.scope_builder(UiBuilder::new().id_salt("home-body"), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body_clip));
            if show_quick {
                let tile_w = (inner_w - (cols as f32 - 1.0) * tile_gap) / cols as f32;
                let (grid, _) = ui.allocate_exact_size(Vec2::new(w, quick_h - 24.0), Sense::hover());
                for (i, card) in quick.iter().enumerate() {
                    let (c, r) = (i % cols, i / cols);
                    let tile = Rect::from_min_size(
                        Pos2::new(
                            grid.left() + side + c as f32 * (tile_w + tile_gap),
                            grid.top() + r as f32 * (tile_h + tile_gap),
                        ),
                        Vec2::new(tile_w, tile_h),
                    );
                    if quick_tile_card(app, ui, tile, card, i) {
                        next_hover = Some((card.cover_key.clone(), card.liked_tile));
                    }
                }
                ui.add_space(24.0);
            } else {
                ui.add_space(8.0);
            }

            for shelf_data in shelves {
                if !shelf_data.cards.is_empty() {
                    draw_carousel_shelf(app, ui, &shelf_data.id, &shelf_data.title, &shelf_data.cards, side);
                }
            }

            if let Some((pending, label)) = load_more {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    let btn = ui.add_enabled(!pending, egui::Button::new(label));
                    if btn.clicked() {
                        load_more_clicked = true;
                    }
                });
            }
            ui.add_space(24.0);
        });

        ui.ctx().data_mut(|d| d.insert_temp(hover_state_id, next_hover));

        // Sticky 56px Home Filter Chips Bar (respects translucent Mica/Acrylic when enabled).
        let sticky_rect = Rect::from_min_size(Pos2::new(origin.x, origin.y + scroll_y), Vec2::new(w, chips_h));
        let fade = (scroll_y / 36.0).clamp(0.0, 1.0);
        if fade > 0.01 {
            let max_a = if app.data.settings.surface_style == crate::library::SurfaceStyle::Standard {
                245.0
            } else {
                (app.data.settings.panel_opacity.clamp(0.10, 0.95) * 215.0).clamp(48.0, 220.0)
            };
            let alpha = (fade * max_a).round() as u8;
            ui.painter().rect_filled(sticky_rect, 0.0, Color32::from_rgba_unmultiplied(PANEL.r(), PANEL.g(), PANEL.b(), alpha));
            if fade > 0.6 {
                ui.painter().hline(sticky_rect.x_range(), sticky_rect.bottom(), Stroke::new(1.0, HAIRLINE));
            }
        }
        let mut chips_ui = ui.new_child(UiBuilder::new().id_salt("home-sticky-chips").max_rect(sticky_rect));
        let chips_row = Rect::from_min_max(sticky_rect.min + Vec2::new(0.0, 8.0), sticky_rect.max);
        home_chips_bar(app, &mut chips_ui, chips_row, side);
    });

    load_more_clicked
}

fn step_ambient_rgb(cur: [f32; 3], target: [f32; 3], dt: f32) -> ([f32; 3], bool) {
    let dt = dt.clamp(0.004, 0.08);
    let k = 1.0 - (-dt * 5.0).exp();
    let mut next = [0.0_f32; 3];
    let mut max_diff = 0.0_f32;
    for i in 0..3 {
        next[i] = cur[i] + (target[i] - cur[i]) * k;
        max_diff = max_diff.max((target[i] - next[i]).abs());
    }
    if max_diff < 0.5 {
        (target, false)
    } else {
        (next, true)
    }
}

fn animate_ambient_color(ctx_ui: &Ui, id: Id, target: Color32, animate: bool) -> Color32 {
    let target_rgb = [target.r() as f32, target.g() as f32, target.b() as f32];
    let cur = ctx_ui.ctx().data(|d| d.get_temp::<[f32; 3]>(id)).unwrap_or(target_rgb);
    if !animate {
        ctx_ui.ctx().data_mut(|d| d.insert_temp(id, target_rgb));
        return target;
    }
    let dt = ctx_ui.input(|i| i.unstable_dt);
    let (next, animating) = step_ambient_rgb(cur, target_rgb, dt);
    ctx_ui.ctx().data_mut(|d| d.insert_temp(id, next));
    if animating && !ctx_ui.input(|i| i.viewport().minimized.unwrap_or(false)) {
        ctx_ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    Color32::from_rgb(
        next[0].round().clamp(0.0, 255.0) as u8,
        next[1].round().clamp(0.0, 255.0) as u8,
        next[2].round().clamp(0.0, 255.0) as u8,
    )
}

fn home_chips_bar(app: &mut App, ui: &mut Ui, row: Rect, side: f32) {
    let chips = [
        (HomeFilter::All, app.t("Tất cả", "All")),
        (HomeFilter::Music, app.t("Âm nhạc", "Music")),
        (HomeFilter::Albums, app.t("Album", "Albums")),
        (HomeFilter::Artists, app.t("Nghệ sĩ", "Artists")),
        (HomeFilter::Playlists, "Playlist"),
    ];
    let mut x = row.left() + side;
    let y = row.center().y - 16.0;
    for (filter, label) in chips {
        let w = pill_width(ui, label);
        if x + w > row.right() - side + 4.0 {
            break;
        }
        let chip = Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, 32.0));
        if pill(ui, chip, ("home-chip", label), label, app.home_filter == filter).clicked() {
            app.actions.push(Action::SetHomeFilter(filter));
        }
        x += w + 8.0;
    }
}

fn activate_card(app: &mut App, card: &CardData) {
    // Clicking a Song card on Home / Quick Picks plays it directly instead of opening a bare Song/Lyrics route.
    if let Some(item) = &card.ytm_item {
        if let crate::ytm::ItemId::Song(vid) = &item.id {
            let sid = crate::library::fnv(&format!("yt:{vid}"));
            if app.player.current == Some(sid) {
                app.actions.push(Action::TogglePlay);
            } else {
                app.actions.push(Action::SelectYouTubeSong(item.clone()));
            }
            return;
        }
    }
    app.actions.push(Action::Navigate(card.route.clone()));
}

fn quick_tile_card(app: &mut App, ui: &mut Ui, r: Rect, card: &CardData, i: usize) -> bool {
    let resp = ui.interact(r, ui.scope_id().with(("quick", i)), Sense::click());
    let hovered = ui.rect_contains_pointer(r) || resp.has_focus();
    focus_ring(ui, &resp, 4.0);
    ui.painter().rect_filled(r, RADIUS_BASE, if hovered { hover_white(51) } else { hover_white(26) });

    let c = Rect::from_min_size(r.min, Vec2::splat(r.height()));
    paint_card_cover(app, ui, c, card, RADIUS_BASE);

    let playing = card_is_playing(app, card);
    let btn_size = if r.height() >= 64.0 { 40.0 } else { 32.0 };
    let pad_x = if r.height() >= 64.0 { 16.0 } else { 10.0 };
    let right_reserve = if hovered || playing { btn_size + 14.0 } else { 12.0 };
    let room = (r.width() - r.height() - pad_x - right_reserve).max(10.0);
    wrapped_text(
        ui.painter(),
        Pos2::new(c.right() + pad_x, r.center().y),
        &card.title,
        semi(if r.height() >= 64.0 { 14.5 } else { 13.5 }),
        TEXT,
        room,
        2,
        Align2::LEFT_CENTER,
    );

    if resp.clicked() {
        activate_card(app, card);
    }
    if let Some(ytm_item) = &card.ytm_item {
        resp.context_menu(|ui| super::ytm::item_menu(app, ui, ytm_item));
    } else if let Some(kind) = &card.play_kind {
        resp.context_menu(|ui| local_item_menu(app, ui, kind));
    }

    let btn_rect = Rect::from_center_size(Pos2::new(r.right() - btn_size * 0.5 - 10.0, r.center().y), Vec2::splat(btn_size));
    if hovered {
        card_play_button(app, ui, btn_rect, card, ("quick-play", i), true);
    } else if playing {
        equalizer_glyph(ui, btn_rect, ui.input(|inp| inp.time));
    }
    hovered
}

pub(super) fn card_is_playing(app: &App, card: &CardData) -> bool {
    if let Some(kind) = &card.play_kind {
        if let Some(ctx) = context_for_recent(kind) {
            return app.context_is_playing(&ctx);
        }
    }
    if let Some(item) = &card.ytm_item {
        if let crate::ytm::ItemId::Song(vid) = &item.id {
            let id = crate::library::fnv(&format!("yt:{vid}"));
            return app.is_playing() && app.player.current == Some(id);
        }
    }
    false
}

fn context_for_recent(kind: &RecentItem) -> Option<ContextRef> {
    Some(match kind {
        RecentItem::Liked => ContextRef::Liked,
        RecentItem::Playlist(p) => ContextRef::Playlist(*p),
        RecentItem::Album(k) => ContextRef::Album(k.clone()),
        RecentItem::Artist(n) => ContextRef::Artist(n.clone()),
        RecentItem::FollowedYouTube(_) => return None,
    })
}

pub(super) fn card_play_button(
    app: &mut App,
    ui: &mut Ui,
    rect: Rect,
    card: &CardData,
    id: impl std::hash::Hash + std::fmt::Debug,
    visible: bool,
) {
    let playing = card_is_playing(app, card);
    let tip = if playing {
        app.t("Tạm dừng", "Pause")
    } else {
        app.t("Phát", "Play")
    };
    if hover_play_button(ui, rect, id, playing, visible, tip).clicked() {
        if let Some(kind) = &card.play_kind {
            if matches!(kind, RecentItem::FollowedYouTube(_)) {
                app.actions.push(Action::Navigate(card.route.clone()));
            } else {
                app.actions.push(Action::PlayItem(kind.clone()));
            }
        } else if let Some(item) = &card.ytm_item {
            match &item.id {
                crate::ytm::ItemId::Song(vid) => {
                    let sid = crate::library::fnv(&format!("yt:{vid}"));
                    if app.player.current == Some(sid) {
                        app.actions.push(Action::TogglePlay);
                    } else {
                        app.actions.push(Action::SelectYouTubeSong(item.clone()));
                    }
                }
                _ => {
                    app.actions.push(Action::Navigate(Route::YouTubeMusicItem(item.clone())));
                }
            }
        }
    }
}

pub(super) fn draw_carousel_shelf(app: &mut App, ui: &mut Ui, id_salt: &str, title: &str, cards: &[CardData], side: f32) {
    if cards.is_empty() {
        return;
    }
    let full_w = ui.available_width();
    let inner_w = (full_w - side * 2.0).max(160.0);
    let gap = 12.0;
    let vis_cols = (((inner_w + gap) / (164.0 + gap)).floor() as usize).clamp(2, 8);
    let card_w = (inner_w - (vis_cols as f32 - 1.0) * gap) / vis_cols as f32;
    let card_h = card_w + 62.0;

    let state_id = ui.scope_id().with(("shelf-carousel-state", id_salt));
    let (mut show_all, mut offset, mut target) = ui
        .ctx()
        .data(|d| d.get_temp::<(bool, f32, f32)>(state_id))
        .unwrap_or((false, 0.0, 0.0));

    let (head, _) = ui.allocate_exact_size(Vec2::new(full_w, 40.0), Sense::hover());
    let show_all_label = if show_all {
        app.t("Thu gọn", "Show less")
    } else {
        app.t("Hiện tất cả", "Show all")
    };
    let toggle_w = text_width(ui, show_all_label, semi(13.0)) + 8.0;
    text(
        ui.painter(),
        Pos2::new(head.left() + side, head.center().y),
        title,
        bold(24.0),
        TEXT,
        (inner_w - toggle_w - 12.0).max(40.0),
        Align2::LEFT_CENTER,
    );
    if cards.len() > vis_cols || show_all {
        let link_rect = Rect::from_min_size(
            Pos2::new(head.right() - side - toggle_w, head.center().y - 12.0),
            Vec2::new(toggle_w, 24.0),
        );
        let resp = ui.interact(link_rect, ui.scope_id().with(("show-all", id_salt)), Sense::click());
        focus_ring(ui, &resp, 4.0);
        let color = if resp.hovered() || resp.has_focus() { TEXT } else { SUBTLE };
        text(ui.painter(), link_rect.right_center(), show_all_label, semi(13.0), color, toggle_w, Align2::RIGHT_CENTER);
        if resp.hovered() {
            ui.painter().hline(link_rect.x_range(), link_rect.bottom() - 3.0, Stroke::new(1.0, TEXT));
        }
        if resp.clicked() {
            show_all = !show_all;
        }
    }

    if show_all {
        draw_card_grid(app, ui, id_salt, cards, side, vis_cols, card_w, card_h, gap);
    } else {
        let max_offset = (cards.len() as f32 - vis_cols as f32).max(0.0);
        target = target.clamp(0.0, max_offset);
        if app.data.settings.effective_lyrics_motion() {
            let dt = ui.input(|i| i.unstable_dt).clamp(0.008, 0.08);
            let step = (dt * 12.0).clamp(0.0, 1.0);
            offset += (target - offset) * step;
            if (target - offset).abs() < 0.005 {
                offset = target;
            } else {
                ui.ctx().request_repaint();
            }
        } else {
            offset = target;
        }

        let (row, _) = ui.allocate_exact_size(Vec2::new(full_w, card_h), Sense::hover());
        let clip = Rect::from_min_size(Pos2::new(row.left() + side, row.top()), Vec2::new(inner_w, card_h));
        let row_hovered = ui.rect_contains_pointer(row);

        ui.scope_builder(UiBuilder::new().id_salt(("shelf-track", id_salt)), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(clip));
            for (i, card) in cards.iter().enumerate() {
                let x = clip.left() + (i as f32 - offset) * (card_w + gap);
                if x + card_w < clip.left() - 4.0 || x > clip.right() + 4.0 {
                    continue;
                }
                let r = Rect::from_min_size(Pos2::new(x, row.top()), Vec2::new(card_w, card_h));
                draw_shelf_card(app, ui, r, card, (id_salt, i));
            }
        });

        let art_center_y = row.top() + 12.0 + (card_w - 24.0) * 0.5;
        if row_hovered && target > 0.01 {
            let left_btn = Rect::from_center_size(Pos2::new(clip.left() + 18.0, art_center_y), Vec2::splat(32.0));
            if carousel_arrow(ui, left_btn, ("carousel-prev", id_salt), Icon::Back).clicked() {
                target = (target - 3.0).max(0.0);
            }
        }
        if row_hovered && target < max_offset - 0.01 {
            let right_btn = Rect::from_center_size(Pos2::new(clip.right() - 18.0, art_center_y), Vec2::splat(32.0));
            if carousel_arrow(ui, right_btn, ("carousel-next", id_salt), Icon::Forward).clicked() {
                target = (target + 3.0).min(max_offset);
            }
        }
    }

    ui.ctx().data_mut(|d| d.insert_temp(state_id, (show_all, offset, target)));
    ui.add_space(20.0);
}

fn draw_card_grid(
    app: &mut App,
    ui: &mut Ui,
    id_salt: &str,
    cards: &[CardData],
    side: f32,
    cols: usize,
    card_w: f32,
    card_h: f32,
    gap: f32,
) {
    let full_w = ui.available_width();
    let rows = cards.len().div_ceil(cols);
    let total_h = rows as f32 * card_h + (rows.saturating_sub(1)) as f32 * gap;
    let (grid, _) = ui.allocate_exact_size(Vec2::new(full_w, total_h), Sense::hover());
    for (i, card) in cards.iter().enumerate() {
        let (c, r) = (i % cols, i / cols);
        let rect = Rect::from_min_size(
            Pos2::new(grid.left() + side + c as f32 * (card_w + gap), grid.top() + r as f32 * (card_h + gap)),
            Vec2::new(card_w, card_h),
        );
        if ui.is_rect_visible(rect) {
            draw_shelf_card(app, ui, rect, card, (id_salt, "grid", i));
        }
    }
}

fn carousel_arrow(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: Icon) -> egui::Response {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
    focus_ring(ui, &resp, rect.width() * 0.5);
    let fill = if resp.hovered() || resp.has_focus() { HOVER } else { Color32::from_black_alpha(205) };
    ui.painter().circle_filled(rect.center(), rect.width() * 0.5, fill);
    ui.painter().circle_stroke(rect.center(), rect.width() * 0.5, Stroke::new(1.0, hover_white(28)));
    icons::paint(
        ui.painter(),
        Rect::from_center_size(rect.center(), Vec2::splat(16.0)),
        icon,
        if resp.hovered() { TEXT } else { SUBTLE },
    );
    resp
}

pub(super) fn draw_shelf_card(
    app: &mut App,
    ui: &mut Ui,
    r: Rect,
    card: &CardData,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
) {
    let resp = ui.interact(r, ui.scope_id().with(("card", id)), Sense::click());
    let hovered = ui.rect_contains_pointer(r) || resp.has_focus();
    focus_ring(ui, &resp, 4.0);
    if hovered {
        ui.painter().rect_filled(r, RADIUS_CARD, CARD_HOVER);
    }
    let pad = 12.0;
    let side = (r.width() - pad * 2.0).max(20.0);
    let c = Rect::from_min_size(r.min + Vec2::splat(pad), Vec2::splat(side));
    let scale = if app.data.settings.prefers_reduced_motion {
        1.0
    } else {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with(("card-thumb-scale", id)),
            if hovered { 1.03 } else { 1.0 },
            0.18,
        )
    };
    let c_scaled = Rect::from_center_size(c.center(), c.size() * scale);
    let cover_r = if card.circle { c_scaled.width() * 0.5 } else { RADIUS_LARGER };
    cover_shadow(ui.painter(), c_scaled, cover_r);
    paint_card_cover(app, ui, c_scaled, card, RADIUS_LARGER);

    text(ui.painter(), Pos2::new(c.left(), c.bottom() + 10.0), &card.title, body_medium(), TEXT, side, Align2::LEFT_TOP);
    wrapped_text(ui.painter(), Pos2::new(c.left(), c.bottom() + 32.0), &card.sub, body_small(), SUBTLE, side, 2, Align2::LEFT_TOP);

    if resp.clicked() {
        activate_card(app, card);
    }
    if let Some(ytm_item) = &card.ytm_item {
        resp.context_menu(|ui| super::ytm::item_menu(app, ui, ytm_item));
    } else if let Some(kind) = &card.play_kind {
        resp.context_menu(|ui| local_item_menu(app, ui, kind));
    }

    let play_rect = Rect::from_center_size(c.right_bottom() - Vec2::splat(28.0), Vec2::splat(44.0));
    let playing = card_is_playing(app, card);
    card_play_button(app, ui, play_rect, card, ("card-play", id), hovered || playing);
}

fn local_item_menu(app: &mut App, ui: &mut Ui, kind: &RecentItem) {
    ui.set_min_width(196.0);
    let is_pinned = app.data.is_pinned(kind);
    if !matches!(kind, RecentItem::FollowedYouTube(_)) {
        if ui.button(app.t("Phát", "Play")).clicked() {
            app.actions.push(Action::PlayItem(kind.clone()));
            ui.close();
        }
    }
    let pin_label = if is_pinned {
        app.t("Bỏ ghim khỏi Thư viện", "Unpin from Your Library")
    } else {
        app.t("Ghim vào Thư viện", "Pin to Your Library")
    };
    if ui.button(pin_label).clicked() {
        app.actions.push(Action::TogglePin(kind.clone()));
        ui.close();
    }
    if let RecentItem::Playlist(id) = kind {
        ui.separator();
        if ui.button(app.t("Sửa chi tiết", "Edit details")).clicked() {
            app.actions.push(Action::BeginEditPlaylist(*id));
            ui.close();
        }
        if ui.button(app.t("Xóa danh sách phát", "Delete playlist")).clicked() {
            app.actions.push(Action::RequestConfirm(ConfirmAction::DeletePlaylist(*id)));
            ui.close();
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Track lists (Liked, Playlist, Album, Artist)

struct Hero {
    kind: String,
    title: String,
    description: Option<String>,
    meta_prefix: Option<(String, Option<Route>)>,
    meta_suffix: String,
    cover_key: String,
    cover_path: Option<String>,
    circle: bool,
    liked: bool,
    verified: bool,
}

struct ListOpts {
    context: ContextRef,
    name: String,
    playlist: Option<u64>,
    album_key: Option<String>,
    show_album: bool,
    show_added: bool,
    use_track_numbers: bool,
    empty_hint: &'static str,
}

fn liked(app: &mut App, ui: &mut Ui) {
    let ids = app.data.liked_ids();
    let added: Vec<u64> = app.data.liked.iter().map(|e| e.added).collect();
    let count = ids.iter().filter(|id| app.lib.tracks.contains_key(id)).count();
    let hero = Hero {
        kind: app.t("Danh sách phát", "Playlist").to_string(),
        title: app.t("Bài hát đã thích", "Liked Songs").to_string(),
        description: None,
        meta_prefix: None,
        meta_suffix: format!("{count} {}", app.t("bài hát", "songs")),
        cover_key: "liked".into(),
        cover_path: None,
        circle: false,
        liked: true,
        verified: false,
    };
    let opts = ListOpts {
        context: ContextRef::Liked,
        name: hero.title.clone(),
        playlist: None,
        album_key: None,
        show_album: true,
        show_added: true,
        use_track_numbers: false,
        empty_hint: app.t(
            "Những bài bạn thích sẽ xuất hiện ở đây.",
            "Songs you like will appear here.",
        ),
    };
    list_page(app, ui, hero, ids, Some(added), opts);
}

fn playlist(app: &mut App, ui: &mut Ui, id: u64) {
    let Some(p) = app.data.playlist(id).cloned() else {
        app.actions.push(Action::Navigate(Route::Home));
        return;
    };
    let ids = p.track_ids();
    let added: Vec<u64> = p.tracks.iter().map(|e| e.added).collect();
    let total: u64 = ids.iter().filter_map(|t| app.lib.track(*t)).map(|t| t.duration_ms).sum();
    let cover_track = ids.iter().find_map(|t| app.lib.track(*t));
    let count = ids.iter().filter(|id| app.lib.tracks.contains_key(id)).count();
    let creator = p.creator.clone().unwrap_or_else(|| app.t("Bạn", "You").to_string());
    let hero = Hero {
        kind: app.t("Danh sách phát", "Playlist").to_string(),
        title: p.name.clone(),
        description: (!p.description.trim().is_empty()).then_some(p.description.clone()),
        meta_prefix: Some((creator, None)),
        meta_suffix: format!("{count} {}, {}", app.t("bài hát", "songs"), long_duration(app.lang(), total)),
        cover_key: if p.image.is_some() {
            format!("pl:{id}")
        } else {
            cover_track.as_ref().map(|t| t.cover_key()).unwrap_or_else(|| format!("pl:{id}"))
        },
        cover_path: p.image.clone().or_else(|| cover_track.as_ref().and_then(|t| t.art_source().map(str::to_string))),
        circle: false,
        liked: false,
        verified: false,
    };
    let opts = ListOpts {
        context: ContextRef::Playlist(id),
        name: p.name,
        playlist: Some(id),
        album_key: None,
        show_album: true,
        show_added: true,
        use_track_numbers: false,
        empty_hint: app.t(
            "Danh sách này chưa có bài nào. Nhấp chuột phải vào một bài hát để thêm vào đây.",
            "This playlist is empty. Right-click any song to add it here.",
        ),
    };
    list_page(app, ui, hero, ids, Some(added), opts);
}

fn album(app: &mut App, ui: &mut Ui, key: &str) {
    let Some(a) = app.lib.album(key).cloned() else {
        app.actions.push(Action::Navigate(Route::Home));
        return;
    };
    let total: u64 = a.tracks.iter().filter_map(|t| app.lib.track(*t)).map(|t| t.duration_ms).sum();
    let display_title = if a.title.trim().is_empty() {
        a.tracks
            .first()
            .and_then(|id| app.lib.track(*id))
            .map(|t| t.title.clone())
            .unwrap_or_else(|| app.t("Đĩa đơn", "Single").to_string())
    } else {
        a.title.clone()
    };
    let has_local_files = a.tracks.iter().any(|id| {
        app.lib
            .track(*id)
            .is_some_and(|t| !t.path.starts_with("yt:") && !t.path.starts_with("demo:"))
    });
    if !app.lib.demo && !has_local_files && !display_title.trim().is_empty() {
        let wake_ctx = ui.ctx().clone();
        match crate::ytm::album_profile_state(&display_title, &a.artist, move || {
            wake_ctx.request_repaint()
        }) {
            crate::ytm::ProfileLookup::Ready((browse_id, page)) if !page.items.is_empty() => {
                super::ytm::draw_ytm_album_page(
                    app,
                    ui,
                    &browse_id,
                    &display_title,
                    &a.artist,
                    &page,
                );
                return;
            }
            crate::ytm::ProfileLookup::Loading => {
                draw_entity_skeleton(ui, false);
                return;
            }
            _ => {}
        }
    }
    let kind_label = if a.tracks.len() <= 3 {
        app.t("Đĩa đơn", "Single")
    } else if a.tracks.len() <= 6 {
        "EP"
    } else {
        "Album"
    };
    let year = a.year.map(|y| format!("{y} • ")).unwrap_or_default();
    let hero = Hero {
        kind: kind_label.to_string(),
        title: display_title.clone(),
        description: None,
        meta_prefix: Some((a.artist.clone(), Some(Route::Artist(a.artist.clone())))),
        meta_suffix: format!(
            "{year}{} {}, {}",
            a.tracks.len(),
            app.t("bài hát", "songs"),
            album_duration(app.lang(), total)
        ),
        cover_key: a.key.clone(),
        cover_path: a.cover_path.clone(),
        circle: false,
        liked: false,
        verified: false,
    };
    let opts = ListOpts {
        context: ContextRef::Album(a.key.clone()),
        name: display_title,
        playlist: None,
        album_key: Some(a.key),
        show_album: false,
        show_added: false,
        use_track_numbers: true,
        empty_hint: "",
    };
    list_page(app, ui, hero, a.tracks, None, opts);
}

fn artist(app: &mut App, ui: &mut Ui, name: &str) {
    let has_local_files = app.lib.artist(name).is_some_and(|a| {
        a.tracks.iter().any(|id| {
            app.lib
                .track(*id)
                .is_some_and(|t| !t.path.starts_with("yt:") && !t.path.starts_with("demo:"))
        })
    });
    if !app.lib.demo {
        let wake_ctx = ui.ctx().clone();
        match crate::ytm::artist_profile_state(name, move || wake_ctx.request_repaint()) {
            crate::ytm::ProfileLookup::Ready(page)
                if !page.items.is_empty() || page.image.is_some() =>
            {
                super::ytm::draw_ytm_artist_page(app, ui, name, &page);
                return;
            }
            crate::ytm::ProfileLookup::Loading if !has_local_files => {
                draw_entity_skeleton(ui, true);
                return;
            }
            _ => {}
        }
    }
    let Some(a) = app.lib.artist(name).cloned() else {
        draw_entity_skeleton(ui, true);
        return;
    };
    let listeners_str = if app.lib.demo {
        let listeners = ((a.tracks.len() * 142_350) + (a.albums.len() * 88_120)).max(124_500);
        format_listeners(app.lang(), listeners)
    } else {
        format!(
            "{} {} • {} {}",
            a.tracks.len(),
            app.t("bài hát", "songs"),
            a.albums.len().max(1),
            app.t("album", "albums")
        )
    };
    let hero = Hero {
        kind: app.t("Nghệ sĩ đã xác minh", "Verified Artist").to_string(),
        title: a.name.clone(),
        description: None,
        meta_prefix: None,
        meta_suffix: listeners_str.clone(),
        cover_key: a.cover_key.clone(),
        cover_path: a.cover_path.clone(),
        circle: true,
        liked: false,
        verified: true,
    };

    let related: Vec<CardData> = app
        .lib
        .artists
        .iter()
        .filter(|other| other.name != a.name)
        .take(10)
        .filter_map(|other| item_of(app, &RecentItem::Artist(other.name.clone())))
        .map(|it| CardData::from_local(&it))
        .collect();
    let album_cards: Vec<CardData> = a
        .albums
        .iter()
        .filter_map(|k| item_of(app, &RecentItem::Album(k.clone())))
        .map(|it| CardData::from_local(&it))
        .collect();
    let albums_full: Vec<crate::library::Album> = a
        .albums
        .iter()
        .filter_map(|k| app.lib.album(k).cloned())
        .collect();

    let see_more_id = ui.scope_id().with(("artist-popular-more", &a.name));
    let disco_chip_id = ui.scope_id().with(("artist-disco-chip", &a.name));
    let disco_view_id = ui.scope_id().with(("artist-disco-view", &a.name));
    let about_modal_id = ui.scope_id().with(("artist-about-modal", &a.name));

    let following = app.data.followed_artists.contains(&a.name);
    let mut see_more = ui.ctx().data(|d| d.get_temp::<bool>(see_more_id)).unwrap_or(false);
    let mut disco_chip = ui.ctx().data(|d| d.get_temp::<u8>(disco_chip_id)).unwrap_or(0);
    let mut disco_list_mode = ui.ctx().data(|d| d.get_temp::<bool>(disco_view_id)).unwrap_or(false);
    let mut about_open = ui.ctx().data(|d| d.get_temp::<bool>(about_modal_id)).unwrap_or(false);

    let context = ContextRef::Artist(a.name.clone());
    let salt = format!("artist:{}", a.name);

    let mut list_scroll = ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]);
    if let Some(forced_y) = ui.ctx().data(|d| d.get_temp::<f32>(Id::NULL.with("album-screenshot-scroll"))) {
        list_scroll = list_scroll.vertical_scroll_offset(forced_y);
    }
    list_scroll.show_viewport(ui, |ui, vp| {
        let w = ui.available_width();
        let side = if w < 640.0 { 16.0 } else { 24.0 };
        let inner_w = (w - side * 2.0).max(200.0);
        let scroll_y = vp.min.y.max(0.0);
        let origin = ui.cursor().min;
        let hero_h = hero_height_for_width(w);
        let hero_color = adaptive_hero_color(app, &hero.cover_key, false);

        // Full-bleed gradient behind hero & top of action bar.
        draw_full_bleed_gradient(ui.painter(), Rect::from_min_size(origin, Vec2::new(w, hero_h + 120.0)), hero_color, PANEL);

        let mini_h = 64.0;
        let mini_fade = ((scroll_y - (hero_h + 16.0)) / 24.0).clamp(0.0, 1.0);

        // Clip scrollable content below mini-topbar when active.
        let body_clip = if mini_fade > 0.05 {
            Rect::from_min_max(Pos2::new(origin.x, origin.y + scroll_y + mini_h), Pos2::new(origin.x + w, origin.y + vp.max.y + 8.0))
        } else {
            Rect::from_min_max(origin, Pos2::new(origin.x + w, origin.y + vp.max.y + 8.0))
        };

        ui.scope_builder(UiBuilder::new().id_salt("artist-scroll-body"), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body_clip));

            let (hero_rect, _) = ui.allocate_exact_size(Vec2::new(w, hero_h), Sense::hover());
            draw_hero(app, ui, hero_rect, &hero, side);

            // Action bar: 56px Play + Shuffle + Follow/Following pill + Dots.
            let (act_rect, _) = ui.allocate_exact_size(Vec2::new(w, 84.0), Sense::hover());
            let cy = act_rect.center().y;
            let play = Rect::from_center_size(Pos2::new(act_rect.left() + side + 28.0, cy), Vec2::splat(56.0));
            let playing = app.context_is_playing(&context);
            let (p_icon, p_tip) = if playing { (Icon::Pause, app.t("Tạm dừng", "Pause")) } else { (Icon::Play, app.t("Phát", "Play")) };
            if round_button(ui, play, "artist-play", p_icon, ACCENT, Color32::BLACK, p_tip).clicked() && !a.tracks.is_empty() {
                if app.player.context_ref == context && app.player.current.is_some() {
                    app.actions.push(Action::TogglePlay);
                } else {
                    app.actions.push(Action::PlayContext {
                        context: context.clone(),
                        name: a.name.clone(),
                        tracks: a.tracks.clone(),
                        start: None,
                    });
                }
            }
            let mut ax = play.right() + 20.0;
            if !a.tracks.is_empty() {
                let prev_r = Rect::from_center_size(Pos2::new(ax + 17.0, cy), Vec2::new(34.0, 44.0));
                let prev_tip = app.t(
                    "Cuộn qua để xem thử các bản nhạc của nghệ sĩ này",
                    "Scroll through to preview tracks from this artist",
                );
                let prev_resp = ui
                    .interact(prev_r, ui.scope_id().with("artist-preview-pill"), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(prev_tip);
                let prev_hov = prev_resp.hovered();
                let prev_sc = if app.data.settings.prefers_reduced_motion {
                    1.0
                } else {
                    ui.ctx().animate_value_with_time(
                        ui.scope_id().with("artist-prev-pill-sc"),
                        if prev_hov { 1.05 } else { 1.0 },
                        0.16,
                    )
                };
                let draw_prev = Rect::from_center_size(prev_r.center(), prev_r.size() * prev_sc);
                let first_tr = app.lib.track(a.tracks[0]);
                let p_key = first_tr.as_ref().map(|t| t.cover_key()).unwrap_or_else(|| hero.cover_key.clone());
                let p_src = first_tr
                    .and_then(|t| t.art_source().map(str::to_string))
                    .or_else(|| hero.cover_path.clone());
                let p_tex = app.art.get(ui.ctx(), &p_key, p_src.as_deref());
                ui.painter().rect_filled(draw_prev, 8.0, Color32::from_black_alpha(150));
                cover(ui, draw_prev.shrink(3.0), &p_tex, 5.0);
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
                if prev_resp.clicked() {
                    let preview_items: Vec<crate::app::PreviewTrack> = a
                        .tracks
                        .iter()
                        .take(20)
                        .filter_map(|&tid| {
                            let tr = app.lib.track(tid)?;
                            Some(crate::app::PreviewTrack {
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
                    let tags = if app.lang() == Lang::Vi {
                        vec!["#nghệ sĩ".into(), "#phổ biến".into(), "#xem thử".into()]
                    } else {
                        vec!["#artist".into(), "#popular".into(), "#preview".into()]
                    };
                    app.actions.push(Action::OpenPreviewFeed(crate::app::PreviewFeedState {
                        entity_title: a.name.clone(),
                        entity_subtitle: None,
                        entity_avatar_key: hero.cover_key.clone(),
                        entity_avatar_src: hero.cover_path.clone(),
                        artist_name: a.name.clone(),
                        items: preview_items,
                        index: 0,
                        scroll_anim: 0.0,
                        last_wheel: None,
                        tags,
                    }));
                }
                ax = prev_r.right() + 22.0;
            } else {
                ax += 10.0;
            }
            let shuf_rect = Rect::from_center_size(Pos2::new(ax, cy), Vec2::splat(32.0));
            let shuf_color = if app.player.shuffle { ACCENT } else { SUBTLE };
            if icon_button(ui, shuf_rect, "artist-shuffle", Icon::Shuffle, shuf_color, app.t("Trộn bài", "Shuffle")).clicked() {
                app.actions.push(Action::ToggleShuffle);
            }
            let follow_label = if following {
                app.t("Đang theo dõi", "Following")
            } else {
                app.t("Theo dõi", "Follow")
            };
            let follow_w = text_width(ui, follow_label, semi(13.0)) + 28.0;
            let follow_rect = Rect::from_center_size(Pos2::new(shuf_rect.right() + 18.0 + follow_w * 0.5, cy), Vec2::new(follow_w, 32.0));
            let f_resp = ui.interact(follow_rect, ui.scope_id().with("artist-follow-btn"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let f_stroke = if f_resp.hovered() { TEXT } else { MUTED };
            ui.painter().rect_stroke(follow_rect, 16.0, Stroke::new(1.0, f_stroke), egui::StrokeKind::Inside);
            text(ui.painter(), follow_rect.center(), follow_label, semi(13.0), TEXT, follow_w, Align2::CENTER_CENTER);
            if f_resp.clicked() {
                app.actions.push(Action::ToggleArtistFollow(a.name.clone()));
            }
            let dots_rect = Rect::from_center_size(Pos2::new(follow_rect.right() + 26.0, cy), Vec2::splat(32.0));
            let dots_resp = icon_button(ui, dots_rect, "artist-dots", Icon::Dots, SUBTLE, app.t("Tùy chọn khác", "More options"));
            egui::Popup::menu(&dots_resp).show(|ui| {
                ui.set_min_width(180.0);
                if ui.button(app.t("Thêm tất cả vào danh sách chờ", "Add all to queue")).clicked() {
                    for &tid in &a.tracks {
                        app.actions.push(Action::Enqueue(tid));
                    }
                    ui.close();
                }
                if ui.button(app.t("Giới thiệu về nghệ sĩ", "About artist")).clicked() {
                    about_open = true;
                    ui.close();
                }
            });

            // Popular tracks (5 rows + See more to 10).
            if !a.tracks.is_empty() {
                let (h_rect, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
                text(ui.painter(), Pos2::new(h_rect.left() + side, h_rect.center().y), app.t("Phổ biến", "Popular"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
                ui.add_space(6.0);
                let max_pop = if see_more { 10 } else { 5 };
                let pop_ids: Vec<TrackId> = a.tracks.iter().copied().take(max_pop).collect();
                let opts = ListOpts {
                    context: context.clone(),
                    name: a.name.clone(),
                    playlist: None,
                    album_key: None,
                    show_album: false,
                    show_added: false,
                    use_track_numbers: false,
                    empty_hint: "",
                };
                let cols = Columns::for_width(w, opts.show_album, false);
                for (i, &tid) in pop_ids.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
                    let row_r = Rect::from_min_size(Pos2::new(r.left() + side - 8.0, r.top()), Vec2::new(w - (side - 8.0) * 2.0, ROW_H));
                    if ui.is_rect_visible(row_r) {
                        track_row(app, ui, row_r, i, tid, 0, &cols, &opts, &a.tracks, false);
                    }
                }
                if a.tracks.len() > 5 {
                    let (more_row, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
                    let lbl = if see_more { app.t("Ẩn bớt", "Show less") } else { app.t("Xem thêm", "See more") };
                    let lw = text_width(ui, lbl, semi(13.0)) + 8.0;
                    let lr = Rect::from_min_size(Pos2::new(more_row.left() + side + 12.0, more_row.center().y - 12.0), Vec2::new(lw, 24.0));
                    let m_resp = ui.interact(lr, ui.scope_id().with("artist-see-more"), Sense::click());
                    text(ui.painter(), lr.left_center(), lbl, semi(13.0), if m_resp.hovered() { TEXT } else { SUBTLE }, lw, Align2::LEFT_CENTER);
                    if m_resp.clicked() {
                        see_more = !see_more;
                    }
                }
                ui.add_space(20.0);
            }

            // Discography section with filter chips + List/Grid toggle.
            if !albums_full.is_empty() {
                let (d_head, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
                text(ui.painter(), Pos2::new(d_head.left() + side, d_head.center().y), app.t("Danh sách đĩa nhạc", "Discography"), bold(24.0), TEXT, inner_w - 80.0, Align2::LEFT_CENTER);
                let list_btn = Rect::from_center_size(Pos2::new(d_head.right() - side - 44.0, d_head.center().y), Vec2::splat(28.0));
                let grid_btn = Rect::from_center_size(Pos2::new(d_head.right() - side - 14.0, d_head.center().y), Vec2::splat(28.0));
                if icon_button(ui, list_btn, "disco-list", Icon::Sort, if disco_list_mode { TEXT } else { SUBTLE }, app.t("Xem danh sách", "List view")).clicked() {
                    disco_list_mode = true;
                }
                if icon_button(ui, grid_btn, "disco-grid", Icon::Grid, if !disco_list_mode { TEXT } else { SUBTLE }, app.t("Xem lưới", "Grid view")).clicked() {
                    disco_list_mode = false;
                }

                let (chip_row, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
                let mut cx = chip_row.left() + side;
                for (idx, label) in [
                    (0u8, app.t("Phát hành phổ biến", "Popular releases")),
                    (1u8, "Album"),
                    (2u8, app.t("Đĩa đơn và EP", "Singles and EPs")),
                ] {
                    let cw = pill_width(ui, label);
                    let cr = Rect::from_min_size(Pos2::new(cx, chip_row.center().y - 16.0), Vec2::new(cw, 32.0));
                    if pill(ui, cr, ("disco-chip", idx), label, disco_chip == idx).clicked() {
                        disco_chip = idx;
                    }
                    cx += cw + 8.0;
                }
                ui.add_space(8.0);

                let filtered_albums: Vec<&crate::library::Album> = albums_full
                    .iter()
                    .filter(|al| match disco_chip {
                        1 => al.tracks.len() >= 3,
                        2 => al.tracks.len() < 3,
                        _ => true,
                    })
                    .collect();

                if disco_list_mode {
                    for (ai, al) in filtered_albums.iter().enumerate() {
                        let (ah, _) = ui.allocate_exact_size(Vec2::new(w, 128.0), Sense::hover());
                        let cov = Rect::from_min_size(Pos2::new(ah.left() + side, ah.top() + 8.0), Vec2::splat(112.0));
                        let tex = app.art.get(ui.ctx(), &al.key, al.cover_path.as_deref());
                        cover_shadow(ui.painter(), cov, 4.0);
                        cover(ui, cov, &tex, 4.0);
                        let tx = cov.right() + 18.0;
                        let tr = (ah.right() - side - tx).max(40.0);
                        let title_r = text(ui.painter(), Pos2::new(tx, cov.top() + 14.0), &al.title, bold(20.0), TEXT, tr, Align2::LEFT_TOP);
                        if link(ui, title_r, ("disco-al-link", ai)) {
                            app.actions.push(Action::Navigate(Route::Album(al.key.clone())));
                        }
                        let meta_s = format!(
                            "{} • {} {}",
                            al.year.map(|y| y.to_string()).unwrap_or_else(|| "Album".into()),
                            al.tracks.len(),
                            app.t("bài hát", "songs")
                        );
                        text(ui.painter(), Pos2::new(tx, cov.top() + 44.0), &meta_s, body(13.0), SUBTLE, tr, Align2::LEFT_TOP);
                        let al_play = Rect::from_min_size(Pos2::new(tx, cov.bottom() - 36.0), Vec2::splat(32.0));
                        let al_ctx = ContextRef::Album(al.key.clone());
                        let al_playing = app.context_is_playing(&al_ctx);
                        let p_ic = if al_playing { Icon::Pause } else { Icon::Play };
                        if round_button(ui, al_play, ("disco-al-play", ai), p_ic, TEXT, Color32::BLACK, app.t("Phát", "Play")).clicked() {
                            if app.player.context_ref == al_ctx && app.player.current.is_some() {
                                app.actions.push(Action::TogglePlay);
                            } else {
                                app.actions.push(Action::PlayContext {
                                    context: al_ctx.clone(),
                                    name: al.title.clone(),
                                    tracks: al.tracks.clone(),
                                    start: None,
                                });
                            }
                        }
                        let al_opts = ListOpts {
                            context: al_ctx,
                            name: al.title.clone(),
                            playlist: None,
                            album_key: Some(al.key.clone()),
                            show_album: false,
                            show_added: false,
                            use_track_numbers: true,
                            empty_hint: "",
                        };
                        let al_cols = Columns::for_width(w, false, false);
                        for (ti, &tid) in al.tracks.iter().enumerate() {
                            let (r, _) = ui.allocate_exact_size(Vec2::new(w, 44.0), Sense::hover());
                            let row_r = Rect::from_min_size(Pos2::new(r.left() + side - 8.0, r.top()), Vec2::new(w - (side - 8.0) * 2.0, 44.0));
                            if ui.is_rect_visible(row_r) {
                                track_row(app, ui, row_r, ti, tid, 0, &al_cols, &al_opts, &al.tracks, true);
                            }
                        }
                        ui.add_space(16.0);
                    }
                } else {
                    let filtered_cards: Vec<CardData> = filtered_albums
                        .iter()
                        .filter_map(|al| item_of(app, &RecentItem::Album(al.key.clone())))
                        .map(|it| CardData::from_local(&it))
                        .collect();
                    let cards_to_show = if filtered_cards.is_empty() { &album_cards } else { &filtered_cards };
                    draw_carousel_shelf(app, ui, "artist-disco-shelf", "", cards_to_show, side);
                }
            }

            // Fans also like carousel.
            if !related.is_empty() {
                draw_carousel_shelf(app, ui, "artist-fans-like", app.t("Người hâm mộ cũng thích", "Fans also like"), &related, side);
            }

            // Clickable About card.
            let about_h = 220.0;
            let (ab_title, _) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::hover());
            text(ui.painter(), Pos2::new(ab_title.left() + side, ab_title.center().y), app.t("Giới thiệu", "About"), bold(24.0), TEXT, inner_w, Align2::LEFT_CENTER);
            let (ab_slot, _) = ui.allocate_exact_size(Vec2::new(w, about_h + 24.0), Sense::hover());
            let card_w = inner_w.min(680.0);
            let card_r = Rect::from_min_size(Pos2::new(ab_slot.left() + side, ab_slot.top() + 8.0), Vec2::new(card_w, about_h));
            let ab_resp = ui.interact(card_r, ui.scope_id().with("artist-about-card"), Sense::click());
            let ab_bg = if ab_resp.hovered() { CARD_HOVER } else { CARD };
            ui.painter().rect_filled(card_r, RADIUS_CARD, ab_bg);
            let tex = app.art.get(ui.ctx(), &a.cover_key, a.cover_path.as_deref());
            let photo_r = Rect::from_min_size(card_r.min + Vec2::splat(20.0), Vec2::splat(about_h - 40.0));
            cover(ui, photo_r, &tex, photo_r.width() * 0.5);
            let bx = photo_r.right() + 24.0;
            let bw = (card_r.right() - bx - 20.0).max(40.0);
            text(ui.painter(), Pos2::new(bx, card_r.top() + 36.0), &listeners_str, bold(16.0), TEXT, bw, Align2::LEFT_TOP);
            let bio = artist_bio(app.lang(), &a.name, a.albums.len(), a.tracks.len());
            wrapped_text(ui.painter(), Pos2::new(bx, card_r.top() + 68.0), &bio, body(13.5), SUBTLE, bw, 4, Align2::LEFT_TOP);
            if ab_resp.clicked() {
                about_open = true;
            }
            ui.add_space(24.0);
        });

        if mini_fade > 0.01 {
            draw_sticky_mini_topbar(
                app,
                ui,
                Rect::from_min_size(Pos2::new(origin.x, origin.y + scroll_y), Vec2::new(w, mini_h)),
                &a.name,
                hero_color,
                mini_fade,
                &context,
                &a.tracks,
                side,
            );
        }
    });

    if about_open {
        draw_artist_about_modal(app, ui, &a.name, &a.cover_key, a.cover_path.as_deref(), &listeners_str, a.albums.len(), a.tracks.len(), &mut about_open);
    }

    ui.ctx().data_mut(|d| {
        d.insert_temp(see_more_id, see_more);
        d.insert_temp(disco_chip_id, disco_chip);
        d.insert_temp(disco_view_id, disco_list_mode);
        d.insert_temp(about_modal_id, about_open);
    });
}

fn format_listeners(lang: Lang, count: usize) -> String {
    let s = count.to_string();
    let mut formatted = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            formatted.push(match lang { Lang::Vi => '.', Lang::En => ',' });
        }
        formatted.push(ch);
    }
    let num: String = formatted.chars().rev().collect();
    match lang {
        Lang::Vi => format!("{num} người nghe hằng tháng"),
        Lang::En => format!("{num} monthly listeners"),
    }
}

fn artist_bio(lang: Lang, name: &str, albums: usize, tracks: usize) -> String {
    match lang {
        Lang::Vi => format!("{name} là nghệ sĩ nổi bật trên Nhịp với {albums} album và {tracks} bản thu được tuyển chọn. Âm nhạc kết hợp giai điệu giàu cảm xúc cùng bản phối tinh tế."),
        Lang::En => format!("{name} is a featured artist on Nhịp with {albums} releases and {tracks} catalog tracks, blending atmospheric arrangements with memorable melodies."),
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_artist_about_modal(
    app: &mut App,
    ui: &mut Ui,
    name: &str,
    cover_key: &str,
    cover_path: Option<&str>,
    listeners: &str,
    albums: usize,
    tracks: usize,
    open: &mut bool,
) {
    let full = ui.ctx().content_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, ui.scope_id().with("artist-about-layer"));
    let mut top = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(full).id_salt("artist-about"));
    top.set_clip_rect(full);
    top.painter().rect_filled(full, 0.0, Color32::from_black_alpha(165));
    if top.interact(full, top.scope_id().with("scrim"), Sense::click()).clicked() {
        *open = false;
    }
    let mw = 540.0f32.min(full.width() - 40.0);
    let mh = 420.0f32.min(full.height() - 60.0);
    let modal = Rect::from_center_size(full.center(), Vec2::new(mw, mh));
    top.interact(modal, top.scope_id().with("panel"), Sense::click());
    top.painter().rect_filled(modal, RADIUS_MODAL, ELEVATED);

    let img_r = Rect::from_min_size(modal.min, Vec2::new(mw, 190.0));
    let tex = app.art.get(ui.ctx(), cover_key, cover_path);
    cover(&top, img_r, &tex, RADIUS_MODAL);
    vertical_gradient(top.painter(), img_r, Color32::TRANSPARENT, Color32::from_black_alpha(190));
    text(top.painter(), Pos2::new(img_r.left() + 24.0, img_r.bottom() - 24.0), name, bold(28.0), TEXT, mw - 72.0, Align2::LEFT_BOTTOM);

    let close = Rect::from_center_size(Pos2::new(modal.right() - 28.0, modal.top() + 28.0), Vec2::splat(32.0));
    top.painter().circle_filled(close.center(), 16.0, Color32::from_black_alpha(160));
    if icon_button(&mut top, close, "about-close", Icon::Close, TEXT, app.t("Đóng", "Close")).clicked() {
        *open = false;
    }

    text(top.painter(), Pos2::new(modal.left() + 24.0, img_r.bottom() + 22.0), listeners, bold(18.0), TEXT, mw - 48.0, Align2::LEFT_TOP);
    let bio = artist_bio(app.lang(), name, albums, tracks);
    wrapped_text(top.painter(), Pos2::new(modal.left() + 24.0, img_r.bottom() + 56.0), &bio, body(14.0), SUBTLE, mw - 48.0, 6, Align2::LEFT_TOP);
}

fn long_duration(lang: Lang, ms: u64) -> String {
    let m = ms / 60_000;
    match (lang, m >= 60) {
        (Lang::Vi, true) => format!("{} giờ {} phút", m / 60, m % 60),
        (Lang::Vi, false) => format!("{m} phút"),
        (Lang::En, true) => format!("{} hr {} min", m / 60, m % 60),
        (Lang::En, false) => format!("{m} min"),
    }
}

pub(super) fn album_duration(lang: Lang, ms: u64) -> String {
    let total_s = ms / 1000;
    let m = total_s / 60;
    let s = total_s % 60;
    match (lang, m >= 60) {
        (Lang::Vi, true) => format!("{} giờ {} phút", m / 60, m % 60),
        (Lang::Vi, false) => format!("{m} phút {s} giây"),
        (Lang::En, true) => format!("{} hr {} min", m / 60, m % 60),
        (Lang::En, false) => format!("{m} min {s} sec"),
    }
}

pub(super) fn format_play_count(lang: Lang, seed: u64) -> String {
    let plays = 420_000 + (seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 4_850_000);
    let s = plays.to_string();
    let sep = match lang {
        Lang::Vi => '.',
        Lang::En => ',',
    };
    let mut out = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(sep);
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub(super) fn draw_entity_skeleton(ui: &mut Ui, circle: bool) {
    let w = ui.available_width();
    let side = if w < 640.0 { 16.0 } else { 24.0 };
    let hero_h = hero_height_for_width(w);
    let (hero_r, _) = ui.allocate_exact_size(Vec2::new(w, hero_h), Sense::hover());
    let art_side = (128.0 + (w - 600.0) / 424.0 * 104.0).clamp(128.0, 232.0);
    let cov = Rect::from_min_size(
        Pos2::new(hero_r.left() + side, hero_r.bottom() - art_side - 18.0),
        Vec2::splat(art_side),
    );
    skeleton_rect(ui, cov, if circle { art_side * 0.5 } else { 6.0 });
    let tx = cov.right() + 24.0;
    let room = (hero_r.right() - tx - side).max(120.0);
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(tx, cov.top() + 18.0), Vec2::new(72.0, 12.0)), 6.0);
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(tx, cov.center().y - 22.0), Vec2::new((room * 0.62).min(380.0), 44.0)), 8.0);
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(tx, cov.bottom() - 24.0), Vec2::new((room * 0.44).min(240.0), 14.0)), 7.0);

    let (act_r, _) = ui.allocate_exact_size(Vec2::new(w, 76.0), Sense::hover());
    skeleton_rect(ui, Rect::from_center_size(Pos2::new(act_r.left() + side + 28.0, act_r.center().y), Vec2::splat(56.0)), 28.0);
    skeleton_rect(ui, Rect::from_center_size(Pos2::new(act_r.left() + side + 88.0, act_r.center().y), Vec2::splat(32.0)), 16.0);
    skeleton_rect(ui, Rect::from_center_size(Pos2::new(act_r.left() + side + 136.0, act_r.center().y), Vec2::splat(32.0)), 16.0);

    const TITLE_W: [f32; 6] = [0.46, 0.38, 0.54, 0.34, 0.49, 0.41];
    const SUB_W: [f32; 6] = [0.22, 0.28, 0.19, 0.25, 0.21, 0.24];
    for row_i in 0..6 {
        let (rr, _) = ui.allocate_exact_size(Vec2::new(w, ROW_H), Sense::hover());
        let rx = rr.left() + side + 36.0;
        let avail = (rr.right() - side - rx - 80.0).max(80.0);
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(rx, rr.top() + 12.0), Vec2::new(avail * TITLE_W[row_i], 13.0)), 6.0);
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(rx, rr.top() + 31.0), Vec2::new(avail * SUB_W[row_i], 10.0)), 5.0);
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(rr.right() - side - 44.0, rr.center().y - 6.0), Vec2::new(34.0, 11.0)), 5.0);
    }
}

pub(super) fn draw_home_skeleton(ui: &mut Ui) {
    let w = ui.available_width();
    let side = home_content_padding(w);
    let inner_w = (w - side * 2.0).max(180.0);
    ui.add_space(68.0);
    let (cols, tile_h, tile_gap) = quick_grid_layout(inner_w);
    let tile_w = (inner_w - (cols as f32 - 1.0) * tile_gap) / cols as f32;
    let rows = 8usize.div_ceil(cols);
    let (grid, _) = ui.allocate_exact_size(
        Vec2::new(w, rows as f32 * tile_h + rows.saturating_sub(1) as f32 * tile_gap),
        Sense::hover(),
    );
    for i in 0..8 {
        let (c, r) = (i % cols, i / cols);
        let tile = Rect::from_min_size(
            Pos2::new(grid.left() + side + c as f32 * (tile_w + tile_gap), grid.top() + r as f32 * (tile_h + tile_gap)),
            Vec2::new(tile_w, tile_h),
        );
        skeleton_rect(ui, tile, RADIUS_BASE);
    }
    ui.add_space(28.0);
    for s in 0..2 {
        let (hdr, _) = ui.allocate_exact_size(Vec2::new(w, 34.0), Sense::hover());
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(hdr.left() + side, hdr.top() + 6.0), Vec2::new(210.0 - s as f32 * 34.0, 20.0)), 8.0);
        ui.add_space(8.0);
        let card_cols = ((inner_w + 12.0) / 184.0).floor().max(2.0) as usize;
        let card_w = (inner_w - (card_cols as f32 - 1.0) * 12.0) / card_cols as f32;
        let (row_r, _) = ui.allocate_exact_size(Vec2::new(w, card_w + 54.0), Sense::hover());
        for c in 0..card_cols {
            let x = row_r.left() + side + c as f32 * (card_w + 12.0);
            let cov = Rect::from_min_size(Pos2::new(x, row_r.top()), Vec2::splat(card_w));
            skeleton_rect(ui, cov, RADIUS_CARD);
            skeleton_rect(ui, Rect::from_min_size(Pos2::new(x, cov.bottom() + 10.0), Vec2::new(card_w * 0.76, 13.0)), 6.0);
            skeleton_rect(ui, Rect::from_min_size(Pos2::new(x, cov.bottom() + 29.0), Vec2::new(card_w * 0.48, 10.0)), 5.0);
        }
        ui.add_space(24.0);
    }
}

pub(super) fn draw_search_skeleton(ui: &mut Ui, side: f32) {
    let w = ui.available_width();
    let inner_w = (w - side * 2.0).max(200.0);
    ui.add_space(12.0);
    let gap = 20.0;
    let top_w = ((inner_w - gap) * 0.40).clamp(200.0, 420.0);
    let songs_w = (inner_w - gap - top_w).max(180.0);
    let (split_r, _) = ui.allocate_exact_size(Vec2::new(w, 268.0), Sense::hover());
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(split_r.left() + side, split_r.top() + 6.0), Vec2::new(168.0, 20.0)), 8.0);
    let top_card = Rect::from_min_size(Pos2::new(split_r.left() + side, split_r.top() + 42.0), Vec2::new(top_w, 224.0));
    ui.painter().rect_filled(top_card, RADIUS_CARD, CARD);
    skeleton_rect(ui, Rect::from_min_size(top_card.min + Vec2::splat(20.0), Vec2::splat(92.0)), 6.0);
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(top_card.left() + 20.0, top_card.top() + 132.0), Vec2::new(top_w * 0.58, 24.0)), 8.0);
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(top_card.left() + 20.0, top_card.bottom() - 36.0), Vec2::new(top_w * 0.36, 16.0)), 8.0);

    let sx = top_card.right() + gap;
    skeleton_rect(ui, Rect::from_min_size(Pos2::new(sx, split_r.top() + 6.0), Vec2::new(96.0, 20.0)), 8.0);
    const RW: [f32; 4] = [0.64, 0.52, 0.70, 0.48];
    for i in 0..4 {
        let ry = split_r.top() + 42.0 + i as f32 * ROW_H;
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(sx + 8.0, ry + 8.0), Vec2::splat(40.0)), 4.0);
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(sx + 60.0, ry + 12.0), Vec2::new((songs_w - 110.0).max(40.0) * RW[i], 13.0)), 6.0);
        skeleton_rect(ui, Rect::from_min_size(Pos2::new(sx + 60.0, ry + 31.0), Vec2::new((songs_w - 110.0).max(40.0) * 0.36, 10.0)), 5.0);
    }
}

fn short_date(lang: Lang, secs: u64) -> String {
    let Some(d) = Utc.timestamp_opt(secs as i64, 0).single() else {
        return String::new();
    };
    let d = d.with_timezone(&Local);
    match lang {
        Lang::Vi => format!("{} thg {}, {}", d.day(), d.month(), d.year()),
        Lang::En => d.format("%b %-d, %Y").to_string(),
    }
}

pub(super) fn hero_art_side(w: f32) -> f32 {
    // Measured from Spotify 1.3.9 at 1304x724 (666..689px main -> 150px art, scaling up to 232px on wide views).
    (148.0 + (w - 640.0) / 420.0 * 84.0).clamp(136.0, 232.0)
}

pub(super) fn hero_height_for_width(w: f32) -> f32 {
    hero_art_side(w) + 56.0
}

pub(super) fn real_artist_albums_from_ytm(
    app: &App,
    artist_name: &str,
    exclude_title: &str,
    ctx: &egui::Context,
) -> Vec<CardData> {
    let primary = split_artists(artist_name)
        .into_iter()
        .next()
        .unwrap_or_else(|| artist_name.to_string());
    if primary.trim().is_empty() {
        return Vec::new();
    }
    let wake = ctx.clone();
    let Some(page) = crate::ytm::artist_profile_cached(&primary, move || wake.request_repaint()) else {
        return Vec::new();
    };
    let excl = fold(exclude_title.trim());
    let has_explicit_albums = page.items.iter().any(|it| {
        matches!(it.id, crate::ytm::ItemId::Album(_))
            && page.release_kind(&it.id) == Some(&crate::ytm::ReleaseKind::Album)
    });
    page.items
        .iter()
        .filter(|it| matches!(it.id, crate::ytm::ItemId::Album(_)))
        .filter(|it| fold(it.title.trim()) != excl)
        .filter(|it| {
            if has_explicit_albums {
                page.release_kind(&it.id) == Some(&crate::ytm::ReleaseKind::Album)
            } else {
                !page.is_single_or_ep(&it.id)
            }
        })
        .take(8)
        .map(|it| {
            let mut enriched = it.clone();
            if enriched.by.is_empty() {
                enriched.by = primary.clone();
            }
            CardData::from_ytm(app, &enriched, Some(&page))
        })
        .collect()
}

fn list_page(app: &mut App, ui: &mut Ui, hero: Hero, ids: Vec<TrackId>, added: Option<Vec<u64>>, opts: ListOpts) {
    let pairs: Vec<(TrackId, u64)> = ids
        .iter()
        .enumerate()
        .filter(|(_, id)| app.lib.tracks.contains_key(id))
        .map(|(i, id)| (*id, added.as_ref().and_then(|a| a.get(i).copied()).unwrap_or(0)))
        .collect();

    let filtered_sorted = filter_and_sort_local_tracks(app, &pairs);
    let live_ids: Vec<TrackId> = filtered_sorted.iter().map(|p| p.0).collect();
    let salt = format!("{:?}", app.route);
    let row_h = if app.data.settings.compact_tracklist { 32.0 } else { ROW_H };
    let compact = app.data.settings.compact_tracklist;

    let mut list_scroll = ScrollArea::vertical().id_salt(salt).auto_shrink([false, false]);
    if let Some(forced_y) = ui.ctx().data(|d| d.get_temp::<f32>(Id::NULL.with("album-screenshot-scroll"))) {
        list_scroll = list_scroll.vertical_scroll_offset(forced_y);
    }
    list_scroll.show_viewport(ui, |ui, vp| {
        let w = ui.available_width();
        let side = if w < 640.0 { 16.0 } else { 24.0 };
        let hero_h = hero_height_for_width(w);
        let actions_h = 84.0;
        let header_h = 36.0;
        let scroll_y = vp.min.y.max(0.0);
        let album_meta = opts.album_key.as_deref().and_then(|k| app.lib.album(k).cloned());
        let more_by_artist: Vec<CardData> = if let Some(ref al) = album_meta {
            if app.lib.demo {
                app.lib
                    .albums
                    .iter()
                    .filter(|other| other.artist == al.artist && other.key != al.key && other.tracks.len() >= 2)
                    .take(8)
                    .filter_map(|other| item_of(app, &RecentItem::Album(other.key.clone())))
                    .map(|it| CardData::from_local(&it))
                    .collect()
            } else {
                real_artist_albums_from_ytm(app, &al.artist, &al.title, ui.ctx())
            }
        } else {
            Vec::new()
        };
        let footer_extra_h = if album_meta.is_some() {
            92.0 + if more_by_artist.is_empty() { 0.0 } else { 290.0 }
        } else {
            0.0
        };
        let total = hero_h + actions_h + header_h + filtered_sorted.len() as f32 * row_h + 48.0 + footer_extra_h;
        let (content, _) = ui.allocate_exact_size(Vec2::new(w, total.max(ui.available_height())), Sense::hover());
        let origin = content.min;

        let hero_color = adaptive_hero_color(app, &hero.cover_key, hero.liked);
        draw_full_bleed_gradient(ui.painter(), Rect::from_min_size(origin, Vec2::new(w, hero_h + 140.0)), hero_color, PANEL);

        let mini_h = 64.0;
        let mini_fade = ((scroll_y - (hero_h + 16.0)) / 24.0).clamp(0.0, 1.0);
        let header_rest_y = hero_h + actions_h;
        let header_stuck = scroll_y + mini_h > header_rest_y;
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

        let cols = Columns::for_width(w, opts.show_album, opts.show_added);
        let rows_top = hero_h + actions_h + header_h + 8.0;

        ui.scope_builder(UiBuilder::new().id_salt("list-body"), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(body_clip));

            draw_hero(app, ui, Rect::from_min_size(origin, Vec2::new(w, hero_h)), &hero, side);
            action_bar(app, ui, Rect::from_min_size(origin + Vec2::new(0.0, hero_h), Vec2::new(w, actions_h)), &opts, &live_ids, side);

            if filtered_sorted.is_empty() && !opts.empty_hint.is_empty() {
                let msg = if app.entity_filter.trim().is_empty() {
                    opts.empty_hint
                } else {
                    app.t("Không tìm thấy bài hát phù hợp.", "No matching songs found.")
                };
                text(ui.painter(), origin + Vec2::new(side + 8.0, rows_top + 18.0), msg, body(14.0), SUBTLE, w - side * 2.0, Align2::LEFT_TOP);
            }

            let first = ((vp.min.y.max(top_guard) - rows_top) / row_h).floor().max(0.0) as usize;
            let last = (((vp.max.y - rows_top) / row_h).ceil().max(0.0) as usize).min(filtered_sorted.len());
            for i in first.min(last)..last {
                let (id, date) = filtered_sorted[i];
                let r = Rect::from_min_size(origin + Vec2::new(16.0, rows_top + i as f32 * row_h), Vec2::new(w - 32.0, row_h));
                track_row(app, ui, r, i, id, date, &cols, &opts, &live_ids, compact);
            }

            if let Some(ref al) = album_meta {
                let foot_y = origin.y + rows_top + filtered_sorted.len() as f32 * row_h + 24.0;
                let yr = al.year.unwrap_or(2025);
                let date_line = match app.lang() {
                    Lang::Vi => format!("Phát hành năm {yr}"),
                    Lang::En => format!("Released {yr}"),
                };
                text(ui.painter(), Pos2::new(origin.x + side, foot_y), &date_line, body(13.5), SUBTLE, w - side * 2.0, Align2::LEFT_TOP);
                text(ui.painter(), Pos2::new(origin.x + side, foot_y + 20.0), &format!("© {yr} {}", al.artist), body(11.5), SUBTLE, w - side * 2.0, Align2::LEFT_TOP);
                text(ui.painter(), Pos2::new(origin.x + side, foot_y + 36.0), &format!("℗ {yr} {}", al.artist), body(11.5), SUBTLE, w - side * 2.0, Align2::LEFT_TOP);
                if !more_by_artist.is_empty() {
                    let shelf_rect = Rect::from_min_size(
                        Pos2::new(origin.x, foot_y + 68.0),
                        Vec2::new(w, 280.0),
                    );
                    let mut shelf_ui = ui.new_child(UiBuilder::new().id_salt("album-more-by").max_rect(shelf_rect));
                    let heading_lbl = match app.lang() {
                        Lang::Vi => format!("Album khác của {}", al.artist),
                        Lang::En => format!("More by {}", al.artist),
                    };
                    draw_carousel_shelf(app, &mut shelf_ui, "more-by-artist", &heading_lbl, &more_by_artist, side);
                }
            }
        });

        // Sticky Mini-Topbar (when hero scrolls out of view).
        if mini_fade > 0.01 {
            draw_sticky_mini_topbar(
                app,
                ui,
                Rect::from_min_size(Pos2::new(origin.x, origin.y + scroll_y), Vec2::new(w, mini_h)),
                &hero.title,
                hero_color,
                mini_fade,
                &opts.context,
                &live_ids,
                side,
            );
        }

        // Sticky 36px Column Header.
        let header_y = if header_stuck {
            origin.y + scroll_y + mini_h
        } else {
            origin.y + header_rest_y
        };
        let header_rect = Rect::from_min_size(Pos2::new(origin.x, header_y), Vec2::new(w, header_h));
        if header_stuck {
            ui.painter().rect_filled(header_rect, 0.0, PANEL);
            ui.painter().hline(header_rect.x_range(), header_rect.bottom(), Stroke::new(1.0, HAIRLINE));
        }
        table_header(app, ui, header_rect, &cols, opts.show_added, opts.album_key.is_some());
    });
}

pub(super) fn filter_and_sort_local_tracks(app: &App, pairs: &[(TrackId, u64)]) -> Vec<(TrackId, u64)> {
    let q = fold(app.entity_filter.trim());
    let mut out: Vec<(TrackId, u64)> = pairs
        .iter()
        .copied()
        .filter(|(id, _)| {
            if q.is_empty() {
                return true;
            }
            app.lib.track(*id).is_some_and(|t| {
                fold(&t.title).contains(&q) || fold(&t.artist).contains(&q) || fold(&t.album).contains(&q)
            })
        })
        .collect();
    sort_local_pairs(app, &mut out, app.table_sort);
    out
}

pub(super) fn sort_local_pairs(app: &App, pairs: &mut [(TrackId, u64)], sort: TableSort) {
    if sort.column == TrackSortColumn::Default {
        return;
    }
    pairs.sort_by(|&(a_id, a_added), &(b_id, b_added)| {
        let (Some(a), Some(b)) = (app.lib.track(a_id), app.lib.track(b_id)) else {
            return Ordering::Equal;
        };
        let ord = match sort.column {
            TrackSortColumn::Default => Ordering::Equal,
            TrackSortColumn::Title => fold(&a.title).cmp(&fold(&b.title)),
            TrackSortColumn::Artist => fold(&a.artist).cmp(&fold(&b.artist)),
            TrackSortColumn::Album => fold(&a.album).cmp(&fold(&b.album)),
            TrackSortColumn::DateAdded => a_added.cmp(&b_added),
            TrackSortColumn::Duration => a.duration_ms.cmp(&b.duration_ms),
        };
        if sort.ascending { ord } else { ord.reverse() }
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_sticky_mini_topbar(
    app: &mut App,
    ui: &mut Ui,
    rect: Rect,
    title: &str,
    hero_color: Color32,
    fade: f32,
    context: &ContextRef,
    tracks: &[TrackId],
    side: f32,
) {
    let alpha = (fade * 255.0).round() as u8;
    let bg = darken(hero_color, 0.55);
    ui.painter().rect_filled(rect, 0.0, Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), alpha));
    ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, Color32::from_black_alpha((80.0 * fade) as u8)));

    let cy = rect.center().y;
    let play = Rect::from_center_size(Pos2::new(rect.left() + side + 20.0, cy), Vec2::splat(40.0));
    let playing = app.context_is_playing(context);
    let (icon, tip) = if playing { (Icon::Pause, app.t("Tạm dừng", "Pause")) } else { (Icon::Play, app.t("Phát", "Play")) };
    if round_button(ui, play, "mini-topbar-play", icon, ACCENT, Color32::BLACK, tip).clicked() && !tracks.is_empty() {
        if app.player.context_ref == *context && app.player.current.is_some() {
            app.actions.push(Action::TogglePlay);
        } else {
            app.actions.push(Action::PlayContext {
                context: context.clone(),
                name: title.to_string(),
                tracks: tracks.to_vec(),
                start: None,
            });
        }
    }
    let tx = play.right() + 14.0;
    let room = (rect.right() - tx - side).max(40.0);
    text(ui.painter(), Pos2::new(tx, cy), title, bold(20.0), TEXT.linear_multiply(fade), room, Align2::LEFT_CENTER);
}

fn draw_hero(app: &mut App, ui: &mut Ui, r: Rect, hero: &Hero, side: f32) {
    let art_side = hero_art_side(r.width());
    let c = Rect::from_min_size(Pos2::new(r.left() + side, r.bottom() - art_side - 18.0), Vec2::splat(art_side));
    let cov_hovered = ui.rect_contains_pointer(c);
    let cov_scale = if app.data.settings.prefers_reduced_motion {
        1.0
    } else {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with(("hero-cov-scale", &hero.cover_key)),
            if cov_hovered { 1.028 } else { 1.0 },
            0.18,
        )
    };
    let c_draw = Rect::from_center_size(c.center(), c.size() * cov_scale);
    let cov_r = if hero.circle { c_draw.width() * 0.5 } else { RADIUS_BASE };
    cover_shadow(ui.painter(), c_draw, cov_r);
    if hero.liked {
        paint_liked_tile(ui, c_draw, RADIUS_BASE);
    } else {
        let tex = app.art.get(ui.ctx(), &hero.cover_key, hero.cover_path.as_deref());
        cover(ui, c_draw, &tex, cov_r);
    }

    let x = c.right() + 24.0;
    let room = (r.right() - x - side).max(80.0);

    let kind_y = c.top() + (art_side * 0.06).clamp(4.0, 12.0) + 6.0;
    let meta_y = c.bottom() - 10.0;
    let desc_y = meta_y - 22.0;
    let title_top = kind_y + 12.0;
    let title_bottom = if hero.description.is_some() { desc_y - 12.0 } else { meta_y - 14.0 };
    let title_avail_h = (title_bottom - title_top).max(36.0);

    let p = ui.painter();
    if hero.verified {
        let badge_r = Rect::from_center_size(Pos2::new(x + 10.0, kind_y), Vec2::splat(20.0));
        icons::paint(p, badge_r, Icon::VerifiedBadge, Color32::from_rgb(0x3d, 0x91, 0xf4));
        text(p, Pos2::new(x + 26.0, kind_y), &hero.kind, semi(13.5), TEXT, room - 26.0, Align2::LEFT_CENTER);
    } else {
        text(p, Pos2::new(x, kind_y), &hero.kind, semi(13.5), TEXT, room, Align2::LEFT_CENTER);
    }

    let title_center_y = (title_top + title_bottom) * 0.5;
    paint_fitted_hero_title(p, Pos2::new(x, title_center_y), &hero.title, room, title_avail_h);

    if let Some(desc) = &hero.description {
        text(p, Pos2::new(x, desc_y), desc, body(13.5), SUBTLE, room, Align2::LEFT_CENTER);
    }

    let mut mx = x;
    if let Some((prefix, route_opt)) = &hero.meta_prefix {
        if let Some(Route::Artist(artist_name)) = route_opt {
            let av_r = Rect::from_center_size(Pos2::new(mx + 12.0, meta_y), Vec2::splat(24.0));
            let av_key = app
                .lib
                .artist(artist_name)
                .map(|ar| (ar.cover_key.clone(), ar.cover_path.clone()))
                .unwrap_or_else(|| (hero.cover_key.clone(), hero.cover_path.clone()));
            let av_tex = app.art.get(ui.ctx(), &av_key.0, av_key.1.as_deref());
            cover(ui, av_r, &av_tex, 12.0);
            mx = av_r.right() + 8.0;
        }
        let prefix_r = text(ui.painter(), Pos2::new(mx, meta_y), prefix, bold(14.0), TEXT, room, Align2::LEFT_CENTER);
        if let Some(target) = route_opt {
            if link(ui, prefix_r, ("hero-owner", prefix)) {
                app.actions.push(Action::Navigate(target.clone()));
            }
        }
        mx = prefix_r.right() + 6.0;
        let dot_r = text(ui.painter(), Pos2::new(mx, meta_y), "•", body(13.5), SUBTLE, 16.0, Align2::LEFT_CENTER);
        mx = dot_r.right() + 6.0;
    }
    let rem = (r.right() - side - mx).max(20.0);
    text(ui.painter(), Pos2::new(mx, meta_y), &hero.meta_suffix, body(13.5), SUBTLE, rem, Align2::LEFT_CENTER);
}

pub(super) fn paint_fitted_hero_title(p: &Painter, pos: Pos2, title: &str, max_w: f32, max_h: f32) {
    // Spotify Encore `headlineLarge` -> `titleMedium` adaptive fitting (up to 96px Extrabold for short titles like "Gieo").
    for sz in [96.0_f32, 84.0, 72.0, 62.0, 52.0, 46.0] {
        let mut job = LayoutJob::single_section(
            title.to_owned(),
            TextFormat {
                font_id: bold(sz),
                line_height: Some(sz * 1.03),
                color: TEXT,
                ..Default::default()
            },
        );
        job.wrap = TextWrapping {
            max_width: f32::INFINITY,
            max_rows: 1,
            break_anywhere: false,
            overflow_character: None,
        };
        let galley = p.layout_job(job);
        if galley.size().x <= max_w && galley.size().y <= max_h * 1.18 {
            let rect = Align2::LEFT_CENTER.anchor_size(pos, galley.size());
            p.galley(rect.min, galley, TEXT);
            return;
        }
    }

    let mut chosen = None;
    for sz in [54.0_f32, 44.0, 36.0, 30.0, 26.0, 22.0] {
        let mut job = LayoutJob::single_section(
            title.to_owned(),
            TextFormat {
                font_id: bold(sz),
                line_height: Some(sz * 1.06),
                color: TEXT,
                ..Default::default()
            },
        );
        job.wrap = TextWrapping {
            max_width: max_w.max(1.0),
            max_rows: if sz <= 30.0 { 3 } else { 2 },
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let galley = p.layout_job(job);
        let fits_h = galley.size().y <= max_h * 1.14;
        chosen = Some(galley);
        if fits_h {
            break;
        }
    }
    if let Some(galley) = chosen {
        let rect = Align2::LEFT_CENTER.anchor_size(pos, galley.size());
        p.galley(rect.min, galley, TEXT);
    }
}

fn action_bar(app: &mut App, ui: &mut Ui, r: Rect, opts: &ListOpts, ids: &[TrackId], side: f32) {
    let cy = r.center().y;
    let play = Rect::from_center_size(Pos2::new(r.left() + side + 28.0, cy), Vec2::splat(56.0));
    let playing = app.context_is_playing(&opts.context);
    let (icon, tip) = if playing { (Icon::Pause, app.t("Tạm dừng", "Pause")) } else { (Icon::Play, app.t("Phát", "Play")) };
    if round_button(ui, play, "page-play", icon, ACCENT, Color32::BLACK, tip).clicked() && !ids.is_empty() {
        if app.player.context_ref == opts.context && app.player.current.is_some() {
            app.actions.push(Action::TogglePlay);
        } else {
            app.actions.push(Action::PlayContext {
                context: opts.context.clone(),
                name: opts.name.clone(),
                tracks: ids.to_vec(),
                start: None,
            });
        }
    }

    let mut x = play.right() + 20.0;
    if let Some(akey) = &opts.album_key {
        // Spotify Album visual preview thumbnail pill (34x44 rounded rect next to Play, Image 10/12/14).
        let prev_r = Rect::from_center_size(Pos2::new(x + 17.0, cy), Vec2::new(34.0, 44.0));
        let prev_resp = ui
            .interact(prev_r, ui.scope_id().with("album-prev-pill"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(app.t(
                "Cuộn qua để xem thử các bản nhạc trong album này",
                "Scroll through to preview tracks from this album",
            ));
        let prev_hov = prev_resp.hovered();
        let prev_sc = if app.data.settings.prefers_reduced_motion {
            1.0
        } else {
            ui.ctx().animate_value_with_time(
                ui.scope_id().with("album-prev-pill-sc"),
                if prev_hov { 1.05 } else { 1.0 },
                0.16,
            )
        };
        let draw_prev = Rect::from_center_size(prev_r.center(), prev_r.size() * prev_sc);
        let cov_src = app.lib.album(akey).and_then(|al| al.cover_path.clone());
        let tex = app.art.get(ui.ctx(), akey, cov_src.as_deref());
        ui.painter().rect_filled(draw_prev, 8.0, Color32::from_black_alpha(150));
        cover(ui, draw_prev.shrink(3.0), &tex, 5.0);
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
        if prev_resp.clicked() && !ids.is_empty() {
            let preview_items: Vec<crate::app::PreviewTrack> = ids
                .iter()
                .take(20)
                .filter_map(|&tid| {
                    let tr = app.lib.track(tid)?;
                    Some(crate::app::PreviewTrack {
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
            let artist_name = app
                .lib
                .album(akey)
                .map(|al| al.artist.clone())
                .unwrap_or_else(|| opts.name.clone());
            let tags = if app.lang() == Lang::Vi {
                vec!["#album".into(), "#thư viện".into(), "#xem thử".into()]
            } else {
                vec!["#album".into(), "#library".into(), "#preview".into()]
            };
            app.actions.push(Action::OpenPreviewFeed(crate::app::PreviewFeedState {
                entity_title: opts.name.clone(),
                entity_subtitle: None,
                entity_avatar_key: akey.clone(),
                entity_avatar_src: cov_src,
                artist_name,
                items: preview_items,
                index: 0,
                scroll_anim: 0.0,
                last_wheel: None,
                tags,
            }));
        }
        x = prev_r.right() + 22.0;
    } else {
        x += 8.0;
    }

    let shuffle_on = app.player.shuffle;
    let s = Rect::from_center_size(Pos2::new(x, cy), Vec2::splat(32.0));
    if icon_button(ui, s, "page-shuffle", Icon::Shuffle, if shuffle_on { ACCENT } else { SUBTLE }, app.t("Trộn bài", "Shuffle")).clicked() {
        app.actions.push(Action::ToggleShuffle);
    }
    x = s.right() + 22.0;

    if let Some(akey) = &opts.album_key {
        let album_saved = app.data.saved_albums.contains(akey);
        let save_r = Rect::from_center_size(Pos2::new(x, cy), Vec2::splat(32.0));
        let (s_ic, s_col, s_tip) = if album_saved {
            (Icon::Saved, ACCENT, app.t("Xóa khỏi Thư viện", "Remove from Your Library"))
        } else {
            (Icon::Save, SUBTLE, app.t("Lưu vào Thư viện", "Save to Your Library"))
        };
        if icon_button_sized(ui, save_r, "album-save-lib", s_ic, s_col, s_tip, 24.0).clicked() {
            app.actions.push(Action::ToggleSaveAlbum(akey.clone()));
        }
        x = save_r.right() + 22.0;
    }

    if let Some(pid) = opts.playlist {
        let pinned = app.data.is_pinned(&RecentItem::Playlist(pid));
        let pin_r = Rect::from_center_size(Pos2::new(x, cy), Vec2::splat(28.0));
        if icon_button(ui, pin_r, "pl-pin", Icon::Pin, if pinned { ACCENT } else { SUBTLE }, app.t("Ghim", "Pin")).clicked() {
            app.actions.push(Action::TogglePin(RecentItem::Playlist(pid)));
        }
        x = pin_r.right() + 20.0;
    }

    let d = Rect::from_center_size(Pos2::new(x, cy), Vec2::splat(32.0));
    let resp = icon_button(ui, d, "page-dots", Icon::Dots, SUBTLE, app.t("Tùy chọn khác", "More options"));
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(196.0);
        if ui.button(app.t("Thêm tất cả vào danh sách chờ", "Add all to queue")).clicked() {
            for &id in ids {
                app.actions.push(Action::Enqueue(id));
            }
            ui.close();
        }
        if let Some(id) = opts.playlist {
            ui.separator();
            if ui.button(app.t("Sửa chi tiết", "Edit details")).clicked() {
                app.actions.push(Action::BeginEditPlaylist(id));
                ui.close();
            }
            if ui.button(app.t("Đổi tên", "Rename")).clicked() {
                app.actions.push(Action::BeginRename(id));
                ui.close();
            }
            if ui.button(app.t("Xóa danh sách phát", "Delete playlist")).clicked() {
                app.actions.push(Action::RequestConfirm(ConfirmAction::DeletePlaylist(id)));
                ui.close();
            }
        }
    });

    // Right-aligned inline entity filter + Sort & View-as controls.
    draw_tracklist_controls(app, ui, r.right() - side, cy, d.right() + 16.0, opts.album_key.is_some());
}

pub(super) fn draw_tracklist_controls(app: &mut App, ui: &mut Ui, right_x: f32, cy: f32, min_left: f32, is_album: bool) {
    let compact = app.data.settings.compact_tracklist;
    let sort_label = if is_album && app.table_sort.column == TrackSortColumn::Default {
        if compact {
            app.t("Thu gọn", "Compact")
        } else {
            app.t("Danh sách", "List")
        }
    } else {
        match app.table_sort.column {
            TrackSortColumn::Default => app.t("Tùy chỉnh", "Custom order"),
            TrackSortColumn::Title => app.t("Tiêu đề", "Title"),
            TrackSortColumn::Artist => app.t("Nghệ sĩ", "Artist"),
            TrackSortColumn::Album => "Album",
            TrackSortColumn::DateAdded => app.t("Ngày thêm", "Date added"),
            TrackSortColumn::Duration => app.t("Thời lượng", "Duration"),
        }
    };
    let sort_w = (text_width(ui, sort_label, body(13.0)) + 34.0).min(140.0);
    let sort_rect = Rect::from_center_size(Pos2::new(right_x - sort_w * 0.5, cy), Vec2::new(sort_w, 32.0));
    let sort_resp = ui.interact(sort_rect, ui.scope_id().with("tracklist-sort-btn"), Sense::click());
    focus_ring(ui, &sort_resp, 4.0);
    let s_col = if sort_resp.hovered() { TEXT } else { SUBTLE };
    text(ui.painter(), Pos2::new(sort_rect.right() - 26.0, cy), sort_label, body(13.0), s_col, sort_w - 28.0, Align2::RIGHT_CENTER);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(Pos2::new(sort_rect.right() - 10.0, cy), Vec2::splat(16.0)),
        if compact { Icon::CompactList } else { Icon::Sort },
        s_col,
    );

    egui::Popup::menu(&sort_resp)
        .frame(menu_frame(ui.style()))
        .show(|ui| {
            const W: f32 = 164.0;
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.set_min_width(W);
            ui.set_max_width(W);
            ui.label(egui::RichText::new(app.t("Sắp xếp theo", "Sort by")).font(semi(11.5)).color(SUBTLE));
            for (col, label) in [
                (TrackSortColumn::Default, app.t("Thứ tự tùy chỉnh", "Custom order")),
                (TrackSortColumn::Title, app.t("Tiêu đề", "Title")),
                (TrackSortColumn::Artist, app.t("Nghệ sĩ", "Artist")),
                (TrackSortColumn::Album, "Album"),
                (TrackSortColumn::DateAdded, app.t("Ngày thêm", "Date added")),
                (TrackSortColumn::Duration, app.t("Thời lượng", "Duration")),
            ] {
                let active = app.table_sort.column == col;
                let (r, resp) = ui.allocate_exact_size(Vec2::new(W, 27.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, RADIUS_SMALLER, hover_white(24));
                }
                text(ui.painter(), Pos2::new(r.left() + 8.0, r.center().y), label, body(13.0), if active { ACCENT } else { TEXT }, W - 30.0, Align2::LEFT_CENTER);
                if active {
                    icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(r.right() - 12.0, r.center().y), Vec2::splat(13.0)), Icon::Checkmark, ACCENT);
                }
                if resp.clicked() {
                    app.actions.push(Action::SetTableSort(TableSort { column: col, ascending: true }));
                    ui.close();
                }
            }
            ui.add_space(3.0);
            ui.separator();
            ui.label(egui::RichText::new(app.t("Xem dưới dạng", "View as")).font(semi(11.5)).color(SUBTLE));
            for (want_compact, label) in [
                (false, app.t("Danh sách", "List")),
                (true, app.t("Thu gọn", "Compact")),
            ] {
                let active = compact == want_compact;
                let (r, resp) = ui.allocate_exact_size(Vec2::new(W, 27.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, RADIUS_SMALLER, hover_white(24));
                }
                text(ui.painter(), Pos2::new(r.left() + 8.0, r.center().y), label, body(13.0), if active { ACCENT } else { TEXT }, W - 30.0, Align2::LEFT_CENTER);
                if active {
                    icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(r.right() - 12.0, r.center().y), Vec2::splat(13.0)), Icon::Checkmark, ACCENT);
                }
                if resp.clicked() {
                    app.actions.push(Action::SetCompactTracklist(want_compact));
                    ui.close();
                }
            }
        });

    let filter_right = sort_rect.left() - 12.0;
    let avail = filter_right - min_left;
    let show_input = app.entity_filter_open || !app.entity_filter.is_empty() || app.focus_entity_filter;
    if show_input && avail >= 110.0 {
        let box_w = avail.min(188.0);
        let box_rect = Rect::from_min_size(Pos2::new(filter_right - box_w, cy - 16.0), Vec2::new(box_w, 32.0));
        ui.painter().rect_filled(box_rect, RADIUS_BASE, hover_white(24));
        icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(box_rect.left() + 14.0, cy), Vec2::splat(15.0)), Icon::Search, SUBTLE);
        let edit_r = Rect::from_min_max(Pos2::new(box_rect.left() + 28.0, cy - 10.0), Pos2::new(box_rect.right() - 26.0, cy + 10.0));
        let mut child = ui.new_child(UiBuilder::new().id_salt("entity-filter-edit").max_rect(edit_r));
        let hint = app.t("Tìm trong danh sách", "Search in list");
        let resp = child.add(
            egui::TextEdit::singleline(&mut app.entity_filter)
                .frame(egui::Frame::NONE)
                .font(body(13.0))
                .hint_text(egui::RichText::new(hint).color(SUBTLE).font(body(13.0)))
                .desired_width(edit_r.width()),
        );
        if app.focus_entity_filter {
            app.focus_entity_filter = false;
            resp.request_focus();
        }
        if !app.entity_filter.is_empty() {
            let x_r = Rect::from_center_size(Pos2::new(box_rect.right() - 14.0, cy), Vec2::splat(20.0));
            if icon_button(ui, x_r, "entity-filter-clear", Icon::Close, SUBTLE, app.t("Xóa bộ lọc", "Clear filter")).clicked() {
                app.actions.push(Action::ClearEntityFilter);
            }
        }
    } else if !is_album && avail >= 32.0 {
        let search_r = Rect::from_center_size(Pos2::new(filter_right - 16.0, cy), Vec2::splat(28.0));
        if icon_button(ui, search_r, "entity-filter-open", Icon::Search, SUBTLE, app.t("Lọc bài hát", "Filter songs")).clicked() {
            app.entity_filter_open = true;
            app.focus_entity_filter = true;
        }
    }
}

pub(super) struct Columns {
    pub index: f32,
    pub title: f32,
    pub album: Option<f32>,
    pub added: Option<f32>,
    pub right: f32,
}

impl Columns {
    pub(super) fn for_width(w: f32, show_album: bool, show_added: bool) -> Self {
        let inner = (w - 32.0).max(200.0);
        let index = 44.0;
        let duration = 112.0;
        // Spotify shows the Album column at 1304x724 (666px main) and hides it on compact windows (<620px).
        let added = (show_added && w >= 960.0).then_some(148.0);
        let album = (show_album && w >= 620.0).then_some(((inner - index - duration - added.unwrap_or(0.0)) * 0.34).max(120.0));
        let title = (inner - index - duration - added.unwrap_or(0.0) - album.unwrap_or(0.0)).max(100.0);
        Self { index, title, album, added, right: inner }
    }
}

pub(super) fn table_header(app: &mut App, ui: &mut Ui, r: Rect, cols: &Columns, show_added_col: bool, show_plays: bool) {
    let (t_title, t_album, t_added) = (
        app.t("Tiêu đề", "Title"),
        "Album",
        app.t("Ngày thêm", "Date added"),
    );
    let y = r.center().y;
    let base = r.left() + 16.0;
    let p = ui.painter();
    text(p, Pos2::new(base + 14.0, y), "#", body(13.0), SUBTLE, 20.0, Align2::LEFT_CENTER);

    header_sort_cell(app, ui, Rect::from_min_size(Pos2::new(base + cols.index, r.top()), Vec2::new(cols.title - 8.0, r.height())), t_title, TrackSortColumn::Title, Align2::LEFT_CENTER);

    if let Some(a) = cols.album {
        header_sort_cell(app, ui, Rect::from_min_size(Pos2::new(base + cols.index + cols.title, r.top()), Vec2::new(a - 8.0, r.height())), t_album, TrackSortColumn::Album, Align2::LEFT_CENTER);
    }
    if show_added_col && cols.added.is_some() {
        let x = base + cols.index + cols.title + cols.album.unwrap_or(0.0);
        header_sort_cell(app, ui, Rect::from_min_size(Pos2::new(x, r.top()), Vec2::new(140.0, r.height())), t_added, TrackSortColumn::DateAdded, Align2::LEFT_CENTER);
    }
    if show_plays && r.width() >= 600.0 {
        text(
            ui.painter(),
            Pos2::new(base + cols.right - 134.0, y),
            app.t("lượt phát", "Plays"),
            body(13.0),
            SUBTLE,
            96.0,
            Align2::RIGHT_CENTER,
        );
    }

    let dur_rect = Rect::from_center_size(Pos2::new(base + cols.right - 40.0, y), Vec2::splat(26.0));
    let dur_resp = ui.interact(dur_rect, ui.scope_id().with("sort-dur"), Sense::click());
    let dur_active = app.table_sort.column == TrackSortColumn::Duration;
    icons::paint(ui.painter(), Rect::from_center_size(dur_rect.center(), Vec2::splat(16.0)), Icon::Clock, if dur_active { ACCENT } else if dur_resp.hovered() { TEXT } else { SUBTLE });
    if dur_resp.clicked() {
        app.actions.push(Action::ToggleTableSort(TrackSortColumn::Duration));
    }

    ui.painter().hline(r.left() + 16.0..=r.right() - 16.0, r.bottom() - 1.0, Stroke::new(1.0, hover_white(26)));
}

fn header_sort_cell(app: &mut App, ui: &mut Ui, rect: Rect, label: &str, col: TrackSortColumn, align: Align2) {
    let resp = ui.interact(rect, ui.scope_id().with(("col-sort", label)), Sense::click());
    focus_ring(ui, &resp, 4.0);
    let active = app.table_sort.column == col;
    let color = if active { ACCENT } else if resp.hovered() { TEXT } else { SUBTLE };
    let drawn = text(ui.painter(), Pos2::new(rect.left(), rect.center().y), label, body(13.0), color, rect.width() - 20.0, align);
    if active {
        let arrow = if app.table_sort.ascending { Icon::ChevronUp } else { Icon::ChevronDown };
        icons::paint(ui.painter(), Rect::from_center_size(Pos2::new(drawn.right() + 10.0, rect.center().y), Vec2::splat(12.0)), arrow, ACCENT);
    }
    if resp.clicked() {
        app.actions.push(Action::ToggleTableSort(col));
    }
}

#[allow(clippy::too_many_arguments)]
fn track_row(app: &mut App, ui: &mut Ui, r: Rect, i: usize, id: TrackId, added: u64, cols: &Columns, opts: &ListOpts, ids: &[TrackId], compact: bool) {
    let Some(t) = app.lib.track(id) else { return };
    let resp = ui.interact(r, ui.scope_id().with(("row", i, id)), Sense::click());
    let hovered = ui.rect_contains_pointer(r) || resp.has_focus();
    focus_ring(ui, &resp, 4.0);
    if hovered {
        ui.painter().rect_filled(r, 4.0, hover_white(26));
    }
    let is_current = app.player.current == Some(id);
    let is_playing = is_current && app.is_playing();
    let title_color = if is_current { ACCENT } else { TEXT };
    let y = r.center().y;
    let base = r.left();

    // Index / hover Play-Pause / equalizer.
    let ind = Rect::from_center_size(Pos2::new(base + 22.0, y), Vec2::splat(20.0));
    if hovered {
        let icon = if is_playing { Icon::Pause } else { Icon::Play };
        if icon_button(ui, ind, ("row-play", i, id), icon, TEXT, "").clicked() {
            if is_current {
                app.actions.push(Action::TogglePlay);
            } else {
                app.actions.push(Action::PlayContext {
                    context: opts.context.clone(),
                    name: opts.name.clone(),
                    tracks: ids.to_vec(),
                    start: Some(i),
                });
            }
        }
    } else if is_playing {
        equalizer_glyph(ui, ind, ui.input(|inp| inp.time));
    } else {
        let label = if opts.use_track_numbers && app.table_sort.column == TrackSortColumn::Default {
            t.track_no.map(|n| n.to_string()).unwrap_or_else(|| (i + 1).to_string())
        } else {
            (i + 1).to_string()
        };
        text(ui.painter(), ind.center(), &label, body(14.0), if is_current { ACCENT } else { SUBTLE }, 30.0, Align2::CENTER_CENTER);
    }

    let is_album_page = opts.album_key.is_some();
    let is_artist_popular = matches!(opts.context, ContextRef::Artist(_));
    let show_plays = (is_album_page || is_artist_popular) && r.width() >= 540.0;
    let plays_reserve = if show_plays { 104.0 } else { 0.0 };
    if compact {
        let tx = base + cols.index;
        let room = (cols.title - 12.0 - plays_reserve).max(40.0);
        let title_w = (room * 0.58).max(40.0);
        let artist_x = tx + title_w + 10.0;
        let artist_w = (room - title_w - 10.0).max(30.0);
        text(ui.painter(), Pos2::new(tx, y), &t.title, body(14.0), title_color, title_w, Align2::LEFT_CENTER);
        paint_artist_links(app, ui, Pos2::new(artist_x, y), &t.artist, artist_w, ("row-artist-compact", i, id));
    } else if is_album_page {
        // Spotify Album rows never repeat the album cover thumbnail per track; 16px Regular title + 14px artist.
        let tx = base + cols.index;
        let room = (cols.title - 16.0 - plays_reserve).max(40.0);
        text(ui.painter(), Pos2::new(tx, y - 10.0), &t.title, body_medium(), title_color, room, Align2::LEFT_CENTER);
        paint_artist_links(app, ui, Pos2::new(tx, y + 11.0), &t.artist, room, ("row-artist", i, id));
    } else if is_artist_popular {
        let cover_rect = Rect::from_center_size(Pos2::new(base + cols.index + 20.0, y), Vec2::splat(40.0));
        let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
        cover(ui, cover_rect, &tex, RADIUS_BASE);
        let tx = cover_rect.right() + 12.0;
        let room = (cols.title - 64.0 - plays_reserve).max(40.0);
        if !t.artist.trim().is_empty() {
            text(ui.painter(), Pos2::new(tx, y - 10.0), &t.title, body_medium(), title_color, room, Align2::LEFT_CENTER);
            paint_artist_links(app, ui, Pos2::new(tx, y + 11.0), &t.artist, room, ("row-artist-pop", i, id));
        } else {
            text(ui.painter(), Pos2::new(tx, y), &t.title, body_medium(), title_color, room, Align2::LEFT_CENTER);
        }
    } else {
        let cover_rect = Rect::from_center_size(Pos2::new(base + cols.index + 20.0, y), Vec2::splat(40.0));
        let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
        cover(ui, cover_rect, &tex, RADIUS_BASE);
        let tx = cover_rect.right() + 12.0;
        let room = (cols.title - 64.0).max(40.0);
        text(ui.painter(), Pos2::new(tx, y - 10.0), &t.title, body_medium(), title_color, room, Align2::LEFT_CENTER);
        paint_artist_links(app, ui, Pos2::new(tx, y + 11.0), &t.artist, room, ("row-artist", i, id));
    }

    if show_plays {
        text(
            ui.painter(),
            Pos2::new(base + cols.right - 134.0, y),
            &format_play_count(app.lang(), id),
            body(14.0),
            SUBTLE,
            96.0,
            Align2::RIGHT_CENTER,
        );
    }

    if let Some(a) = cols.album {
        let pos = Pos2::new(base + cols.index + cols.title, y);
        let rect = text(ui.painter(), pos, &t.album, body(13.0), SUBTLE, a - 16.0, Align2::LEFT_CENTER);
        if link(ui, rect, ("row-album", i, id)) {
            app.actions.push(Action::Navigate(Route::Album(t.album_key())));
        }
    }
    if cols.added.is_some() && added > 0 {
        let x = base + cols.index + cols.title + cols.album.unwrap_or(0.0);
        text(ui.painter(), Pos2::new(x, y), &short_date(app.lang(), added), body(13.0), SUBTLE, 140.0, Align2::LEFT_CENTER);
    }

    // Right edge: hover-reveal Save/Saved, duration, Dots.
    let right = base + cols.right;
    let liked = app.data.is_liked(id);
    if hovered || liked {
        let heart = Rect::from_center_size(Pos2::new(right - 82.0, y), Vec2::splat(24.0));
        let (icon, color) = if liked { (Icon::Saved, ACCENT) } else { (Icon::Save, SUBTLE) };
        let tip = if liked {
            app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs")
        } else {
            app.t("Thêm vào Bài hát đã thích", "Save to Liked Songs")
        };
        if icon_button(ui, heart, ("row-like", i, id), icon, color, tip).clicked() {
            app.actions.push(Action::ToggleLike(id));
        }
    }
    text(ui.painter(), Pos2::new(right - 38.0, y), &mmss(t.duration_ms), body(13.0), SUBTLE, 50.0, Align2::RIGHT_CENTER);

    if resp.double_clicked() {
        app.actions.push(Action::PlayContext {
            context: opts.context.clone(),
            name: opts.name.clone(),
            tracks: ids.to_vec(),
            start: Some(i),
        });
    }
    track_menu(app, &resp, id, opts.playlist, i, ids);
    if hovered {
        let dots = Rect::from_center_size(Pos2::new(right - 14.0, y), Vec2::splat(24.0));
        let tip = app.t("Tùy chọn khác", "More options");
        let d = icon_button(ui, dots, ("row-dots", i, id), Icon::Dots, TEXT, tip);
        track_popup(app, &d, id, opts.playlist, i, ids);
    }
}

pub(super) fn paint_artist_links(
    app: &mut App,
    ui: &mut Ui,
    pos: Pos2,
    artist_str: &str,
    max_w: f32,
    id_prefix: impl std::hash::Hash + std::fmt::Debug + Copy,
) -> bool {
    let parts = split_artists(artist_str);
    let font = body(13.5);
    let mut clicked_any = false;
    if parts.len() <= 1 {
        let name = parts.into_iter().next().unwrap_or_else(|| artist_str.to_string());
        let w = text_width(ui, artist_str, font.clone()).min(max_w);
        let hit = Rect::from_min_size(Pos2::new(pos.x, pos.y - 9.0), Vec2::new(w.max(8.0), 18.0));
        let can_link = !name.trim().is_empty() && (!app.lib.demo || app.lib.artist(&name).is_some());
        let hov = can_link && ui.rect_contains_pointer(hit);
        let drawn = text(
            ui.painter(),
            pos,
            artist_str,
            font,
            if hov { TEXT } else { SUBTLE },
            max_w,
            Align2::LEFT_CENTER,
        );
        if can_link && link(ui, drawn, id_prefix) {
            app.actions.push(Action::Navigate(Route::Artist(name)));
            clicked_any = true;
        }
        return clicked_any;
    }
    let mut x = pos.x;
    let end = pos.x + max_w;
    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            let sep_r = text(ui.painter(), Pos2::new(x, pos.y), ", ", font.clone(), SUBTLE, (end - x).max(1.0), Align2::LEFT_CENTER);
            x = sep_r.right();
        }
        if x >= end - 8.0 {
            break;
        }
        let rem = (end - x).max(1.0);
        let w = text_width(ui, part, font.clone()).min(rem);
        let hit = Rect::from_min_size(Pos2::new(x, pos.y - 9.0), Vec2::new(w.max(8.0), 18.0));
        let can_link = !part.trim().is_empty() && (!app.lib.demo || app.lib.artist(part).is_some());
        let hov = can_link && ui.rect_contains_pointer(hit);
        let drawn = text(
            ui.painter(),
            Pos2::new(x, pos.y),
            part,
            font.clone(),
            if hov { TEXT } else { SUBTLE },
            rem,
            Align2::LEFT_CENTER,
        );
        if can_link && link(ui, drawn, (id_prefix, idx)) {
            app.actions.push(Action::Navigate(Route::Artist(part.clone())));
            clicked_any = true;
        }
        x = drawn.right();
    }
    clicked_any
}

pub fn equalizer_glyph(ui: &Ui, rect: Rect, time: f64) {
    for (i, speed) in [5.0, 7.3, 6.1].iter().enumerate() {
        let h = 5.0 + 9.0 * ((time * speed + i as f64 * 1.7).sin().abs() as f32);
        let x = rect.center().x - 5.5 + i as f32 * 4.5;
        let bar = Rect::from_min_max(Pos2::new(x, rect.center().y + 7.0 - h), Pos2::new(x + 3.0, rect.center().y + 7.0));
        ui.painter().rect_filled(bar, 0.0, ACCENT);
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(66));
}

pub(super) fn wrapped_text(
    p: &Painter,
    pos: Pos2,
    s: &str,
    font: egui::FontId,
    color: Color32,
    max_w: f32,
    max_rows: usize,
    align: Align2,
) -> Rect {
    let mut job = LayoutJob::single_section(s.to_owned(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = TextWrapping {
        max_width: max_w.max(1.0),
        max_rows,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = p.layout_job(job);
    let rect = align.anchor_size(pos, galley.size());
    p.galley(rect.min, galley, color);
    rect
}

/// Makes painted text clickable, with the underline the original shows on hover.
pub(super) fn link(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
    focus_ring(ui, &resp, 2.0);
    if resp.hovered() || resp.has_focus() {
        ui.painter().hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, TEXT));
    }
    resp.clicked()
}

pub fn track_menu(app: &mut App, resp: &egui::Response, id: TrackId, playlist: Option<u64>, index: usize, ids: &[TrackId]) {
    let mut out = Vec::new();
    resp.context_menu(|ui| menu_contents(app, ui, id, playlist, index, ids, &mut out));
    app.actions.extend(out);
}

fn track_popup(app: &mut App, resp: &egui::Response, id: TrackId, playlist: Option<u64>, index: usize, ids: &[TrackId]) {
    let mut out = Vec::new();
    egui::Popup::menu(resp).show(|ui| menu_contents(app, ui, id, playlist, index, ids, &mut out));
    app.actions.extend(out);
}

fn menu_contents(app: &App, ui: &mut Ui, id: TrackId, playlist: Option<u64>, index: usize, ids: &[TrackId], out: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    if ui.button(app.t("Thêm vào danh sách chờ", "Add to queue")).clicked() {
        out.push(Action::Enqueue(id));
        ui.close();
    }
    let like_label = if app.data.is_liked(id) {
        app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs")
    } else {
        app.t("Thêm vào Bài hát đã thích", "Save to Liked Songs")
    };
    if ui.button(like_label).clicked() {
        out.push(Action::ToggleLike(id));
        ui.close();
    }
    ui.menu_button(app.t("Thêm vào danh sách phát", "Add to playlist"), |ui| {
        if ui.button(app.t("Danh sách phát mới", "New playlist")).clicked() {
            out.push(Action::CreatePlaylist(Some(id)));
            ui.close();
        }
        ui.separator();
        for p in &app.data.playlists {
            if ui.button(&p.name).clicked() {
                out.push(Action::AddToPlaylist(p.id, id));
                ui.close();
            }
        }
    });
    if let Some(pid) = playlist {
        ui.separator();
        let neighbors = [
            (index.wrapping_sub(1), app.t("Chuyển lên", "Move up")),
            (index + 1, app.t("Chuyển xuống", "Move down")),
        ];
        for (neighbor, label) in neighbors {
            if let Some(&other) = ids.get(neighbor) {
                if ui.button(label).clicked() {
                    out.push(Action::MoveInPlaylist(pid, id, other));
                    ui.close();
                }
            }
        }
        if ui.button(app.t("Xóa khỏi danh sách này", "Remove from this playlist")).clicked() {
            out.push(Action::RemoveFromPlaylist(pid, id));
            ui.close();
        }
    }
    if let Some(t) = app.lib.track(id) {
        ui.separator();
        if let Some(video_id) = t.youtube_id() {
            if ui.button(app.t("Mở trên YouTube Music", "Open on YouTube Music")).clicked() {
                crate::ytm::open_browser(&crate::ytm::ItemId::Song(video_id.into()));
                ui.close();
            }
            return;
        }
        if ui.button(app.t("Đến nghệ sĩ", "Go to artist")).clicked() {
            let name = split_artists(&t.artist).into_iter().next().unwrap_or_default();
            out.push(Action::Navigate(Route::Artist(name)));
            ui.close();
        }
        if ui.button(app.t("Đến album", "Go to album")).clicked() {
            out.push(Action::Navigate(Route::Album(t.album_key())));
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_grid_breakpoints_match_spotify_spec() {
        assert_eq!(quick_grid_layout(450.0), (2, 48.0, 8.0));
        assert_eq!(quick_grid_layout(650.0), (3, 48.0, 8.0));
        assert_eq!(quick_grid_layout(900.0), (4, 48.0, 8.0));
        assert_eq!(quick_grid_layout(1200.0), (4, 64.0, 12.0));
        assert_eq!(quick_grid_layout(1650.0), (4, 80.0, 12.0));
    }

    #[test]
    fn home_uses_the_measured_content_container_at_125_percent_scale() {
        let inner = 666.0 - 2.0 * home_content_padding(666.0);
        let (cols, height, gap) = quick_grid_layout(inner);
        assert_eq!(inner, 586.0);
        assert_eq!((cols, height, gap), (2, 48.0, 8.0));
        assert_eq!((inner - gap) / cols as f32, 289.0);
    }

    #[test]
    fn columns_hide_album_below_620_and_date_below_960() {
        let narrow = Columns::for_width(580.0, true, true);
        assert!(narrow.album.is_none());
        assert!(narrow.added.is_none());

        // At 1304x724 default desktop viewport, main panel is 666px and Spotify shows the Album column.
        let desktop_default = Columns::for_width(666.0, true, true);
        assert!(desktop_default.album.is_some());
        assert!(desktop_default.added.is_none());

        let wide = Columns::for_width(1020.0, true, true);
        assert!(wide.album.is_some());
        assert!(wide.added.is_some());
    }

    #[test]
    fn artist_sidebar_item_prefers_artist_avatar_and_never_collides_on_shared_yt_track() {
        let mut app = crate::app::tests::test_app();
        app.lib.demo = false;
        let collab = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("anThan00001".into()),
            title: "An Thần".into(),
            by: "Low G, Thắng".into(),
            image: Some("https://i.ytimg.com/vi/anThan00001/hqdefault.jpg".into()),
            duration_ms: 174_000,
            album: String::new(),
        };
        let track = collab.track().unwrap();
        app.lib = crate::library::Library::new(vec![track]);
        app.data.followed_artists.insert("Low G".into());
        app.data.followed_artists.insert("Thắng".into());

        // Before artist profile hydration, neither artist gets the shared collab song's cover stamped onto them.
        let low_g_pre = item_of(&app, &RecentItem::Artist("Low G".into())).unwrap();
        let thang_pre = item_of(&app, &RecentItem::Artist("Thắng".into())).unwrap();
        assert_eq!(low_g_pre.cover_path, None);
        assert_eq!(thang_pre.cover_path, None);
        assert_ne!(low_g_pre.cover_key, thang_pre.cover_key);

        // Once real artist portraits are recorded, each artist resolves its own unique portrait.
        app.data.record_artist_avatar("Low G", "https://lh3.googleusercontent.com/lowg-avatar");
        app.data.record_artist_avatar("Thắng", "https://lh3.googleusercontent.com/thang-avatar");
        let low_g = item_of(&app, &RecentItem::Artist("Low G".into())).unwrap();
        let thang = item_of(&app, &RecentItem::Artist("Thắng".into())).unwrap();
        assert_eq!(low_g.cover_path.as_deref(), Some("https://lh3.googleusercontent.com/lowg-avatar"));
        assert_eq!(thang.cover_path.as_deref(), Some("https://lh3.googleusercontent.com/thang-avatar"));
        assert_ne!(low_g.cover_key, thang.cover_key);
    }

    #[test]
    fn activating_home_song_card_plays_directly_instead_of_navigating_to_song_lyrics() {
        let mut app = crate::app::tests::test_app();
        let song = crate::ytm::Item {
            id: crate::ytm::ItemId::Song("poGyHfrJ_uo".into()),
            title: "Khóc Đấy".into(),
            by: "Thắng".into(),
            image: None,
            duration_ms: 228_000,
            album: "Cái Thứ Hai".into(),
        };
        let card = CardData::from_ytm(&app, &song, None);
        activate_card(&mut app, &card);
        assert!(matches!(app.actions.last(), Some(Action::SelectYouTubeSong(it)) if it.id == song.id));
    }

    #[test]
    fn ambient_color_transition_converges_without_u8_quantization_deadlock() {
        // Previously Color32 u8 truncation stalled forever at small dt=0.008 whenever per-channel delta < 18,
        // spinning ctx.request_repaint() at 100% CPU while idle on Home.
        let mut rgb = [42.0_f32, 42.0, 42.0];
        let target = [56.0_f32, 54.0, 58.0];
        let mut settled_frame = None;
        for frame in 1..=200 {
            let (next, animating) = step_ambient_rgb(rgb, target, 0.008);
            rgb = next;
            if !animating {
                settled_frame = Some(frame);
                break;
            }
        }
        assert!(settled_frame.is_some(), "ambient color failed to converge; stuck at {rgb:?}");
        assert_eq!(rgb, target);
        // Once settled, subsequent idle frames must report animating == false.
        let (again, animating) = step_ambient_rgb(rgb, target, 0.008);
        assert_eq!(again, target);
        assert!(!animating);
    }
}
