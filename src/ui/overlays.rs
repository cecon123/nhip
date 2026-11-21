//! Overlays rendered above the main layout: modals, top popovers, miniplayer, and toasts.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, Id, Key, LayerId, Order, Pos2, Rect, RichText,
    ScrollArea, Sense, Stroke, TextEdit, Ui, UiBuilder, Vec2,
};

use super::icons::{self, Icon};
use super::widgets::*;
use crate::app::{Action, App, ConfirmAction, Route, TopPopover, PLAYLIST_DESCRIPTION_MAX_CHARS};
use crate::library::{Lang, RecentItem};
use crate::theme::*;

const ABOUT_OPEN_ID: &str = "nhip_about_modal_open";

pub fn open_about(ctx: &egui::Context) {
    ctx.data_mut(|d| d.insert_temp(Id::unique(ABOUT_OPEN_ID), true));
}

pub fn draw(app: &mut App, ui: &mut Ui, full: Rect) {
    if let Some(popover) = app.top_popover {
        draw_top_popover(app, ui, full, popover);
    }
    if app.miniplayer_open {
        draw_miniplayer(app, ui, full);
    }
    if app.preview_feed.is_some() {
        draw_preview_feed_modal(app, ui, full);
    }
    if app.help_open {
        shortcuts(app, ui, full);
    }
    if app.edit_playlist.is_some() {
        edit_playlist_modal(app, ui, full);
    }
    if app.rename.is_some() {
        rename_modal(app, ui, full, false);
    }
    if app.rename_folder.is_some() {
        rename_modal(app, ui, full, true);
    }
    if app.confirm.is_some() {
        confirm_modal(app, ui, full);
    }
    if ui.ctx().data(|d| d.get_temp::<bool>(Id::unique(ABOUT_OPEN_ID)).unwrap_or(false)) {
        about_modal(app, ui, full);
    }
    toast(app, ui, full);
}

/// Dims the window behind a dialog; returns a `Ui` on the foreground layer and whether the scrim was clicked.
fn scrim(ui: &mut Ui, full: Rect, id: &str) -> (Ui, bool) {
    let layer = LayerId::new(Order::Foreground, Id::unique(id));
    let mut top = ui.new_child(UiBuilder::new().layer_id(layer).max_rect(full).id_salt(id));
    top.set_clip_rect(full);
    top.painter().rect_filled(full, 0.0, Color32::from_black_alpha(180));
    let resp = top.interact(full, top.scope_id().with("scrim"), Sense::click());
    (top, resp.clicked())
}

fn paint_modal_card(ui: &mut Ui, panel: Rect, id: &str) {
    ui.interact(panel, ui.scope_id().with(id), Sense::click());
    let shadow = egui::epaint::Shadow {
        offset: [0, 16],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(180),
    };
    let radius = card_radius(RADIUS_MODAL as u8);
    ui.painter().add(shadow.as_shape(panel, radius));
    ui.painter().rect_filled(panel, radius, ELEVATED);
    ui.painter().rect_stroke(panel, radius, Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);
}

/// Shared compact desktop context/popover menu row (`30px` tall, icon + label + optional right shortcut/check).
pub fn menu_row(
    ui: &mut Ui,
    icon: Option<Icon>,
    label: &str,
    shortcut: Option<&str>,
    checked: bool,
    enabled: bool,
) -> egui::Response {
    let target_w = ui.min_rect().width().max(210.0);
    let w = ui.available_width().min(target_w).max(136.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 30.0), if enabled { Sense::click() } else { Sense::hover() });
    if enabled && resp.hovered() {
        ui.painter().rect_filled(rect, RADIUS_SMALLER, Color32::from_white_alpha(25));
    }
    let fg = if !enabled {
        TEXT.linear_multiply(OPACITY_DISABLED)
    } else if checked {
        ACCENT
    } else {
        TEXT
    };
    let icon_color = if !enabled {
        SUBTLE.linear_multiply(OPACITY_DISABLED)
    } else if checked {
        ACCENT
    } else {
        SUBTLE
    };
    let mut text_x = rect.left() + 10.0;
    if let Some(ic) = icon {
        let ir = Rect::from_center_size(Pos2::new(rect.left() + 17.0, rect.center().y), Vec2::splat(14.0));
        icons::paint(ui.painter(), ir, ic, icon_color);
        text_x = rect.left() + 32.0;
    }
    let mut right_reserve = 10.0;
    if checked {
        let cr = Rect::from_center_size(Pos2::new(rect.right() - 15.0, rect.center().y), Vec2::splat(14.0));
        icons::paint(ui.painter(), cr, Icon::Checkmark, ACCENT);
        right_reserve = 28.0;
    } else if let Some(hint) = shortcut {
        let hw = text_width(ui, hint, marginal_small());
        text(
            ui.painter(),
            Pos2::new(rect.right() - 10.0, rect.center().y),
            hint,
            marginal_small(),
            if enabled { SUBTLE } else { MUTED },
            hw + 4.0,
            Align2::RIGHT_CENTER,
        );
        right_reserve = hw + 18.0;
    }
    let max_label_w = (rect.right() - right_reserve - text_x).max(20.0);
    text(
        ui.painter(),
        Pos2::new(text_x, rect.center().y),
        label,
        body(13.0),
        fg,
        max_label_w,
        Align2::LEFT_CENTER,
    );
    resp
}

fn draw_top_popover(app: &mut App, ui: &mut Ui, full: Rect, popover: TopPopover) {
    let dismiss_layer = LayerId::new(Order::Foreground, Id::unique("popover-backdrop"));
    let mut backdrop = ui.new_child(UiBuilder::new().layer_id(dismiss_layer).max_rect(full).id_salt("popover-backdrop"));
    backdrop.set_clip_rect(full);
    let bg_resp = backdrop.interact(full, Id::unique("popover-backdrop-click"), Sense::click());

    let anchor_rect = match popover {
        TopPopover::Notifications => {
            let w = 304.0f32.min(full.width() - 32.0);
            let right = (full.right() - 190.0).clamp(w + 16.0, full.right() - 16.0);
            Rect::from_min_size(Pos2::new(right - w, full.top() + TOP_BAR + 4.0), Vec2::new(w, 260.0))
        }
        TopPopover::Account => {
            let w = 196.0f32.min(full.width() - 32.0);
            let right = (full.right() - 140.0).clamp(w + 16.0, full.right() - 16.0);
            Rect::from_min_size(Pos2::new(right - w, full.top() + TOP_BAR + 4.0), Vec2::new(w, 200.0))
        }
    };

    let pop_layer = LayerId::new(Order::Foreground, Id::unique(("top-popover", popover as u8)));
    let area_resp = egui::Area::new(Id::unique(("top-popover-area", popover as u8)))
        .order(pop_layer.order)
        .fixed_pos(anchor_rect.min)
        .show(ui.ctx(), |ui| {
            let frame = match popover {
                TopPopover::Account => menu_frame(ui.style()),
                TopPopover::Notifications => popup_frame(ui.style()),
            };
            frame.show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.set_width(anchor_rect.width());
                match popover {
                    TopPopover::Notifications => notifications_popover(app, ui),
                    TopPopover::Account => account_popover(app, ui),
                }
            });
        })
        .response;

    if bg_resp.clicked() && !area_resp.hovered() {
        app.actions.push(Action::ClosePopovers);
    }
}

fn notifications_popover(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(app.t("Có gì mới", "What's New")).font(title_small()).color(TEXT));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let close = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
            icons::paint(ui.painter(), Rect::from_center_size(close.0.center(), Vec2::splat(14.0)), Icon::Close, SUBTLE);
            if close.1.clicked() {
                app.actions.push(Action::ClosePopovers);
            }
        });
    });
    ui.label(
        RichText::new(app.t(
            "Bản phát hành mới từ nghệ sĩ và hoạt động gần đây của bạn.",
            "The latest releases from artists and your recent library updates.",
        ))
        .font(marginal())
        .color(SUBTLE),
    );
    ui.add_space(10.0);
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, HAIRLINE));
    ui.add_space(10.0);

    let recent_tracks: Vec<_> = app
        .data
        .listening_history
        .iter()
        .filter_map(|&id| app.lib.track(id))
        .take(4)
        .collect();
    if recent_tracks.is_empty() {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(app.t("Bạn đã xem hết thông báo", "You're all caught up")).font(semi(14.0)).color(TEXT));
            ui.add_space(4.0);
            ui.label(
                RichText::new(app.t("Chưa có cập nhật mới nào.", "Check back later for new releases."))
                    .font(body(12.5))
                    .color(SUBTLE),
            );
        });
        ui.add_space(16.0);
    } else {
        for track in recent_tracks {
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 48.0), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(rect, RADIUS_BASE, Color32::from_white_alpha(18));
            }
            let art_r = Rect::from_min_size(rect.min + Vec2::new(6.0, 6.0), Vec2::splat(36.0));
            let tex = app.art.get(ui.ctx(), &track.cover_key(), track.art_source());
            cover(ui, art_r, &tex, RADIUS_BASE);
            let tx = art_r.right() + 10.0;
            let tw = (rect.right() - tx - 8.0).max(20.0);
            text(ui.painter(), Pos2::new(tx, rect.top() + 14.0), &track.title, semi(13.5), TEXT, tw, Align2::LEFT_CENTER);
            text(ui.painter(), Pos2::new(tx, rect.top() + 33.0), &track.artist, marginal(), SUBTLE, tw, Align2::LEFT_CENTER);
            if resp.clicked() {
                app.actions.push(Action::ClosePopovers);
                app.actions.push(Action::Navigate(Route::Album(track.album_key())));
            }
        }
    }
}

