//! Window layout: the top bar, the panels and the player along the bottom.

pub mod icons;
mod backdrop;
mod lyrics;
mod lyrics_gpu;
mod overlays;
mod pages;
mod player_bar;
mod profile;
mod right_panel;
mod settings;
mod sidebar;
pub mod widgets;
pub mod ytm;

use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Id, Key, LayerId, Layout, Order, Pos2, Rect,
    RichText, ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, Vec2,
};

use crate::app::{Action, App, RightTab, Route, TopPopover};
use crate::library::{fold, RecentItem, SurfaceStyle};
use crate::queue::Repeat;
use crate::theme::*;
use icons::Icon;
use overlays::menu_row;
use pages::{item_of, paint_item_cover};
use widgets::*;

/// Critically-damped exponential slide step for full-window Cinema transition.
/// Clamping `dt` to `<= 28ms` prevents the first layout frame from teleporting across the screen,
/// while smooth exponential deceleration eliminates the sluggish ramp-up of symmetric cubic easing.
fn step_cinema_slide(current: f32, target: f32, dt: f32) -> (f32, bool) {
    let diff = target - current;
    if diff.abs() < 0.0025 {
        return (target, false);
    }
    let dt = dt.clamp(0.001, 0.028);
    let k = 1.0 - (-dt * 12.5).exp();
    let next = (current + diff * k).clamp(0.0, 1.0);
    let still_moving = (target - next).abs() >= 0.0025;
    (if still_moving { next } else { target }, still_moving)
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    let full = ui.max_rect();
    let surface = app.data.settings.surface_style;
    draw_window_surface(app, ui, full, surface);

    let top = Rect::from_min_size(full.min, Vec2::new(full.width(), TOP_BAR));
    let bar = Rect::from_min_max(Pos2::new(full.left(), full.bottom() - PLAYER_BAR), full.max);
    let body = Rect::from_min_max(Pos2::new(full.left() + GAP, top.bottom()), Pos2::new(full.right() - GAP, bar.top()));

    let want_cinema =
        app.route == Route::Lyrics && (app.lyrics_cinema || app.lyrics_fullscreen.is_some());
    let target_cinema = if want_cinema { 1.0_f32 } else { 0.0_f32 };
    let slide_id = Id::NULL.with("cinema-slide-progress");
    let (cinema_ease, cinema_animating) = if let Some(forced) = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(Id::NULL.with("cinema-slide-override")))
    {
        (forced.clamp(0.0, 1.0), false)
    } else if app.screenshot.is_some() || !app.data.settings.effective_lyrics_motion() {
        ui.ctx().data_mut(|d| d.insert_temp(slide_id, target_cinema));
        (target_cinema, false)
    } else {
        let cur = ui.ctx().data(|d| d.get_temp::<f32>(slide_id)).unwrap_or(target_cinema);
        let dt = ui.input(|i| i.unstable_dt);
        let (next, moving) = step_cinema_slide(cur, target_cinema, dt);
        ui.ctx().data_mut(|d| d.insert_temp(slide_id, next));
        (next, moving)
    };
    if cinema_animating {
        ui.ctx().request_repaint();
    }
    // Signal underlying center lyrics panel to skip redundant offscreen GPU blur passes while the
    // full-window cinema stage is already sliding over it, keeping frame pacing rock-solid at 60–144fps.
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            Id::NULL.with("cinema-slide-obscuring"),
            cinema_ease > 0.15 && cinema_ease < 0.999,
        );
    });

    if cinema_ease >= 0.999 {
        if app.lyrics_fullscreen.is_none() {
            window_drag(ui, top);
        }
        // Share identical `id_salt("lyrics-cinema-stage")` with the sliding overlay so ScrollArea,
        // LyricsFollowState, BackdropState, and line animations never reset or snap on landing.
        let mut stage = ui.new_child(UiBuilder::new().id_salt("lyrics-cinema-stage").max_rect(full));
        stage.set_clip_rect(full);
        lyrics::draw(app, &mut stage);
        let menu = Rect::from_center_size(Pos2::new(top.left() + 34.0, top.center().y), Vec2::splat(28.0));
        let response = icon_button_sized(
            ui,
            menu,
            "main-menu",
            Icon::Dots,
            Color32::from_white_alpha(225),
            app.t("Trình đơn", "Menu"),
            18.0,
        );
        main_menu(app, ui, &response);
        if app.lyrics_fullscreen.is_none() {
            let cap = Rect::from_min_size(Pos2::new(top.right() - 135.0, top.top()), Vec2::new(135.0, 32.0));
            ui.painter().rect_filled(cap, 0.0, Color32::from_black_alpha(110));
            window_buttons(ui, top);
            resize_edges(ui, full);
        }
        overlays::draw(app, ui, full);
        return;
    }

    let user_collapsed = app.data.settings.left_collapsed;
    let max_left = app.max_left_width(full.width());
    let mut left_w = if user_collapsed {
        LEFT_COLLAPSED
    } else {
        app.data.settings.left_width.clamp(LEFT_MIN, max_left)
    };
    let requested_right = app.data.settings.show_right.then_some(app.data.settings.right_width);
    let avail = body.width() - left_w - GAP * 2.0;
    let (mut right_w, mut overlay_right) = dock_or_overlay_right(avail, requested_right);
    if overlay_right && !app.right_tab.is_queue() && !app.friend_activity_open && app.player.current.is_none() {
        right_w = 0.0;
        overlay_right = false;
    }

    // Auto-collapse left sidebar visually on narrow windows (e.g. 800x600) so the right panel
    // can dock cleanly beside main content when MIN_MAIN_WIDTH (416px) still fits, or overlay cleanly.
    let mut auto_collapsed = false;
    if overlay_right && !user_collapsed {
        let avail_collapsed = body.width() - LEFT_COLLAPSED - GAP * 2.0;
        let (rw_collapsed, ov_collapsed) = dock_or_overlay_right(avail_collapsed, requested_right);
        left_w = LEFT_COLLAPSED;
        right_w = rw_collapsed;
        overlay_right = ov_collapsed;
        auto_collapsed = true;
    }
    if overlay_right && !app.right_tab.is_queue() && !app.friend_activity_open && app.player.current.is_none() {
        right_w = 0.0;
        overlay_right = false;
    }
    let effective_collapsed = user_collapsed || auto_collapsed;

    let left = Rect::from_min_max(body.min, Pos2::new(body.left() + left_w, body.bottom()));
    let right_edge = if right_w > 0.0 && !overlay_right { body.right() - right_w - GAP } else { body.right() };
    let main = Rect::from_min_max(Pos2::new(left.right() + GAP, body.top()), Pos2::new(right_edge, body.bottom()));

    window_drag(ui, top);
    top_bar(app, ui, top);

    let saved_collapsed = app.data.settings.left_collapsed;
    let panel_opacity = app.data.settings.panel_opacity.clamp(0.10, 0.95);
    app.data.settings.left_collapsed = effective_collapsed;
    panel(ui, left, "library", surface, panel_opacity, |ui| sidebar::draw(app, ui));
    app.data.settings.left_collapsed = saved_collapsed;

    panel(ui, main, "main", surface, panel_opacity, |ui| pages::draw(app, ui));

    if right_w > 0.0 {
        let right = Rect::from_min_max(Pos2::new(body.right() - right_w, body.top()), body.max);
        if overlay_right {
            // Paint a dim scrim over main/left and a drop shadow behind the floating right panel.
            egui::Area::new(Id::unique("right-overlay-scrim"))
                .order(Order::Foreground)
                .fixed_pos(body.min)
                .show(ui.ctx(), |ui| {
                    let (scrim_rect, scrim_resp) = ui.allocate_exact_size(body.size(), Sense::click());
                    ui.painter().rect_filled(scrim_rect, RADIUS, Color32::from_black_alpha(150));
                    if scrim_resp.clicked() {
                        app.actions.push(Action::CloseRight);
                    }
                });
            egui::Area::new(Id::unique("right-panel-overlay"))
                .order(Order::Foreground)
                .fixed_pos(right.min)
                .show(ui.ctx(), |ui| {
                    ui.set_min_size(right.size());
                    let shadow = egui::epaint::Shadow {
                        offset: [-8, 0],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(190),
                    };
                    ui.painter().add(shadow.as_shape(right, card_radius(RADIUS)));
                    panel(ui, right, "right-overlay", surface, panel_opacity, |ui| right_panel::draw(app, ui));
                    let edge = Rect::from_min_max(Pos2::new(right.left() - GAP, right.top()), Pos2::new(right.left(), right.bottom()));
                    resize_right(app, ui, edge, body.width() - left_w - GAP * 2.0);
                });
        } else {
            panel(ui, right, "right", surface, panel_opacity, |ui| right_panel::draw(app, ui));
            resize_right(
                app,
                ui,
                Rect::from_min_max(Pos2::new(main.right(), body.top()), Pos2::new(right.left(), body.bottom())),
                body.width() - left_w - GAP * 2.0,
            );
        }
    }

    if !auto_collapsed {
        resize_left(
            app,
            ui,
            Rect::from_min_max(Pos2::new(left.right(), body.top()), Pos2::new(main.left(), body.bottom())),
            body.left(),
            full.width(),
        );
    }

    player_bar::draw(app, ui, bar);

    // Smooth elevated sheet rise + crossfade transition when entering or exiting full-window Cinema View.
    // Shares `id_salt("lyrics-cinema-stage")` with the settled full-window stage for zero jump on landing.
    if cinema_ease > 0.001 {
        let layer = LayerId::new(Order::Foreground, Id::NULL.with("cinema-slide-layer"));
        let scrim_painter = ui.ctx().layer_painter(layer);
        scrim_painter.rect_filled(
            full,
            0.0,
            Color32::from_black_alpha((175.0 * cinema_ease).round().clamp(0.0, 255.0) as u8),
        );
        let max_rise = (full.height() * 0.24).min(210.0);
        let slide_y = max_rise * (1.0 - cinema_ease);
        let slide_rect = full.translate(Vec2::new(0.0, slide_y));
        let shadow = egui::epaint::Shadow {
            offset: [0, -10],
            blur: 28,
            spread: 0,
            color: Color32::from_black_alpha((130.0 * cinema_ease).round().clamp(0.0, 255.0) as u8),
        };
        let top_rad = ((1.0 - cinema_ease) * 16.0).round() as u8;
        scrim_painter.add(shadow.as_shape(slide_rect, egui::CornerRadius::same(top_rad)));

        let mut stage = ui.new_child(
            UiBuilder::new()
                .layer_id(layer)
                .id_salt("lyrics-cinema-stage")
                .max_rect(slide_rect),
        );
        stage.set_clip_rect(full.intersect(slide_rect));
        stage.set_opacity((cinema_ease * 1.45).clamp(0.0, 1.0));
        let prev_cinema = app.lyrics_cinema;
        let prev_route = app.route.clone();
        app.lyrics_cinema = true;
        app.route = Route::Lyrics;
        lyrics::draw(app, &mut stage);
        app.lyrics_cinema = prev_cinema;
        app.route = prev_route;
    }

    overlays::draw(app, ui, full);
    resize_edges(ui, full);
}

