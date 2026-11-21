//! The 88px bottom NowPlayingBar (8px top gap + 72px controls + 8px bottom margin): track info, transport controls & scrubber, utility & volume cluster.

use eframe::egui::{self, Align2, Color32, CursorIcon, Id, Order, Pos2, Rect, Sense, Stroke, Ui, Vec2};

use super::icons::{self, Icon, SpicyIcon};
use super::widgets::*;
use crate::app::{Action, App, RightTab, Route};
use crate::library::split_artists;
use crate::queue::Repeat;
use crate::theme::*;

pub fn draw(app: &mut App, ui: &mut Ui, bar: Rect) {
    // Spotify `.Root__now-playing-bar`: `padding: 8px` on all sides, with 30% / 40% (max 722px) / 30% columns.
    let inner = Rect::from_min_max(
        Pos2::new(bar.left() + 8.0, bar.top() + 8.0),
        Pos2::new(bar.right() - 8.0, bar.bottom() - 8.0),
    );
    let w = inner.width().max(1.0);
    let side_min = 180.0_f32.min(w * 0.35);
    let center_w = (w * 0.40).min(722.0).min((w - side_min * 2.0).max(120.0));
    let side_w = ((w - center_w) * 0.5).max(side_min);

    let left_rect = Rect::from_min_size(inner.min, Vec2::new(side_w, inner.height()));
    let center_rect = Rect::from_min_size(Pos2::new(left_rect.right(), inner.top()), Vec2::new(center_w, inner.height()));
    let right_rect = Rect::from_min_max(Pos2::new(center_rect.right(), inner.top()), inner.max);

    now_playing(app, ui, left_rect);
    transport(app, ui, center_rect);
    extras(app, ui, right_rect);
}