fn account_popover(app: &mut App, ui: &mut Ui) {
    let user_label = app.t("Hồ sơ Nhịp", "Nhịp Profile");
    if menu_row(ui, None, user_label, None, false, true).clicked() {
        app.actions.push(Action::ClosePopovers);
        app.actions.push(Action::Navigate(Route::Profile));
    }
    let taste_label = app.t("Gu âm nhạc", "Music taste");
    if menu_row(ui, Some(Icon::HeartFilled), taste_label, None, false, true).clicked() {
        app.actions.push(Action::ClosePopovers);
        app.actions.push(Action::Navigate(Route::TasteProfile));
    }
    let priv_label = app.t("Phiên riêng tư", "Private session");
    if menu_row(ui, None, priv_label, None, app.data.settings.private_session, true).clicked() {
        app.actions.push(Action::TogglePrivateSession);
    }
    let settings_label = app.t("Cài đặt", "Settings");
    if menu_row(ui, Some(Icon::Gear), settings_label, Some("Ctrl+,"), false, true).clicked() {
        app.actions.push(Action::ClosePopovers);
        app.actions.push(Action::Navigate(Route::Settings));
    }
    let keys_label = app.t("Phím tắt", "Keyboard shortcuts");
    if menu_row(ui, Some(Icon::Info), keys_label, Some("Ctrl+/"), false, true).clicked() {
        app.actions.push(Action::ClosePopovers);
        app.help_open = true;
    }
    ui.add_space(4.0);
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, HAIRLINE));
    ui.add_space(4.0);
    let about_label = app.t("Giới thiệu Nhịp", "About Nhịp");
    if menu_row(ui, None, about_label, None, false, true).clicked() {
        app.actions.push(Action::ClosePopovers);
        open_about(ui.ctx());
    }
}

fn draw_miniplayer(app: &mut App, ui: &mut Ui, full: Rect) {
    super::lyrics::draw_spicy_popup(app, ui, full);
}

fn paint_keycap_combo(ui: &mut Ui, right_x: f32, cy: f32, keys: &[&str]) {
    let font = semi(12.0);
    let mut x = right_x;
    for (idx, &k) in keys.iter().enumerate().rev() {
        let kw = (text_width(ui, k, font.clone()) + 14.0).max(26.0);
        let kr = Rect::from_min_size(Pos2::new(x - kw, cy - 12.0), Vec2::new(kw, 24.0));
        ui.painter().rect_filled(kr, RADIUS_BASE, TAB_ACTIVE);
        ui.painter().rect_stroke(kr, RADIUS_BASE, Stroke::new(1.0, Color32::from_white_alpha(48)), egui::StrokeKind::Inside);
        ui.painter().hline(
            (kr.left() + 2.0)..=(kr.right() - 2.0),
            kr.bottom() - 1.0,
            Stroke::new(1.5, Color32::from_black_alpha(120)),
        );
        text(ui.painter(), kr.center(), k, font.clone(), TEXT, kw - 4.0, Align2::CENTER_CENTER);
        x -= kw;
        if idx > 0 {
            let plus_w = 12.0;
            text(ui.painter(), Pos2::new(x - plus_w * 0.5, cy), "+", marginal(), SUBTLE, plus_w, Align2::CENTER_CENTER);
            x -= plus_w;
        }
    }
}

fn shortcuts(app: &mut App, ui: &mut Ui, full: Rect) {
    let (mut top, dismissed) = scrim(ui, full, "shortcuts");
    if dismissed {
        app.help_open = false;
        return;
    }
    let w = 680.0f32.min(full.width() - 40.0);
    let h = 560.0f32.min(full.height() - 64.0);
    let panel = Rect::from_center_size(full.center(), Vec2::new(w, h));
    paint_modal_card(&mut top, panel, "shortcuts-panel");

    let title = app.t("Phím tắt", "Keyboard Shortcuts");
    let sub = app.t("Nhấn Ctrl + / hoặc ? để đóng/mở bảng này.", "Press Ctrl+/ or ? to toggle this modal.");
    text(top.painter(), panel.min + Vec2::new(24.0, 20.0), title, title_medium(), TEXT, w - 80.0, Align2::LEFT_TOP);
    text(top.painter(), panel.min + Vec2::new(24.0, 52.0), sub, body_small(), SUBTLE, w - 80.0, Align2::LEFT_TOP);
    let close = Rect::from_center_size(panel.right_top() + Vec2::new(-30.0, 30.0), Vec2::splat(32.0));
    if icon_button(&mut top, close, "help-close", Icon::Close, SUBTLE, app.t("Đóng", "Close")).clicked() {
        app.help_open = false;
        return;
    }

    struct Group {
        vi: &'static str,
        en: &'static str,
        items: &'static [(&'static str, &'static str, &'static [&'static str])],
    }
    const GROUPS: &[Group] = &[
        Group {
            vi: "Phát nhạc",
            en: "Playback",
            items: &[
                ("Phát / Tạm dừng", "Play / Pause", &["Space"]),
                ("Bài tiếp theo", "Skip to next track", &["Ctrl", "→"]),
                ("Bài trước", "Skip to previous track", &["Ctrl", "←"]),
                ("Tua tới 5 giây", "Seek forward 5s", &["Shift", "→"]),
                ("Tua lùi 5 giây", "Seek backward 5s", &["Shift", "←"]),
                ("Tăng âm lượng", "Raise volume", &["Ctrl", "↑"]),
                ("Giảm âm lượng", "Lower volume", &["Ctrl", "↓"]),
                ("Tắt / Bật tiếng", "Mute / Unmute", &["M"]),
                ("Trộn bài", "Toggle shuffle", &["Ctrl", "S"]),
                ("Lặp lại", "Toggle repeat", &["Ctrl", "R"]),
                ("Thích bài đang phát", "Like / Unlike current song", &["Alt", "Shift", "B"]),
            ],
        },
        Group {
            vi: "Điều hướng",
            en: "Navigation",
            items: &[
                ("Quay lại", "Back in history", &["Ctrl", "Alt", "←"]),
                ("Đi tiếp", "Forward in history", &["Ctrl", "Alt", "→"]),
                ("Trang chủ", "Go to Home", &["Alt", "Shift", "H"]),
                ("Bài hát đã thích", "Liked Songs", &["Alt", "Shift", "S"]),
                ("Danh sách chờ", "Queue", &["Alt", "Shift", "Q"]),
                ("Đã phát gần đây", "Recently Played", &["Alt", "Shift", "Y"]),
                ("Đang phát", "Now Playing View", &["Alt", "Shift", "J"]),
                ("Hoạt động bạn bè", "Friend Activity", &["Alt", "Shift", "U"]),
                ("Cài đặt", "Preferences / Settings", &["Ctrl", ","]),
            ],
        },
        Group {
            vi: "Tìm kiếm & Lọc",
            en: "Search / Filter",
            items: &[
                ("Tìm nhanh", "Quick search", &["Ctrl", "K"]),
                ("Thanh tìm kiếm", "Focus search bar", &["Ctrl", "L"]),
                ("Lọc trong trang hiện tại", "Filter page / tracklist", &["Ctrl", "F"]),
                ("Lọc trong Thư viện", "Filter Your Library", &["Ctrl", "Alt", "F"]),
                ("Xóa / Đóng hộp thoại", "Clear search / Close overlay", &["Esc"]),
            ],
        },
        Group {
            vi: "Thư viện",
            en: "Library",
            items: &[
                ("Tạo danh sách phát mới", "Create new playlist", &["Ctrl", "N"]),
                ("Tạo thư mục mới", "Create new folder", &["Ctrl", "Shift", "N"]),
                ("Tất cả trong Thư viện", "All Library items", &["Alt", "Shift", "0"]),
                ("Lọc Playlist", "Filter Playlists", &["Alt", "Shift", "1"]),
                ("Lọc Nghệ sĩ", "Filter Artists", &["Alt", "Shift", "3"]),
                ("Lọc Album", "Filter Albums", &["Alt", "Shift", "4"]),
            ],
        },
        Group {
            vi: "Bố cục",
            en: "Layout",
            items: &[
                ("Thu gọn / Mở Thư viện", "Toggle Left Library sidebar", &["Alt", "Shift", "L"]),
                ("Đóng / Mở bảng bên phải", "Toggle Right sidebar", &["Alt", "Shift", "R"]),
                ("Đổi kích thước Thư viện", "Resize Left sidebar (-/+10px)", &["Alt", "Shift", "← / →"]),
                ("Đổi kích thước bảng phải", "Resize Right sidebar (-/+10px)", &["Alt", "Shift", "↓ / ↑"]),
                ("Chuyển vùng Thư viện / Chính / Phải", "Focus Main / Right region", &["Alt", "Shift", "2 / 5"]),
                ("Bảng phím tắt", "Keyboard shortcuts", &["Ctrl", "/"]),
            ],
        },
    ];

    let lang = app.lang();
    let area = Rect::from_min_max(panel.min + Vec2::new(24.0, 84.0), panel.max - Vec2::new(16.0, 20.0));
    let mut body_ui = top.new_child(UiBuilder::new().id_salt("help-body").max_rect(area));
    ScrollArea::vertical().id_salt("help-scroll").auto_shrink([false, false]).show(&mut body_ui, |ui| {
        for (gi, group) in GROUPS.iter().enumerate() {
            if gi > 0 {
                ui.add_space(14.0);
            }
            let heading = if lang == Lang::Vi { group.vi } else { group.en };
            ui.label(RichText::new(heading).font(title_small()).color(TEXT));
            ui.add_space(6.0);
            ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, HAIRLINE));
            ui.add_space(4.0);
            for (vi, en, keys) in group.items {
                let label = if lang == Lang::Vi { *vi } else { *en };
                let row_w = ui.available_width();
                let (r, _) = ui.allocate_exact_size(Vec2::new(row_w, 34.0), Sense::hover());
                text(ui.painter(), r.left_center(), label, body_small(), SUBTLE, row_w * 0.56, Align2::LEFT_CENTER);
                paint_keycap_combo(ui, r.right() - 12.0, r.center().y, keys);
            }
        }
        ui.add_space(8.0);
    });
}

