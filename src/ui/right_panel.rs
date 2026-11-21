//! The panel on the right: Now Playing View, Queue, Recently Played, and Friend Activity.

use std::sync::Arc;

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Id, LayerId, Order, Pos2, Rect, RichText,
    ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2,
};

use super::icons::{self, Icon};
use super::widgets::*;
use crate::app::{Action, App, RightTab, Route};
use crate::library::{split_artists, Track, TrackId};
use crate::queue::ContextRef;
use crate::theme::*;

#[derive(Clone, Copy)]
enum RowKind {
    NowPlaying,
    Queued(usize),
    Upcoming,
    History,
}

#[derive(Clone, Copy)]
struct QueueDragState {
    from: usize,
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    let rect = ui.max_rect();
    if app.friend_activity_open {
        friend_activity(app, ui, rect);
        return;
    }

    let expanded_lyrics =
        app.right_tab == RightTab::NowPlaying && app.lyrics_card_expanded && app.route != Route::Lyrics;

    if app.right_tab == RightTab::NowPlaying {
        if let Some(t) = app.current_track() {
            let texture = app.art.backdrop_texture(ui.ctx(), &t.cover_key(), t.art_source());
            let motion = app.data.settings.effective_lyrics_motion();
            let speed = if app.is_playing() { 1.15 } else { 0.22 };
            let phase = if motion { ui.input(|i| i.time) as f32 * speed } else { 0.0 };
            super::backdrop::paint(ui, rect, &texture, phase, 1.0, true, None);
            if motion && !ui.input(|i| i.viewport().minimized.unwrap_or(false)) {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(25));
            }
        }
    }

    if expanded_lyrics && app.current_track().is_some() {
        super::lyrics::card(app, ui);
        return;
    }

    let header_h = 64.0;
    let header_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header_h));
    let panel_hovered = ui.rect_contains_pointer(rect);
    if app.right_tab.is_queue() {
        draw_queue_header(app, ui, header_rect);
    }

    let body_rect = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + header_h), rect.max);
    let mut child = ui.new_child(UiBuilder::new().id_salt("right-body").max_rect(body_rect));
    child.set_clip_rect(body_rect.intersect(ui.clip_rect()));
    match app.right_tab {
        RightTab::NowPlaying => now_playing(app, &mut child, panel_hovered),
        RightTab::Queue => queue(app, &mut child),
        RightTab::History => history(app, &mut child),
    }
    if app.right_tab == RightTab::NowPlaying {
        if let Some(track) = app.current_track().filter(|_| ui.ctx().data(|d| d.get_temp::<bool>(Id::NULL.with("npv-scrolled")).unwrap_or(false))) {
            let texture = app.art.backdrop_texture(ui.ctx(), &track.cover_key(), track.art_source());
            let motion = app.data.settings.effective_lyrics_motion();
            let speed = if app.is_playing() { 1.15 } else { 0.22 };
            let phase = if motion { ui.input(|i| i.time) as f32 * speed } else { 0.0 };
            let mut fade = ui.new_child(UiBuilder::new().id_salt("npv-header-fade").max_rect(rect));
            fade.set_clip_rect(Rect::from_min_max(rect.min, Pos2::new(rect.right(), header_rect.bottom() + 32.0)).intersect(ui.clip_rect()));
            super::backdrop::paint(&fade, rect, &texture, phase, 1.0, true, Some(header_h));
            vertical_gradient(
                ui.painter(),
                Rect::from_min_max(rect.min, Pos2::new(rect.right(), header_rect.bottom() + 20.0)),
                Color32::from_black_alpha(140),
                Color32::TRANSPARENT,
            );
        }
        draw_npv_header(app, ui, rect, header_rect);
    }
}

fn close_circle_button(ui: &mut Ui, center: Pos2, id: &str, tip: &str) -> egui::Response {
    let rect = Rect::from_center_size(center, Vec2::splat(32.0));
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(tip);
    if resp.hovered() {
        ui.painter().circle_filled(rect.center(), 15.0, HOVER);
    }
    let fg = if resp.hovered() { TEXT } else { SUBTLE };
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(16.0)), Icon::Close, fg);
    resp
}

fn draw_queue_header(app: &mut App, ui: &mut Ui, header: Rect) {
    let mut x = header.left() + 16.0;
    let cy = header.center().y;
    for (tab, label) in [
        (RightTab::Queue, app.t("Danh sách chờ", "Queue")),
        (RightTab::History, app.t("Đã phát gần đây", "Recently played")),
    ] {
        let width = text_width(ui, label, bold(14.0));
        let area = Rect::from_center_size(Pos2::new(x + width * 0.5, cy), Vec2::new(width + 8.0, 38.0));
        let response = ui.interact(area, ui.scope_id().with(label), Sense::click());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tab, app.right_tab == tab, label));
        if response.clicked() {
            app.right_tab = tab;
        }
        let active = app.right_tab == tab;
        text(
            ui.painter(),
            area.center(),
            label,
            bold(14.0),
            if active || response.hovered() { TEXT } else { SUBTLE },
            width + 4.0,
            Align2::CENTER_CENTER,
        );
        if active {
            ui.painter().hline(
                (area.left() + 4.0)..=(area.right() - 4.0),
                area.bottom() - 2.0,
                Stroke::new(2.0, ACCENT),
            );
        }
        x += width + 22.0;
    }
    let tip = app.t("Đóng", "Close");
    if close_circle_button(ui, Pos2::new(header.right() - 24.0, cy), "right-close", tip).clicked() {
        app.actions.push(Action::CloseRight);
    }
}