fn right_layout(available: f32, requested: Option<f32>) -> (f32, bool) {
    let Some(requested) = requested else { return (0.0, false) };
    let width = requested.clamp(RIGHT_MIN, RIGHT_MAX)
        .min((available - MIN_MAIN_WIDTH).max(RIGHT_MIN))
        .min(available.max(0.0));
    (width, available - width < MIN_MAIN_WIDTH)
}

/// Enforce Spotify's `MIN_MAIN_WIDTH` (416px) when deciding whether the right panel overlays main.
fn dock_or_overlay_right(available: f32, requested: Option<f32>) -> (f32, bool) {
    let (width, overlay) = right_layout(available, requested);
    (width, overlay || (width > 0.0 && available - width < MIN_MAIN_WIDTH))
}

#[cfg(test)]
mod layout_tests {
    use super::{dock_or_overlay_right, right_layout};

    #[test]
    fn right_panel_stays_reachable_at_narrow_widths() {
        assert_eq!(right_layout(800.0, Some(354.0)), (354.0, false));
        assert_eq!(right_layout(660.0, Some(354.0)), (260.0, true));
        assert_eq!(right_layout(500.0, Some(354.0)), (260.0, true));
        assert_eq!(right_layout(500.0, None), (0.0, false));
    }

    #[test]
    fn small_window_docks_when_both_panel_minimums_fit() {
        // 800px window with collapsed left (72px) and outer/inner gaps leaves 696px.
        assert_eq!(dock_or_overlay_right(696.0, Some(354.0)), (280.0, false));
        // Wide enough for 416px main + 354px right docks without overlay.
        assert_eq!(dock_or_overlay_right(800.0, Some(354.0)), (354.0, false));
    }

    #[test]
    fn cinema_slide_is_monotonic_clamps_first_frame_spike_and_settles() {
        // Even if the initial layout frame stalls for 120ms, dt clamping prevents jumping > 35%.
        let (first, moving) = super::step_cinema_slide(0.0, 1.0, 0.12);
        assert!(moving && first > 0.15 && first < 0.35, "first={first}");
        // Converges smoothly to 1.0 within ~0.55s at 60fps.
        let mut p = 0.0_f32;
        for _ in 0..36 {
            let (next, _) = super::step_cinema_slide(p, 1.0, 1.0 / 60.0);
            assert!(next >= p, "must advance monotonically");
            p = next;
        }
        assert_eq!(p, 1.0);
        // Reverses smoothly mid-flight back to 0.0 without snapping.
        let (rev, rev_moving) = super::step_cinema_slide(0.6, 0.0, 1.0 / 60.0);
        assert!(rev_moving && rev < 0.6 && rev > 0.4);
    }
}

/// The window has no system title bar: the top bar moves it instead.
fn window_drag(ui: &mut Ui, top: Rect) {
    let resp = ui.interact(top, ui.scope_id().with("window-drag"), Sense::click_and_drag());
    if resp.double_clicked() {
        toggle_maximize(ui.ctx());
    } else if resp.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
}

fn toggle_maximize(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!maximized));
}

/// Without a system frame the edges have to resize the window themselves.
pub(super) fn resize_edges(ui: &mut Ui, full: Rect) {
    use egui::{viewport::ResizeDirection as D, CursorIcon as C};
    if ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    const T: f32 = 5.0;
    let (x0, x1, y0, y1) = (full.left(), full.right(), full.top(), full.bottom());
    let edges = [
        (Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x0 + T, y1)), D::West, C::ResizeHorizontal),
        (Rect::from_min_max(Pos2::new(x1 - T, y0), Pos2::new(x1, y1)), D::East, C::ResizeHorizontal),
        (Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y0 + T)), D::North, C::ResizeVertical),
        (Rect::from_min_max(Pos2::new(x0, y1 - T), Pos2::new(x1, y1)), D::South, C::ResizeVertical),
        (Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x0 + T * 2.0, y0 + T * 2.0)), D::NorthWest, C::ResizeNwSe),
        (Rect::from_min_max(Pos2::new(x1 - T * 2.0, y1 - T * 2.0), Pos2::new(x1, y1)), D::SouthEast, C::ResizeNwSe),
        (Rect::from_min_max(Pos2::new(x1 - T * 2.0, y0), Pos2::new(x1, y0 + T * 2.0)), D::NorthEast, C::ResizeNeSw),
        (Rect::from_min_max(Pos2::new(x0, y1 - T * 2.0), Pos2::new(x0 + T * 2.0, y1)), D::SouthWest, C::ResizeNeSw),
    ];
    for (i, (rect, direction, cursor)) in edges.into_iter().enumerate() {
        let resp = ui.interact(rect, ui.scope_id().with(("edge", i)), Sense::drag()).on_hover_cursor(cursor);
        if resp.drag_started() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(direction));
        }
    }
}