fn edit_playlist_modal(app: &mut App, ui: &mut Ui, full: Rect) {
    let (mut top, dismissed) = scrim(ui, full, "edit-playlist");
    if dismissed {
        app.actions.push(Action::CloseEditPlaylist);
        return;
    }

    let Some(state) = app.edit_playlist.as_ref() else { return };
    let pid = state.id;
    let preview_path = state.image.clone();
    let fallback_track = app.data.playlist(pid).and_then(|p| p.tracks.iter().find_map(|e| app.lib.track(e.id)));
    let (cover_key, cover_src) = if let Some(path) = preview_path.as_deref() {
        (format!("edit-pl:{pid}:{path}"), Some(path.to_string()))
    } else if let Some(track) = fallback_track {
        (track.cover_key(), track.art_source().map(str::to_string))
    } else {
        (format!("pl:{pid}"), None)
    };
    let cover_tex = cover_src.as_deref().map(|src| app.art.get(ui.ctx(), &cover_key, Some(src)));

    let title = app.t("Chỉnh sửa chi tiết", "Edit details");
    let t_name = app.t("Tên", "Name");
    let t_desc = app.t("Mô tả", "Description");
    let t_desc_hint = app.t("Thêm mô tả tùy chọn", "Add an optional description");
    let t_choose = app.t("Chọn ảnh", "Choose photo");
    let t_remove = app.t("Xóa ảnh", "Remove photo");
    let t_save = app.t("Lưu", "Save");
    let t_cancel = app.t("Hủy", "Cancel");
    let t_disclaimer = app.t(
        "Ảnh bìa được lưu cục bộ cho danh sách phát này.",
        "Cover art is stored locally for this playlist.",
    );

    let w = 540.0f32.min(full.width() - 40.0);
    let h = 376.0f32.min(full.height() - 48.0);
    let panel = Rect::from_center_size(full.center(), Vec2::new(w, h));
    paint_modal_card(&mut top, panel, "edit-playlist-panel");

    text(top.painter(), panel.min + Vec2::new(24.0, 22.0), title, title_medium(), TEXT, w - 80.0, Align2::LEFT_TOP);
    let close = Rect::from_center_size(panel.right_top() + Vec2::new(-30.0, 30.0), Vec2::splat(32.0));
    if icon_button(&mut top, close, "edit-pl-close", Icon::Close, SUBTLE, app.t("Đóng", "Close")).clicked() {
        app.actions.push(Action::CloseEditPlaylist);
        return;
    }

    // Left 180x180 cover art preview + hover actions.
    let art_rect = Rect::from_min_size(panel.min + Vec2::new(24.0, 72.0), Vec2::splat(180.0));
    cover_shadow(top.painter(), art_rect, RADIUS_BASE);
    if let Some(tex) = &cover_tex {
        cover(&top, art_rect, tex, RADIUS_BASE);
    } else {
        top.painter().rect_filled(art_rect, RADIUS_BASE, CARD_HOVER);
        icons::paint(
            top.painter(),
            Rect::from_center_size(art_rect.center(), Vec2::splat(56.0)),
            Icon::Library,
            MUTED,
        );
    }
    let art_hover = top.rect_contains_pointer(art_rect);
    let art_resp = top.interact(art_rect, Id::unique("edit-pl-art"), Sense::click());
    if art_hover {
        top.painter().rect_filled(art_rect, RADIUS_BASE, Color32::from_black_alpha(160));
        icons::paint(
            top.painter(),
            Rect::from_center_size(art_rect.center() - Vec2::new(0.0, 12.0), Vec2::splat(36.0)),
            Icon::CardToggle,
            TEXT,
        );
        text(
            top.painter(),
            art_rect.center() + Vec2::new(0.0, 22.0),
            t_choose,
            semi(14.0),
            TEXT,
            160.0,
            Align2::CENTER_CENTER,
        );
    }
    if art_resp.clicked() {
        app.actions.push(Action::PickPlaylistCover(pid));
    }

    let has_custom_image = preview_path.is_some();
    if has_custom_image {
        let rm_rect = Rect::from_min_size(Pos2::new(art_rect.left(), art_rect.bottom() + 8.0), Vec2::new(180.0, 26.0));
        let rm_resp = top.interact(rm_rect, Id::unique("edit-pl-clear-img"), Sense::click());
        let color = if rm_resp.hovered() { TEXT } else { SUBTLE };
        text(top.painter(), rm_rect.center(), t_remove, marginal(), color, 180.0, Align2::CENTER_CENTER);
        if rm_resp.clicked() {
            app.actions.push(Action::ClearPlaylistCover);
        }
    }

    // Right column: Name, validation error, Description + len/300 counter.
    let col_left = art_rect.right() + 18.0;
    let col_w = (panel.right() - 24.0 - col_left).max(160.0);
    let Some(state) = app.edit_playlist.as_mut() else { return };

    text(top.painter(), Pos2::new(col_left, art_rect.top()), t_name, semi(12.0), SUBTLE, col_w, Align2::LEFT_TOP);
    let name_box = Rect::from_min_size(Pos2::new(col_left, art_rect.top() + 18.0), Vec2::new(col_w, 38.0));
    top.painter().rect_filled(name_box, RADIUS_BASE, TAB_ACTIVE);
    let name_inner = name_box.shrink2(Vec2::new(10.0, 8.0));
    let mut name_ui = top.new_child(UiBuilder::new().id_salt("edit-pl-name").max_rect(name_inner));
    let name_resp = name_ui.add(
        TextEdit::singleline(&mut state.name)
            .frame(egui::Frame::NONE)
            .font(body_small())
            .desired_width(name_inner.width()),
    );
    if state.focus {
        state.focus = false;
        name_resp.request_focus();
    }
    if name_resp.changed() && !state.name.trim().is_empty() {
        state.error = None;
    }
    let name_stroke = if state.error.is_some() {
        Stroke::new(1.5, Color32::from_rgb(0xf1, 0x5e, 0x6c))
    } else if name_resp.has_focus() {
        Stroke::new(1.5, TEXT)
    } else {
        Stroke::new(1.0, HAIRLINE)
    };
    top.painter().rect_stroke(name_box, RADIUS_BASE, name_stroke, egui::StrokeKind::Inside);

    let mut desc_top = name_box.bottom() + 14.0;
    if let Some(err) = &state.error {
        text(
            top.painter(),
            Pos2::new(col_left, name_box.bottom() + 4.0),
            err,
            marginal(),
            Color32::from_rgb(0xf1, 0x5e, 0x6c),
            col_w,
            Align2::LEFT_TOP,
        );
        desc_top = name_box.bottom() + 24.0;
    }

    text(top.painter(), Pos2::new(col_left, desc_top), t_desc, semi(12.0), SUBTLE, col_w, Align2::LEFT_TOP);
    let desc_h = (art_rect.bottom() - (desc_top + 18.0)).max(84.0);
    let desc_box = Rect::from_min_size(Pos2::new(col_left, desc_top + 18.0), Vec2::new(col_w, desc_h));
    top.painter().rect_filled(desc_box, RADIUS_BASE, TAB_ACTIVE);
    let desc_inner = desc_box.shrink2(Vec2::new(10.0, 8.0));
    let mut desc_ui = top.new_child(UiBuilder::new().id_salt("edit-pl-desc").max_rect(desc_inner));
    let desc_resp = desc_ui.add(
        TextEdit::multiline(&mut state.description)
            .frame(egui::Frame::NONE)
            .font(body_small())
            .hint_text(RichText::new(t_desc_hint).color(MUTED).font(body_small()))
            .desired_width(desc_inner.width())
            .desired_rows(4),
    );
    let desc_stroke = if desc_resp.has_focus() {
        Stroke::new(1.5, TEXT)
    } else {
        Stroke::new(1.0, HAIRLINE)
    };
    top.painter().rect_stroke(desc_box, RADIUS_BASE, desc_stroke, egui::StrokeKind::Inside);

    let char_len = state.description.chars().count();
    let counter_color = if char_len > PLAYLIST_DESCRIPTION_MAX_CHARS {
        Color32::from_rgb(0xf1, 0x5e, 0x6c)
    } else {
        SUBTLE
    };
    text(
        top.painter(),
        Pos2::new(desc_box.right(), desc_box.bottom() + 6.0),
        &format!("{char_len}/{PLAYLIST_DESCRIPTION_MAX_CHARS}"),
        marginal(),
        counter_color,
        80.0,
        Align2::RIGHT_TOP,
    );

    // Footer disclaimer + Cancel / Save pill buttons.
    text(
        top.painter(),
        Pos2::new(panel.left() + 24.0, panel.bottom() - 28.0),
        t_disclaimer,
        marginal_small(),
        MUTED,
        w - 220.0,
        Align2::LEFT_CENTER,
    );

    let save_w = (text_width(&top, t_save, bold(14.0)) + 44.0).max(88.0);
    let save_r = Rect::from_min_size(Pos2::new(panel.right() - 24.0 - save_w, panel.bottom() - 56.0), Vec2::new(save_w, 38.0));
    let save_resp = top.interact(save_r, Id::unique("edit-pl-save"), Sense::click());
    let save_scaled = hover_scale_rect(save_r, save_resp.hovered(), save_resp.is_pointer_button_down_on());
    let save_bg = if save_resp.hovered() { Color32::from_rgb(0xf0, 0xf0, 0xf0) } else { TEXT };
    top.painter().rect_filled(save_scaled, save_scaled.height() * 0.5, save_bg);
    text(top.painter(), save_scaled.center(), t_save, bold(14.0), Color32::BLACK, save_w, Align2::CENTER_CENTER);

    let cancel_w = (text_width(&top, t_cancel, semi(14.0)) + 28.0).max(74.0);
    let cancel_r = Rect::from_min_size(Pos2::new(save_r.left() - 10.0 - cancel_w, save_r.top()), Vec2::new(cancel_w, 38.0));
    let cancel_resp = top.interact(cancel_r, Id::unique("edit-pl-cancel"), Sense::click());
    if cancel_resp.hovered() {
        top.painter().rect_filled(cancel_r, cancel_r.height() * 0.5, Color32::from_white_alpha(18));
    }
    text(top.painter(), cancel_r.center(), t_cancel, semi(14.0), TEXT, cancel_w, Align2::CENTER_CENTER);

    let enter_pressed = name_resp.lost_focus() && top.input(|i| i.key_pressed(Key::Enter));
    if save_resp.clicked() || enter_pressed {
        app.actions.push(Action::SaveEditPlaylist);
    } else if cancel_resp.clicked() {
        app.actions.push(Action::CloseEditPlaylist);
    }
}