fn draw_npv_header(app: &mut App, ui: &mut Ui, panel_rect: Rect, header: Rect) {
    let cy = header.center().y;
    let dots_open_id = Id::NULL.with("npv-dots-menu-open");
    let dots_open = ui.ctx().data_mut(|d| d.get_temp::<bool>(dots_open_id).unwrap_or(false));
    let panel_hovered = ui.rect_contains_pointer(panel_rect) || dots_open;
    let slide = ui
        .ctx()
        .animate_bool_with_time(ui.scope_id().with("npv-collapse-slide"), panel_hovered, 0.12);

    let mut controls_left = header.right() - 16.0;
    let mut next_dots_open = false;
    if slide > 0.01 {
        let expand_x = header.right() - 32.0;
        let expand_rect = Rect::from_center_size(Pos2::new(expand_x, cy), Vec2::splat(32.0));
        let expand_tip = app.t("Chế độ rạp chiếu phim", "Expand view");
        let expand_resp = ui
            .interact(expand_rect, ui.scope_id().with("npv-expand"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(expand_tip);
        if expand_resp.hovered() {
            ui.painter().circle_filled(expand_rect.center(), 14.0, HOVER);
        }
        let expand_tint = (if expand_resp.hovered() { TEXT } else { SUBTLE }).linear_multiply(slide);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(expand_rect.center(), Vec2::splat(16.0)),
            Icon::Fullscreen,
            expand_tint,
        );
        if expand_resp.clicked() {
            app.actions.push(Action::ToggleLyricsCinema);
        }
        controls_left = expand_rect.left();

        if let Some(track) = app.current_track() {
            let dots_x = expand_x - 40.0;
            let dots_rect = Rect::from_center_size(Pos2::new(dots_x, cy), Vec2::splat(32.0));
            let dots_tip = app.t("Tùy chọn khác", "More options");
            let dots_resp = ui
                .interact(dots_rect, ui.scope_id().with("npv-dots"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(dots_tip);
            if dots_resp.hovered() || dots_open {
                ui.painter().circle_filled(dots_rect.center(), 14.0, HOVER);
            }
            let dots_tint = (if dots_resp.hovered() || dots_open { TEXT } else { SUBTLE }).linear_multiply(slide);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(dots_rect.center(), Vec2::splat(16.0)),
                Icon::Dots,
                dots_tint,
            );
            next_dots_open = queue_track_popup(app, &dots_resp, track.id, RowKind::NowPlaying);
            controls_left = dots_rect.left();
        }

        let cx = egui::lerp((header.left() - 16.0)..=(header.left() + 32.0), slide);
        let collapse_rect = Rect::from_center_size(Pos2::new(cx, cy), Vec2::splat(32.0));
        let clipped = collapse_rect.intersect(header);
        if clipped.width() > 4.0 {
            let tip = app.t("Thu gọn Đang phát", "Hide Now Playing view");
            let resp = ui
                .interact(clipped, ui.scope_id().with("npv-collapse"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tip);
            let tint = (if resp.hovered() { TEXT } else { SUBTLE }).linear_multiply(slide);
            let mut p = ui.painter().clone();
            p.set_clip_rect(header.intersect(ui.clip_rect()));
            icons::paint(
                &p,
                Rect::from_center_size(collapse_rect.center(), Vec2::splat(16.0)),
                Icon::LibraryExpand,
                tint,
            );
            if resp.clicked() {
                app.actions.push(Action::CloseRight);
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(dots_open_id, next_dots_open));

    let (title, route) = npv_context_title_and_route(app);
    let title_left = header.left() + 16.0 + slide * 44.0;
    let title_max_w = (controls_left - title_left - 8.0).max(40.0);
    let title_rect = text(
        ui.painter(),
        Pos2::new(title_left, cy),
        &title,
        bold(16.0),
        TEXT,
        title_max_w,
        Align2::LEFT_CENTER,
    );
    if let Some(target_route) = route {
        let resp = ui
            .interact(title_rect.expand2(Vec2::new(0.0, 4.0)), ui.scope_id().with("npv-ctx"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.hovered() {
            ui.painter().hline(title_rect.x_range(), title_rect.bottom(), Stroke::new(1.0, TEXT));
        }
        if resp.clicked() {
            app.actions.push(Action::Navigate(target_route));
        }
    }
}

fn npv_context_title_and_route(app: &App) -> (String, Option<Route>) {
    match &app.player.context_ref {
        ContextRef::Liked => (app.t("Bài hát đã thích", "Liked Songs").to_owned(), Some(Route::Liked)),
        ContextRef::Playlist(pid) => {
            let name = app
                .data
                .playlist(*pid)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| app.player.context_name.clone());
            (name, Some(Route::Playlist(*pid)))
        }
        ContextRef::Album(key) => {
            let name = app
                .lib
                .album(key)
                .map(|a| a.title.clone())
                .unwrap_or_else(|| app.player.context_name.clone());
            (name, Some(Route::Album(key.clone())))
        }
        ContextRef::Artist(name) => (name.clone(), Some(Route::Artist(name.clone()))),
        ContextRef::All | ContextRef::Autoplay | ContextRef::None => {
            if let Some(t) = app.current_track() {
                if !t.album.trim().is_empty() {
                    (t.album.clone(), Some(Route::Album(t.album_key())))
                } else {
                    (app.t("Đang phát", "Now playing").to_owned(), None)
                }
            } else {
                (app.t("Đang phát", "Now playing").to_owned(), None)
            }
        }
    }
}

fn friend_activity(app: &mut App, ui: &mut Ui, rect: Rect) {
    let header = Rect::from_min_size(rect.min, Vec2::new(rect.width(), 56.0));
    text(
        ui.painter(),
        Pos2::new(header.left() + 16.0, header.center().y),
        app.t("Hoạt động của bạn bè", "Friend Activity"),
        bold(16.0),
        TEXT,
        rect.width() - 64.0,
        Align2::LEFT_CENTER,
    );
    let tip = app.t("Đóng", "Close");
    if close_circle_button(ui, Pos2::new(header.right() - 24.0, header.center().y), "fa-close", tip).clicked() {
        app.actions.push(Action::CloseRight);
    }
    let body = Rect::from_min_max(Pos2::new(rect.left(), header.bottom()), rect.max);
    let mut child = ui.new_child(UiBuilder::new().id_salt("fa-body").max_rect(body));
    let sub = if app.data.settings.private_session {
        app.t(
            "Phiên riêng tư đang bật. Lịch sử nghe của bạn được ẩn.",
            "Private Session is enabled. Your listening activity is hidden.",
        )
    } else {
        app.t(
            "Nhịp phát nhạc cục bộ và YouTube Music không cần tài khoản.",
            "Local playback and YouTube Music work here without an account.",
        )
    };
    empty(&mut child, app.t("Hoạt động của bạn bè", "Friend Activity"), sub);
}

fn empty(ui: &mut Ui, title: &str, sub: &str) {
    ui.add_space(70.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width((ui.available_width() - 40.0).max(120.0));
        ui.label(RichText::new(title).font(bold(18.0)));
        ui.add_space(8.0);
        ui.label(RichText::new(sub).color(SUBTLE));
    });
}

fn now_playing(app: &mut App, ui: &mut Ui, panel_hovered: bool) {
    let Some(t) = app.current_track() else {
        let (a, b) = (
            app.t("Chưa phát gì", "Nothing playing"),
            app.t("Chọn một bài hát để bắt đầu.", "Pick a song to get started."),
        );
        empty(ui, a, b);
        return;
    };

    let track_changed = ui.ctx().data_mut(|d| {
        let id = Id::NULL.with("npv-last-track");
        let prev = d.get_temp::<TrackId>(id);
        d.insert_temp(id, t.id);
        prev != Some(t.id)
    });

    let mut scroll = ScrollArea::vertical().id_salt("np-scroll").auto_shrink([false,false]);
    if let Some(offset) = ui.ctx().data(|d| d.get_temp::<f32>(Id::NULL.with("npv-screenshot-scroll"))) {
        scroll = scroll.vertical_scroll_offset(offset);
    }
    let scroll = scroll.show(ui, |ui| {
            if track_changed {
                ui.scroll_to_rect(Rect::from_min_size(ui.cursor().min, Vec2::ONE), Some(Align::TOP));
            }
            let w = ui.available_width();
            let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
            let cover_resp = npv_artwork(ui, &tex, app.data.settings.prefers_reduced_motion);
            if cover_resp.clicked() {
                app.actions.push(Action::Navigate(track_route(&t)));
            }

            // Title (marquee if overflowing) + clickable per-artist links + hover Share + Save/Saved checkmark.
            let (row, _) = ui.allocate_exact_size(Vec2::new(w, 60.0), Sense::hover());
            let share_alpha = ui.ctx().animate_bool_with_time(
                ui.scope_id().with("npv-share-alpha"),
                panel_hovered,
                0.12,
            );
            let right_reserve = if share_alpha > 0.05 { 96.0 } else { 64.0 };
            let room = (w - right_reserve - 16.0).max(60.0);
            let title_rect = Rect::from_min_size(row.min + Vec2::new(16.0, 4.0), Vec2::new(room, 28.0));
            marquee_text(ui, title_rect, ("npv-title", t.id), &t.title, bold(22.0), TEXT);
            let title_click = ui
                .interact(title_rect, ui.scope_id().with("npv-title-link"), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if title_click.clicked() {
                app.actions.push(Action::Navigate(track_route(&t)));
            }

            let artists_rect = Rect::from_min_size(row.min + Vec2::new(16.0, 35.0), Vec2::new(room, 20.0));
            draw_artist_links(app, ui, artists_rect, &t.artist, 14.0, "npv-artist");

            if share_alpha > 0.01 {
                let share_rect = Rect::from_center_size(Pos2::new(row.right() - 66.0, row.center().y), Vec2::splat(30.0));
                let share_resp = ui
                    .interact(share_rect, ui.scope_id().with("np-panel-share"), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand)
                    .on_hover_text(app.t("Sao chép liên kết bài hát", "Copy link to song"));
                let share_col = (if share_resp.hovered() { TEXT } else { SUBTLE }).linear_multiply(share_alpha);
                paint_share_icon(ui.painter(), share_rect.center(), share_col);
                if share_resp.clicked() {
                    let share_text = t
                        .youtube_id()
                        .map(|id| format!("https://music.youtube.com/watch?v={id}"))
                        .unwrap_or_else(|| format!("{} — {}", t.title, t.artist));
                    ui.ctx().copy_text(share_text);
                    app.toast = Some((
                        app.t("Đã sao chép liên kết vào bảng nhớ tạm", "Link copied to clipboard").to_string(),
                        std::time::Instant::now(),
                    ));
                }
            }

            let liked = app.data.is_liked(t.id);
            let (icon, color, tip) = if liked {
                (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
            } else {
                (Icon::Save, SUBTLE, app.t("Lưu vào Bài hát đã thích", "Save to Liked Songs"))
            };
            let heart = Rect::from_center_size(Pos2::new(row.right() - 32.0, row.center().y), Vec2::splat(30.0));
            if icon_button_sized(ui, heart, "np-panel-like", icon, color, tip, 22.0).clicked() {
                app.actions.push(Action::ToggleLike(t.id));
            }

            // Suppress duplicate Lyrics card when Route::Lyrics is already active in center.
            if app.route != Route::Lyrics {
                ui.add_space(12.0);
                super::lyrics::card(app, ui);
            } else {
                ui.add_space(12.0);
            }

            about_artist_card(app, ui, &t, w);
            ui.add_space(12.0);
            credits_card(app, ui, &t, w);

            if let Some(next) = app.player.peek_next().and_then(|id| app.lib.track(id)) {
                ui.add_space(12.0);
                next_in_queue_card(app, ui, &next, w);
            }
            ui.add_space(16.0);
        });
    ui.ctx().data_mut(|d| d.insert_temp(Id::NULL.with("npv-scrolled"), scroll.state.clamped_offset().y > 0.5));
}

fn npv_artwork(ui: &mut Ui, texture: &egui::TextureHandle, reduced_motion: bool) -> egui::Response {
    let w = ui.available_width();
    let side = (w - 32.0).max(0.0);
    let (slot, _) = ui.allocate_exact_size(Vec2::new(w, side + 16.0), Sense::hover());
    let rect = Rect::from_min_size(slot.min + Vec2::new(16.0, 0.0), Vec2::splat(side));
    let response = ui
        .interact(rect, ui.scope_id().with("npv-cover"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    let sc = if reduced_motion {
        1.0
    } else {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with("npv-cover-scale"),
            if response.hovered() { 1.018 } else { 1.0 },
            0.18,
        )
    };
    let draw_rect = Rect::from_center_size(rect.center(), rect.size() * sc);
    cover_shadow(ui.painter(), draw_rect, RADIUS_CARD);
    cover(ui, draw_rect, texture, RADIUS_CARD);
    focus_ring(ui, &response, RADIUS_CARD);
    response
}

pub(super) fn track_route(t: &Track) -> Route {
    if !t.album.trim().is_empty() {
        Route::Album(t.album_key())
    } else if let Some(video_id) = t.youtube_id() {
        Route::YouTubeMusicItem(crate::ytm::Item {
            id: crate::ytm::ItemId::Song(video_id.into()),
            title: t.title.clone(),
            by: t.artist.clone(),
            image: t.art_url.clone(),
            duration_ms: t.duration_ms,
            album: t.album.clone(),
        })
    } else {
        Route::Album(t.album_key())
    }
}

pub(super) fn draw_artist_links(app: &mut App, ui: &mut Ui, rect: Rect, artist_str: &str, font_size: f32, salt: &str) {
    let artists = split_artists(artist_str);
    let mut x = rect.left();
    let cy = rect.center().y;
    let font = body(font_size);
    for (idx, name) in artists.iter().enumerate() {
        if idx > 0 {
            let sep_w = text_width(ui, ", ", font.clone());
            if x + sep_w > rect.right() {
                break;
            }
            text(ui.painter(), Pos2::new(x, cy), ", ", font.clone(), SUBTLE, sep_w + 2.0, Align2::LEFT_CENTER);
            x += sep_w;
        }
        let remain = (rect.right() - x).max(0.0);
        if remain < 16.0 {
            break;
        }
        let w = text_width(ui, name, font.clone()).min(remain);
        let link_rect = Rect::from_center_size(Pos2::new(x + w * 0.5, cy), Vec2::new(w, rect.height()));
        let resp = ui
            .interact(link_rect, ui.scope_id().with((salt, idx, name.as_str())), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let color = if resp.hovered() { TEXT } else { SUBTLE };
        text(ui.painter(), Pos2::new(x, cy), name, font.clone(), color, remain, Align2::LEFT_CENTER);
        if resp.hovered() {
            ui.painter().hline(x..=(x + w), link_rect.bottom() - 2.0, Stroke::new(1.0, TEXT));
        }
        if resp.clicked() {
            app.actions.push(Action::Navigate(Route::Artist(name.clone())));
        }
        x += w;
    }
}

fn about_artist_card(app: &mut App, ui: &mut Ui, t: &Track, panel_w: f32) {
    let primary = split_artists(&t.artist).into_iter().next().unwrap_or_else(|| t.artist.clone());
    let artist_info = app.lib.artist(&primary).cloned();
    let artist_page = if !app.lib.demo {
        let wake = ui.ctx().clone();
        crate::ytm::artist_profile_cached(&primary, move || wake.request_repaint())
    } else {
        None
    };
    let card_w = (panel_w - 32.0).max(160.0);
    let banner_h = card_w * 0.67;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(16.0);
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 18))
            .corner_radius(card_radius(RADIUS_CARD as u8))
            .show(ui, |ui| ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                ui.set_width(card_w);
                let (banner_rect, banner_resp) =
                    ui.allocate_exact_size(Vec2::new(card_w, banner_h), Sense::click());

                let ytm_cover_key = format!("ytm-artist:{primary}");
                let (key, path) = if let Some(img) = artist_page.as_ref().and_then(|p| p.image.as_deref()) {
                    (ytm_cover_key.as_str(), Some(img))
                } else {
                    (
                        artist_info.as_ref().map(|a| a.cover_key.as_str()).unwrap_or("artist-fallback"),
                        artist_info.as_ref().and_then(|a| a.cover_path.as_deref()),
                    )
                };
                let tex = app.art.get(ui.ctx(), key, path);

                // Draw aspect-fill top artist image + rounded top-corner dark readability scrim.
                let top_radius = egui::CornerRadius {
                    nw: RADIUS_CARD as u8,
                    ne: RADIUS_CARD as u8,
                    sw: 0,
                    se: 0,
                };
                let uv = aspect_fill_uv(tex.size_vec2(), banner_rect.size());
                egui::Image::new(&tex)
                    .uv(uv)
                    .corner_radius(top_radius)
                    .paint_at(ui, banner_rect);
                ui.painter()
                    .rect_filled(banner_rect, top_radius, Color32::from_black_alpha(115));
                let lower_scrim = Rect::from_min_max(
                    Pos2::new(banner_rect.left(), banner_rect.top() + RADIUS_CARD + 2.0),
                    banner_rect.max,
                );
                if lower_scrim.height() > 2.0 {
                    vertical_gradient(
                        ui.painter(),
                        lower_scrim,
                        Color32::TRANSPARENT,
                        Color32::from_black_alpha(88),
                    );
                }
                text(
                    ui.painter(),
                    banner_rect.min + Vec2::new(16.0, 14.0),
                    app.t("Giới thiệu về nghệ sĩ", "About the artist"),
                    bold(15.0),
                    TEXT,
                    card_w - 32.0,
                    Align2::LEFT_TOP,
                );

                // Body below banner
                egui::Frame::new().inner_margin(16.0).show(ui, |ui| {
                    let inner_w = (card_w - 32.0).max(100.0);
                    ui.set_width(inner_w);

                    // Artist name + verified checkmark
                    let (name_row, _) = ui.allocate_exact_size(Vec2::new(inner_w, 24.0), Sense::hover());
                    let name_rect = text(
                        ui.painter(),
                        name_row.left_center(),
                        &primary,
                        bold(16.0),
                        TEXT,
                        inner_w - 26.0,
                        Align2::LEFT_CENTER,
                    );
                    let badge_pos = Pos2::new((name_rect.right() + 12.0).min(name_row.right() - 10.0), name_row.center().y);
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(badge_pos, Vec2::splat(16.0)),
                        Icon::VerifiedBadge,
                        TEXT,
                    );
                    let name_click = ui
                        .interact(name_rect, ui.scope_id().with("about-artist-name"), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    if name_click.hovered() {
                        ui.painter().hline(name_rect.x_range(), name_rect.bottom(), Stroke::new(1.0, TEXT));
                    }

                    ui.add_space(6.0);
                    // Monthly listeners / stats + Follow button row
                    let track_count = artist_info.as_ref().map(|a| a.tracks.len()).unwrap_or(1);
                    let album_count = artist_info.as_ref().map(|a| a.albums.len()).unwrap_or(1);
                    let stats = if let Some(sub) = artist_page.as_ref().and_then(|p| p.subtitle.as_deref()) {
                        super::ytm::format_artist_subtitle(app.lang(), sub)
                    } else if app.lib.demo {
                        let listeners = 120_000 + (crate::library::fnv(&primary) % 880_000);
                        format!(
                            "{} {} • {} {}",
                            format_thousands(listeners),
                            app.t("người nghe hằng tháng", "monthly listeners"),
                            album_count,
                            app.t("album", "albums")
                        )
                    } else {
                        format!(
                            "{track_count} {} • {album_count} {}",
                            app.t("bài hát", "songs"),
                            app.t("album", "albums")
                        )
                    };

                    let mut follow_clicked = false;
                    let followed = app.data.followed_artists.contains(&primary);
                    let follow_label = if followed {
                        app.t("Đang theo dõi", "Following")
                    } else {
                        app.t("Theo dõi", "Follow")
                    };
                    let follow_w = text_width(ui, follow_label, semi(13.0)) + 26.0;
                    let stats_text = ui.painter().layout(stats, body(14.0), SUBTLE,
                        (inner_w-follow_w-12.0).max(60.0));
                    let (stats_row, _) = ui.allocate_exact_size(Vec2::new(inner_w, stats_text.size().y.max(32.0)), Sense::hover());
                    ui.painter().galley(stats_row.min, stats_text, SUBTLE);
                    let follow_rect = Rect::from_center_size(
                        Pos2::new(stats_row.right() - follow_w * 0.5, stats_row.center().y),
                        Vec2::new(follow_w, 30.0),
                    );
                    let follow_resp = ui
                        .interact(follow_rect, ui.scope_id().with("about-follow"), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    let stroke_col = if follow_resp.hovered() { TEXT } else { MUTED };
                    ui.painter().rect_stroke(follow_rect, 15.0, Stroke::new(1.0, stroke_col), StrokeKind::Inside);
                    text(
                        ui.painter(),
                        follow_rect.center(),
                        follow_label,
                        semi(12.5),
                        TEXT,
                        follow_w,
                        Align2::CENTER_CENTER,
                    );
                    if follow_resp.clicked() {
                        follow_clicked = true;
                    }

                    ui.add_space(6.0);
                    let bio = if let Some(desc) = artist_page
                        .as_ref()
                        .and_then(|p| p.description.as_deref())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        desc.chars().take(180).collect::<String>()
                    } else {
                        String::new()
                    };
                    ui.label(RichText::new(bio).font(body(13.0)).color(SUBTLE));

                    if follow_clicked {
                        app.actions.push(Action::ToggleArtistFollow(primary.clone()));
                    } else if banner_resp.clicked() || name_click.clicked() {
                        app.actions.push(Action::Navigate(Route::Artist(primary.clone())));
                    }
                });
            }));
    });
}

fn paint_share_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let s = Stroke::new(1.6, color);
    // Open tray bottom + sides
    let left = center.x - 6.0;
    let right = center.x + 6.0;
    let bottom = center.y + 6.5;
    let rim_top = center.y + 0.5;
    painter.line_segment([Pos2::new(left, rim_top), Pos2::new(left, bottom)], s);
    painter.line_segment([Pos2::new(left, bottom), Pos2::new(right, bottom)], s);
    painter.line_segment([Pos2::new(right, bottom), Pos2::new(right, rim_top)], s);
    // Up arrow stem + head
    let tip = Pos2::new(center.x, center.y - 7.0);
    painter.line_segment([Pos2::new(center.x, center.y + 2.5), tip], s);
    painter.line_segment([tip, Pos2::new(center.x - 3.5, tip.y + 3.5)], s);
    painter.line_segment([tip, Pos2::new(center.x + 3.5, tip.y + 3.5)], s);
}

fn aspect_fill_uv(tex_size: Vec2, target_size: Vec2) -> Rect {
    let tex_aspect = tex_size.x / tex_size.y.max(1.0);
    let dst_aspect = target_size.x / target_size.y.max(1.0);
    if tex_aspect > dst_aspect {
        let u_span = (dst_aspect / tex_aspect).clamp(0.0, 1.0);
        let u0 = (1.0 - u_span) * 0.5;
        Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(1.0 - u0, 1.0))
    } else {
        let v_span = (tex_aspect / dst_aspect).clamp(0.0, 1.0);
        let v0 = (1.0 - v_span) * 0.25;
        Rect::from_min_max(Pos2::new(0.0, v0), Pos2::new(1.0, (v0 + v_span).min(1.0)))
    }
}

fn format_thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

fn credits_card(app: &mut App, ui: &mut Ui, t: &Track, panel_w: f32) {
    let card_w = (panel_w - 32.0).max(160.0);
    let show_all_id = Id::NULL.with("npv-credits-show-all");
    let mut show_all = ui.ctx().data_mut(|d| d.get_temp::<bool>(show_all_id).unwrap_or(false));
    let artists = split_artists(&t.artist);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(16.0);
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 18))
            .corner_radius(card_radius(RADIUS_CARD as u8))
            .inner_margin(16.0)
            .show(ui, |ui| ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                let inner_w = (card_w - 32.0).max(100.0);
                ui.set_width(inner_w);

                let toggle_label = if show_all { app.t("Thu gọn", "Show less") }
                    else { app.t("Hiện tất cả", "Show all") };
                let toggle_w = text_width(ui, toggle_label, semi(13.0));
                let heading = ui.painter().layout(app.t("Người tham gia thực hiện", "Credits").to_owned(),
                    bold(15.0), TEXT, (inner_w-toggle_w-12.0).max(60.0));
                let (hdr, _) = ui.allocate_exact_size(Vec2::new(inner_w, heading.size().y.max(26.0)), Sense::hover());
                ui.painter().galley(hdr.min, heading, TEXT);
                let tr = text(ui.painter(), hdr.right_center(), toggle_label, semi(13.0), SUBTLE, toggle_w, Align2::RIGHT_CENTER);
                if ui.interact(tr.expand(4.0), ui.scope_id().with("credits-toggle"), Sense::click()).clicked() {
                    show_all = !show_all;
                    ui.ctx().data_mut(|d| d.insert_temp(show_all_id, show_all));
                }
                ui.add_space(12.0);
                for artist in artists.iter().take(if show_all { usize::MAX } else { 3 }) {
                    let followed = app.data.followed_artists.contains(artist);
                    let label = if followed { app.t("Đang theo dõi", "Following") } else { app.t("Theo dõi", "Follow") };
                    let follow_w = text_width(ui, label, semi(12.0)) + 24.0;
                    let (row, _) = ui.allocate_exact_size(Vec2::new(inner_w, 52.0), Sense::hover());
                    let name = text(ui.painter(), row.min, artist, semi(15.0), TEXT,
                        (inner_w-follow_w-10.0).max(24.0), Align2::LEFT_TOP);
                    text(ui.painter(), row.min+Vec2::new(0.0,23.0), app.t("Nghệ sĩ chính", "Main Artist"),
                        body(13.0), SUBTLE, (inner_w-follow_w-10.0).max(24.0), Align2::LEFT_TOP);
                    if ui.interact(name, ui.scope_id().with(("credit-artist", artist)), Sense::click()).clicked() {
                        app.actions.push(Action::Navigate(Route::Artist(artist.clone())));
                    }
                    let follow = Rect::from_center_size(Pos2::new(row.right()-follow_w*0.5, row.top()+20.0), Vec2::new(follow_w,30.0));
                    let response = ui.interact(follow, ui.scope_id().with(("credit-follow", artist)), Sense::click())
                        .on_hover_cursor(CursorIcon::PointingHand);
                    ui.painter().rect_stroke(follow, 15.0, Stroke::new(1.0, if response.hovered() { TEXT } else { Color32::from_white_alpha(100) }), StrokeKind::Inside);
                    text(ui.painter(), follow.center(), label, semi(12.0), TEXT, follow_w, Align2::CENTER_CENTER);
                    if response.clicked() { app.actions.push(Action::ToggleArtistFollow(artist.clone())); }
                    focus_ring(ui, &response, 15.0);
                }

                if show_all {
                    if !t.album.is_empty() {
                        ui.add_space(6.0);
                        credit_line(ui, inner_w, &t.album, app.t("Album", "Album"));
                    }
                    if let Some(genre) = &t.genre {
                        ui.add_space(6.0);
                        credit_line(ui, inner_w, genre, app.t("Thể loại", "Genre"));
                    }
                    if let Some(year) = t.year {
                        ui.add_space(6.0);
                        credit_line(ui, inner_w, &year.to_string(), app.t("Năm phát hành", "Release year"));
                    }
                    let source_val = if t.youtube_id().is_some() {
                        "YouTube Music Stream"
                    } else if t.demo_seed().is_some() {
                        "Nhịp Demo Audio"
                    } else {
                        "Local Audio Library"
                    };
                    ui.add_space(6.0);
                    credit_line(ui, inner_w, source_val, app.t("Nguồn", "Source"));
                }
            }));
    });
}