fn now_playing(app: &mut App, ui: &mut Ui, r: Rect) {
    let Some(t) = app.current_track() else { return };

    // `.main-nowPlayingBar-left`: `padding-inline-start: 8px` -> 56x56 cover starts at bar.left() + 16px.
    let c = Rect::from_center_size(Pos2::new(r.left() + 8.0 + 28.0, r.center().y), Vec2::splat(56.0));
    let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
    cover(ui, c, &tex, RADIUS_BASE);

    let npv_open = app.data.settings.show_right && app.right_tab == RightTab::NowPlaying && !app.friend_activity_open;
    let cover_tip = if npv_open {
        app.t("Thu gọn chế độ xem Đang phát", "Collapse Now Playing view")
    } else {
        app.t("Mở rộng chế độ xem Đang phát", "Expand Now Playing view")
    };
    let cover_resp = ui
        .interact(c, ui.scope_id().with("np-cover"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(cover_tip);
    if cover_resp.hovered() {
        let chevron_center = Pos2::new(c.right() - 14.0, c.top() + 14.0);
        ui.painter().circle_filled(chevron_center, 11.0, Color32::from_black_alpha(190));
        let chevron_icon = if npv_open { Icon::ChevronDown } else { Icon::ChevronUp };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(chevron_center, Vec2::splat(14.0)),
            chevron_icon,
            TEXT,
        );
    }
    if cover_resp.clicked() {
        app.actions.push(Action::ToggleRight(RightTab::NowPlaying));
    }
    super::pages::track_menu(app, &cover_resp, t.id, None, 0, &[]);

    let reserve_right = 36.0;
    let text_left = c.right() + 16.0;
    let max_text_w = (r.right() - text_left - reserve_right).max(40.0);

    // Title with 12 px/s ping-pong marquee when overflowing + hover underline & full-title tooltip.
    let title_full_w = text_width(ui, &t.title, body_small());
    let title_w = title_full_w.min(max_text_w);
    let title_rect = Rect::from_min_size(Pos2::new(text_left, r.center().y - 18.0), Vec2::new(title_w, 18.0));
    marquee_text(ui, title_rect, "np-title-marquee", &t.title, body_small(), TEXT);

    let mut title_resp = ui
        .interact(title_rect, ui.scope_id().with("np-title"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand);
    if title_full_w > max_text_w {
        title_resp = title_resp.on_hover_text(&t.title);
    }
    if title_resp.hovered() {
        ui.painter().hline(title_rect.x_range(), title_rect.bottom() - 1.0, Stroke::new(1.0, TEXT));
    }
    if title_resp.clicked() {
        let route = if !t.album.trim().is_empty() {
            Route::Album(t.album_key())
        } else if let Some(id) = t.youtube_id() {
            Route::YouTubeMusicItem(crate::ytm::Item {
                id: crate::ytm::ItemId::Song(id.into()),
                title: t.title.clone(),
                by: t.artist.clone(),
                image: t.art_url.clone(),
                duration_ms: t.duration_ms,
                album: t.album.clone(),
            })
        } else {
            Route::Album(t.album_key())
        };
        app.actions.push(Action::Navigate(route));
    }
    super::pages::track_menu(app, &title_resp, t.id, None, 0, &[]);

    // Clickable per-artist links separated by ", ".
    let artists = split_artists(&t.artist);
    let mut ax = text_left;
    let x_limit = text_left + max_text_w;
    let artist_y = r.center().y + 9.0;
    let full_artists_overflow = text_width(ui, &t.artist, marginal()) > max_text_w;

    for (i, artist_name) in artists.iter().enumerate() {
        if i > 0 {
            let sep = ", ";
            let sep_w = text_width(ui, sep, marginal());
            if ax + sep_w >= x_limit {
                break;
            }
            text(ui.painter(), Pos2::new(ax, artist_y), sep, marginal(), SUBTLE, x_limit - ax, Align2::LEFT_CENTER);
            ax += sep_w;
        }
        let rem = x_limit - ax;
        if rem <= 8.0 {
            break;
        }
        let full_aw = text_width(ui, artist_name, marginal());
        let draw_w = full_aw.min(rem);
        let a_rect = Rect::from_min_size(Pos2::new(ax, artist_y - 8.0), Vec2::new(draw_w, 16.0));
        let mut a_resp = ui
            .interact(a_rect, ui.scope_id().with(("np-artist", i)), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand);
        if full_aw > rem || full_artists_overflow {
            a_resp = a_resp.on_hover_text(&t.artist);
        }
        let hovered = a_resp.hovered();
        let color = if hovered { TEXT } else { SUBTLE };
        let drawn = text(ui.painter(), Pos2::new(ax, artist_y), artist_name, marginal(), color, rem, Align2::LEFT_CENTER);
        if hovered {
            ui.painter().hline(drawn.x_range(), drawn.bottom(), Stroke::new(1.0, TEXT));
        }
        if a_resp.clicked() {
            app.actions.push(Action::Navigate(Route::Artist(artist_name.clone())));
        }
        ax += drawn.width();
    }

    // Save / Saved (+ circle or green checkmark).
    let content_right = title_rect.right().max(ax);
    let save_max_x = r.right() - 16.0;
    let save_cx = (content_right + 18.0).min(save_max_x);
    let save_rect = Rect::from_center_size(Pos2::new(save_cx, r.center().y), Vec2::splat(24.0));
    let liked = app.data.is_liked(t.id);
    let (save_icon, save_base, save_tip) = if liked {
        (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
    } else {
        (Icon::Save, SUBTLE, app.t("Lưu vào Bài hát đã thích", "Save to Liked Songs"))
    };
    let save_resp = ui
        .interact(save_rect, ui.scope_id().with("np-like"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(save_tip);
    let save_scale = hover_scale(save_resp.hovered(), save_resp.is_pointer_button_down_on());
    let save_color = if save_resp.hovered() {
        if liked { ACCENT_HOVER } else { TEXT }
    } else {
        save_base
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(save_rect.center(), Vec2::splat(16.0 * save_scale)),
        save_icon,
        save_color,
    );
    if save_resp.clicked() {
        app.actions.push(Action::ToggleLike(t.id));
    }
}

fn transport(app: &mut App, ui: &mut Ui, r: Rect) {
    let cx = r.center().x;
    let y = r.top() + 22.0;
    let playing = app.is_playing();
    let has_track = app.can_play_in_app();
    let can_start_any = has_track || (app.current_track().is_none() && !app.lib.order.is_empty());
    let youtube = app.current_track().and_then(|track| track.youtube_id().map(str::to_owned));

    let compact = r.width() < 240.0;
    let step_inner = if compact { 40.0 } else { 48.0 }; // 32px button + 16px (or 8px compact) gap
    let step_outer = if compact { 36.0 } else { 40.0 }; // 32px button + 8px (or 4px compact) gap

    let (t_shuffle, t_prev, t_next) = (
        app.t("Trộn bài", "Shuffle"),
        app.t("Trước", "Previous"),
        app.t("Tiếp", "Next"),
    );
    let t_repeat = match app.player.repeat {
        Repeat::Off => app.t("Bật lặp lại", "Enable repeat"),
        Repeat::Context => app.t("Bật lặp lại một bài", "Enable repeat one"),
        Repeat::Track => app.t("Tắt lặp lại", "Disable repeat"),
    };

    // Shuffle
    let shuffle_color = if app.player.shuffle { ACCENT } else { SUBTLE };
    let shuffle = Rect::from_center_size(Pos2::new(cx - step_inner - step_outer, y), Vec2::splat(32.0));
    if icon_button(ui, shuffle, "shuffle", Icon::Shuffle, shuffle_color, t_shuffle)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::ToggleShuffle);
    }
    if app.player.shuffle {
        ui.painter().circle_filled(Pos2::new(shuffle.center().x, y + 18.0), 2.0, ACCENT);
    }

    // Previous
    let nav_color = if app.current_track().is_some() { SUBTLE } else { DISABLED };
    let prev = Rect::from_center_size(Pos2::new(cx - step_inner, y), Vec2::splat(32.0));
    if icon_button(ui, prev, "prev", Icon::Previous, nav_color, t_prev)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::Previous);
    }

    // Center 32x32 white circle Play/Pause (HOVER_SCALE = 1.04 on hover, subtle 0.96 press scale).
    let main = Rect::from_center_size(Pos2::new(cx, y), Vec2::splat(32.0));
    let can_play = can_start_any || youtube.is_some();
    let (icon, tip) = if playing {
        (Icon::Pause, app.t("Tạm dừng", "Pause"))
    } else if can_play {
        (Icon::Play, app.t("Phát", "Play"))
    } else {
        (Icon::Play, app.t("Phát trong Nhịp chưa khả dụng", "Playback in Nhịp is unavailable"))
    };
    let fill = if can_play { TEXT } else { MUTED };
    let play_resp = ui
        .interact(main, ui.scope_id().with("play"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(tip);
    let pressed = play_resp.is_pointer_button_down_on();
    let scale = if pressed {
        0.96
    } else {
        hover_scale(play_resp.hovered(), false)
    };
    ui.painter().circle_filled(main.center(), main.width() * 0.5 * scale, fill);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(main.center(), Vec2::splat(16.0 * scale)),
        icon,
        Color32::BLACK,
    );
    if play_resp.clicked() && can_play {
        app.actions.push(Action::TogglePlay);
    }

    // Next
    let next = Rect::from_center_size(Pos2::new(cx + step_inner, y), Vec2::splat(32.0));
    if icon_button(ui, next, "next", Icon::Next, nav_color, t_next)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::Next);
    }

    // Repeat / RepeatOne
    let (repeat_icon, repeat_color) = match app.player.repeat {
        Repeat::Off => (Icon::Repeat, SUBTLE),
        Repeat::Context => (Icon::Repeat, ACCENT),
        Repeat::Track => (Icon::RepeatOne, ACCENT),
    };
    let repeat = Rect::from_center_size(Pos2::new(cx + step_inner + step_outer, y), Vec2::splat(32.0));
    if icon_button(ui, repeat, "repeat", repeat_icon, repeat_color, t_repeat)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::CycleRepeat);
    }
    if app.player.repeat != Repeat::Off {
        ui.painter().circle_filled(Pos2::new(repeat.center().x, y + 18.0), 2.0, ACCENT);
    }

    // Scrubber row (Spotify `encore-text-marginal` 12px time labels, 50px slot width, 8px gap).
    let by = r.top() + 54.0;
    let duration = app.duration_ms();
    let position = app.position_ms();
    let time_w = 50.0;
    let bw = (r.width() - (time_w * 2.0 + 16.0)).clamp(80.0, 626.0);
    let track = Rect::from_center_size(Pos2::new(cx, by), Vec2::new(bw, 12.0));

    let value = match (app.seek_drag, duration) {
        (Some(v), _) => v,
        (None, 0) => 0.0,
        (None, d) => (position as f32 / d as f32).min(1.0),
    };
    let shown_ms = app.seek_drag.map(|v| (v as f64 * duration as f64) as u64).unwrap_or(position);

    text(
        ui.painter(),
        Pos2::new(track.left() - 8.0, by),
        &mmss(shown_ms),
        marginal(),
        SUBTLE,
        time_w,
        Align2::RIGHT_CENTER,
    );

    let remaining = app.show_remaining_time() && duration > 0;
    let right_label = if remaining {
        format!("-{}", mmss(duration.saturating_sub(shown_ms)))
    } else {
        mmss(duration)
    };
    let right_rect = Rect::from_min_size(Pos2::new(track.right() + 8.0, by - 9.0), Vec2::new(time_w, 18.0));
    let right_resp = ui
        .interact(right_rect, ui.scope_id().with("remaining-time"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(app.t("Chuyển đổi thời gian còn lại", "Toggle remaining time"));
    let right_color = if right_resp.hovered() { TEXT } else { SUBTLE };
    text(
        ui.painter(),
        Pos2::new(track.right() + 8.0, by),
        &right_label,
        marginal(),
        right_color,
        time_w,
        Align2::LEFT_CENTER,
    );
    if right_resp.clicked() {
        app.actions.push(Action::ToggleRemainingTime);
    }

    let scrubber_duration = (has_track && duration > 0).then_some(duration);
    let result = spotify_scrubber(ui, track, "seek", if has_track { value } else { 0.0 }, scrubber_duration);
    if has_track && duration > 0 {
        if let Some(v) = result.dragging {
            app.seek_drag = Some(v);
        }
        if result.released {
            if let Some(v) = app.seek_drag.take() {
                app.actions.push(Action::SeekTo((v as f64 * duration as f64) as u64));
            }
        }
    } else {
        app.seek_drag = None;
    }
}

fn extras(app: &mut App, ui: &mut Ui, r: Rect) {
    let cy = r.center().y;
    let s = &app.data.settings;
    let (volume, muted, show_right) = (s.volume, s.muted, s.show_right);
    let tab = app.right_tab;

    // Spotify `.main-nowPlayingBar-extraControls`: 32x32 buttons touching edge-to-edge (0px gap, 8px inner padding),
    // and `[data-testid="volume-bar"]` = 125px total (32px Mute button + 93px slider) + 8px trailing margin to PiP.
    let (btn_sz, gap, inline_slider) = if r.width() >= 296.0 {
        (32.0, 0.0, true)
    } else if r.width() >= 200.0 {
        (32.0, 0.0, false)
    } else {
        (28.0, 0.0, false)
    };
    let dot_offset = if btn_sz >= 32.0 { 14.0 } else { 12.5 };
    let mut x = r.right();

    // 6. Fullscreen
    let full_rect = Rect::from_center_size(Pos2::new(x - btn_sz * 0.5, cy), Vec2::splat(btn_sz));
    let is_full = if app.route == Route::Lyrics {
        app.lyrics_fullscreen.is_some() || app.lyrics_cinema
    } else {
        ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false))
    };
    let (full_icon, full_tip) = if is_full {
        (Icon::FullscreenExit, app.t("Thoát toàn màn hình", "Exit full screen"))
    } else {
        (Icon::Fullscreen, app.t("Toàn màn hình", "Full screen"))
    };
    let full_color = if is_full && app.route == Route::Lyrics { ACCENT } else { SUBTLE };
    if icon_button(ui, full_rect, "fullscreen", full_icon, full_color, full_tip)
        .on_hover_cursor(CursorIcon::PointingHand)
        .clicked()
    {
        if app.route == Route::Lyrics {
            if app.lyrics_fullscreen.is_some() {
                app.actions.push(Action::ToggleLyricsFullscreen);
            } else {
                app.actions.push(Action::ToggleLyricsCinema);
            }
        } else {
            app.actions.push(Action::ToggleLyricsCinema);
        }
    }
    x -= btn_sz + gap;

    // 5. Miniplayer / Popup Lyrics (`Icons.PiPMode`, Icons.ts:160)
    let mini_rect = Rect::from_center_size(Pos2::new(x - btn_sz * 0.5, cy), Vec2::splat(btn_sz));
    let mini_active = app.miniplayer_open;
    let mini_tip = if mini_active {
        app.t("Đóng cửa sổ lời bài hát nổi", "Close Popup Lyrics")
    } else {
        app.t("Cửa sổ lời bài hát nổi", "Popup Lyrics")
    };
    let mini_resp = ui
        .interact(mini_rect, Id::unique("miniplayer"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(mini_tip);
    mini_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, mini_tip));
    focus_ring(ui, &mini_resp, RADIUS_BASE);
    let mini_col = if mini_active {
        if mini_resp.hovered() { ACCENT_HOVER } else { ACCENT }
    } else if mini_resp.hovered() {
        TEXT
    } else {
        SUBTLE
    };
    let mini_scale = hover_scale(mini_resp.hovered(), mini_resp.is_pointer_button_down_on());
    icons::paint_spicy(ui.painter(), Rect::from_center_size(mini_rect.center(), Vec2::splat(16.0 * mini_scale)), SpicyIcon::PiPMode, mini_col);
    if mini_resp.clicked() {
        app.actions.push(Action::ToggleMiniplayer);
    }
    if mini_active {
        ui.painter().circle_filled(Pos2::new(mini_rect.center().x, cy + dot_offset), 1.8, ACCENT);
    }
    x -= btn_sz + gap;

    // 4. Volume cluster
    let vol_icon = if muted || volume <= 0.001 {
        Icon::Mute
    } else if volume < 1.0 / 3.0 {
        Icon::VolumeLow
    } else if volume < 2.0 / 3.0 {
        Icon::VolumeMedium
    } else {
        Icon::Volume
    };
    let (t_mute, t_unmute) = (app.t("Tắt tiếng", "Mute"), app.t("Bật tiếng", "Unmute"));
    let shown = if muted { 0.0 } else { volume };

    if inline_slider {
        // 8px trailing margin between volume slider and PiP/Miniplayer, 93px slider + 32px speaker = 125px total.
        x -= 8.0;
        let slider_w = 93.0;
        let vol_bar = Rect::from_center_size(Pos2::new(x - slider_w * 0.5, cy), Vec2::new(slider_w, 12.0));
        let result = spotify_scrubber(ui, vol_bar, "volume", shown, None);
        if let Some(v) = result.dragging {
            app.actions.push(Action::SetVolume(v));
        }
        x = vol_bar.left();
        let speaker = Rect::from_center_size(Pos2::new(x - btn_sz * 0.5, cy), Vec2::splat(btn_sz));
        if icon_button(ui, speaker, "mute", vol_icon, SUBTLE, if muted { t_unmute } else { t_mute })
            .on_hover_cursor(CursorIcon::PointingHand)
            .clicked()
        {
            app.actions.push(Action::ToggleMute);
        }
        let cluster_rect = Rect::from_min_max(
            Pos2::new(speaker.left() - 2.0, r.top() + 8.0),
            Pos2::new(vol_bar.right() + 2.0, r.bottom() - 8.0),
        );
        if ui.rect_contains_pointer(cluster_rect) {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta.abs() > 0.0 {
                app.actions.push(Action::SetVolume((shown + delta / 1200.0).clamp(0.0, 1.0)));
            }
        }
        x = speaker.left() - gap;
    } else {
        let speaker = Rect::from_center_size(Pos2::new(x - btn_sz * 0.5, cy), Vec2::splat(btn_sz));
        let speaker_resp = icon_button(ui, speaker, "mute", vol_icon, SUBTLE, if muted { t_unmute } else { t_mute })
            .on_hover_cursor(CursorIcon::PointingHand);
        if speaker_resp.clicked() {
            app.actions.push(Action::ToggleMute);
        }
        volume_popover(app, ui, speaker, shown, &speaker_resp);
        if ui.rect_contains_pointer(speaker.expand(4.0)) {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta.abs() > 0.0 {
                app.actions.push(Action::SetVolume((shown + delta / 1200.0).clamp(0.0, 1.0)));
            }
        }
        x = speaker.left() - gap;
    }

    // 3. Queue, 2. Now Playing View, 1. Lyrics (left-to-right: Lyrics -> NPV -> Queue)
    for (id, icon, want, tip) in [
        ("q", Icon::Queue, Some(RightTab::Queue), app.t("Danh sách chờ", "Queue")),
        ("np", Icon::NowPlaying, Some(RightTab::NowPlaying), app.t("Đang phát", "Now playing")),
        ("lyr", Icon::Lyrics, None, app.t("Lời bài hát", "Lyrics")),
    ] {
        let rect = Rect::from_center_size(Pos2::new(x - btn_sz * 0.5, cy), Vec2::splat(btn_sz));
        let active = want.map_or(app.route == Route::Lyrics, |want| {
            show_right && !app.friend_activity_open && (tab == want || want.is_queue() && tab.is_queue())
        });
        let btn_resp = if id == "lyr" {
            let resp = ui
                .interact(rect, Id::unique(id), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(tip);
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
            focus_ring(ui, &resp, RADIUS_BASE);
            let wand_col = if active {
                if resp.hovered() { ACCENT_HOVER } else { ACCENT }
            } else if resp.hovered() {
                TEXT
            } else {
                SUBTLE
            };
            let wand_scale = hover_scale(resp.hovered(), resp.is_pointer_button_down_on());
            icons::paint_spicy(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(19.0 * wand_scale)), SpicyIcon::LyricsPage, wand_col);
            resp
        } else {
            icon_button(ui, rect, id, icon, if active { ACCENT } else { SUBTLE }, tip)
                .on_hover_cursor(CursorIcon::PointingHand)
        };
        if btn_resp.clicked() {
            app.actions.push(want.map_or(Action::ToggleLyrics, Action::ToggleRight));
        }
        if active {
            ui.painter().circle_filled(Pos2::new(rect.center().x, cy + dot_offset), 1.8, ACCENT);
        }
        x -= btn_sz + gap;
    }
}