fn rename_modal(app: &mut App, ui: &mut Ui, full: Rect, is_folder: bool) {
    let salt = if is_folder { "rename-folder" } else { "rename" };
    let (mut top, dismissed) = scrim(ui, full, salt);
    if dismissed {
        if is_folder {
            app.rename_folder = None;
        } else {
            app.rename = None;
        }
        return;
    }

    let title = if is_folder {
        app.t("Đổi tên thư mục", "Rename folder")
    } else {
        app.t("Đổi tên danh sách phát", "Rename playlist")
    };
    let save = app.t("Lưu", "Save");
    let cancel = app.t("Hủy", "Cancel");

    let w = 420.0f32.min(full.width() - 40.0);
    let panel = Rect::from_center_size(full.center(), Vec2::new(w, 184.0));
    paint_modal_card(&mut top, panel, salt);
    text(top.painter(), panel.min + Vec2::new(24.0, 22.0), title, title_small(), TEXT, w - 80.0, Align2::LEFT_TOP);

    let close = Rect::from_center_size(panel.right_top() + Vec2::new(-28.0, 28.0), Vec2::splat(28.0));
    if icon_button(&mut top, close, (salt, "close"), Icon::Close, SUBTLE, app.t("Đóng", "Close")).clicked() {
        if is_folder {
            app.rename_folder = None;
        } else {
            app.rename = None;
        }
        return;
    }

    let field = Rect::from_min_size(panel.min + Vec2::new(24.0, 68.0), Vec2::new(w - 48.0, 38.0));
    top.painter().rect_filled(field, RADIUS_BASE, TAB_ACTIVE);
    let inner = field.shrink2(Vec2::new(10.0, 8.0));
    let state = if is_folder { app.rename_folder.as_mut() } else { app.rename.as_mut() };
    let Some(state) = state else { return };
    let mut edit_ui = top.new_child(UiBuilder::new().id_salt((salt, "edit")).max_rect(inner));
    let resp = edit_ui.add(TextEdit::singleline(&mut state.name).frame(egui::Frame::NONE).font(body_small()).desired_width(inner.width()));
    if state.focus {
        state.focus = false;
        resp.request_focus();
    }
    let stroke = if resp.has_focus() {
        Stroke::new(1.5, TEXT)
    } else {
        Stroke::new(1.0, HAIRLINE)
    };
    top.painter().rect_stroke(field, RADIUS_BASE, stroke, egui::StrokeKind::Inside);

    let enter = resp.lost_focus() && top.input(|i| i.key_pressed(Key::Enter));
    let save_w = (text_width(&top, save, bold(14.0)) + 40.0).max(84.0);
    let save_r = Rect::from_min_size(panel.right_bottom() - Vec2::new(save_w + 24.0, 56.0), Vec2::new(save_w, 36.0));
    let save_b = top.interact(save_r, Id::unique((salt, "save")), Sense::click());
    let scaled = hover_scale_rect(save_r, save_b.hovered(), save_b.is_pointer_button_down_on());
    top.painter().rect_filled(scaled, scaled.height() * 0.5, if save_b.hovered() { ACCENT_HOVER } else { ACCENT });
    text(top.painter(), scaled.center(), save, bold(14.0), Color32::BLACK, save_w, Align2::CENTER_CENTER);

    let cancel_w = (text_width(&top, cancel, semi(14.0)) + 24.0).max(70.0);
    let cancel_r = Rect::from_min_size(Pos2::new(save_r.left() - 10.0 - cancel_w, save_r.top()), Vec2::new(cancel_w, 36.0));
    let cancel_b = top.interact(cancel_r, Id::unique((salt, "cancel")), Sense::click());
    if cancel_b.hovered() {
        top.painter().rect_filled(cancel_r, cancel_r.height() * 0.5, Color32::from_white_alpha(18));
    }
    text(top.painter(), cancel_r.center(), cancel, semi(14.0), TEXT, cancel_w, Align2::CENTER_CENTER);

    if cancel_b.clicked() {
        if is_folder {
            app.rename_folder = None;
        } else {
            app.rename = None;
        }
    } else if save_b.clicked() || enter {
        let id = state.id;
        let name = state.name.clone();
        if is_folder {
            app.actions.push(Action::RenameFolder(id, name));
        } else {
            app.actions.push(Action::SaveRename(id, name));
        }
    }
}