fn main_menu(app: &mut App, ui: &Ui, resp: &egui::Response) {
    let mut open_help = false;
    let mut open_about_dialog = false;
    let mut out = Vec::new();
    let lang = app.lang();
    let t = |vi: &'static str, en: &'static str| match lang {
        crate::library::Lang::Vi => vi,
        crate::library::Lang::En => en,
    };

    egui::Popup::menu(resp)
        .frame(menu_frame(ui.style()))
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.set_min_width(156.0);

            ui.menu_button(t("Tệp", "File"), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_min_width(196.0);
                if menu_row(ui, Some(Icon::Plus), t("Danh sách phát mới", "New Playlist"), Some("Ctrl+N"), false, true).clicked() {
                    out.push(Action::CreatePlaylist(None));
                    ui.close();
                }
                if menu_row(ui, Some(Icon::Folder), t("Thư mục danh sách phát mới", "New Playlist Folder"), Some("Ctrl+Shift+N"), false, true).clicked() {
                    out.push(Action::CreateFolder(None));
                    ui.close();
                }
                if menu_row(ui, None, t("Phiên riêng tư", "Private Session"), None, app.data.settings.private_session, true).clicked() {
                    out.push(Action::TogglePrivateSession);
                    ui.close();
                }
                ui.separator();
                if menu_row(ui, Some(Icon::Close), t("Thoát", "Exit"), Some("Alt+F4"), false, true).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    ui.close();
                }
            });

            ui.menu_button(t("Chỉnh sửa", "Edit"), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_min_width(196.0);
                if menu_row(ui, Some(Icon::Search), t("Tìm kiếm", "Search"), Some("Ctrl+L"), false, true).clicked() {
                    app.focus_search = true;
                    app.search_dropdown_open = true;
                    ui.close();
                }
                if menu_row(ui, Some(Icon::Search), t("Lọc trong Thư viện", "Filter in Your Library"), Some("Ctrl+Alt+F"), false, true).clicked() {
                    app.data.settings.left_collapsed = false;
                    app.lib_search = true;
                    app.focus_lib_search = true;
                    ui.close();
                }
                ui.separator();
                if menu_row(ui, Some(Icon::Gear), t("Tùy chọn", "Preferences"), Some("Ctrl+,"), false, true).clicked() {
                    out.push(Action::Navigate(Route::Settings));
                    ui.close();
                }
            });

            ui.menu_button(t("Xem", "View"), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_min_width(208.0);
                let z = ui.ctx().zoom_factor();
                if menu_row(ui, None, t("Phóng to", "Zoom In"), Some("Ctrl++"), false, z < 1.5).clicked() {
                    ui.ctx().set_zoom_factor((z + 0.1).min(1.5));
                    ui.close();
                }
                if menu_row(ui, None, t("Thu nhỏ", "Zoom Out"), Some("Ctrl+-"), false, z > 0.75).clicked() {
                    ui.ctx().set_zoom_factor((z - 0.1).max(0.75));
                    ui.close();
                }
                if menu_row(ui, None, t("Đặt lại thu phóng", "Reset Zoom"), Some("Ctrl+0"), false, (z - 1.0).abs() > 0.01).clicked() {
                    ui.ctx().set_zoom_factor(1.0);
                    ui.close();
                }
                ui.separator();
                if menu_row(ui, Some(Icon::Library), t("Thanh Thư viện", "Your Library"), Some("Alt+Shift+L"), !app.data.settings.left_collapsed, true).clicked() {
                    out.push(Action::ToggleLeft);
                    ui.close();
                }
                if menu_row(ui, Some(Icon::NowPlaying), t("Bảng Đang phát", "Now Playing Panel"), Some("Alt+Shift+R"), app.data.settings.show_right && app.right_tab == RightTab::NowPlaying, true).clicked() {
                    out.push(Action::ToggleRight(RightTab::NowPlaying));
                    ui.close();
                }
                if menu_row(ui, Some(Icon::Queue), t("Danh sách chờ", "Queue"), Some("Alt+Shift+Q"), app.data.settings.show_right && app.right_tab.is_queue(), true).clicked() {
                    out.push(Action::ToggleRight(RightTab::Queue));
                    ui.close();
                }
                if menu_row(ui, Some(Icon::FriendActivity), t("Hoạt động bạn bè", "Friend Activity"), Some("Alt+Shift+U"), app.friend_activity_open, true).clicked() {
                    out.push(Action::ToggleFriendActivity);
                    ui.close();
                }
                let is_fs = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                if menu_row(ui, Some(Icon::Fullscreen), t("Toàn màn hình", "Fullscreen"), Some("F11"), is_fs, true).clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fs));
                    ui.close();
                }
            });

            ui.menu_button(t("Phát nhạc", "Playback"), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_min_width(200.0);
                let play_lbl = if app.is_playing() { t("Tạm dừng", "Pause") } else { t("Phát", "Play") };
                let play_ic = if app.is_playing() { Icon::Pause } else { Icon::Play };
                if menu_row(ui, Some(play_ic), play_lbl, Some("Space"), false, true).clicked() {
                    out.push(Action::TogglePlay);
                    ui.close();
                }
                if menu_row(ui, Some(Icon::Next), t("Tiếp theo", "Next"), Some("Ctrl+→"), false, true).clicked() {
                    out.push(Action::Next);
                    ui.close();
                }
                if menu_row(ui, Some(Icon::Previous), t("Trước", "Previous"), Some("Ctrl+←"), false, true).clicked() {
                    out.push(Action::Previous);
                    ui.close();
                }
                if menu_row(ui, None, t("Tua tới", "Seek Forward"), Some("Shift+→"), false, true).clicked() {
                    out.push(Action::SeekBy(5000));
                    ui.close();
                }
                if menu_row(ui, None, t("Tua lùi", "Seek Backward"), Some("Shift+←"), false, true).clicked() {
                    out.push(Action::SeekBy(-5000));
                    ui.close();
                }
                ui.separator();
                if menu_row(ui, Some(Icon::Shuffle), t("Trộn bài", "Shuffle"), Some("Ctrl+S"), app.player.shuffle, true).clicked() {
                    out.push(Action::ToggleShuffle);
                    ui.close();
                }
                let repeat_ic = if app.player.repeat == Repeat::Track { Icon::RepeatOne } else { Icon::Repeat };
                if menu_row(ui, Some(repeat_ic), t("Lặp lại", "Repeat"), Some("Ctrl+R"), app.player.repeat != Repeat::Off, true).clicked() {
                    out.push(Action::CycleRepeat);
                    ui.close();
                }
                ui.separator();
                if menu_row(ui, Some(Icon::Volume), t("Tăng âm lượng", "Volume Up"), Some("Ctrl+↑"), false, true).clicked() {
                    out.push(Action::SetVolume((app.data.settings.volume + 0.1).min(1.0)));
                    ui.close();
                }
                if menu_row(ui, Some(Icon::VolumeLow), t("Giảm âm lượng", "Volume Down"), Some("Ctrl+↓"), false, true).clicked() {
                    out.push(Action::SetVolume((app.data.settings.volume - 0.1).max(0.0)));
                    ui.close();
                }
                if menu_row(ui, Some(Icon::Mute), t("Tắt tiếng", "Mute"), Some("M"), app.data.settings.muted, true).clicked() {
                    out.push(Action::ToggleMute);
                    ui.close();
                }
            });

            ui.menu_button(t("Trợ giúp", "Help"), |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_min_width(188.0);
                if menu_row(ui, Some(Icon::Info), t("Phím tắt", "Keyboard Shortcuts"), Some("Ctrl+/"), false, true).clicked() {
                    open_help = true;
                    ui.close();
                }
                if menu_row(ui, None, t("Cài đặt", "Settings"), Some("Ctrl+,"), false, true).clicked() {
                    out.push(Action::Navigate(Route::Settings));
                    ui.close();
                }
                ui.separator();
                if menu_row(ui, Some(Icon::VerifiedBadge), t("Giới thiệu Nhịp", "About Nhịp"), None, false, true).clicked() {
                    open_about_dialog = true;
                    ui.close();
                }
            });
        });

    app.help_open |= open_help;
    if open_about_dialog {
        overlays::open_about(ui.ctx());
    }
    app.actions.extend(out);
}