fn credit_line(ui: &mut Ui, inner_w: f32, value: &str, role: &str) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(inner_w, 34.0), Sense::hover());
    text(ui.painter(), r.min, value, semi(13.5), TEXT, inner_w, Align2::LEFT_TOP);
    text(ui.painter(), r.min + Vec2::new(0.0, 17.0), role, body(12.0), SUBTLE, inner_w, Align2::LEFT_TOP);
}

fn next_in_queue_card(app: &mut App, ui: &mut Ui, next: &Arc<Track>, panel_w: f32) {
    let card_w = (panel_w - 32.0).max(160.0);
    let kind = if app.player.queue.front() == Some(&next.id) {
        RowKind::Queued(0)
    } else {
        RowKind::Upcoming
    };

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(16.0);
        egui::Frame::new()
            .fill(Color32::from_rgba_unmultiplied(255, 255, 255, 18))
            .corner_radius(card_radius(RADIUS_CARD as u8))
            .inner_margin(12.0)
            .show(ui, |ui| ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                let inner_w = (card_w - 24.0).max(100.0);
                ui.set_width(inner_w);
                let open_lbl = app.t("Mở danh sách chờ", "Open queue");
                let open_w = text_width(ui, open_lbl, semi(12.5)) + 12.0;
                let heading = ui.painter().layout(app.t("Tiếp theo trong danh sách chờ", "Next in queue").to_owned(),
                    bold(14.5), TEXT, (inner_w-open_w-16.0).max(60.0));
                let (hdr, _) = ui.allocate_exact_size(Vec2::new(inner_w, heading.size().y.max(26.0)), Sense::hover());
                ui.painter().galley(hdr.min+Vec2::new(4.0,0.0), heading, TEXT);
                let or = text(
                    ui.painter(),
                    hdr.right_center() - Vec2::new(4.0, 0.0),
                    open_lbl,
                    semi(12.5),
                    SUBTLE,
                    open_w,
                    Align2::RIGHT_CENTER,
                );
                let open_resp = ui
                    .interact(or.expand(4.0), ui.scope_id().with("npv-open-queue"), Sense::click())
                    .on_hover_cursor(CursorIcon::PointingHand);
                if open_resp.hovered() {
                    ui.painter().hline(or.x_range(), or.bottom(), Stroke::new(1.0, TEXT));
                }
                if open_resp.clicked() {
                    app.actions.push(Action::ToggleRight(RightTab::Queue));
                }
                ui.add_space(4.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(inner_w, ROW_H), Sense::hover());
                queue_row(app, ui, r, next.id, ("next-card", 0), kind);
            }));
    });
}