fn confirm_modal(app: &mut App, ui: &mut Ui, full: Rect) {
    let Some(pending) = app.confirm.clone() else { return };
    let (mut top, dismissed) = scrim(ui, full, "confirm-modal");
    if dismissed {
        app.actions.push(Action::CancelConfirm);
        return;
    }

    let (title, body_text, confirm_label) = match &pending {
        ConfirmAction::ClearQueue => (
            app.t("Xóa danh sách chờ?", "Clear queue?"),
            app.t("Tất cả bài hát tiếp theo trong hàng chờ sẽ bị xóa.", "All upcoming tracks in your queue will be removed.").to_string(),
            app.t("Xóa", "Clear"),
        ),
        ConfirmAction::DeletePlaylist(id) => {
            let name = app.data.playlist(*id).map(|p| p.name.as_str()).unwrap_or("");
            (
                app.t("Xóa khỏi Thư viện?", "Delete from Your Library?"),
                match app.lang() {
                    Lang::Vi => format!("Danh sách phát \"{name}\" sẽ bị xóa vĩnh viễn khỏi Thư viện của bạn."),
                    Lang::En => format!("This will permanently delete \"{name}\" from Your Library."),
                },
                app.t("Xóa", "Delete"),
            )
        }
        ConfirmAction::Unpin(item) => {
            let name = match item {
                RecentItem::Liked => app.t("Bài hát đã thích", "Liked Songs").to_string(),
                RecentItem::Playlist(id) => app.data.playlist(*id).map(|p| p.name.clone()).unwrap_or_default(),
                RecentItem::Album(k) => app.lib.album(k).map(|a| a.title.clone()).unwrap_or_else(|| k.clone()),
                RecentItem::Artist(n) => n.clone(),
                RecentItem::FollowedYouTube(id) => id.clone(),
            };
            (
                app.t("Bỏ ghim khỏi Thư viện?", "Unpin from Your Library?"),
                match app.lang() {
                    Lang::Vi => format!("Bỏ ghim \"{name}\" khỏi đầu Thư viện."),
                    Lang::En => format!("Unpin \"{name}\" from the top of Your Library."),
                },
                app.t("Bỏ ghim", "Unpin"),
            )
        }
        ConfirmAction::DeleteTtml(id) => {
            let name = app.track(*id).map(|t| t.title.clone()).unwrap_or_default();
            (
                app.t("Xóa tệp lời TTML?", "Remove imported lyrics?"),
                match app.lang() {
                    Lang::Vi => format!("Gỡ tệp lời đã nhập cho \"{name}\"."),
                    Lang::En => format!("Remove the imported TTML lyrics file for \"{name}\"."),
                },
                app.t("Xóa", "Remove"),
            )
        }
        ConfirmAction::DeleteFolder(id) => {
            let name = app.data.folder(*id).map(|f| f.name.as_str()).unwrap_or("");
            (
                app.t("Xóa thư mục?", "Delete folder?"),
                match app.lang() {
                    Lang::Vi => format!("Thư mục \"{name}\" sẽ bị xóa. Các playlist bên trong vẫn được giữ lại."),
                    Lang::En => format!("Delete folder \"{name}\"? Playlists inside will remain in Your Library."),
                },
                app.t("Xóa", "Delete"),
            )
        }
    };

    let cancel_label = app.t("Hủy", "Cancel");
    let w = 420.0f32.min(full.width() - 40.0);
    let h = 196.0;
    let panel = Rect::from_center_size(full.center(), Vec2::new(w, h));
    paint_modal_card(&mut top, panel, "confirm-card");

    text(top.painter(), panel.min + Vec2::new(24.0, 24.0), title, bold(20.0), TEXT, w - 48.0, Align2::LEFT_TOP);
    let body_rect = Rect::from_min_max(panel.min + Vec2::new(24.0, 58.0), panel.max - Vec2::new(24.0, 68.0));
    let mut body_ui = top.new_child(UiBuilder::new().id_salt("confirm-body").max_rect(body_rect));
    body_ui.add(egui::Label::new(RichText::new(body_text).font(body_small()).color(SUBTLE)).wrap());

    let confirm_w = (text_width(&top, confirm_label, bold(14.0)) + 40.0).max(88.0);
    let confirm_r = Rect::from_min_size(panel.right_bottom() - Vec2::new(confirm_w + 24.0, 56.0), Vec2::new(confirm_w, 36.0));
    let confirm_resp = top.interact(confirm_r, Id::unique("confirm-accept"), Sense::click());
    let scaled = hover_scale_rect(confirm_r, confirm_resp.hovered(), confirm_resp.is_pointer_button_down_on());
    let confirm_bg = if confirm_resp.hovered() { ACCENT_HOVER } else { ACCENT };
    top.painter().rect_filled(scaled, scaled.height() * 0.5, confirm_bg);
    text(top.painter(), scaled.center(), confirm_label, bold(14.0), Color32::BLACK, confirm_w, Align2::CENTER_CENTER);

    let cancel_w = (text_width(&top, cancel_label, semi(14.0)) + 28.0).max(74.0);
    let cancel_r = Rect::from_min_size(Pos2::new(confirm_r.left() - 10.0 - cancel_w, confirm_r.top()), Vec2::new(cancel_w, 36.0));
    let cancel_resp = top.interact(cancel_r, Id::unique("confirm-cancel"), Sense::click());
    if cancel_resp.hovered() {
        top.painter().rect_filled(cancel_r, cancel_r.height() * 0.5, Color32::from_white_alpha(18));
    }
    text(top.painter(), cancel_r.center(), cancel_label, semi(14.0), TEXT, cancel_w, Align2::CENTER_CENTER);

    if confirm_resp.clicked() {
        app.actions.push(Action::AcceptConfirm);
    } else if cancel_resp.clicked() {
        app.actions.push(Action::CancelConfirm);
    }
}

fn about_modal(app: &mut App, ui: &mut Ui, full: Rect) {
    let (mut top, dismissed) = scrim(ui, full, "about-modal");
    if dismissed || top.input(|i| i.key_pressed(Key::Escape)) {
        top.ctx().data_mut(|d| d.insert_temp(Id::unique(ABOUT_OPEN_ID), false));
        return;
    }
    let w = 420.0f32.min(full.width() - 40.0);
    let h = 230.0;
    let panel = Rect::from_center_size(full.center(), Vec2::new(w, h));
    paint_modal_card(&mut top, panel, "about-card");

    let close = Rect::from_center_size(panel.right_top() + Vec2::new(-28.0, 28.0), Vec2::splat(28.0));
    if icon_button(&mut top, close, "about-close", Icon::Close, SUBTLE, app.t("Đóng", "Close")).clicked() {
        top.ctx().data_mut(|d| d.insert_temp(Id::unique(ABOUT_OPEN_ID), false));
        return;
    }

    let icon_tex_id = Id::NULL.with("about-nhip-icon-tex");
    let icon_tex = top
        .ctx()
        .data(|d| d.get_temp::<egui::TextureHandle>(icon_tex_id))
        .unwrap_or_else(|| {
            const SZ: usize = 128;
            let rgba = crate::render_nhip_icon_rgba(SZ);
            let img = egui::ColorImage::from_rgba_unmultiplied([SZ, SZ], &rgba);
            let tex = top
                .ctx()
                .load_texture("nhip-app-icon-128", img, egui::TextureOptions::LINEAR);
            top.ctx().data_mut(|d| d.insert_temp(icon_tex_id, tex.clone()));
            tex
        });
    let icon_r = Rect::from_min_size(panel.min + Vec2::new(24.0, 22.0), Vec2::splat(52.0));
    top.painter().image(
        icon_tex.id(),
        icon_r,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE,
    );

    text(top.painter(), panel.min + Vec2::new(88.0, 24.0), "Nhịp", title_medium(), TEXT, w - 130.0, Align2::LEFT_TOP);
    text(
        top.painter(),
        panel.min + Vec2::new(88.0, 56.0),
        &format!("{} {}", app.t("Phiên bản", "Version"), env!("CARGO_PKG_VERSION")),
        semi(14.0),
        ACCENT,
        w - 130.0,
        Align2::LEFT_TOP,
    );
    let desc_rect = Rect::from_min_max(panel.min + Vec2::new(24.0, 92.0), panel.max - Vec2::new(24.0, 64.0));
    let mut desc_ui = top.new_child(UiBuilder::new().id_salt("about-body").max_rect(desc_rect));
    desc_ui.add(
        egui::Label::new(
            RichText::new(app.t(
                "Trình phát nhạc desktop viết bằng Rust & egui với bộ chỉnh âm 6 băng tần, chuyển êm bài hát và lời đồng bộ TTML.",
                "A desktop music player written in Rust & egui featuring a 6-band equalizer, crossfade/AutoMix transitions, and synced TTML lyrics.",
            ))
            .font(body_small())
            .color(SUBTLE),
        )
        .wrap(),
    );

    let ok = app.t("Đóng", "Close");
    let ok_w = (text_width(&top, ok, bold(14.0)) + 40.0).max(84.0);
    let ok_r = Rect::from_min_size(panel.right_bottom() - Vec2::new(ok_w + 24.0, 56.0), Vec2::new(ok_w, 36.0));
    let ok_b = top.interact(ok_r, Id::unique("about-ok"), Sense::click());
    let scaled = hover_scale_rect(ok_r, ok_b.hovered(), ok_b.is_pointer_button_down_on());
    top.painter().rect_filled(scaled, scaled.height() * 0.5, TEXT);
    text(top.painter(), scaled.center(), ok, bold(14.0), Color32::BLACK, ok_w, Align2::CENTER_CENTER);
    if ok_b.clicked() {
        top.ctx().data_mut(|d| d.insert_temp(Id::unique(ABOUT_OPEN_ID), false));
    }
}

fn paint_share_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let s = rect.width().min(rect.height());
    let c = rect.center();
    let sw = (s * 0.085).clamp(1.4, 2.0);
    let st = Stroke::new(sw, color);
    // Open tray U-shape
    let l = c.x - s * 0.34;
    let r = c.x + s * 0.34;
    let t = c.y - s * 0.06;
    let b = c.y + s * 0.36;
    painter.line_segment([Pos2::new(l, t), Pos2::new(l, b)], st);
    painter.line_segment([Pos2::new(l, b), Pos2::new(r, b)], st);
    painter.line_segment([Pos2::new(r, b), Pos2::new(r, t)], st);
    // Upward arrow shaft + head
    let tip = Pos2::new(c.x, c.y - s * 0.38);
    painter.line_segment([Pos2::new(c.x, c.y + s * 0.14), tip], st);
    painter.line_segment([tip, Pos2::new(c.x - s * 0.18, tip.y + s * 0.20)], st);
    painter.line_segment([tip, Pos2::new(c.x + s * 0.18, tip.y + s * 0.20)], st);
}

fn paint_down_arrow(painter: &egui::Painter, center: Pos2, size: f32, color: Color32) {
    let sw = (size * 0.12).clamp(1.6, 2.2);
    let st = Stroke::new(sw, color);
    let top = Pos2::new(center.x, center.y - size * 0.38);
    let tip = Pos2::new(center.x, center.y + size * 0.38);
    painter.line_segment([top, tip], st);
    painter.line_segment([tip, Pos2::new(center.x - size * 0.28, tip.y - size * 0.28)], st);
    painter.line_segment([tip, Pos2::new(center.x + size * 0.28, tip.y - size * 0.28)], st);
}