fn window_buttons(ui: &mut Ui, top: Rect) -> f32 {
    // 135x32 Windows caption spacer parity (3 x 45px).
    const W: f32 = 45.0;
    const H: f32 = 32.0;
    let y = top.top();
    let maximized = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
    for (i, kind) in ["close", "max", "min"].iter().enumerate() {
        let x = top.right() - W * (i as f32 + 1.0);
        let rect = Rect::from_min_size(Pos2::new(x, y), Vec2::new(W, H));
        let resp = ui.interact(rect, ui.scope_id().with(("win", *kind)), Sense::click());
        if resp.hovered() {
            let bg = if *kind == "close" {
                Color32::from_rgb(0xe8, 0x11, 0x23)
            } else {
                HOVER
            };
            ui.painter().rect_filled(rect, 0.0, bg);
        }
        let c = rect.center();
        let stroke = Stroke::new(1.0, if resp.hovered() { TEXT } else { SUBTLE });
        let p = ui.painter();
        match *kind {
            "close" => {
                p.line_segment([c + Vec2::new(-5.0, -5.0), c + Vec2::new(5.0, 5.0)], stroke);
                p.line_segment([c + Vec2::new(5.0, -5.0), c + Vec2::new(-5.0, 5.0)], stroke);
                if resp.clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            "max" => {
                if maximized {
                    // Double-square Windows Restore icon.
                    let back = Rect::from_center_size(c + Vec2::new(1.5, -1.5), Vec2::splat(8.0));
                    p.line_segment([Pos2::new(back.left(), back.top() + 2.0), back.left_top()], stroke);
                    p.line_segment([back.left_top(), back.right_top()], stroke);
                    p.line_segment([back.right_top(), back.right_bottom()], stroke);
                    p.line_segment([back.right_bottom(), Pos2::new(back.right() - 2.0, back.bottom())], stroke);
                    let front = Rect::from_center_size(c + Vec2::new(-1.5, 1.5), Vec2::splat(8.0));
                    p.rect_filled(front, 0.0, if resp.hovered() { HOVER } else { BG });
                    p.rect_stroke(front, 0.0, stroke, egui::StrokeKind::Inside);
                } else {
                    let box_rect = Rect::from_center_size(c, Vec2::splat(10.0));
                    p.rect_stroke(box_rect, 0.0, stroke, egui::StrokeKind::Inside);
                }
                if resp.clicked() {
                    toggle_maximize(ui.ctx());
                }
            }
            _ => {
                p.hline(c.x - 5.0..=c.x + 5.0, c.y, stroke);
                if resp.clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
        }
    }
    W * 3.0
}

fn draw_window_surface(app: &mut App, ui: &mut Ui, full: Rect, surface: SurfaceStyle) {
    if surface == SurfaceStyle::Standard {
        ui.painter().rect_filled(full, 0.0, BG);
        return;
    }

    // Transparent root framebuffer + native Windows 11 DWM Mica / Mica Alt / Acrylic (`app.rs::apply_window_surface`):
    // keep the veil cleanly parameterized so users can dial from ultra-clear glass to deep smoked tint.
    let op = app.data.settings.surface_opacity.clamp(0.0, 0.95);
    let tint_strength = app.data.settings.surface_tint_strength.clamp(0.0, 1.0);
    let veil_scale = match surface {
        SurfaceStyle::Acrylic => 0.56,
        SurfaceStyle::Mica => 0.34,
        SurfaceStyle::MicaAlt => 0.40,
        SurfaceStyle::Standard => 1.0,
    };
    let tint_a = (op * veil_scale * 255.0).round().clamp(0.0, 215.0) as u8;
    if tint_a > 0 {
        ui.painter()
            .rect_filled(full, 0.0, Color32::from_rgba_unmultiplied(10, 12, 16, tint_a));
    }

    // Optional album-art ambient glow blended smoothly over Mica/Acrylic when `surface_tint_strength > 0`.
    if tint_strength > 0.01 {
        if let Some(t) = app.current_track() {
            let key = t.cover_key();
            let tex = app.art.backdrop_texture(ui.ctx(), &key, t.art_source());
            let uv = Rect::from_min_max(Pos2::new(0.08, 0.08), Pos2::new(0.92, 0.92));
            let art_a = (tint_strength * 68.0).round().clamp(1.0, 72.0) as u8;
            ui.painter()
                .image(tex.id(), full, uv, Color32::from_rgba_unmultiplied(220, 232, 245, art_a));
        }
    }
}

fn panel(ui: &mut Ui, rect: Rect, id: &str, surface: SurfaceStyle, panel_opacity: f32, add: impl FnOnce(&mut Ui)) {
    let op = panel_opacity.clamp(0.10, 0.95);
    let (fill, rim) = match surface {
        SurfaceStyle::Standard => (PANEL, None),
        SurfaceStyle::Acrylic => {
            let a = (op * 0.68 * 255.0).round().clamp(18.0, 215.0) as u8;
            (
                Color32::from_rgba_unmultiplied(18, 20, 24, a),
                Some(Color32::from_white_alpha(28)),
            )
        }
        SurfaceStyle::Mica | SurfaceStyle::MicaAlt => {
            let a = (op * 0.62 * 255.0).round().clamp(16.0, 210.0) as u8;
            (
                Color32::from_rgba_unmultiplied(16, 18, 22, a),
                Some(Color32::from_white_alpha(22)),
            )
        }
    };
    ui.painter().rect_filled(rect, RADIUS, fill);
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(rect).layout(Layout::top_down(Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add(&mut child);
    if surface == SurfaceStyle::Standard {
        // egui clips to rectangles; mask artwork/lyrics overflow at the shell's rounded corners.
        let mut mask = egui::epaint::Mesh::default();
        let r = RADIUS as f32;
        for (corner, inward) in [
            (rect.left_top(), Vec2::new(1.0, 1.0)),
            (rect.right_top(), Vec2::new(-1.0, 1.0)),
            (rect.left_bottom(), Vec2::new(1.0, -1.0)),
            (rect.right_bottom(), Vec2::new(-1.0, -1.0)),
        ] {
            let base = mask.vertices.len() as u32;
            mask.colored_vertex(corner, BG);
            for i in 0..=12 {
                let angle = std::f32::consts::FRAC_PI_2 * i as f32 / 12.0;
                mask.colored_vertex(corner + inward * Vec2::new(r * (1.0 - angle.cos()), r * (1.0 - angle.sin())), BG);
                if i > 0 {
                    mask.add_triangle(base, base + i, base + i + 1);
                }
            }
        }
        ui.painter().add(egui::Shape::mesh(mask));
    }
    if let Some(stroke_col) = rim {
        ui.painter().rect_stroke(rect, RADIUS, Stroke::new(1.0, stroke_col), egui::StrokeKind::Inside);
    }
}

fn resize_left(app: &mut App, ui: &mut Ui, gap: Rect, body_left: f32, window_w: f32) {
    let resp = ui
        .interact(gap.expand2(Vec2::new(2.0, 0.0)), ui.scope_id().with("resize-left"), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal);
    if resp.hovered() || resp.dragged() {
        let color = if resp.dragged() { TEXT.linear_multiply(0.6) } else { TEXT.linear_multiply(0.25) };
        ui.painter().vline(gap.center().x, gap.top() + 8.0..=gap.bottom() - 8.0, Stroke::new(2.0, color));
    }
    let max_w = app.max_left_width(window_w);
    if resp.double_clicked() {
        let s = &mut app.data.settings;
        if s.left_collapsed {
            s.left_collapsed = false;
            s.left_width = LEFT_DEFAULT.min(max_w);
        } else if s.left_width >= LEFT_MAX {
            s.left_width = LEFT_DEFAULT.min(max_w);
        } else if window_w >= 1024.0 {
            s.left_width = LEFT_EXPANDED_MIN.min(max_w);
        } else {
            s.left_collapsed = true;
        }
        let w = app.data.settings.left_width;
        app.actions.push(Action::SetLeftWidth(w));
        return;
    }
    if resp.dragged() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let target = pos.x - body_left;
            let snap_mid = (LEFT_COLLAPSED + LEFT_MIN) * 0.5;
            let s = &mut app.data.settings;
            if target < snap_mid {
                s.left_collapsed = true;
            } else {
                s.left_collapsed = false;
                if window_w >= 1024.0 && max_w >= LEFT_EXPANDED_MIN {
                    if target <= LEFT_MAX {
                        s.left_width = target.clamp(LEFT_MIN, LEFT_MAX);
                    } else if target < LEFT_MAX + 100.0 {
                        // Elastic resistance before snapping into Expanded mode.
                        s.left_width = (LEFT_MAX + (target - LEFT_MAX) * 0.2).min(max_w);
                    } else {
                        s.left_width = target.clamp(LEFT_EXPANDED_MIN.min(max_w), max_w);
                    }
                } else {
                    s.left_width = target.clamp(LEFT_MIN, max_w);
                }
            }
        }
    }
    if resp.drag_stopped() {
        let w = app.data.settings.left_width;
        app.actions.push(Action::SetLeftWidth(w));
    }
}

fn resize_right(app: &mut App, ui: &mut Ui, gap: Rect, available: f32) {
    let resp = ui
        .interact(gap.expand2(Vec2::new(2.0, 0.0)), ui.scope_id().with("resize-right"), Sense::click_and_drag())
        .on_hover_cursor(CursorIcon::ResizeHorizontal);
    if resp.hovered() || resp.dragged() {
        let color = if resp.dragged() { TEXT.linear_multiply(0.6) } else { TEXT.linear_multiply(0.25) };
        ui.painter().vline(gap.center().x, gap.top() + 8.0..=gap.bottom() - 8.0, Stroke::new(2.0, color));
    }
    if resp.double_clicked() {
        app.data.settings.right_width = RIGHT_WIDTH;
        let w = app.data.settings.left_width;
        app.actions.push(Action::SetLeftWidth(w));
        return;
    }
    if resp.dragged() {
        let max_r = RIGHT_MAX.min((available - 360.0).max(RIGHT_MIN));
        let w = app.data.settings.right_width - resp.drag_delta().x;
        app.data.settings.right_width = w.clamp(RIGHT_MIN, max_r);
    }
    if resp.drag_stopped() {
        let w = app.data.settings.left_width;
        app.actions.push(Action::SetLeftWidth(w));
    }
}

fn top_bar(app: &mut App, ui: &mut Ui, top: Rect) {
    let cy = top.center().y;
    let t_back = app.t("Quay lại", "Go back");
    let t_forward = app.t("Đi tiếp", "Go forward");
    let t_home = app.t("Trang chủ", "Home");
    let t_placeholder = app.t("Bạn muốn phát nội dung gì?", "What do you want to play?");
    let t_browse = app.t("Duyệt tìm", "Browse");
    let t_whats_new = app.t("Có gì mới", "What's New");
    let t_friend = app.t("Hoạt động bạn bè", "Friend Activity");
    let t_account = app.t("Tài khoản", "Account");

    // Left cluster: `...` menu button (20px padding) + 24px gap + transparent Back/Forward (24px icons, 8px gap).
    let menu = Rect::from_center_size(Pos2::new(top.left() + 34.0, cy), Vec2::splat(28.0));
    let menu_tip = app.t("Trình đơn", "Menu");
    let menu_resp = icon_button_sized(ui, menu, "main-menu", Icon::Dots, SUBTLE, menu_tip, 18.0);
    main_menu(app, ui, &menu_resp);

    let mut x = menu.right() + 24.0;
    for (id, icon, enabled, action, tip) in [
        ("back", Icon::Back, !app.back.is_empty(), Action::Back, t_back),
        ("fwd", Icon::Forward, !app.forward.is_empty(), Action::Forward, t_forward),
    ] {
        let rect = Rect::from_center_size(Pos2::new(x + 12.0, cy), Vec2::new(24.0, 48.0));
        let mut resp = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(tip);
        if !enabled {
            resp = resp.on_hover_cursor(CursorIcon::NotAllowed);
        }
        let color = if !enabled {
            TEXT.linear_multiply(OPACITY_DISABLED)
        } else if resp.hovered() {
            TEXT
        } else {
            SUBTLE
        };
        icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(24.0)), icon, color);
        if resp.clicked() && enabled {
            app.actions.push(action);
        }
        x += 28.0;
    }
    let left_end = x;

    // Right cluster: Windows caption buttons (135x32), 16px gap, 48x48 Avatar pill (32x32 inner),
    // Friend Activity (>=720px, visible at 800x600 like Spotify Desktop 1.3.9), What's New Bell (32x32 + unread dot).
    let controls = window_buttons(ui, top);
    let mut rx = top.right() - controls - 16.0;

    let avatar_outer = Rect::from_center_size(Pos2::new(rx - 24.0, cy), Vec2::splat(48.0));
    let av_resp = ui.interact(avatar_outer, ui.scope_id().with("top-avatar"), Sense::click()).on_hover_text(t_account);
    let av_bg = if av_resp.hovered() || app.top_popover == Some(TopPopover::Account) { HOVER } else { ELEVATED };
    ui.painter().circle_filled(avatar_outer.center(), 24.0, av_bg);
    let inner_color = if app.data.settings.private_session {
        Color32::from_rgb(0x50, 0x9b, 0xf5)
    } else if app.data.settings.accent_theme == crate::library::AccentTheme::SpotifyGreen {
        ACCENT_ALT
    } else {
        accent_color(app.data.settings.accent_theme)
    };
    let avatar_initial = app
        .data
        .settings
        .profile_name
        .trim()
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "N".to_string());
    ui.painter().circle_filled(avatar_outer.center(), 16.0, inner_color);
    text(ui.painter(), avatar_outer.center(), &avatar_initial, bold(14.0), Color32::BLACK, 24.0, Align2::CENTER_CENTER);
    if app.data.settings.private_session {
        ui.painter().circle_filled(avatar_outer.center() + Vec2::new(11.0, 11.0), 4.0, Color32::from_rgb(0x1f, 0x73, 0xe8));
    }
    if av_resp.clicked() {
        app.actions.push(Action::ToggleAccountMenu);
    }
    rx = avatar_outer.left() - 8.0;

    if top.width() >= 720.0 {
        let friend_r = Rect::from_center_size(Pos2::new(rx - 16.0, cy), Vec2::splat(32.0));
        let active = app.friend_activity_open && app.data.settings.show_right;
        let ic = if active { Icon::FriendActivityFilled } else { Icon::FriendActivity };
        let fg = if active { TEXT } else { SUBTLE };
        if icon_button_sized(ui, friend_r, "top-friend-activity", ic, fg, t_friend, 16.0).clicked() {
            app.actions.push(Action::ToggleFriendActivity);
        }
        rx = friend_r.left() - 8.0;
    }

    let bell_r = Rect::from_center_size(Pos2::new(rx - 16.0, cy), Vec2::splat(32.0));
    let bell_active = app.top_popover == Some(TopPopover::Notifications);
    let bell_ic = if bell_active { Icon::BellFilled } else { Icon::Bell };
    let bell_fg = if bell_active { TEXT } else { SUBTLE };
    if icon_button_sized(ui, bell_r, "top-notifications", bell_ic, bell_fg, t_whats_new, 16.0).clicked() {
        app.actions.push(Action::ToggleNotifications);
    }
    if app.unread_notifications {
        ui.painter().circle_filled(bell_r.center() + Vec2::new(7.0, -7.0), 3.5, Color32::from_rgb(0x1f, 0x73, 0xe8));
    }
    let right_start = bell_r.left();

    // Spotify's 350..546px searchContainer includes Home and its 8px gap.
    let cluster_min = left_end + 12.0;
    let cluster_max = right_start - 12.0;
    let avail = cluster_max - cluster_min;
    if avail < 110.0 {
        return;
    }

    let target_search_w = (top.width() * 0.5).clamp(350.0, 546.0) - 56.0;
    let max_search_for_avail = (avail - 56.0).max(48.0);
    let mut search_w = target_search_w.min(max_search_for_avail);
    let collapsed_search = search_w <= 400.0 && !app.search_has_focus && app.search.is_empty() && max_search_for_avail < 350.0;
    if collapsed_search {
        search_w = 48.0;
    }

    let total_center_w = 48.0 + 8.0 + search_w;
    let ideal_cluster_left = top.center().x - total_center_w * 0.5;
    let cluster_left = ideal_cluster_left.clamp(cluster_min, (cluster_max - total_center_w).max(cluster_min));

    let home_rect = Rect::from_min_size(Pos2::new(cluster_left, cy - 24.0), Vec2::splat(48.0));
    let on_home = app.route == Route::Home;
    let home_resp = ui.interact(home_rect, ui.scope_id().with("home"), Sense::click()).on_hover_text(t_home);
    home_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, t_home));
    focus_ring(ui, &home_resp, 24.0);
    let home_scaled = hover_scale_rect(home_rect, home_resp.hovered(), home_resp.is_pointer_button_down_on());
    let home_bg = if home_resp.hovered() { HOVER } else { ELEVATED };
    ui.painter().circle_filled(home_scaled.center(), home_scaled.width() * 0.5, home_bg);
    let home_icon = if on_home { Icon::HomeFilled } else { Icon::Home };
    let home_fg = if on_home || home_resp.hovered() { TEXT } else { SUBTLE };
    icons::paint(ui.painter(), Rect::from_center_size(home_scaled.center(), Vec2::splat(24.0)), home_icon, home_fg);
    if home_resp.clicked() {
        if on_home {
            app.actions.push(Action::ClosePopovers);
            app.actions.push(Action::ScrollHomeToTop);
        } else {
            app.actions.push(Action::Navigate(Route::Home));
        }
    }

    let search_rect = Rect::from_min_size(Pos2::new(home_rect.right() + 8.0, cy - 24.0), Vec2::new(search_w, 48.0));
    let search_hover = ui.rect_contains_pointer(search_rect);

    if collapsed_search {
        let s_resp = ui.interact(search_rect, ui.scope_id().with("search-collapsed"), Sense::click()).on_hover_text(t_placeholder);
        let scaled = hover_scale_rect(search_rect, s_resp.hovered(), s_resp.is_pointer_button_down_on());
        ui.painter().circle_filled(scaled.center(), scaled.width() * 0.5, if s_resp.hovered() { HOVER } else { ELEVATED });
        icons::paint(
            ui.painter(),
            Rect::from_center_size(scaled.center(), Vec2::splat(24.0)),
            Icon::Search,
            if s_resp.hovered() { TEXT } else { SUBTLE },
        );
        if s_resp.clicked() {
            app.focus_search = true;
            app.search_dropdown_open = true;
        }
        return;
    }

    let pill_bg = if search_hover || app.search_has_focus { HOVER } else { ELEVATED };
    ui.painter().rect_filled(search_rect, 24.0, pill_bg);
    let lead_color = if search_hover || app.search_has_focus { TEXT } else { SUBTLE };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(Pos2::new(search_rect.left() + 24.0, cy), Vec2::splat(24.0)),
        Icon::Search,
        lead_color,
    );

    // Trailing Browse divider (1px solid #7c7c7c, 24px tall) & 24x24 Browse button inside Search pill.
    let browse_rect = Rect::from_center_size(Pos2::new(search_rect.right() - 24.0, cy), Vec2::splat(32.0));
    let div_x = search_rect.right() - 48.0;
    ui.painter().vline(div_x, (cy - 12.0)..=(cy + 12.0), Stroke::new(1.0, MUTED));
    let on_browse = app.route == Route::Search && app.search.is_empty();
    let browse_icon = if on_browse { Icon::BrowseFilled } else { Icon::Browse };
    let browse_color = if on_browse { TEXT } else { SUBTLE };
    if icon_button_sized(ui, browse_rect, "browse-btn", browse_icon, browse_color, t_browse, 24.0).clicked() {
        app.clear_search();
        app.actions.push(Action::ClosePopovers);
        app.actions.push(Action::Navigate(Route::Search));
    }

    let clear_reserve = if !app.search.is_empty() {
        34.0
    } else if search_hover && !app.search_has_focus && search_w >= 320.0 {
        74.0
    } else {
        8.0
    };
    let edit_rect = Rect::from_min_max(
        Pos2::new(search_rect.left() + 48.0, search_rect.top()),
        Pos2::new(div_x - clear_reserve, search_rect.bottom()),
    );
    let mut edit_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("search-edit")
            .max_rect(edit_rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    let edit = TextEdit::singleline(&mut app.search)
        .frame(egui::Frame::NONE)
        .align(Align2::LEFT_CENTER)
        .font(body_medium())
        .hint_text(RichText::new(t_placeholder).color(SUBTLE).font(body_medium()))
        .desired_width(edit_rect.width().max(40.0));
    let resp = edit_ui.add_sized(Vec2::new(edit_rect.width().max(40.0), 48.0), edit);

    if app.focus_search {
        app.focus_search = false;
        resp.request_focus();
    }
    app.search_has_focus = resp.has_focus();

    if resp.has_focus() {
        ui.painter().rect_stroke(search_rect, 24.0, Stroke::new(2.0, TEXT), egui::StrokeKind::Inside);
    } else if search_hover {
        ui.painter().rect_stroke(search_rect, 24.0, Stroke::new(1.0, Color32::from_white_alpha(26)), egui::StrokeKind::Inside);
    }

    if resp.gained_focus() {
        app.actions.push(Action::SetSearchDropdown(true));
    }
    if resp.changed() {
        app.actions.push(Action::SearchInputChanged);
        if app.route == Route::Search {
            app.ytm.search_changed();
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(crate::app::SEARCH_DEBOUNCE_MS));
    }
    let enter_submit = (resp.has_focus() || resp.lost_focus()) && ui.input(|i| i.key_pressed(Key::Enter));
    if enter_submit {
        app.actions.push(Action::SubmitSearch);
    }

    if !app.search.is_empty() {
        let x_rect = Rect::from_center_size(Pos2::new(div_x - 18.0, cy), Vec2::splat(26.0));
        let clear = app.t("Xóa tìm kiếm", "Clear search");
        if icon_button_sized(ui, x_rect, "clear-search", Icon::Close, SUBTLE, clear, 18.0).clicked() {
            app.clear_search();
            app.focus_search = true;
            app.actions.push(Action::SetSearchDropdown(true));
        }
    } else if search_hover && !resp.has_focus() && search_w >= 320.0 {
        paint_search_kbd_hint(ui, div_x - 10.0, cy);
    }

    if app.search_dropdown_open {
        draw_search_dropdown(app, ui, search_rect);
    }
}