fn queue(app: &mut App, ui: &mut Ui) {
    if app.player.current.is_none() && app.player.queue.is_empty() {
        let (a, b) = (
            app.t("Thêm vào danh sách chờ", "Add to your queue"),
            app.t(
                "Chọn \"Thêm vào danh sách chờ\" trong menu của một bài hát.",
                "Tap \"Add to queue\" in a song's menu to see it here.",
            ),
        );
        empty(ui, a, b);
        return;
    }
    let upcoming = app.player.upcoming(60);
    let queued: Vec<TrackId> = app.player.queue.iter().copied().collect();
    let (t_now, t_next, t_clear) = (
        app.t("Đang phát", "Now playing"),
        app.t("Tiếp theo trong danh sách chờ", "Next in queue"),
        app.t("Xóa danh sách chờ", "Clear queue"),
    );
    let t_from = app.t("Nội dung tiếp theo từ", "Next from");
    let context_name = app.player.context_name.clone();
    let context_route = context_route_for(&app.player.context_ref);
    let current = app.player.current;

    let drag_id = Id::NULL.with("queue-reorder-drag");
    let mut active_drag = ui.ctx().data_mut(|d| d.get_temp::<QueueDragState>(drag_id));

    ScrollArea::vertical()
        .id_salt("queue-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = ui.available_width();
            const QUEUE_ROW_H: f32 = 64.0;
            if let Some(current) = current {
                heading(ui, t_now, None);
                let (r, _) = ui.allocate_exact_size(Vec2::new(w, QUEUE_ROW_H), Sense::hover());
                queue_row(app, ui, r.shrink2(Vec2::new(8.0, 0.0)), current, ("now", 0), RowKind::NowPlaying);
            }

            if !queued.is_empty() {
                ui.add_space(12.0);
                if heading(ui, t_next, Some(t_clear)) {
                    app.actions.push(Action::RequestClearQueue);
                }
                let mut row_rects = Vec::with_capacity(queued.len());
                for (i, id) in queued.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, QUEUE_ROW_H), Sense::hover());
                    let shrunk = r.shrink2(Vec2::new(8.0, 0.0));
                    row_rects.push(shrunk);
                    let is_dragged = active_drag.is_some_and(|d| d.from == i);
                    if is_dragged {
                        ui.painter().rect_filled(shrunk, 4.0, Color32::from_white_alpha(14));
                    }
                    let drag_started = queue_row(app, ui, shrunk, *id, ("queued", i), RowKind::Queued(i));
                    if drag_started {
                        active_drag = Some(QueueDragState { from: i });
                    }
                }

                if let Some(drag) = active_drag {
                    let pointer_pos = ui.input(|inp| inp.pointer.interact_pos());
                    let still_down = ui.input(|inp| inp.pointer.any_down());
                    if let Some(pos) = pointer_pos {
                        let mut target_slot = row_rects.len();
                        for (idx, rr) in row_rects.iter().enumerate() {
                            if pos.y < rr.center().y {
                                target_slot = idx;
                                break;
                            }
                        }
                        if still_down {
                            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                            if target_slot != drag.from && target_slot != drag.from + 1 {
                                let y = if target_slot < row_rects.len() {
                                    row_rects[target_slot].top()
                                } else {
                                    row_rects.last().map_or(0.0, |rr| rr.bottom())
                                };
                                let x_range = row_rects
                                    .first()
                                    .map_or(0.0..=w, |rr| (rr.left() + 8.0)..=(rr.right() - 8.0));
                                let overlay = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::NULL.with("queue-drop-line")));
                                overlay.hline(x_range, y, Stroke::new(1.0, ACCENT));
                                if let Some(first_x) = row_rects.first().map(|rr| rr.left() + 8.0) {
                                    overlay.circle_filled(Pos2::new(first_x, y), 3.0, ACCENT);
                                }
                            }
                        } else {
                            let to = if target_slot > drag.from {
                                target_slot - 1
                            } else {
                                target_slot
                            };
                            if to != drag.from && to < queued.len() {
                                app.actions.push(Action::MoveQueueItem(drag.from, to));
                            }
                            active_drag = None;
                        }
                    } else if !still_down {
                        active_drag = None;
                    }
                }
            } else {
                active_drag = None;
            }

            if !upcoming.is_empty() {
                ui.add_space(12.0);
                clickable_context_heading(app, ui, t_from, &context_name, context_route);
                for (i, id) in upcoming.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(w, QUEUE_ROW_H), Sense::hover());
                    queue_row(app, ui, r.shrink2(Vec2::new(8.0, 0.0)), *id, ("up", i), RowKind::Upcoming);
                }
            }
            ui.add_space(12.0);
        });

    ui.ctx().data_mut(|d| {
        if let Some(drag) = active_drag {
            d.insert_temp(drag_id, drag);
        } else {
            d.remove::<QueueDragState>(drag_id);
        }
    });
}