fn paint_preview_eq(painter: &egui::Painter, rect: Rect, playing: bool, time: f64, color: Color32) {
    let bw = (rect.width() * 0.22).clamp(2.2, 3.5);
    let gap = bw * 0.75;
    let total_w = bw * 3.0 + gap * 2.0;
    let x0 = rect.center().x - total_w * 0.5;
    let bot = rect.bottom() - 2.0;
    let max_h = (rect.height() - 4.0).max(8.0);
    let phases = [0.0f64, 2.1, 4.3];
    let speeds = [6.4f64, 8.2, 5.7];
    for i in 0..3 {
        let frac = if playing {
            0.28 + 0.72 * ((time * speeds[i] + phases[i]).sin() * 0.5 + 0.5) as f32
        } else {
            [0.65f32, 0.35, 0.50][i]
        };
        let h = (max_h * frac).max(3.0);
        let r = Rect::from_min_max(
            Pos2::new(x0 + i as f32 * (bw + gap), bot - h),
            Pos2::new(x0 + i as f32 * (bw + gap) + bw, bot),
        );
        painter.rect_filled(r, 1.5, color);
    }
}

/// Spotify Desktop Discovery Preview Feed modal (opened from Artist/Album `34x44` preview pill).
fn draw_preview_feed_modal(app: &mut App, ui: &mut Ui, full: Rect) {
    let (mut top, dismissed) = scrim(ui, full, "preview-feed-modal");
    if dismissed {
        app.actions.push(Action::ClosePreviewFeed);
        return;
    }

    let Some(feed_snapshot) = app.preview_feed.clone() else {
        return;
    };
    if feed_snapshot.items.is_empty() {
        app.actions.push(Action::ClosePreviewFeed);
        return;
    }

    let idx = feed_snapshot.index.min(feed_snapshot.items.len() - 1);
    let cur = &feed_snapshot.items[idx];
    let playing = app.is_playing();
    let reduced = app.data.settings.prefers_reduced_motion;

    // Smooth vertical reel animation towards `idx`.
    let dt = top.input(|i| i.unstable_dt).clamp(0.001, 0.05);
    let anim = if reduced {
        idx as f32
    } else if let Some(feed_mut) = app.preview_feed.as_mut() {
        let target = idx as f32;
        let diff = target - feed_mut.scroll_anim;
        if diff.abs() < 0.003 {
            feed_mut.scroll_anim = target;
        } else {
            feed_mut.scroll_anim += diff * (dt * 14.0).clamp(0.10, 0.38);
            top.ctx().request_repaint();
        }
        feed_mut.scroll_anim
    } else {
        idx as f32
    };
    if playing && !reduced {
        top.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }

    let w = (full.width() - 84.0).clamp(560.0, 968.0);
    let h = (full.height() - 88.0).clamp(400.0, 616.0);
    let panel = Rect::from_center_size(full.center(), Vec2::new(w, h));

    // Wheel scrolling steps cleanly 1 track per flick inside the modal.
    if top.rect_contains_pointer(panel) {
        let wheel_y = top.input(|i| i.smooth_scroll_delta.y);
        if wheel_y.abs() > 14.0 {
            let can_step = app
                .preview_feed
                .as_ref()
                .is_some_and(|f| f.last_wheel.is_none_or(|t| t.elapsed().as_millis() >= 220));
            if can_step {
                if let Some(f) = app.preview_feed.as_mut() {
                    f.last_wheel = Some(std::time::Instant::now());
                }
                let step = if wheel_y < 0.0 { 1 } else { -1 };
                app.actions.push(Action::PreviewFeedStep(step));
            }
        }
    }

    // Modal card backdrop: matte #121212 + ambient dominant-color right glow.
    top.interact(panel, top.scope_id().with("preview-feed-card"), Sense::click());
    let radius = CornerRadius::same(12);
    let shadow = egui::epaint::Shadow {
        offset: [0, 18],
        blur: 36,
        spread: 0,
        color: Color32::from_black_alpha(190),
    };
    top.painter().add(shadow.as_shape(panel, radius));
    top.painter().rect_filled(panel, radius, Color32::from_rgb(0x12, 0x12, 0x12));

    // Right-side ambient glow derived from the active track cover art.
    let accent = app.art.lyric_colors(&cur.cover_key)[0];
    let glow_rect = Rect::from_min_max(
        Pos2::new(panel.left() + w * 0.44, panel.top()),
        panel.max,
    );
    {
        let mut glow_ui = top.new_child(UiBuilder::new().id_salt("preview-glow").max_rect(panel));
        glow_ui.set_clip_rect(panel);
        let mut mesh = egui::Mesh::default();
        let left_c = Color32::TRANSPARENT;
        let right_mid = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 105);
        let right_edge = Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 42);
        mesh.colored_vertex(glow_rect.left_top(), left_c);
        mesh.colored_vertex(glow_rect.right_top(), right_edge);
        mesh.colored_vertex(Pos2::new(glow_rect.left(), glow_rect.center().y), left_c);
        mesh.colored_vertex(Pos2::new(glow_rect.right(), glow_rect.center().y), right_mid);
        mesh.colored_vertex(glow_rect.left_bottom(), left_c);
        mesh.colored_vertex(glow_rect.right_bottom(), right_edge);
        mesh.add_triangle(0, 1, 3);
        mesh.add_triangle(0, 3, 2);
        mesh.add_triangle(2, 3, 5);
        mesh.add_triangle(2, 5, 4);
        glow_ui.painter().add(egui::Shape::mesh(mesh));
    }

    // Geometry splits: left info (~48%), center-right 9:16 reel (~31%), far-right action rail (~21%).
    let reel_w = (h * 0.455).clamp(200.0, 292.0);
    let reel_h = (h - 54.0).max(280.0);
    let reel_left = panel.left() + (w * 0.485).max(240.0);
    let reel_right = (reel_left + reel_w).min(panel.right() - 108.0);
    let actual_reel_w = (reel_right - reel_left).max(160.0);
    let reel_cx = (reel_left + reel_right) * 0.5;
    let rail_cx = (reel_right + panel.right()) * 0.5;

    // Top-left entity header ("Low G" / Album title).
    text(
        top.painter(),
        Pos2::new(panel.left() + 26.0, panel.top() + 28.0),
        &feed_snapshot.entity_title,
        bold(15.0),
        TEXT,
        (reel_left - panel.left() - 40.0).max(80.0),
        Align2::LEFT_CENTER,
    );

    // Top-right controls: Volume/Mute, live 3-bar Equalizer, Close X.
    let top_cy = panel.top() + 28.0;
    let close_r = Rect::from_center_size(Pos2::new(panel.right() - 28.0, top_cy), Vec2::splat(30.0));
    if icon_button(&mut top, close_r, "pf-close", Icon::Close, TEXT, app.t("Đóng", "Close")).clicked() {
        app.actions.push(Action::ClosePreviewFeed);
        return;
    }
    let eq_r = Rect::from_center_size(Pos2::new(panel.right() - 62.0, top_cy), Vec2::splat(26.0));
    let eq_resp = top
        .interact(eq_r, top.scope_id().with("pf-eq"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if playing { app.t("Tạm dừng", "Pause") } else { app.t("Phát", "Play") });
    let t_now = top.input(|i| i.time);
    paint_preview_eq(top.painter(), eq_r.shrink(5.0), playing, t_now, if eq_resp.hovered() { TEXT } else { Color32::from_white_alpha(230) });
    if eq_resp.clicked() {
        app.actions.push(Action::TogglePlay);
    }
    let vol_r = Rect::from_center_size(Pos2::new(panel.right() - 96.0, top_cy), Vec2::splat(30.0));
    let vol_ic = if app.data.settings.muted || app.data.settings.volume <= 0.001 {
        Icon::Mute
    } else {
        Icon::Volume
    };
    if icon_button(&mut top, vol_r, "pf-mute", vol_ic, TEXT, app.t("Tắt / bật tiếng", "Mute / Unmute")).clicked() {
        app.actions.push(Action::ToggleMute);
    }

    // Left column: track title, artist, 48x48 cover + interactive scrubber, bottom artist follow + hashtags.
    let lx = panel.left() + 28.0;
    let left_w = (reel_left - lx - 24.0).max(160.0);
    let info_cy = panel.center().y - 12.0;

    // Track title (28px bold).
    text(
        top.painter(),
        Pos2::new(lx, info_cy - 34.0),
        &cur.title,
        bold(28.0),
        TEXT,
        left_w,
        Align2::LEFT_CENTER,
    );
    // Artist subtitle (clickable -> navigate to artist & close modal).
    let primary_artist = cur
        .artist
        .split(',')
        .next()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&feed_snapshot.artist_name);
    let art_r = Rect::from_min_size(Pos2::new(lx, info_cy - 10.0), Vec2::new(left_w, 22.0));
    let art_resp = top
        .interact(art_r, top.scope_id().with("pf-track-artist"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    text(
        top.painter(),
        art_r.left_center(),
        &cur.artist,
        body(14.0),
        if art_resp.hovered() { TEXT } else { SUBTLE },
        left_w,
        Align2::LEFT_CENTER,
    );
    if art_resp.clicked() && !primary_artist.is_empty() {
        app.actions.push(Action::ClosePreviewFeed);
        app.actions.push(Action::Navigate(Route::Artist(primary_artist.to_string())));
    }

    // 48x48 cover + sleek scrubber bar.
    let thumb_r = Rect::from_min_size(Pos2::new(lx, info_cy + 26.0), Vec2::splat(48.0));
    let cur_tex = app.art.get(top.ctx(), &cur.cover_key, cur.cover_path.as_deref());
    cover(&mut top, thumb_r, &cur_tex, 4.0);
    let thumb_resp = top
        .interact(thumb_r, top.scope_id().with("pf-thumb-play"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if thumb_resp.clicked() {
        app.actions.push(Action::TogglePlay);
    }

    let scrub_x0 = thumb_r.right() + 16.0;
    let scrub_x1 = (lx + left_w - 28.0).max(scrub_x0 + 60.0);
    let scrub_hit = Rect::from_min_max(
        Pos2::new(scrub_x0, thumb_r.center().y - 10.0),
        Pos2::new(scrub_x1, thumb_r.center().y + 10.0),
    );
    let scrub_id = top.scope_id().with("pf-scrubber");
    let scrub_resp = top
        .interact(scrub_hit, scrub_id, Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let dur_ms = app.duration_ms().max(cur.duration_ms).max(1);
    let drag_key = scrub_id.with("drag-frac");
    let mut drag_frac = top.ctx().data(|d| d.get_temp::<f32>(drag_key));
    if scrub_resp.dragged() {
        if let Some(pos) = top.ctx().pointer_interact_pos() {
            let f = ((pos.x - scrub_x0) / (scrub_x1 - scrub_x0).max(1.0)).clamp(0.0, 1.0);
            drag_frac = Some(f);
            top.ctx().data_mut(|d| d.insert_temp(drag_key, f));
        }
    } else if scrub_resp.drag_stopped() {
        if let Some(f) = drag_frac.take() {
            top.ctx().data_mut(|d| d.remove::<f32>(drag_key));
            app.actions.push(Action::SeekTo((f as f64 * dur_ms as f64) as u64));
        }
    } else if scrub_resp.clicked() {
        if let Some(pos) = top.ctx().pointer_interact_pos() {
            let f = ((pos.x - scrub_x0) / (scrub_x1 - scrub_x0).max(1.0)).clamp(0.0, 1.0);
            app.actions.push(Action::SeekTo((f as f64 * dur_ms as f64) as u64));
        }
    } else if drag_frac.is_some() {
        drag_frac = None;
        top.ctx().data_mut(|d| d.remove::<f32>(drag_key));
    }
    let frac = drag_frac.unwrap_or_else(|| (app.position_ms() as f32 / dur_ms as f32).clamp(0.0, 1.0));
    let bar_h = if scrub_resp.hovered() || scrub_resp.dragged() { 4.0 } else { 3.0 };
    let bar_rect = Rect::from_min_max(
        Pos2::new(scrub_x0, thumb_r.center().y - bar_h * 0.5),
        Pos2::new(scrub_x1, thumb_r.center().y + bar_h * 0.5),
    );
    top.painter().rect_filled(bar_rect, bar_h * 0.5, Color32::from_white_alpha(48));
    let fill_x = scrub_x0 + (scrub_x1 - scrub_x0) * frac;
    let fill_rect = Rect::from_min_max(bar_rect.min, Pos2::new(fill_x.max(scrub_x0), bar_rect.max.y));
    top.painter().rect_filled(fill_rect, bar_h * 0.5, TEXT);
    let knob_r = if scrub_resp.hovered() || scrub_resp.dragged() { 5.0 } else { 3.5 };
    top.painter().circle_filled(Pos2::new(fill_x, thumb_r.center().y), knob_r, TEXT);

    // Bottom-left anchor: artist avatar + name + concise listener stat + Follow pill + hashtag pills.
    let tags_y = panel.bottom() - 34.0;
    let artist_row_y = tags_y - 36.0;
    let av_r = Rect::from_center_size(Pos2::new(lx + 14.0, artist_row_y), Vec2::splat(28.0));
    let av_tex = app.art.get(top.ctx(), &feed_snapshot.entity_avatar_key, feed_snapshot.entity_avatar_src.as_deref());
    cover(&mut top, av_r, &av_tex, 14.0);

    let follow_target = if !primary_artist.is_empty() {
        primary_artist.to_string()
    } else {
        feed_snapshot.artist_name.clone()
    };
    let mut ax = av_r.right() + 10.0;
    let name_w = text_width(&top, &follow_target, semi(13.5)).min(left_w * 0.42);
    text(top.painter(), Pos2::new(ax, artist_row_y), &follow_target, semi(13.5), TEXT, name_w + 2.0, Align2::LEFT_CENTER);
    ax += name_w + 6.0;
    if let Some(sub) = &feed_snapshot.entity_subtitle {
        if !sub.is_empty() {
            let stat = format!("• {sub}");
            let sw = text_width(&top, &stat, body(13.0)).min(90.0);
            text(top.painter(), Pos2::new(ax, artist_row_y), &stat, body(13.0), SUBTLE, sw + 2.0, Align2::LEFT_CENTER);
            ax += sw + 10.0;
        }
    }
    let following = app.data.followed_artists.contains(&follow_target);
    let fl_lbl = if following {
        app.t("Đang theo dõi", "Following")
    } else {
        app.t("Theo dõi", "Follow")
    };
    let fl_w = text_width(&top, fl_lbl, semi(12.5)) + 24.0;
    let fl_r = Rect::from_min_size(Pos2::new(ax, artist_row_y - 14.0), Vec2::new(fl_w.min((reel_left - ax - 16.0).max(60.0)), 28.0));
    let fl_resp = top
        .interact(fl_r, top.scope_id().with("pf-follow"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    top.painter().rect_stroke(
        fl_r,
        14.0,
        Stroke::new(1.0, if fl_resp.hovered() { TEXT } else { MUTED }),
        egui::StrokeKind::Inside,
    );
    text(top.painter(), fl_r.center(), fl_lbl, semi(12.5), TEXT, fl_r.width() - 8.0, Align2::CENTER_CENTER);
    if fl_resp.clicked() && !follow_target.is_empty() {
        app.actions.push(Action::ToggleArtistFollow(follow_target.clone()));
    }

    // Hashtag pills row.
    let mut hx = lx;
    for (ti, tag) in feed_snapshot.tags.iter().take(4).enumerate() {
        let tw = text_width(&top, tag, body(12.0)) + 22.0;
        if hx + tw > reel_left - 16.0 {
            break;
        }
        let tr = Rect::from_min_size(Pos2::new(hx, tags_y - 14.0), Vec2::new(tw, 28.0));
        let tresp = top
            .interact(tr, top.scope_id().with(("pf-tag", ti)), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let fill = if tresp.hovered() {
            Color32::from_rgb(0x30, 0x30, 0x30)
        } else {
            Color32::from_rgb(0x23, 0x23, 0x23)
        };
        top.painter().rect_filled(tr, 14.0, fill);
        text(top.painter(), tr.center(), tag, body(12.0), TEXT, tw - 10.0, Align2::CENTER_CENTER);
        if tresp.clicked() {
            let q = tag.trim_start_matches('#').to_string();
            app.actions.push(Action::ClosePreviewFeed);
            app.actions.push(Action::SearchCategory(q));
        }
        hx += tw + 8.0;
    }

    // Center-right 9:16 Vertical Reel with smooth animated peeking prev/next cards.
    let reel_clip = Rect::from_min_max(
        Pos2::new(reel_left - 2.0, panel.top() + 1.0),
        Pos2::new(reel_right + 2.0, panel.bottom() - 1.0),
    );
    {
        let mut reel_ui = top.new_child(UiBuilder::new().id_salt("pf-reel").max_rect(reel_clip));
        reel_ui.set_clip_rect(reel_clip);
        let stride = reel_h + 18.0;
        let first_vis = (anim.floor() as isize - 1).max(0) as usize;
        let last_vis = ((anim.ceil() as isize + 1).max(0) as usize).min(feed_snapshot.items.len() - 1);
        for i in first_vis..=last_vis {
            let item = &feed_snapshot.items[i];
            let dy = (i as f32 - anim) * stride;
            let card_r = Rect::from_center_size(Pos2::new(reel_cx, panel.center().y + dy), Vec2::new(actual_reel_w, reel_h));
            let card_tex = app.art.get(reel_ui.ctx(), &item.cover_key, item.cover_path.as_deref());
            // Aspect-fill center-crop cover into tall 9:16 card.
            let sz = card_tex.size();
            let uv = if sz[0] > 0 && sz[1] > 0 {
                let img_aspect = sz[0] as f32 / sz[1] as f32;
                let box_aspect = card_r.width() / card_r.height().max(1.0);
                if img_aspect > box_aspect {
                    let vis_w = (box_aspect / img_aspect).clamp(0.18, 1.0);
                    let lx_uv = (1.0 - vis_w) * 0.5;
                    Rect::from_min_max(Pos2::new(lx_uv, 0.0), Pos2::new(lx_uv + vis_w, 1.0))
                } else {
                    let vis_h = (img_aspect / box_aspect).clamp(0.18, 1.0);
                    let ty_uv = (1.0 - vis_h) * 0.5;
                    Rect::from_min_max(Pos2::new(0.0, ty_uv), Pos2::new(1.0, ty_uv + vis_h))
                }
            } else {
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))
            };
            let is_active = i == idx;
            let card_resp = reel_ui
                .interact(card_r, reel_ui.scope_id().with(("pf-card", i)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let tint = if is_active { Color32::WHITE } else { Color32::from_gray(190) };
            let img_shape = egui::epaint::RectShape::filled(card_r, CornerRadius::same(12), tint)
                .with_texture(card_tex.id(), uv);
            reel_ui.painter().add(img_shape);
            // Subtle inner highlight border.
            reel_ui.painter().rect_stroke(
                card_r,
                12.0,
                Stroke::new(1.0, Color32::from_white_alpha(22)),
                egui::StrokeKind::Inside,
            );
            if is_active && !playing {
                let play_c = Rect::from_center_size(card_r.center(), Vec2::splat(52.0));
                reel_ui.painter().circle_filled(play_c.center(), 26.0, Color32::from_black_alpha(150));
                icons::paint(reel_ui.painter(), Rect::from_center_size(play_c.center() + Vec2::new(2.0, 0.0), Vec2::splat(22.0)), Icon::Play, TEXT);
            }
            if card_resp.clicked() {
                if is_active {
                    app.actions.push(Action::TogglePlay);
                } else {
                    let delta = i as isize - idx as isize;
                    app.actions.push(Action::PreviewFeedStep(delta));
                }
            }
        }
    }

    // Far-right vertical action rail: Dots, Share, Add to Queue, Save/Like.
    let cur_track_id = cur.local_id.or_else(|| {
        cur.ytm_item.as_ref().and_then(|it| match &it.id {
            crate::ytm::ItemId::Song(vid) => Some(crate::library::fnv(&format!("yt:{vid}"))),
            _ => None,
        })
    });
    let is_liked = cur_track_id.is_some_and(|id| app.data.is_liked(id));
    let rail_mid = panel.center().y - 10.0;

    let dots_r = Rect::from_center_size(Pos2::new(rail_cx, rail_mid - 72.0), Vec2::splat(34.0));
    let dots_resp = icon_button_sized(&mut top, dots_r, "pf-dots", Icon::Dots, TEXT, app.t("Tùy chọn khác", "More options"), 22.0);
    egui::Popup::menu(&dots_resp).show(|ui| {
        ui.set_min_width(196.0);
        if menu_row(ui, Some(Icon::Queue), app.t("Thêm vào danh sách chờ", "Add to queue"), None, false, true).clicked() {
            if let Some(ytm_it) = &cur.ytm_item {
                app.actions.push(Action::EnqueueYouTubeSong(ytm_it.clone()));
            } else if let Some(tid) = cur.local_id {
                app.actions.push(Action::Enqueue(tid));
            }
            ui.close();
        }
        if menu_row(ui, None, app.t("Chuyển tới nghệ sĩ", "Go to artist"), None, false, !primary_artist.is_empty()).clicked() {
            app.actions.push(Action::ClosePreviewFeed);
            app.actions.push(Action::Navigate(Route::Artist(primary_artist.to_string())));
            ui.close();
        }
        if menu_row(ui, None, app.t("Sao chép liên kết", "Copy link"), None, false, true).clicked() {
            let link = cur.url.clone().unwrap_or_else(|| format!("{} - {}", cur.title, cur.artist));
            ui.ctx().copy_text(link);
            app.toast(app.t("Đã sao chép liên kết", "Link copied").to_string());
            ui.close();
        }
    });

    let share_r = Rect::from_center_size(Pos2::new(rail_cx, rail_mid - 24.0), Vec2::splat(34.0));
    let share_resp = top
        .interact(share_r, top.scope_id().with("pf-share"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(app.t("Chia sẻ", "Share"));
    paint_share_icon(top.painter(), Rect::from_center_size(share_r.center(), Vec2::splat(22.0)), if share_resp.hovered() { TEXT } else { Color32::from_white_alpha(230) });
    if share_resp.clicked() {
        let link = cur.url.clone().unwrap_or_else(|| format!("{} - {}", cur.title, cur.artist));
        top.ctx().copy_text(link);
        app.toast(app.t("Đã sao chép liên kết", "Link copied").to_string());
    }

    let queue_r = Rect::from_center_size(Pos2::new(rail_cx, rail_mid + 24.0), Vec2::splat(34.0));
    if icon_button_sized(&mut top, queue_r, "pf-queue", Icon::Queue, TEXT, app.t("Thêm vào danh sách chờ", "Add to queue"), 22.0).clicked() {
        if let Some(ytm_it) = &cur.ytm_item {
            app.actions.push(Action::EnqueueYouTubeSong(ytm_it.clone()));
        } else if let Some(tid) = cur.local_id {
            app.actions.push(Action::Enqueue(tid));
        }
    }

    let like_r = Rect::from_center_size(Pos2::new(rail_cx, rail_mid + 72.0), Vec2::splat(34.0));
    let (l_ic, l_col, l_tip) = if is_liked {
        (Icon::Saved, ACCENT, app.t("Xóa khỏi Bài hát đã thích", "Remove from Liked Songs"))
    } else {
        (Icon::Save, TEXT, app.t("Lưu vào Bài hát đã thích", "Save to Liked Songs"))
    };
    if icon_button_sized(&mut top, like_r, "pf-like", l_ic, l_col, l_tip, 24.0).clicked() {
        if let Some(tid) = cur_track_id {
            app.actions.push(Action::ToggleLike(tid));
        }
    }

    // Bottom-right navigation: white "Tiếp ↓" pill on card 0; stacked ↑ / ↓ chevrons on cards > 0.
    let has_next = idx + 1 < feed_snapshot.items.len();
    if idx == 0 && has_next {
        let lbl = app.t("Tiếp", "Next");
        let btn_w = (text_width(&top, lbl, bold(14.5)) + 52.0).clamp(88.0, 120.0);
        let btn_r = Rect::from_center_size(Pos2::new(rail_cx, panel.bottom() - 42.0), Vec2::new(btn_w, 44.0));
        let b_resp = top
            .interact(btn_r, top.scope_id().with("pf-next-pill"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let scaled = hover_scale_rect(btn_r, b_resp.hovered(), b_resp.is_pointer_button_down_on());
        top.painter().rect_filled(scaled, scaled.height() * 0.5, TEXT);
        text(
            top.painter(),
            Pos2::new(scaled.center().x - 10.0, scaled.center().y),
            lbl,
            bold(14.5),
            Color32::BLACK,
            btn_w - 30.0,
            Align2::CENTER_CENTER,
        );
        paint_down_arrow(
            top.painter(),
            Pos2::new(scaled.center().x + text_width(&top, lbl, bold(14.5)) * 0.5 + 8.0, scaled.center().y),
            14.0,
            Color32::BLACK,
        );
        if b_resp.clicked() {
            app.actions.push(Action::PreviewFeedStep(1));
        }
    } else if feed_snapshot.items.len() > 1 {
        let up_r = Rect::from_center_size(Pos2::new(rail_cx, panel.bottom() - 82.0), Vec2::splat(36.0));
        if icon_button_sized(&mut top, up_r, "pf-prev-btn", Icon::ChevronUp, TEXT, app.t("Bài trước", "Previous preview"), 20.0).clicked() {
            app.actions.push(Action::PreviewFeedStep(-1));
        }
        let dn_r = Rect::from_center_size(Pos2::new(rail_cx, panel.bottom() - 38.0), Vec2::splat(36.0));
        let dn_col = if has_next { TEXT } else { MUTED };
        if icon_button_sized(&mut top, dn_r, "pf-next-btn", Icon::ChevronDown, dn_col, app.t("Bài tiếp", "Next preview"), 20.0).clicked() && has_next {
            app.actions.push(Action::PreviewFeedStep(1));
        }
    }

    // Outer subtle modal hairline border.
    top.painter().rect_stroke(panel, radius, Stroke::new(1.0, Color32::from_white_alpha(18)), egui::StrokeKind::Inside);
}

fn toast(app: &App, ui: &mut Ui, full: Rect) {
    let Some((message, _)) = &app.toast else { return };
    let layer = LayerId::new(Order::Tooltip, Id::unique("toast"));
    let p = ui.ctx().layer_painter(layer);
    let w = text_width(ui, message, semi(14.0)).min(full.width() - 80.0) + 40.0;
    let r = Rect::from_center_size(Pos2::new(full.center().x, full.bottom() - PLAYER_BAR - 36.0), Vec2::new(w, 40.0));
    let shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };
    p.add(shadow.as_shape(r, CornerRadius::same(RADIUS_CARD as u8)));
    p.rect_filled(r, RADIUS_CARD, Color32::from_rgb(0x2e, 0x77, 0xd0));
    text(&p, r.center(), message, semi(14.0), TEXT, w - 24.0, Align2::CENTER_CENTER);
}