fn paint_search_kbd_hint(ui: &mut Ui, right_x: f32, cy: f32) {
    let mut x = right_x;
    for label in ["K", "Ctrl"] {
        let font = semi(11.0);
        let kw = (text_width(ui, label, font.clone()) + 10.0).max(20.0);
        let kr = Rect::from_min_size(Pos2::new(x - kw, cy - 10.0), Vec2::new(kw, 20.0));
        ui.painter().rect_filled(kr, RADIUS_BASE, Color32::from_white_alpha(20));
        ui.painter().rect_stroke(kr, RADIUS_BASE, Stroke::new(1.0, Color32::from_white_alpha(45)), egui::StrokeKind::Inside);
        text(ui.painter(), kr.center(), label, font, SUBTLE, kw, Align2::CENTER_CENTER);
        x -= kw + 4.0;
    }
}

fn draw_search_dropdown(app: &mut App, ui: &mut Ui, search_rect: Rect) {
    let full = ui.ctx().content_rect();
    let backdrop_layer = LayerId::new(Order::Foreground, Id::unique("search-dd-backdrop"));
    let mut backdrop = ui.new_child(UiBuilder::new().layer_id(backdrop_layer).max_rect(full).id_salt("search-dd-backdrop"));
    backdrop.set_clip_rect(full);
    let bg_resp = backdrop.interact(full, Id::unique("search-dd-backdrop-click"), Sense::click());

    let dd_w = search_rect.width().max(350.0);
    let dd_pos = Pos2::new(search_rect.left(), search_rect.bottom() + 6.0);
    let area_resp = egui::Area::new(Id::unique("search-instant-dropdown"))
        .order(Order::Foreground)
        .fixed_pos(dd_pos)
        .show(ui.ctx(), |ui| {
            popup_frame(ui.style()).show(ui, |ui| {
                ui.set_width(dd_w - 10.0);
                ui.set_max_height(360.0);
                let q = app.search.trim().to_string();
                if !q.is_empty() {
                    draw_search_dropdown_hints(app, ui);
                }
                ScrollArea::vertical().id_salt("search-dd-scroll").auto_shrink([false, true]).show(ui, |ui| {
                    if q.is_empty() {
                        draw_search_dropdown_empty(app, ui);
                    } else {
                        draw_search_dropdown_results(app, ui, &q);
                    }
                });
            });
        })
        .response;

    if bg_resp.clicked()
        && !area_resp.hovered()
        && !egui::Popup::is_any_open(ui.ctx())
        && !search_rect.contains(bg_resp.interact_pointer_pos().unwrap_or(Pos2::ZERO))
    {
        app.actions.push(Action::SetSearchDropdown(false));
    }
}