fn context_route_for(context: &ContextRef) -> Option<Route> {
    match context {
        ContextRef::Liked => Some(Route::Liked),
        ContextRef::Playlist(pid) => Some(Route::Playlist(*pid)),
        ContextRef::Album(key) => Some(Route::Album(key.clone())),
        ContextRef::Artist(name) => Some(Route::Artist(name.clone())),
        ContextRef::All | ContextRef::Autoplay | ContextRef::None => None,
    }
}

fn clickable_context_heading(app: &mut App, ui: &mut Ui, prefix: &str, context_name: &str, route: Option<Route>) {
    let w = ui.available_width();
    let max_total = (w - 32.0).max(40.0);
    let label = if context_name.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix} {context_name}")
    };
    let galley = ui.painter().layout(label, bold(16.0), TEXT, max_total);
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, galley.size().y.max(24.0) + 12.0), Sense::hover());
    let text_pos = Pos2::new(r.left() + 16.0, r.top() + 6.0);
    let text_rect = Rect::from_min_size(text_pos, galley.size());
    ui.painter().galley(text_pos, galley, TEXT);
    if let Some(target) = route {
        let resp = ui
            .interact(text_rect.expand2(Vec2::new(0.0, 3.0)), ui.scope_id().with("next-from-link"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if resp.hovered() {
            ui.painter().hline(text_rect.x_range(), text_rect.bottom(), Stroke::new(1.0, TEXT));
        }
        if resp.clicked() {
            app.actions.push(Action::Navigate(target));
        }
    }
}

fn history(app: &mut App, ui: &mut Ui) {
    let ids: Vec<_> = app
        .data
        .listening_history
        .iter()
        .copied()
        .filter(|id| app.lib.tracks.contains_key(id))
        .collect();
    if ids.is_empty() {
        empty(
            ui,
            app.t("Chưa nghe bài nào", "Nothing played yet"),
            app.t("Các bài bạn đã nghe trong Nhịp sẽ hiện ở đây.", "Music played in Nhịp will appear here."),
        );
        return;
    }
    ScrollArea::vertical()
        .id_salt("listening-history")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for (index, id) in ids.into_iter().enumerate() {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 64.0), Sense::hover());
                queue_row(app, ui, rect.shrink2(Vec2::new(8.0, 0.0)), id, ("history", index), RowKind::History);
            }
        });
}