/// Vertical volume slider popover shown on hover when the window is too narrow for the 125px inline slider.
fn volume_popover(app: &mut App, ui: &mut Ui, anchor: Rect, shown: f32, anchor_resp: &egui::Response) {
    let state_id = ui.scope_id().with("vol-pop-state");
    let (was_open, was_dragging) = ui.ctx().data_mut(|d| {
        (
            d.get_temp::<bool>(state_id.with("open")).unwrap_or(false),
            d.get_temp::<bool>(state_id.with("drag")).unwrap_or(false),
        )
    });

    let pop_size = Vec2::new(36.0, 128.0);
    let pop_rect = Rect::from_min_size(
        Pos2::new(anchor.center().x - pop_size.x * 0.5, anchor.top() - pop_size.y - 6.0),
        pop_size,
    );
    let bridge = Rect::from_min_max(
        pop_rect.min,
        Pos2::new(pop_rect.right().max(anchor.right()), anchor.bottom()),
    );
    let open = anchor_resp.hovered()
        || (was_open && (ui.rect_contains_pointer(bridge) || was_dragging));
    ui.ctx().data_mut(|d| d.insert_temp(state_id.with("open"), open));
    if !open {
        ui.ctx().data_mut(|d| d.insert_temp(state_id.with("drag"), false));
        return;
    }

    egui::Area::new(Id::unique("player-bar-vol-popover"))
        .order(Order::Foreground)
        .fixed_pos(pop_rect.min)
        .show(ui.ctx(), |ui| {
            let shadow = ui.style().visuals.popup_shadow;
            ui.painter().add(shadow.as_shape(pop_rect, card_radius(RADIUS_CARD as u8)));
            ui.painter().rect_filled(pop_rect, RADIUS_CARD, POPUP);
            ui.painter().rect_stroke(pop_rect, RADIUS_CARD, Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);

            let track = Rect::from_center_size(pop_rect.center(), Vec2::new(4.0, 100.0));
            let hit = Rect::from_center_size(pop_rect.center(), Vec2::new(24.0, 104.0));
            let resp = ui.interact(hit, state_id.with("vslider"), Sense::click_and_drag());
            let dragging = resp.dragged() || resp.is_pointer_button_down_on();
            ui.ctx().data_mut(|d| d.insert_temp(state_id.with("drag"), dragging));

            let mut current = shown.clamp(0.0, 1.0);
            if dragging || resp.clicked() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    current = ((track.bottom() - pos.y) / track.height()).clamp(0.0, 1.0);
                    app.actions.push(Action::SetVolume(current));
                }
            }
            if ui.rect_contains_pointer(pop_rect) {
                let delta = ui.input(|i| i.smooth_scroll_delta.y);
                if delta.abs() > 0.0 {
                    current = (current + delta / 1200.0).clamp(0.0, 1.0);
                    app.actions.push(Action::SetVolume(current));
                }
            }

            let active = resp.hovered() || dragging;
            let fill_top = track.bottom() - track.height() * current;
            let filled = Rect::from_min_max(Pos2::new(track.left(), fill_top), track.max);
            let p = ui.painter();
            p.rect_filled(track, 2.0, TRACK_ALPHA);
            p.rect_filled(filled, 2.0, if active { ACCENT_ALT } else { TEXT });
            if active {
                let knob_center = Pos2::new(track.center().x, fill_top);
                let knob_rect = Rect::from_center_size(knob_center, Vec2::splat(12.0));
                let knob_shadow = egui::epaint::Shadow {
                    offset: [0, 2],
                    blur: 4,
                    spread: 0,
                    color: Color32::from_black_alpha(0x80),
                };
                p.add(knob_shadow.as_shape(knob_rect, pill_corner_radius()));
                p.circle_filled(knob_center, 6.0, TEXT);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_cluster_shows_inline_93px_volume_at_desktop_and_collapses_at_compact() {
        let mut app = crate::app::tests::test_app();
        app.lib = crate::library::Library::new(crate::library::demo_tracks());
        app.lib.demo = true;
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        apply_style(&ctx);

        let mut has_93px_track = |width: f32| -> (bool, f32) {
            let bar = Rect::from_min_size(Pos2::ZERO, Vec2::new(width, PLAYER_BAR));
            let inner_w = width - 16.0;
            let side_min = 180.0_f32.min(inner_w * 0.35);
            let center_w = (inner_w * 0.40).min(722.0).min((inner_w - side_min * 2.0).max(120.0));
            let side_w = ((inner_w - center_w) * 0.5).max(side_min);
            let right_left = 8.0 + side_w + center_w;

            let mut out = ctx.run_ui(
                egui::RawInput { screen_rect: Some(bar), ..Default::default() },
                |ui| {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar));
                    draw(&mut app, &mut child, bar);
                },
            );
            out.textures_delta.clear();
            let mut found_93 = false;
            let mut min_right_shape_x = f32::INFINITY;
            for cs in &out.shapes {
                if let egui::Shape::Rect(r) = &cs.shape {
                    if r.rect.left() >= right_left - 2.0 && (r.rect.width() - 93.0).abs() <= 1.0 && r.rect.height() <= 6.0 {
                        found_93 = true;
                    }
                    if r.rect.left() >= right_left - 40.0 && r.rect.right() > right_left {
                        min_right_shape_x = min_right_shape_x.min(r.rect.left());
                    }
                }
            }
            (found_93, min_right_shape_x)
        };

        let (desktop_inline, _) = has_93px_track(1304.0);
        assert!(desktop_inline, "expected 93px inline volume bar at 1304px desktop width");

        let (compact_inline, _) = has_93px_track(800.0);
        assert!(!compact_inline, "expected volume slider to collapse into popover at 800px compact width");
    }
}