fn draw_search_dropdown_hints(app: &App, ui: &mut Ui) {
    let w = ui.available_width();
    let (bar, _) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::hover());
    let cy = bar.center().y - 2.0;
    let mut x = bar.left() + 2.0;
    for key in ["↑", "↓"] {
        x = paint_dropdown_keycap(ui, Pos2::new(x, cy), key) + 4.0;
    }
    x += 2.0;
    let move_lbl = app.t("Di chuyển", "Navigate");
    text(ui.painter(), Pos2::new(x, cy), move_lbl, marginal(), SUBTLE, 100.0, Align2::LEFT_CENTER);

    let search_lbl = app.t("Tìm kiếm", "Search");
    let search_w = text_width(ui, search_lbl, marginal());
    text(
        ui.painter(),
        Pos2::new(bar.right() - 2.0, cy),
        search_lbl,
        marginal(),
        SUBTLE,
        search_w + 4.0,
        Align2::RIGHT_CENTER,
    );
    let enter_lbl = "Enter";
    let enter_w = (text_width(ui, enter_lbl, semi(10.5)) + 10.0).max(20.0);
    paint_dropdown_keycap(ui, Pos2::new(bar.right() - 2.0 - search_w - 6.0 - enter_w, cy), enter_lbl);

    ui.painter().hline(bar.x_range(), bar.bottom(), Stroke::new(1.0, HAIRLINE));
    ui.add_space(6.0);
}

fn paint_dropdown_keycap(ui: &Ui, left_cy: Pos2, label: &str) -> f32 {
    let font = semi(10.5);
    let kw = (text_width(ui, label, font.clone()) + 10.0).max(18.0);
    let kr = Rect::from_min_size(Pos2::new(left_cy.x, left_cy.y - 9.0), Vec2::new(kw, 18.0));
    ui.painter().rect_filled(kr, 4.0, Color32::from_white_alpha(18));
    ui.painter().rect_stroke(kr, 4.0, Stroke::new(1.0, Color32::from_white_alpha(38)), egui::StrokeKind::Inside);
    text(ui.painter(), kr.center(), label, font, SUBTLE, kw, Align2::CENTER_CENTER);
    kr.right()
}