/// Returns true when the action pill on the right was clicked.
fn heading(ui: &mut Ui, title: &str, action: Option<&str>) -> bool {
    let w = ui.available_width();
    let pill_w = action.map_or(0.0, |a| text_width(ui, a, semi(11.5)) + 20.0);
    let title_max_w = (w - 32.0 - if action.is_some() { pill_w + 10.0 } else { 0.0 }).max(80.0);
    let galley = ui.painter().layout(title.to_owned(), bold(16.0), TEXT, title_max_w);
    let row_h = galley.size().y.max(26.0) + 10.0;
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, row_h), Sense::hover());
    ui.painter().galley(Pos2::new(r.left() + 16.0, r.center().y - galley.size().y * 0.5), galley, TEXT);
    let mut clicked = false;
    if let Some(a) = action {
        let pill_rect = Rect::from_center_size(
            Pos2::new(r.right() - 16.0 - pill_w * 0.5, r.center().y),
            Vec2::new(pill_w, 24.0),
        );
        let resp = ui
            .interact(pill_rect, ui.scope_id().with(("hd", title)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        let draw_rect = hover_scale_rect(pill_rect, resp.hovered(), resp.is_pointer_button_down_on());
        let border = if resp.hovered() { TEXT } else { MUTED };
        let fg = if resp.hovered() { TEXT } else { SUBTLE };
        if resp.hovered() {
            ui.painter().rect_filled(draw_rect, pill_corner_radius(), hover_white(16));
        }
        ui.painter().rect_stroke(
            draw_rect,
            pill_corner_radius(),
            Stroke::new(1.0, border),
            StrokeKind::Inside,
        );
        text(ui.painter(), draw_rect.center(), a, semi(11.5), fg, pill_w - 8.0, Align2::CENTER_CENTER);
        clicked = resp.clicked();
    }
    clicked
}

/// Renders a single queue/history/now-playing row. Returns `true` if a queued drag started on this row.
fn queue_row(app: &mut App, ui: &mut Ui, r: Rect, id: TrackId, salt: (&str, usize), kind: RowKind) -> bool {
    let Some(t) = app.lib.track(id) else { return false };
    let sense = match kind {
        RowKind::Queued(_) => Sense::click_and_drag(),
        _ => Sense::click(),
    };
    let resp = ui.interact(r, ui.scope_id().with(salt), sense);
    let hovered = ui.rect_contains_pointer(r);
    if hovered {
        ui.painter().rect_filled(r, RADIUS_BASE, hover_white(26));
    }

    let cover_side = if r.height() >= 60.0 { 48.0 } else { 40.0 };
    let c = Rect::from_center_size(Pos2::new(r.left() + 8.0 + cover_side * 0.5, r.center().y), Vec2::splat(cover_side));
    let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
    cover(ui, c, &tex, RADIUS_BASE);

    let current = app.player.current == Some(id);
    let playing_this = current && app.is_playing();
    let right_controls_w = if hovered { 42.0 } else { 8.0 };
    let text_left = c.right() + 12.0;
    let room = (r.right() - text_left - right_controls_w).max(40.0);
    let title_font = if cover_side >= 48.0 { body_medium() } else { body_small() };
    let artist_sz = if cover_side >= 48.0 { 14.0 } else { 12.5 };
    text(
        ui.painter(),
        Pos2::new(text_left, r.center().y - 11.0),
        &t.title,
        title_font,
        if current { ACCENT } else { TEXT },
        room,
        Align2::LEFT_CENTER,
    );
    let artist_rect = Rect::from_min_size(
        Pos2::new(text_left, r.center().y + 1.0),
        Vec2::new(room, 20.0),
    );
    draw_artist_links(app, ui, artist_rect, &t.artist, artist_sz, &format!("{}-{}-art", salt.0, salt.1));

    // Cover hover-play scrim overlay + click to play/pause
    if hovered {
        ui.painter().rect_filled(c, 4.0, Color32::from_black_alpha(140));
        let play_icon = if playing_this { Icon::Pause } else { Icon::Play };
        icons::paint(ui.painter(), Rect::from_center_size(c.center(), Vec2::splat(16.0)), play_icon, TEXT);
        let cover_btn = ui
            .interact(c, ui.scope_id().with((salt, "cover-play")), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if cover_btn.clicked() {
            trigger_row_play(app, &t, kind);
        }
    }

    // Hover `...` context menu button (plus right-click menu on the row)
    if hovered {
        let dots = Rect::from_center_size(Pos2::new(r.right() - 20.0, r.center().y), Vec2::splat(26.0));
        let tip = app.t("Tùy chọn khác", "More options");
        let dots_resp = icon_button(ui, dots, (salt, "dots"), Icon::Dots, TEXT, tip);
        queue_track_popup(app, &dots_resp, id, kind);
    }
    queue_track_context_menu(app, &resp, id, kind);

    if resp.double_clicked() {
        trigger_row_play(app, &t, kind);
    }

    matches!(kind, RowKind::Queued(_)) && resp.drag_started()
}

fn trigger_row_play(app: &mut App, t: &Track, kind: RowKind) {
    match kind {
        RowKind::NowPlaying => {
            if app.can_play_in_app() {
                app.actions.push(Action::TogglePlay);
            }
        }
        RowKind::Queued(i) => app.actions.push(Action::PlayQueued(i)),
        RowKind::Upcoming => app.actions.push(Action::JumpUpcoming(t.id)),
        RowKind::History => {
            if let Some(item) = crate::ytm::Item::from_track(t) {
                app.actions.push(Action::SelectYouTubeSong(item));
            } else {
                app.actions.push(Action::PlayContext {
                    context: ContextRef::All,
                    name: app.t("Đã phát gần đây", "Recently played").to_owned(),
                    tracks: vec![t.id],
                    start: Some(0),
                });
            }
        }
    }
}

fn queue_track_context_menu(app: &mut App, resp: &egui::Response, id: TrackId, kind: RowKind) {
    let mut out = Vec::new();
    resp.context_menu(|ui| queue_menu_contents(app, ui, id, kind, &mut out));
    app.actions.extend(out);
}

fn queue_track_popup(app: &mut App, resp: &egui::Response, id: TrackId, kind: RowKind) -> bool {
    let mut out = Vec::new();
    let mut open = false;
    egui::Popup::menu(resp).show(|ui| {
        open = true;
        queue_menu_contents(app, ui, id, kind, &mut out);
    });
    app.actions.extend(out);
    open
}

fn queue_menu_contents(app: &App, ui: &mut Ui, id: TrackId, kind: RowKind, out: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    if let RowKind::Queued(i) = kind {
        if ui.button(app.t("Xóa khỏi danh sách chờ", "Remove from queue")).clicked() {
            out.push(Action::Dequeue(i));
            ui.close();
        }
        if ui.button(app.t("Phát ngay", "Play now")).clicked() {
            out.push(Action::PlayQueued(i));
            ui.close();
        }
        ui.separator();
    }
    if ui.button(app.t("Thêm vào danh sách chờ", "Add to queue")).clicked() {
        out.push(Action::Enqueue(id));
        ui.close();
    }
    let like_label = if app.data.is_liked(id) {
        app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs")
    } else {
        app.t("Lưu vào Bài hát đã thích", "Save to Liked Songs")
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
    fn npv_cards_stack_inside_the_sidebar_instead_of_spilling_horizontally() {
        for width in [260.0, 303.0, 388.0] {
            for kind in ["credits", "artist", "next"] {
                let mut app = crate::app::tests::test_app();
                app.lib = crate::library::Library::new(crate::library::demo_tracks());
                app.lib.demo = true;
                let mut track = (*app.lib.track(app.lib.order[0]).unwrap()).clone();
                track.artist = "Mer, ESCAH, Hoan Le".into();
                let track = Arc::new(track);
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                crate::theme::apply_style(&ctx);
                let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 1400.0));
                let mut used = Rect::NOTHING;
                let mut output = ctx.run_ui(egui::RawInput {
                    screen_rect: Some(bounds), ..Default::default()
                }, |ui| {
                    let mut child = ui.new_child(UiBuilder::new().max_rect(bounds).layout(egui::Layout::top_down(Align::Min)));
                    match kind {
                        "credits" => credits_card(&mut app, &mut child, &track, width),
                        "artist" => about_artist_card(&mut app, &mut child, &track, width),
                        _ => next_in_queue_card(&mut app, &mut child, &track, width),
                    }
                    used = child.min_rect();
                });
                output.textures_delta.clear();
                assert!(used.right() <= bounds.right() + 1.0,
                    "{kind} overflowed {width}px sidebar: {used:?}");
                let minimum = match kind { "credits" => 140.0, "artist" => 210.0, _ => 88.0 };
                assert!(used.height() >= minimum, "{kind} collapsed into a row: {used:?}");
            }
        }
    }

    #[test]
    fn npv_artwork_paints_the_content_width_even_with_a_small_texture() {
        for (panel_w, expected_side) in [(260.0, 228.0), (303.0, 271.0), (480.0, 448.0)] {
            let ctx = egui::Context::default();
            let texture = ctx.load_texture(
                "small-cover",
                egui::ColorImage::new([192, 192], vec![Color32::WHITE; 192 * 192]),
                egui::TextureOptions::LINEAR,
            );
            let panel = Rect::from_min_size(Pos2::new(993.0, 128.0), Vec2::new(panel_w, 500.0));
            let expected = Rect::from_min_size(Pos2::new(1009.0, 128.0), Vec2::splat(expected_side));
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 800.0))),
                ..Default::default()
            };
            let mut output = ctx.run_ui(input, |ui| {
                let mut child = ui.new_child(UiBuilder::new().max_rect(panel));
                assert_eq!(npv_artwork(&mut child, &texture, false).rect, expected);
            });
            output.textures_delta.clear();
            assert!(output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Rect(rect)
                    if rect.fill_texture_id() == texture.id() && rect.rect == expected)
            }), "artwork did not fill the {panel_w}px panel");
        }
    }
}