fn draw_search_dropdown_empty(app: &mut App, ui: &mut Ui) {
    let has_queries = !app.data.recent_searches.is_empty();
    let recent_tracks: Vec<_> = app
        .data
        .listening_history
        .iter()
        .filter_map(|&id| app.lib.track(id))
        .take(6)
        .collect();
    if has_queries || !recent_tracks.is_empty() {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(app.t("Các tìm kiếm gần đây", "Recent searches"))
                    .font(bold(16.0))
                    .color(TEXT),
            );
            if has_queries {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(app.t("Xóa tất cả", "Clear all"))
                                    .font(marginal())
                                    .color(SUBTLE),
                            )
                            .frame(false),
                        )
                        .clicked()
                    {
                        app.actions.push(Action::ClearRecentSearches);
                    }
                });
            }
        });
        ui.add_space(6.0);

        // Rich recent song/artist rows matching Spotify Desktop (`48x48` covers + `Bài hát • Nghệ sĩ`).
        for track in recent_tracks {
            let rw = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(rw, 58.0), Sense::click());
            let hov = ui.rect_contains_pointer(rect);
            if hov {
                ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(18));
            }
            let art_r = Rect::from_min_size(rect.min + Vec2::new(4.0, 5.0), Vec2::splat(48.0));
            let tex = app.art.get(ui.ctx(), &track.cover_key(), track.art_source());
            cover(ui, art_r, &tex, RADIUS_BASE);
            if hov {
                ui.painter().rect_filled(art_r, RADIUS_BASE, Color32::from_black_alpha(135));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(art_r.center(), Vec2::splat(16.0)),
                    Icon::Play,
                    TEXT,
                );
            }
            let tx = art_r.right() + 12.0;
            let tw = (rect.right() - tx - 10.0).max(20.0);
            text(
                ui.painter(),
                Pos2::new(tx, rect.top() + 19.0),
                &track.title,
                semi(14.5),
                TEXT,
                tw,
                Align2::LEFT_CENTER,
            );
            let sub = format!("{} • {}", app.t("Bài hát", "Song"), track.artist);
            text(
                ui.painter(),
                Pos2::new(tx, rect.top() + 39.0),
                &sub,
                marginal(),
                SUBTLE,
                tw,
                Align2::LEFT_CENTER,
            );
            if resp.clicked() {
                app.actions.push(Action::SetSearchDropdown(false));
                app.actions.push(Action::PlayContext {
                    context: crate::queue::ContextRef::All,
                    name: track.title.clone(),
                    tracks: vec![track.id],
                    start: Some(0),
                });
            }
        }

        let recents: Vec<String> = app.data.recent_searches.iter().take(5).cloned().collect();
        for item in recents {
            let rw = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(rw, 46.0), Sense::click());
            if ui.rect_contains_pointer(rect) {
                ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(18));
            }
            let badge_c = Pos2::new(rect.left() + 24.0, rect.center().y);
            ui.painter().circle_filled(badge_c, 18.0, ELEVATED);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(badge_c, Vec2::splat(16.0)),
                Icon::Search,
                SUBTLE,
            );
            text(
                ui.painter(),
                Pos2::new(rect.left() + 52.0, rect.center().y),
                &item,
                semi(14.0),
                TEXT,
                (rw - 86.0).max(20.0),
                Align2::LEFT_CENTER,
            );
            let rm_r = Rect::from_center_size(Pos2::new(rect.right() - 18.0, rect.center().y), Vec2::splat(22.0));
            let rm = icon_button_sized(ui, rm_r, ("rm-recent", &item), Icon::Close, SUBTLE, app.t("Xóa", "Remove"), 14.0);
            if rm.clicked() {
                app.actions.push(Action::RemoveRecentSearch(item));
            } else if resp.clicked() {
                app.actions.push(Action::SelectRecentSearch(item));
            }
        }
        ui.add_space(10.0);
    }

    ui.label(RichText::new(app.t("Thể loại phổ biến", "Browse categories")).font(title_small()).color(TEXT));
    ui.add_space(6.0);
    let cats = ["Nhạc", "V-Pop", "Indie", "Pop", "Lo-fi", "Jazz", "Rock", "Chill"];
    ui.horizontal_wrapped(|ui| {
        for cat in cats {
            let w = pill_width(ui, cat);
            let (r, _) = ui.allocate_exact_size(Vec2::new(w, 30.0), Sense::hover());
            if pill(ui, r, ("cat", cat), cat, false).clicked() {
                app.actions.push(Action::SearchCategory(cat.to_string()));
            }
        }
    });
}

fn draw_search_dropdown_results(app: &mut App, ui: &mut Ui, q: &str) {
    let fq = fold(q);
    let mut any = false;

    if crate::ytm::from_url(q).is_some() {
        let open_lbl = app.t("Mở liên kết trực tiếp trong Nhịp", "Open link directly in Nhịp");
        if menu_row(ui, Some(Icon::Play), open_lbl, Some("Enter"), false, true).clicked() {
            app.actions.push(Action::SubmitSearch);
        }
        return;
    }

    let debouncing = !app.lib.demo && app.instant_query.trim() != q;
    let (ytm_poll_pending, ytm_items) = if !app.lib.demo && !debouncing {
        app.ytm.poll_instant_search(ui.ctx(), q)
    } else {
        (false, Vec::new())
    };
    let ytm_pending = debouncing || ytm_poll_pending;

    // Spotify-style top query autocomplete rows (`🔍` in circular badge + bold query suggestions).
    let mut query_suggestions: Vec<String> = vec![q.to_lowercase()];
    for item in ytm_items.iter().take(6) {
        if query_suggestions.len() >= 3 {
            break;
        }
        let cand = if item.by.is_empty() {
            item.title.to_lowercase()
        } else {
            let primary_by = item.by.split(',').next().unwrap_or(&item.by).trim();
            format!("{} {}", primary_by.to_lowercase(), item.title.to_lowercase())
        };
        if !query_suggestions.iter().any(|s| fold(s) == fold(&cand)) {
            query_suggestions.push(cand);
        }
    }
    for (si, sug) in query_suggestions.into_iter().enumerate() {
        let rw = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(rw, 48.0), Sense::click());
        if ui.rect_contains_pointer(rect) {
            ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(18));
        }
        let badge_c = Pos2::new(rect.left() + 24.0, rect.center().y);
        ui.painter().circle_filled(badge_c, 19.0, ELEVATED);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(badge_c, Vec2::splat(17.0)),
            Icon::Search,
            SUBTLE,
        );
        text(
            ui.painter(),
            Pos2::new(rect.left() + 56.0, rect.center().y),
            &sug,
            if si == 0 { bold(14.5) } else { semi(14.5) },
            TEXT,
            (rw - 68.0).max(20.0),
            Align2::LEFT_CENTER,
        );
        if resp.clicked() {
            app.actions.push(Action::SelectRecentSearch(sug));
        }
    }
    ui.add_space(4.0);

    // Rich 48x48 instant YouTube Music entity matches with `+`/`✓` Save-to-Liked action button.
    if !ytm_items.is_empty() {
        any = true;
        for item in ytm_items.iter().take(6) {
            let rw = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(rw, 58.0), Sense::click());
            let row_hovered = ui.rect_contains_pointer(rect);
            if row_hovered {
                ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(20));
            }
            let art_r = Rect::from_min_size(rect.min + Vec2::new(4.0, 5.0), Vec2::splat(48.0));
            let is_artist = matches!(item.id, crate::ytm::ItemId::Artist(_));
            let is_song = matches!(item.id, crate::ytm::ItemId::Song(_));
            let tex = app.art.get(ui.ctx(), &item.cover_key(), item.image.as_deref());
            cover(ui, art_r, &tex, if is_artist { 24.0 } else { RADIUS_BASE });
            if row_hovered && is_song {
                ui.painter().rect_filled(art_r, RADIUS_BASE, Color32::from_black_alpha(135));
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(art_r.center(), Vec2::splat(16.0)),
                    Icon::Play,
                    TEXT,
                );
            }
            let liked_song = match &item.id {
                crate::ytm::ItemId::Song(vid) => app.data.is_liked(crate::library::fnv(&format!("yt:{vid}"))),
                _ => false,
            };
            let right_reserve = if is_song { 42.0 } else { 12.0 };
            let tx = art_r.right() + 12.0;
            let tw = (rect.right() - tx - right_reserve).max(20.0);
            text(ui.painter(), Pos2::new(tx, rect.top() + 19.0), &item.title, semi(14.5), TEXT, tw, Align2::LEFT_CENTER);
            let kind_lbl = pages::ytm_kind_label(app, item, None);
            let sub = if item.by.is_empty() {
                kind_lbl.to_string()
            } else {
                format!("{kind_lbl} • {}", item.by)
            };
            text(ui.painter(), Pos2::new(tx, rect.top() + 39.0), &sub, marginal(), SUBTLE, tw, Align2::LEFT_CENTER);

            let mut sub_clicked = false;
            if is_song {
                let save_r = Rect::from_center_size(Pos2::new(rect.right() - 20.0, rect.center().y), Vec2::splat(28.0));
                let (s_ic, s_col, s_tip) = if liked_song {
                    (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
                } else {
                    (Icon::Save, SUBTLE, app.t("Lưu vào Bài hát đã thích", "Save to Liked Songs"))
                };
                if icon_button_sized(ui, save_r, ("dd-ytm-save", &item.title, &item.by), s_ic, s_col, s_tip, 17.0).clicked() {
                    sub_clicked = true;
                    app.actions.push(Action::LikeYouTubeSong(item.clone()));
                }
            }
            if resp.clicked() && !sub_clicked {
                app.actions.push(Action::SetSearchDropdown(false));
                if is_song {
                    app.actions.push(Action::SelectYouTubeSong(item.clone()));
                } else {
                    app.actions.push(Action::Navigate(Route::YouTubeMusicItem(item.clone())));
                }
            }
        }
        ui.add_space(6.0);
    }

    let matching_tracks: Vec<_> = app
        .lib
        .order
        .iter()
        .filter_map(|&id| app.lib.track(id))
        .filter(|t| app.lib.demo || !t.path.starts_with("yt:"))
        .filter(|t| fold(&t.title).contains(&fq) || fold(&t.artist).contains(&fq))
        .take(5)
        .collect();

    if !matching_tracks.is_empty() {
        any = true;
        ui.label(RichText::new(app.t("Bài hát", "Songs")).font(semi(13.0)).color(SUBTLE));
        ui.add_space(4.0);
        let match_ids: Vec<_> = matching_tracks.iter().map(|t| t.id).collect();
        for (idx, track) in matching_tracks.into_iter().enumerate() {
            let rw = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(rw, 52.0), Sense::click());
            let row_hovered = ui.rect_contains_pointer(rect);
            if row_hovered {
                ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(20));
            }
            let art_r = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::splat(40.0));
            let tex = app.art.get(ui.ctx(), &track.cover_key(), track.art_source());
            cover(ui, art_r, &tex, RADIUS_BASE);

            let is_current = app.player.current == Some(track.id);
            let play_resp = ui
                .interact(art_r, ui.scope_id().with(("dd-play", track.id)), Sense::click())
                .on_hover_cursor(CursorIcon::PointingHand);
            if row_hovered || play_resp.hovered() {
                ui.painter().rect_filled(art_r, RADIUS_BASE, Color32::from_black_alpha(135));
                let play_icon = if is_current && app.is_playing() { Icon::Pause } else { Icon::Play };
                icons::paint(
                    ui.painter(),
                    Rect::from_center_size(art_r.center(), Vec2::splat(16.0)),
                    play_icon,
                    TEXT,
                );
            }

            let liked = app.data.is_liked(track.id);
            let show_trailing = row_hovered || liked;
            let right_pad = if row_hovered { 74.0 } else if liked { 44.0 } else { 10.0 };
            let tx = art_r.right() + 10.0;
            let tw = (rect.right() - tx - right_pad).max(20.0);
            text(
                ui.painter(),
                Pos2::new(tx, rect.top() + 17.0),
                &track.title,
                semi(14.0),
                if is_current { ACCENT } else { TEXT },
                tw,
                Align2::LEFT_CENTER,
            );
            text(ui.painter(), Pos2::new(tx, rect.top() + 35.0), &track.artist, marginal(), SUBTLE, tw, Align2::LEFT_CENTER);

            let mut clicked_sub = false;
            if show_trailing {
                let like_x = if row_hovered { rect.right() - 50.0 } else { rect.right() - 22.0 };
                let like_r = Rect::from_center_size(Pos2::new(like_x, rect.center().y), Vec2::splat(26.0));
                let (like_icon, like_col, like_tip) = if liked {
                    (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
                } else {
                    (Icon::Save, SUBTLE, app.t("Lưu vào Bài hát đã thích", "Save to Liked Songs"))
                };
                let like_resp = icon_button_sized(ui, like_r, ("dd-like", track.id), like_icon, like_col, like_tip, 16.0);
                if like_resp.clicked() {
                    app.actions.push(Action::ToggleLike(track.id));
                    clicked_sub = true;
                }
            }
            if row_hovered {
                let dots_r = Rect::from_center_size(Pos2::new(rect.right() - 20.0, rect.center().y), Vec2::splat(26.0));
                let dots_resp = icon_button_sized(
                    ui,
                    dots_r,
                    ("dd-dots", track.id),
                    Icon::Dots,
                    SUBTLE,
                    app.t("Tùy chọn khác", "More options"),
                    16.0,
                );
                pages::track_menu(app, &dots_resp, track.id, None, idx, &match_ids);
                if dots_resp.clicked() {
                    clicked_sub = true;
                }
            }

            if play_resp.clicked() || (resp.clicked() && !clicked_sub) {
                app.actions.push(Action::SetSearchDropdown(false));
                if is_current && play_resp.clicked() {
                    app.actions.push(Action::TogglePlay);
                } else {
                    app.actions.push(Action::PlayContext {
                        context: crate::queue::ContextRef::Album(track.album_key()),
                        name: track.album.clone(),
                        tracks: vec![track.id],
                        start: Some(0),
                    });
                }
            }
        }
        ui.add_space(6.0);
    }

    let matching_items: Vec<_> = app
        .data
        .playlists
        .iter()
        .filter(|p| fold(&p.name).contains(&fq))
        .map(|p| RecentItem::Playlist(p.id))
        .chain(
            app.data
                .followed_ytm
                .iter()
                .filter(|f| fold(&f.snapshot.title).contains(&fq))
                .map(|f| RecentItem::FollowedYouTube(f.id.clone())),
        )
        .chain(
            app.lib
                .albums
                .iter()
                .filter(|a| {
                    let has_local = a.tracks.iter().any(|tid| {
                        app.lib
                            .track(*tid)
                            .is_some_and(|t| !t.path.starts_with("yt:") && !t.path.starts_with("demo:"))
                    });
                    (app.lib.demo || has_local || app.data.saved_albums.contains(&a.key))
                        && (fold(&a.title).contains(&fq) || fold(&a.artist).contains(&fq))
                })
                .map(|a| RecentItem::Album(a.key.clone())),
        )
        .chain(
            app.lib
                .artists
                .iter()
                .filter(|a| {
                    let has_local = a.tracks.iter().any(|tid| {
                        app.lib
                            .track(*tid)
                            .is_some_and(|t| !t.path.starts_with("yt:") && !t.path.starts_with("demo:"))
                    });
                    (app.lib.demo || has_local || app.data.followed_artists.contains(&a.name))
                        && fold(&a.name).contains(&fq)
                })
                .map(|a| RecentItem::Artist(a.name.clone())),
        )
        .filter_map(|k| item_of(app, &k))
        .take(4)
        .collect();

    if !matching_items.is_empty() {
        any = true;
        ui.label(RichText::new(app.t("Thư viện của bạn", "Your Library")).font(semi(13.0)).color(SUBTLE));
        ui.add_space(4.0);
        for item in matching_items {
            let rw = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(rw, 46.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(20));
            }
            let art_r = Rect::from_min_size(rect.min + Vec2::new(6.0, 5.0), Vec2::splat(36.0));
            paint_item_cover(app, ui, art_r, &item);
            let tx = art_r.right() + 10.0;
            let tw = (rect.right() - tx - 10.0).max(20.0);
            text(ui.painter(), Pos2::new(tx, rect.top() + 14.0), &item.title, semi(14.0), TEXT, tw, Align2::LEFT_CENTER);
            text(ui.painter(), Pos2::new(tx, rect.top() + 32.0), &item.sub, marginal(), SUBTLE, tw, Align2::LEFT_CENTER);
            if resp.clicked() {
                app.actions.push(Action::SetSearchDropdown(false));
                app.actions.push(Action::Navigate(item.route));
            }
        }
        ui.add_space(6.0);
    }

    let force_skel = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("force-search-skeleton")))
        .unwrap_or(false);
    if force_skel || (!any && ytm_pending) {
            // Spotify Search Instant Dropdown 6-row shimmer skeleton (Image 15 exact proportions).
            const TOP_W: [f32; 6] = [0.72, 0.64, 0.54, 0.51, 0.48, 0.61];
            const SUB_W: [f32; 6] = [0.38, 0.31, 0.34, 0.24, 0.31, 0.29];
            for row_i in 0..6 {
                let rw = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(Vec2::new(rw, 58.0), Sense::hover());
                let thumb = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::splat(46.0));
                skeleton_rect(ui, thumb, 6.0);
                let tx = thumb.right() + 14.0;
                let avail_w = (rect.right() - tx - 12.0).max(60.0);
                let line1 = Rect::from_min_size(Pos2::new(tx, rect.top() + 12.0), Vec2::new(avail_w * TOP_W[row_i], 15.0));
                let line2 = Rect::from_min_size(Pos2::new(tx, rect.top() + 33.0), Vec2::new(avail_w * SUB_W[row_i], 11.0));
                skeleton_rect(ui, line1, 7.5);
                skeleton_rect(ui, line2, 5.5);
            }
            return;
        }

        if !any {
            ui.label(
                RichText::new(app.t("Nhấn Enter để tìm trên YouTube Music", "Press Enter to search on YouTube Music"))
                    .font(body_small())
                    .color(SUBTLE),
            );
            ui.add_space(4.0);
        }

    let see_all = match app.lang() {
        crate::library::Lang::Vi => format!("Xem tất cả kết quả cho \"{q}\""),
        crate::library::Lang::En => format!("See all results for \"{q}\""),
    };
    if menu_row(ui, Some(Icon::Search), &see_all, Some("Enter"), false, true).clicked() {
        app.actions.push(Action::SubmitSearch);
    }
}
