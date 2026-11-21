//! The library panel on the left (Spotify Your Library X / YLX 3-state sidebar).

use std::cmp::Reverse;
use std::time::Instant;

use chrono::{Local, TimeZone};
use eframe::egui::{
    self, Align2, Color32, ColorImage, CursorIcon, Id, LayerId, Order, Pos2, Rect, Response,
    RichText, ScrollArea, Sense, Stroke, TextureHandle, TextureOptions, Ui, UiBuilder, Vec2,
};

use super::icons::{self, Icon};
use super::pages::{item_of, Item};
use super::widgets::*;
use crate::app::{
    Action, App, ConfirmAction, LibraryCreatorFilter, LibraryFilter, LibrarySortBy,
    LibraryViewMode, RenameState, Route,
};
use crate::library::{fold, RecentItem};
use crate::queue::ContextRef;
use crate::theme::*;

const LIKED_GRADIENT_TOP_LEFT: Color32 = Color32::from_rgb(0x45, 0x0a, 0xf5);
const LIKED_GRADIENT_BOTTOM_RIGHT: Color32 = Color32::from_rgb(0xc4, 0xef, 0xd9);
const COMPACT_ROW_H: f32 = 32.0;
const DEFAULT_ROW_H: f32 = 64.0;
const RAIL_ROW_H: f32 = 64.0;

enum EntryKind {
    Item(Item),
    Folder {
        id: u64,
        name: String,
        count: usize,
    },
}

struct Entry {
    kind: EntryKind,
    title: String,
    sub: String,
    pinned: bool,
    added: u64,
    recent_rank: usize,
    creator: String,
}

impl Entry {
    fn is_playable(&self) -> bool {
        matches!(
            &self.kind,
            EntryKind::Item(item)
                if matches!(
                    item.kind,
                    RecentItem::Liked
                        | RecentItem::Playlist(_)
                        | RecentItem::Album(_)
                        | RecentItem::Artist(_)
                        | RecentItem::FollowedYouTube(_)
                )
        )
    }

    fn context(&self) -> ContextRef {
        match &self.kind {
            EntryKind::Item(item) => context_of(&item.kind),
            EntryKind::Folder { .. } => ContextRef::None,
        }
    }
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    let rect = ui.max_rect();
    let window_w = ui.ctx().input(|i| i.content_rect().width());
    let collapsed = app.data.settings.left_collapsed || rect.width() <= LEFT_COLLAPSED + 8.0;
    let is_wide = !collapsed && rect.width() >= LEFT_EXPANDED_MIN - 4.0;
    let can_expand = window_w >= 1024.0 || app.data.settings.left_width >= LEFT_MAX;

    if collapsed {
        draw_collapsed(app, ui, rect);
        return;
    }

    let panel_hovered = ui.rect_contains_pointer(rect);
    let mut y = rect.top();
    let header_h = 59.0;
    draw_header(
        app,
        ui,
        Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), header_h)),
        can_expand,
        panel_hovered,
    );
    y += header_h;

    let inside_folder = app.opened_folder.is_some();
    if is_wide && !inside_folder {
        let bar_h = 48.0;
        let bar_rect = Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), bar_h));
        draw_wide_toolbar(app, ui, bar_rect);
        y += bar_h;
    } else {
        if !inside_folder {
            let chips_h = 48.0;
            let chips_rect =
                Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), chips_h));
            draw_filter_chips(app, ui, chips_rect);
            y += chips_h;
        }
        if app.lib_search {
            let tools_h = 38.0;
            let tools_rect =
                Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), tools_h));
            draw_search_and_sort(app, ui, tools_rect, false);
            y += tools_h;
        }
    }

    let view = app.data.settings.library_view;
    if is_wide && matches!(view, LibraryViewMode::CompactList | LibraryViewMode::DefaultList) {
        let col_h = 28.0;
        let col_rect =
            Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), col_h));
        draw_table_header(app, ui, col_rect);
        y += col_h;
    }

    let entries = collect_entries(app);
    let list_rect = Rect::from_min_max(Pos2::new(rect.left(), y + 2.0), rect.max);
    let mut list_ui = ui.new_child(UiBuilder::new().id_salt("lib-list").max_rect(list_rect));
    list_ui.set_clip_rect(list_rect.intersect(ui.clip_rect()));

    ScrollArea::vertical()
        .id_salt("library-scroll")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            if !is_wide && !app.lib_search {
                let tools_h = 34.0;
                let (tools_rect, _) = ui.allocate_exact_size(
                    Vec2::new(rect.width(), tools_h),
                    Sense::hover(),
                );
                let tools_rect = Rect::from_min_size(
                    Pos2::new(rect.left(), tools_rect.top()),
                    Vec2::new(rect.width(), tools_h),
                );
                draw_search_and_sort(app, ui, tools_rect, false);
                ui.add_space(4.0);
            } else {
                ui.add_space(4.0);
            }
            if entries.is_empty() {
                draw_empty_state(app, ui);
                return;
            }
            match view {
                LibraryViewMode::DefaultList => draw_list_rows(app, ui, &entries, DEFAULT_ROW_H, false, is_wide),
                LibraryViewMode::CompactList => draw_list_rows(app, ui, &entries, COMPACT_ROW_H, true, is_wide),
                LibraryViewMode::CompactGrid => draw_grid(app, ui, &entries, true),
                LibraryViewMode::DefaultGrid => draw_grid(app, ui, &entries, false),
            }
            ui.add_space(8.0);
        });
}

fn draw_collapsed(app: &mut App, ui: &mut Ui, rect: Rect) {
    // Spotify `.main-yourLibraryX-headerIsCollapsed`: 107px header (16px top pad, 16px gap, 35px + button) + 8px list gap.
    let header_h = 115.0;
    let lib_rect = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 32.0), Vec2::splat(40.0));
    if icon_button_sized(
        ui,
        lib_rect,
        "lib-expand-rail",
        Icon::Library,
        SUBTLE,
        app.t("Mở Thư viện", "Open Your Library"),
        24.0,
    )
    .clicked()
    {
        app.actions.push(Action::ToggleLeft);
    }

    let plus_rect = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 81.5), Vec2::splat(35.0));
    let plus_resp = ui
        .interact(plus_rect, ui.scope_id().with("lib-rail-plus"), Sense::click())
        .on_hover_text(app.t("Tạo playlist hoặc thư mục", "Create playlist or folder"));
    let plus_bg = if plus_resp.hovered() { HOVER } else { ELEVATED };
    ui.painter().circle_filled(plus_rect.center(), 17.5, plus_bg);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(plus_rect.center(), Vec2::splat(16.0)),
        Icon::Plus,
        if plus_resp.hovered() { TEXT } else { SUBTLE },
    );
    focus_ring(ui, &plus_resp, 17.5);
    show_create_menu(app, ui, &plus_resp);

    let entries = collect_entries(app);
    let list_rect = Rect::from_min_max(Pos2::new(rect.left(), rect.top() + header_h), rect.max);
    let mut list_ui = ui.new_child(UiBuilder::new().id_salt("lib-rail-list").max_rect(list_rect));
    list_ui.set_clip_rect(list_rect.intersect(ui.clip_rect()));

    ScrollArea::vertical()
        .id_salt("library-rail-scroll")
        .auto_shrink([false, false])
        .show(&mut list_ui, |ui| {
            let w = ui.available_width();
            for (idx, entry) in entries.iter().enumerate() {
                let (row_rect, _) = ui.allocate_exact_size(Vec2::new(w, RAIL_ROW_H), Sense::hover());
                draw_rail_item(app, ui, row_rect, entry, idx);
            }
            ui.add_space(8.0);
        });
}

fn draw_rail_item(app: &mut App, ui: &mut Ui, row_rect: Rect, entry: &Entry, idx: usize) {
    let resp = ui.interact(
        row_rect,
        ui.scope_id().with(("lib-rail-item", idx)),
        Sense::click(),
    );
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &entry.title));
    let selected = is_entry_selected(app, entry);
    let live = is_entry_playing(app, entry);
    let cover_rect = Rect::from_center_size(row_rect.center(), Vec2::splat(48.0));
    let bg_rect = Rect::from_center_size(row_rect.center(), Vec2::splat(56.0));
    if selected {
        ui.painter().rect_filled(bg_rect, RADIUS_LARGER, CARD_HOVER);
    } else if resp.hovered() {
        ui.painter().rect_filled(bg_rect, RADIUS_LARGER, hover_white(20));
    }

    paint_entry_cover(app, ui, cover_rect, entry, RADIUS_BASE);

    if live {
        let is_circle = matches!(&entry.kind, EntryKind::Item(item) if item.circle);
        let stroke = Stroke::new(2.0, ACCENT);
        if is_circle {
            ui.painter()
                .circle_stroke(cover_rect.center(), cover_rect.width() * 0.5 + 2.0, stroke);
        } else {
            ui.painter().rect_stroke(
                cover_rect.expand(2.0),
                RADIUS_LARGER,
                stroke,
                egui::StrokeKind::Outside,
            );
        }
    }

    if resp.hovered() {
        paint_rail_tooltip(ui, row_rect, entry, live);
    }
    handle_entry_interactions(app, ui, &resp, entry, None);
}

fn paint_rail_tooltip(ui: &Ui, row_rect: Rect, entry: &Entry, live: bool) {
    let title_font = semi(14.0);
    let sub_font = body(12.5);
    let title_w = text_width(ui, &entry.title, title_font.clone());
    let sub_w = text_width(ui, &entry.sub, sub_font.clone())
        + if entry.pinned { 18.0 } else { 0.0 }
        + if live { 22.0 } else { 0.0 };
    let tip_w = title_w.max(sub_w).clamp(120.0, 260.0) + 24.0;
    let tip_h = 54.0;
    let tip_pos = Pos2::new(row_rect.right() + 10.0, (row_rect.center().y - tip_h * 0.5).max(8.0));
    let tip_rect = Rect::from_min_size(tip_pos, Vec2::new(tip_w, tip_h));
    let layer = ui
        .ctx()
        .layer_painter(LayerId::new(Order::Tooltip, Id::unique("lib-rail-tip")));
    let shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(160),
    };
    layer.add(shadow.as_shape(tip_rect, card_radius(RADIUS_CARD as u8)));
    layer.rect_filled(tip_rect, RADIUS_CARD, POPUP);
    layer.rect_stroke(tip_rect, RADIUS_CARD, Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);

    let title_color = if live { ACCENT } else { TEXT };
    text(
        &layer,
        Pos2::new(tip_rect.left() + 12.0, tip_rect.top() + 17.0),
        &entry.title,
        title_font,
        title_color,
        tip_w - 24.0,
        Align2::LEFT_CENTER,
    );

    let mut sx = tip_rect.left() + 12.0;
    let sy = tip_rect.top() + 37.0;
    if entry.pinned {
        icons::paint(
            &layer,
            Rect::from_center_size(Pos2::new(sx + 6.0, sy), Vec2::splat(12.0)),
            Icon::Pin,
            ACCENT,
        );
        sx += 16.0;
    }
    let sub_max = (tip_rect.right() - 12.0 - sx - if live { 20.0 } else { 0.0 }).max(10.0);
    text(
        &layer,
        Pos2::new(sx, sy),
        &entry.sub,
        sub_font,
        SUBTLE,
        sub_max,
        Align2::LEFT_CENTER,
    );
    if live {
        icons::paint(
            &layer,
            Rect::from_center_size(Pos2::new(tip_rect.right() - 18.0, sy), Vec2::splat(14.0)),
            Icon::PlayingSpeaker,
            ACCENT,
        );
    }
}

fn draw_header(app: &mut App, ui: &mut Ui, rect: Rect, can_expand: bool, panel_hovered: bool) {
    let content = Rect::from_min_max(rect.min + Vec2::new(16.0, 16.0), rect.max - Vec2::new(16.0, 8.0));
    let cy = content.center().y;
    let right_controls_w = if can_expand { 43.0 } else { 0.0 };
    let create_right = content.right() - right_controls_w;
    let create_rect = Rect::from_center_size(Pos2::new(create_right - 17.5, cy), Vec2::splat(35.0));

    if let Some(folder_id) = app.opened_folder {
        let back_rect = Rect::from_center_size(Pos2::new(content.left() + 16.0, cy), Vec2::splat(32.0));
        let back_resp = ui
            .interact(back_rect, ui.scope_id().with("folder-back"), Sense::click())
            .on_hover_text(app.t("Quay lại Thư viện", "Back to Your Library"));
        if back_resp.hovered() {
            ui.painter().circle_filled(back_rect.center(), 16.0, TINT_HOVER);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(back_rect.center(), Vec2::splat(16.0)),
            Icon::Back,
            if back_resp.hovered() { TEXT } else { SUBTLE },
        );
        focus_ring(ui, &back_resp, 16.0);
        if back_resp.clicked() {
            app.actions.push(Action::OpenFolder(None));
        }

        let title_rect = Rect::from_min_max(
            Pos2::new(back_rect.right() + 6.0, cy - 16.0),
            Pos2::new(create_rect.left() - 8.0, cy + 16.0),
        );
        let folder_name = app
            .data
            .folder(folder_id)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| app.t("Thư mục", "Folder").to_string());
        if app.rename_folder.as_ref().is_some_and(|r| r.id == folder_id) {
            draw_inline_folder_rename(app, ui, title_rect, folder_id);
        } else {
            let name_resp = ui
                .interact(title_rect, ui.scope_id().with("folder-title"), Sense::click())
                .on_hover_text(app.t("Nhấn để đổi tên thư mục", "Click to rename folder"));
            text(
                ui.painter(),
                title_rect.left_center(),
                &folder_name,
                bold(16.0),
                if name_resp.hovered() { TEXT } else { Color32::from_rgb(0xf2, 0xf2, 0xf2) },
                title_rect.width(),
                Align2::LEFT_CENTER,
            );
            if name_resp.clicked() {
                app.rename_folder = Some(RenameState {
                    id: folder_id,
                    name: folder_name,
                    focus: true,
                });
            }
        }
    } else {
        let title_rect = Rect::from_min_max(
            Pos2::new(content.left(), cy - 16.0),
            Pos2::new(create_rect.left() - 8.0, cy + 16.0),
        );
        let tip = app.t("Thu gọn Thư viện", "Collapse Your Library");
        let heading = ui
            .interact(title_rect, ui.scope_id().with("library-heading"), Sense::click())
            .on_hover_cursor(CursorIcon::PointingHand)
            .on_hover_text(tip);
        heading.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        let reveal = ui.ctx().animate_bool_with_time(
            ui.scope_id().with("lib-collapse-reveal"),
            panel_hovered || heading.has_focus(),
            0.12,
        );
        let slide = reveal * 26.0;
        if reveal > 0.01 {
            let icon_x = egui::lerp((content.left() - 26.0)..=(content.left() + 8.0), reveal);
            let mut clipped = ui.painter().clone();
            clipped.set_clip_rect(rect.intersect(ui.clip_rect()));
            let icon_tint = (if heading.hovered() { TEXT } else { SUBTLE }).linear_multiply(reveal);
            icons::paint(
                &clipped,
                Rect::from_center_size(Pos2::new(icon_x, cy), Vec2::splat(17.0)),
                Icon::LibraryCollapse,
                icon_tint,
            );
        }
        text(
            ui.painter(),
            Pos2::new(content.left() + slide, cy),
            app.t("Thư viện", "Your Library"),
            bold(16.0),
            if heading.hovered() { TEXT } else { Color32::from_rgb(0xf2, 0xf2, 0xf2) },
            (title_rect.width() - slide).max(24.0),
            Align2::LEFT_CENTER,
        );
        focus_ring(ui, &heading, RADIUS_BASE);
        if heading.clicked() {
            app.actions.push(Action::ToggleLeft);
        }
    }

    // + Create circular button opening popover menu
    let create_resp = ui
        .interact(create_rect, ui.scope_id().with("create-pill"), Sense::click())
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_text(app.t("Tạo playlist hoặc thư mục", "Create playlist or folder"));
    let create_bg = if create_resp.is_pointer_button_down_on() {
        ELEVATED_PRESS
    } else if create_resp.hovered() {
        HOVER
    } else {
        ELEVATED
    };
    ui.painter().circle_filled(create_rect.center(), 17.5, create_bg);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(create_rect.center(), Vec2::splat(16.0)),
        Icon::Plus,
        if create_resp.hovered() { TEXT } else { SUBTLE },
    );
    let create_tip = app.t("Tạo playlist hoặc thư mục", "Create playlist or folder");
    create_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, create_tip));
    focus_ring(ui, &create_resp, 17.5);
    show_create_menu(app, ui, &create_resp);

    if can_expand {
        let expand_rect = Rect::from_center_size(Pos2::new(content.right() - 17.5, cy), Vec2::splat(35.0));
        let is_expanded_state = app.data.settings.left_width >= LEFT_MAX;
        let (icon, tip) = if is_expanded_state {
            (Icon::Shrink, app.t("Thu hẹp Thư viện", "Reduce Your Library"))
        } else {
            (Icon::Expand, app.t("Mở rộng Thư viện", "Enlarge Your Library"))
        };
        let resp = ui
            .interact(expand_rect, ui.scope_id().with("lib-expand"), Sense::click())
            .on_hover_text(tip);
        if resp.hovered() {
            ui.painter().circle_filled(expand_rect.center(), 17.5, TINT_HOVER);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(expand_rect.center(), Vec2::splat(16.0)),
            icon,
            if resp.hovered() { TEXT } else { SUBTLE },
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
        focus_ring(ui, &resp, 17.5);
        if resp.clicked() {
            app.actions.push(Action::ExpandLeft);
        }
    }
}

fn show_create_menu(app: &mut App, ui: &Ui, anchor: &Response) {
    let mut create_playlist = false;
    let mut create_folder = false;
    let t_pl_title = app.t("Tạo danh sách phát mới", "Create a new playlist");
    let t_pl_sub = app.t("Tạo danh sách phát gồm bài hát", "Create a playlist with songs");
    let t_fd_title = app.t("Tạo thư mục danh sách phát", "Create a playlist folder");
    let t_fd_sub = app.t("Sắp xếp danh sách phát của bạn", "Organize your playlists");
    let frame = popup_frame(ui.style());

    egui::Popup::menu(anchor).frame(frame).show(|ui| {
        ui.set_min_width(248.0);
        if create_menu_item(ui, "create-pl", Icon::Plus, t_pl_title, t_pl_sub).clicked() {
            create_playlist = true;
            ui.close();
        }
        ui.add_space(2.0);
        if create_menu_item(ui, "create-folder", Icon::Folder, t_fd_title, t_fd_sub).clicked() {
            create_folder = true;
            ui.close();
        }
    });

    if (create_playlist || create_folder)
        && matches!(app.lib_filter, LibraryFilter::Albums | LibraryFilter::Artists)
    {
        app.actions.push(Action::SetLibraryFilter(LibraryFilter::All));
    }
    if create_playlist {
        app.actions.push(Action::CreatePlaylist(None));
    }
    if create_folder {
        app.actions.push(Action::CreateFolder(None));
    }
}

fn create_menu_item(ui: &mut Ui, id: &str, icon: Icon, title: &str, sub: &str) -> Response {
    let w = ui.available_width().max(232.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, RADIUS_BASE, TINT_HOVER);
    }
    let badge = Rect::from_center_size(Pos2::new(rect.left() + 24.0, rect.center().y), Vec2::splat(36.0));
    ui.painter().circle_filled(badge.center(), 18.0, TAB_ACTIVE);
    icons::paint(
        ui.painter(),
        Rect::from_center_size(badge.center(), Vec2::splat(16.0)),
        icon,
        TEXT,
    );
    text(
        ui.painter(),
        Pos2::new(badge.right() + 10.0, rect.center().y - 9.0),
        title,
        semi(14.0),
        TEXT,
        w - 64.0,
        Align2::LEFT_CENTER,
    );
    text(
        ui.painter(),
        Pos2::new(badge.right() + 10.0, rect.center().y + 10.0),
        sub,
        body(12.0),
        SUBTLE,
        w - 64.0,
        Align2::LEFT_CENTER,
    );
    let _ = id;
    resp
}

fn draw_wide_toolbar(app: &mut App, ui: &mut Ui, rect: Rect) {
    let tools_w = if app.lib_search { 248.0 } else { 188.0 };
    let chips_rect = Rect::from_min_max(rect.min, Pos2::new((rect.right() - tools_w).max(rect.left() + 120.0), rect.bottom()));
    let tools_rect = Rect::from_min_max(Pos2::new(chips_rect.right(), rect.top()), rect.max);
    draw_filter_chips(app, ui, chips_rect);
    draw_search_and_sort(app, ui, tools_rect, true);
}

fn draw_filter_chips(app: &mut App, ui: &mut Ui, rect: Rect) {
    let inner = Rect::from_min_max(
        Pos2::new(rect.left() + 16.0, rect.top() + 8.0),
        Pos2::new(rect.right() - 16.0, rect.bottom() - 8.0),
    );
    let mut chip_ui = ui.new_child(UiBuilder::new().id_salt("lib-chips-area").max_rect(inner));
    chip_ui.set_clip_rect(inner.intersect(ui.clip_rect()));

    ScrollArea::horizontal()
        .id_salt("lib-chips-scroll")
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
        .show(&mut chip_ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let has_active = app.lib_filter != LibraryFilter::All
                    || app.lib_creator_filter != LibraryCreatorFilter::All;
                if has_active {
                    let (x_rect, x_resp) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
                    let bg = if x_resp.hovered() { TINT_HOVER } else { TINT_BASE };
                    ui.painter().circle_filled(x_rect.center(), 16.0, bg);
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(x_rect.center(), Vec2::splat(14.0)),
                        Icon::Close,
                        TEXT,
                    );
                    focus_ring(ui, &x_resp, 16.0);
                    if x_resp
                        .on_hover_text(app.t("Xóa bộ lọc", "Clear filters"))
                        .clicked()
                    {
                        app.actions.push(Action::SetLibraryFilter(LibraryFilter::All));
                    }
                }

                if app.lib_filter == LibraryFilter::Playlists {
                    let pl_label = app.t("Playlist", "Playlists");
                    let pl_w = text_width(ui, pl_label, body(14.0)) + 24.0;
                    let (pl_rect, _) = ui.allocate_exact_size(Vec2::new(pl_w, 32.0), Sense::hover());
                    let sub_active = app.lib_creator_filter != LibraryCreatorFilter::All;
                    let pl_resp = chip_pill(ui, pl_rect, ("chip", "pl"), pl_label, !sub_active, sub_active);
                    if pl_resp.clicked() {
                        app.actions.push(Action::SetLibraryFilter(LibraryFilter::All));
                    }

                    for (creator, label) in [
                        (LibraryCreatorFilter::ByYou, app.t("Của bạn", "By you")),
                        (LibraryCreatorFilter::BySpotify, app.t("Của Spotify", "By Spotify")),
                    ] {
                        let w = text_width(ui, label, body(14.0)) + 24.0;
                        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::hover());
                        let active = app.lib_creator_filter == creator;
                        if chip_pill(ui, r, ("subchip", label), label, active, false).clicked() {
                            let next = if active { LibraryCreatorFilter::All } else { creator };
                            app.actions.push(Action::SetLibraryCreatorFilter(next));
                        }
                    }
                } else {
                    for (filter, label) in [
                        (LibraryFilter::Playlists, app.t("Playlist", "Playlists")),
                        (LibraryFilter::Albums, "Album"),
                        (LibraryFilter::Artists, app.t("Nghệ sĩ", "Artists")),
                    ] {
                        let w = text_width(ui, label, body(14.0)) + 24.0;
                        let (r, _) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::hover());
                        let active = app.lib_filter == filter;
                        if chip_pill(ui, r, ("chip", label), label, active, false).clicked() {
                            let next = if active { LibraryFilter::All } else { filter };
                            app.actions.push(Action::SetLibraryFilter(next));
                        }
                    }
                }
            });
        });
}

fn chip_pill(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    label: &str,
    active_white: bool,
    parent_tinted: bool,
) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
    let (bg, fg) = if active_white {
        (TEXT, Color32::BLACK)
    } else if parent_tinted {
        (if resp.hovered() { Color32::from_rgb(0x45, 0x45, 0x45) } else { TAB_ACTIVE }, TEXT)
    } else if resp.hovered() {
        (TINT_HOVER, TEXT)
    } else {
        (TINT_BASE, TEXT)
    };
    ui.painter().rect_filled(rect, rect.height() * 0.5, bg);
    text(
        ui.painter(),
        rect.center(),
        label,
        body(14.0),
        fg,
        rect.width() - 12.0,
        Align2::CENTER_CENTER,
    );
    focus_ring(ui, &resp, 16.0);
    resp
}

fn draw_search_and_sort(app: &mut App, ui: &mut Ui, rect: Rect, is_wide: bool) {
    let cy = rect.center().y;
    let hint = app.t("Tìm trong Thư viện", "Search in Your Library");

    let sort_by = app.data.settings.effective_library_sort();
    let sort_label = sort_name(app, sort_by);
    let view_mode = app.data.settings.library_view;
    let view_icon = match view_mode {
        LibraryViewMode::CompactList => Icon::CompactList,
        LibraryViewMode::DefaultList => Icon::Sort,
        LibraryViewMode::CompactGrid | LibraryViewMode::DefaultGrid => Icon::Grid,
    };

    let hide_sort_label = app.lib_search && !is_wide;
    let sort_w = if hide_sort_label {
        32.0
    } else {
        text_width(ui, sort_label, body(13.0)) + 28.0
    };
    let sort_rect = Rect::from_min_size(
        Pos2::new(rect.right() - sort_w - 14.0, cy - 14.0),
        Vec2::new(sort_w, 28.0),
    );
    let sort_resp = ui
        .interact(sort_rect, ui.scope_id().with("lib-sort"), Sense::click())
        .on_hover_text(format!("{}: {}", sort_label, view_name(app, view_mode)));
    sort_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, sort_label));
    let tint = if sort_resp.hovered() { TEXT } else { SUBTLE };
    if !hide_sort_label {
        text(
            ui.painter(),
            sort_rect.left_center(),
            sort_label,
            body(13.0),
            tint,
            (sort_w - 22.0).max(1.0),
            Align2::LEFT_CENTER,
        );
    }
    let icon_pos = if hide_sort_label {
        sort_rect.center()
    } else {
        Pos2::new(sort_rect.right() - 8.0, sort_rect.center().y)
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(icon_pos, Vec2::splat(15.0)),
        view_icon,
        tint,
    );
    show_sort_and_view_menu(app, ui, &sort_resp, sort_by, view_mode);

    if !app.lib_search {
        let glass = Rect::from_center_size(Pos2::new(rect.left() + 26.0, cy), Vec2::splat(32.0));
        let glass_resp = ui
            .interact(glass, ui.scope_id().with("lib-search-btn"), Sense::click())
            .on_hover_text(hint);
        if glass_resp.hovered() {
            ui.painter().circle_filled(glass.center(), 16.0, TINT_HOVER);
        }
        icons::paint(
            ui.painter(),
            Rect::from_center_size(glass.center(), Vec2::splat(16.0)),
            Icon::Search,
            if glass_resp.hovered() { TEXT } else { SUBTLE },
        );
        if glass_resp.clicked() {
            app.lib_search = true;
            app.focus_lib_search = true;
        }
    } else {
        let pill_left = rect.left() + 12.0;
        let pill_right = (sort_rect.left() - 8.0).max(pill_left + 80.0);
        let pill_rect = Rect::from_min_max(Pos2::new(pill_left, cy - 16.0), Pos2::new(pill_right, cy + 16.0));
        ui.painter().rect_filled(pill_rect, RADIUS_BASE, TAB_ACTIVE);
        ui.painter().rect_stroke(pill_rect, RADIUS_BASE, Stroke::new(1.0, TINT_BASE), egui::StrokeKind::Inside);

        let glass_rect = Rect::from_center_size(Pos2::new(pill_rect.left() + 16.0, cy), Vec2::splat(28.0));
        let glass_resp = ui
            .interact(glass_rect, ui.scope_id().with("lib-search-collapse"), Sense::click())
            .on_hover_text(hint);
        icons::paint(
            ui.painter(),
            Rect::from_center_size(glass_rect.center(), Vec2::splat(15.0)),
            Icon::Search,
            SUBTLE,
        );
        if glass_resp.clicked() && app.lib_query.is_empty() {
            app.lib_search = false;
        }

        let clear_space = if app.lib_query.is_empty() { 8.0 } else { 28.0 };
        let edit_rect = Rect::from_min_max(
            Pos2::new(pill_rect.left() + 30.0, cy - 10.0),
            Pos2::new((pill_rect.right() - clear_space).max(pill_rect.left() + 40.0), cy + 10.0),
        );
        let mut child = ui.new_child(UiBuilder::new().id_salt("lib-filter").max_rect(edit_rect));
        let edit = child.add(
            egui::TextEdit::singleline(&mut app.lib_query)
                .frame(egui::Frame::NONE)
                .font(body(13.0))
                .hint_text(RichText::new(hint).color(SUBTLE).font(body(13.0)))
                .desired_width(edit_rect.width()),
        );
        if app.focus_lib_search {
            app.focus_lib_search = false;
            edit.request_focus();
        }

        if !app.lib_query.is_empty() {
            let x_rect = Rect::from_center_size(Pos2::new(pill_rect.right() - 14.0, cy), Vec2::splat(24.0));
            let x_resp = ui
                .interact(x_rect, ui.scope_id().with("lib-search-clear"), Sense::click())
                .on_hover_text(app.t("Xóa tìm kiếm", "Clear filter"));
            icons::paint(
                ui.painter(),
                Rect::from_center_size(x_rect.center(), Vec2::splat(13.0)),
                Icon::Close,
                if x_resp.hovered() { TEXT } else { SUBTLE },
            );
            if x_resp.clicked() {
                app.lib_query.clear();
                app.focus_lib_search = true;
            }
        } else if edit.lost_focus() && !glass_resp.clicked() {
            app.lib_search = false;
        }
    }
}

fn show_sort_and_view_menu(
    app: &mut App,
    ui: &Ui,
    anchor: &Response,
    current_sort: LibrarySortBy,
    current_view: LibraryViewMode,
) {
    const MENU_W: f32 = 160.0;
    let mut next_sort = None;
    let mut next_view = None;
    let frame = menu_frame(ui.style());
    let t_sort_by = app.t("Sắp xếp theo", "Sort by");
    let t_view_as = app.t("Xem dưới dạng", "View as");
    let sort_options = [
        (LibrarySortBy::Recents, app.t("Gần đây", "Recents")),
        (LibrarySortBy::RecentlyAdded, app.t("Mới thêm gần đây", "Recently added")),
        (LibrarySortBy::Alphabetical, app.t("Thứ tự chữ cái", "Alphabetical")),
        (LibrarySortBy::Creator, app.t("Người tạo", "Creator")),
    ];
    let view_options = [
        (LibraryViewMode::CompactList, Icon::CompactList, app.t("Danh sách thu gọn", "Compact list")),
        (LibraryViewMode::DefaultList, Icon::Sort, app.t("Danh sách mặc định", "Default list")),
        (LibraryViewMode::CompactGrid, Icon::Grid, app.t("Lưới thu gọn", "Compact grid")),
        (LibraryViewMode::DefaultGrid, Icon::Grid, app.t("Lưới mặc định", "Default grid")),
    ];

    egui::Popup::menu(anchor)
        .frame(frame)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.set_min_width(MENU_W);
            ui.set_max_width(MENU_W);
            ui.add_space(2.0);
            ui.label(RichText::new(t_sort_by).font(semi(11.5)).color(SUBTLE));
            ui.add_space(2.0);
            for (sort, label) in sort_options {
                let active = sort == current_sort;
                let (r, resp) = ui.allocate_exact_size(Vec2::new(MENU_W, 27.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(r, RADIUS_BASE, TINT_HOVER);
                }
                text(
                    ui.painter(),
                    Pos2::new(r.left() + 8.0, r.center().y),
                    label,
                    body(13.0),
                    if active { ACCENT } else { TEXT },
                    MENU_W - 32.0,
                    Align2::LEFT_CENTER,
                );
                if active {
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(Pos2::new(r.right() - 12.0, r.center().y), Vec2::splat(13.0)),
                        Icon::Checkmark,
                        ACCENT,
                    );
                }
                if resp.clicked() {
                    next_sort = Some(sort);
                }
            }

            ui.add_space(4.0);
            let (div_r, _) = ui.allocate_exact_size(Vec2::new(MENU_W, 1.0), Sense::hover());
            ui.painter().rect_filled(div_r, 0.0, HAIRLINE);
            ui.add_space(4.0);

            ui.label(RichText::new(t_view_as).font(semi(11.5)).color(SUBTLE));
            ui.add_space(4.0);
            let (tray_rect, _) = ui.allocate_exact_size(Vec2::new(MENU_W, 32.0), Sense::hover());
            ui.painter().rect_filled(tray_rect, RADIUS_CARD, CARD);
            let cell_w = (tray_rect.width() - 6.0) / 4.0;
            for (i, (mode, icon, tip)) in view_options.into_iter().enumerate() {
                let cell = Rect::from_min_size(
                    Pos2::new(tray_rect.left() + 3.0 + i as f32 * cell_w, tray_rect.top() + 3.0),
                    Vec2::new(cell_w, 26.0),
                );
                let resp = ui
                    .interact(cell, ui.scope_id().with(("view-seg", i)), Sense::click())
                    .on_hover_text(tip);
                let active = mode == current_view;
                if active {
                    ui.painter().rect_filled(cell, RADIUS_LARGER, TAB_ACTIVE);
                } else if resp.hovered() {
                    ui.painter().rect_filled(cell, RADIUS_LARGER, TINT_BASE);
                }
                let tint = if active {
                    ACCENT
                } else if resp.hovered() {
                    TEXT
                } else {
                    SUBTLE
                };
                if mode == LibraryViewMode::CompactGrid {
                    paint_compact_grid_icon(ui.painter(), cell.center(), tint);
                } else {
                    let glyph_sz = if mode == LibraryViewMode::DefaultGrid { 16.0 } else { 15.0 };
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(cell.center(), Vec2::splat(glyph_sz)),
                        icon,
                        tint,
                    );
                }
                if resp.clicked() {
                    next_view = Some(mode);
                }
            }
        });

    if let Some(sort) = next_sort {
        app.actions.push(Action::SetLibrarySort(sort));
    }
    if let Some(view) = next_view {
        app.actions.push(Action::SetLibraryView(view));
    }
}

fn paint_compact_grid_icon(painter: &egui::Painter, center: Pos2, color: Color32) {
    let cell = 3.0;
    let gap = 2.0;
    let total = cell * 3.0 + gap * 2.0;
    let origin = center - Vec2::splat(total * 0.5);
    for row in 0..3 {
        for col in 0..3 {
            let pos = origin + Vec2::new(col as f32 * (cell + gap), row as f32 * (cell + gap));
            painter.rect_filled(Rect::from_min_size(pos, Vec2::splat(cell)), 0.8, color);
        }
    }
}

fn draw_table_header(app: &App, ui: &mut Ui, rect: Rect) {
    let title_x = rect.left() + 16.0;
    let added_x = rect.right() - 180.0;
    let played_x = rect.right() - 16.0;
    let cy = rect.center().y;
    text(
        ui.painter(),
        Pos2::new(title_x, cy),
        app.t("Tiêu đề", "Title"),
        body(12.0),
        SUBTLE,
        (added_x - title_x - 12.0).max(20.0),
        Align2::LEFT_CENTER,
    );
    text(
        ui.painter(),
        Pos2::new(added_x, cy),
        app.t("Ngày thêm", "Date added"),
        body(12.0),
        SUBTLE,
        96.0,
        Align2::LEFT_CENTER,
    );
    text(
        ui.painter(),
        Pos2::new(played_x, cy),
        app.t("Đã phát", "Played"),
        body(12.0),
        SUBTLE,
        72.0,
        Align2::RIGHT_CENTER,
    );
    ui.painter().hline(
        (rect.left() + 12.0)..=(rect.right() - 12.0),
        rect.bottom() - 1.0,
        Stroke::new(1.0, HAIRLINE),
    );
}

fn draw_empty_state(app: &App, ui: &mut Ui) {
    ui.add_space(32.0);
    let msg = if !app.lib_query.trim().is_empty() || app.lib_filter != LibraryFilter::All {
        app.t("Không tìm thấy kết quả phù hợp", "No matching items found")
    } else if app.opened_folder.is_some() {
        app.t("Thư mục trống", "This folder is empty")
    } else {
        app.t("Thư viện của bạn đang trống", "Your library is empty")
    };
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(Vec2::new(w, 40.0), Sense::hover());
    text(ui.painter(), r.center(), msg, body(13.5), SUBTLE, w - 24.0, Align2::CENTER_CENTER);
}

fn draw_list_rows(
    app: &mut App,
    ui: &mut Ui,
    entries: &[Entry],
    row_h: f32,
    compact: bool,
    is_wide: bool,
) {
    let w = ui.available_width();
    for (idx, entry) in entries.iter().enumerate() {
        let (slot, _) = ui.allocate_exact_size(Vec2::new(w, row_h), Sense::hover());
        let r = Rect::from_min_size(
            Pos2::new(slot.left() + 8.0, slot.top()),
            Vec2::new((w - 16.0).max(40.0), row_h - 2.0),
        );
        draw_list_row(app, ui, r, entry, idx, compact, is_wide);
    }
}

fn draw_list_row(
    app: &mut App,
    ui: &mut Ui,
    r: Rect,
    entry: &Entry,
    idx: usize,
    compact: bool,
    is_wide: bool,
) {
    let resp = ui.interact(r, ui.scope_id().with(("lib-row", idx)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &entry.title));
    let selected = is_entry_selected(app, entry);
    let live = is_entry_playing(app, entry);
    let hovered = resp.hovered() || ui.rect_contains_pointer(r);

    if selected {
        ui.painter().rect_filled(r, RADIUS_LARGER, CARD_HOVER);
    } else if hovered {
        ui.painter().rect_filled(r, RADIUS_LARGER, hover_white(18));
    }

    let is_renaming_folder = matches!(
        &entry.kind,
        EntryKind::Folder { id, .. }
            if app.rename_folder.as_ref().is_some_and(|st| st.id == *id)
    );

    let right_limit = if is_wide {
        r.right() - 188.0
    } else {
        r.right() - if live { 30.0 } else { 8.0 }
    };

    let mut play_btn_resp: Option<Response> = None;
    let text_left = if compact {
        let mut x = r.left() + 8.0;
        if entry.pinned {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(Pos2::new(x + 6.0, r.center().y), Vec2::splat(12.0)),
                Icon::Pin,
                ACCENT,
            );
            x += 18.0;
        } else if matches!(entry.kind, EntryKind::Folder { .. }) {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(Pos2::new(x + 7.0, r.center().y), Vec2::splat(14.0)),
                Icon::Folder,
                SUBTLE,
            );
            x += 20.0;
        }
        x
    } else {
        let cover_rect = Rect::from_center_size(Pos2::new(r.left() + 32.0, r.center().y), Vec2::splat(48.0));
        paint_entry_cover(app, ui, cover_rect, entry, RADIUS_BASE);
        if hovered && entry.is_playable() {
            let is_circle = matches!(&entry.kind, EntryKind::Item(item) if item.circle);
            let scrim_radius = if is_circle { cover_rect.width() * 0.5 } else { RADIUS_BASE };
            ui.painter()
                .rect_filled(cover_rect, scrim_radius, Color32::from_black_alpha(135));
            let play_icon = if live { Icon::Pause } else { Icon::Play };
            let p_resp = ui
                .interact(cover_rect, ui.scope_id().with(("lib-cover-play", idx)), Sense::click())
                .on_hover_text(if live {
                    app.t("Tạm dừng", "Pause")
                } else {
                    app.t("Phát", "Play")
                });
            icons::paint(
                ui.painter(),
                Rect::from_center_size(cover_rect.center(), Vec2::splat(20.0)),
                play_icon,
                TEXT,
            );
            play_btn_resp = Some(p_resp);
        }
        cover_rect.right() + 12.0
    };

    if is_renaming_folder {
        if let EntryKind::Folder { id, .. } = entry.kind {
            let edit_rect = Rect::from_min_max(
                Pos2::new(text_left, r.center().y - 13.0),
                Pos2::new((right_limit - 6.0).max(text_left + 80.0), r.center().y + 13.0),
            );
            draw_inline_folder_rename(app, ui, edit_rect, id);
            return;
        }
    }

    let title_color = if live { ACCENT } else { TEXT };
    let text_max_w = (right_limit - text_left).max(20.0);

    if compact {
        let title_font = body(13.5);
        let title_w = text_width(ui, &entry.title, title_font.clone()).min(text_max_w * 0.68);
        text(
            ui.painter(),
            Pos2::new(text_left, r.center().y),
            &entry.title,
            title_font,
            title_color,
            title_w,
            Align2::LEFT_CENTER,
        );
        let tag_x = text_left + title_w + 6.0;
        let tag_rem = (right_limit - tag_x).max(0.0);
        if tag_rem > 24.0 {
            let tag = compact_tag(app, entry);
            text(
                ui.painter(),
                Pos2::new(tag_x, r.center().y),
                &format!("• {tag}"),
                body(12.0),
                SUBTLE,
                tag_rem,
                Align2::LEFT_CENTER,
            );
        }
    } else {
        // Spotify `.e-101040-legacy-list-row-title` (16px Regular) + `-detail` (14px Regular).
        text(
            ui.painter(),
            Pos2::new(text_left, r.center().y - 11.0),
            &entry.title,
            body_medium(),
            title_color,
            text_max_w,
            Align2::LEFT_CENTER,
        );
        let mut sx = text_left;
        let sy = r.center().y + 11.0;
        if entry.pinned {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(Pos2::new(sx + 6.0, sy), Vec2::splat(12.0)),
                Icon::Pin,
                ACCENT,
            );
            sx += 16.0;
        }
        text(
            ui.painter(),
            Pos2::new(sx, sy),
            &entry.sub,
            body_small(),
            SUBTLE,
            (right_limit - sx).max(12.0),
            Align2::LEFT_CENTER,
        );
    }

    if is_wide {
        let added_str = format_added_date(entry.added);
        text(
            ui.painter(),
            Pos2::new(r.right() - 172.0, r.center().y),
            &added_str,
            body(12.5),
            SUBTLE,
            96.0,
            Align2::LEFT_CENTER,
        );
        if live {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(Pos2::new(r.right() - 20.0, r.center().y), Vec2::splat(16.0)),
                Icon::PlayingSpeaker,
                ACCENT,
            );
        } else if entry.recent_rank < usize::MAX {
            let played_label = if entry.recent_rank == 0 {
                app.t("Đang ghim", "Pinned")
            } else {
                app.t("Gần đây", "Recent")
            };
            text(
                ui.painter(),
                Pos2::new(r.right() - 12.0, r.center().y),
                played_label,
                body(12.0),
                MUTED,
                64.0,
                Align2::RIGHT_CENTER,
            );
        }
    } else if live {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(Pos2::new(r.right() - 16.0, r.center().y), Vec2::splat(16.0)),
            Icon::PlayingSpeaker,
            ACCENT,
        );
    }

    handle_entry_interactions(app, ui, &resp, entry, play_btn_resp);
}

fn draw_grid(app: &mut App, ui: &mut Ui, entries: &[Entry], compact_grid: bool) {
    let w = (ui.available_width() - 16.0).max(120.0);
    let gap = 8.0;
    let target_card_w = if compact_grid { 108.0 } else { 132.0 };
    let cols = ((w + gap) / (target_card_w + gap)).floor().max(2.0) as usize;
    let card_w = ((w - gap * (cols as f32 - 1.0)) / cols as f32).max(64.0);
    let pad = if compact_grid { 4.0 } else { 8.0 };
    let cover_side = (card_w - pad * 2.0).max(40.0);
    let card_h = if compact_grid {
        cover_side + pad * 2.0
    } else {
        cover_side + pad * 2.0 + 44.0
    };

    for (row_idx, chunk) in entries.chunks(cols).enumerate() {
        let (row_rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), card_h + gap), Sense::hover());
        for (col_idx, entry) in chunk.iter().enumerate() {
            let idx = row_idx * cols + col_idx;
            let card_rect = Rect::from_min_size(
                Pos2::new(row_rect.left() + 8.0 + col_idx as f32 * (card_w + gap), row_rect.top()),
                Vec2::new(card_w, card_h),
            );
            draw_grid_card(app, ui, card_rect, cover_side, pad, entry, idx, compact_grid);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_grid_card(
    app: &mut App,
    ui: &mut Ui,
    card_rect: Rect,
    cover_side: f32,
    pad: f32,
    entry: &Entry,
    idx: usize,
    compact_grid: bool,
) {
    let resp = ui.interact(card_rect, ui.scope_id().with(("lib-grid-card", idx)), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &entry.title));
    let selected = is_entry_selected(app, entry);
    let live = is_entry_playing(app, entry);
    let hovered = resp.hovered() || ui.rect_contains_pointer(card_rect);

    if selected {
        ui.painter().rect_filled(card_rect, RADIUS_CARD, CARD_HOVER);
    } else if hovered {
        ui.painter().rect_filled(card_rect, RADIUS_CARD, ELEVATED);
    }

    let cover_rect = Rect::from_min_size(
        Pos2::new(card_rect.left() + pad, card_rect.top() + pad),
        Vec2::splat(cover_side),
    );
    paint_entry_cover(app, ui, cover_rect, entry, RADIUS_LARGER);

    if compact_grid && live {
        let is_circle = matches!(&entry.kind, EntryKind::Item(item) if item.circle);
        let stroke = Stroke::new(2.0, ACCENT);
        if is_circle {
            ui.painter().circle_stroke(cover_rect.center(), cover_rect.width() * 0.5, stroke);
        } else {
            ui.painter().rect_stroke(cover_rect, RADIUS_LARGER, stroke, egui::StrokeKind::Inside);
        }
    }

    let mut play_resp = None;
    if entry.is_playable() {
        let btn_size = (cover_side * 0.34).clamp(28.0, 42.0);
        let btn_rect = Rect::from_min_size(
            Pos2::new(cover_rect.right() - btn_size - 6.0, cover_rect.bottom() - btn_size - 6.0),
            Vec2::splat(btn_size),
        );
        let p_resp = hover_play_button(
            ui,
            btn_rect,
            ("lib-grid-play", idx),
            live,
            hovered,
            if live { app.t("Tạm dừng", "Pause") } else { app.t("Phát", "Play") },
        );
        if hovered {
            play_resp = Some(p_resp);
        }
    }

    if compact_grid {
        if hovered {
            paint_rail_tooltip(ui, card_rect, entry, live);
        }
    } else {
        let is_renaming_folder = matches!(
            &entry.kind,
            EntryKind::Folder { id, .. }
                if app.rename_folder.as_ref().is_some_and(|st| st.id == *id)
        );
        if is_renaming_folder {
            if let EntryKind::Folder { id, .. } = entry.kind {
                let edit_rect = Rect::from_min_size(
                    Pos2::new(cover_rect.left(), cover_rect.bottom() + 6.0),
                    Vec2::new(cover_side, 24.0),
                );
                draw_inline_folder_rename(app, ui, edit_rect, id);
                return;
            }
        }
        let title_y = cover_rect.bottom() + 14.0;
        let sub_y = cover_rect.bottom() + 32.0;
        let text_max = cover_side - if live { 18.0 } else { 0.0 };
        text(
            ui.painter(),
            Pos2::new(cover_rect.left(), title_y),
            &entry.title,
            semi(13.5),
            if live { ACCENT } else { TEXT },
            text_max.max(20.0),
            Align2::LEFT_CENTER,
        );
        if live {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(Pos2::new(cover_rect.right() - 7.0, title_y), Vec2::splat(13.0)),
                Icon::PlayingSpeaker,
                ACCENT,
            );
        }
        let mut sx = cover_rect.left();
        if entry.pinned {
            icons::paint(
                ui.painter(),
                Rect::from_center_size(Pos2::new(sx + 5.0, sub_y), Vec2::splat(11.0)),
                Icon::Pin,
                ACCENT,
            );
            sx += 14.0;
        }
        text(
            ui.painter(),
            Pos2::new(sx, sub_y),
            &entry.sub,
            body(12.0),
            SUBTLE,
            (cover_rect.right() - sx).max(16.0),
            Align2::LEFT_CENTER,
        );
    }

    handle_entry_interactions(app, ui, &resp, entry, play_resp);
}

fn draw_inline_folder_rename(app: &mut App, ui: &mut Ui, rect: Rect, folder_id: u64) {
    let Some(state) = app.rename_folder.as_mut().filter(|r| r.id == folder_id) else {
        return;
    };
    ui.painter().rect_filled(rect, RADIUS_BASE, HOVER);
    ui.painter().rect_stroke(rect, RADIUS_BASE, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
    let inner = rect.shrink2(Vec2::new(6.0, 2.0));
    let mut edit_ui = ui.new_child(UiBuilder::new().id_salt(("folder-rename", folder_id)).max_rect(inner));
    let resp = edit_ui.add(
        egui::TextEdit::singleline(&mut state.name)
            .frame(egui::Frame::NONE)
            .font(body(13.5))
            .desired_width(inner.width()),
    );
    if state.focus {
        state.focus = false;
        resp.request_focus();
    }
    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
    if enter || resp.lost_focus() {
        let name = state.name.clone();
        app.actions.push(Action::RenameFolder(folder_id, name));
    }
}

fn handle_entry_interactions(
    app: &mut App,
    ui: &Ui,
    resp: &Response,
    entry: &Entry,
    play_btn_resp: Option<Response>,
) {
    let play_clicked = play_btn_resp.as_ref().is_some_and(Response::clicked);
    if play_clicked {
        if let EntryKind::Item(item) = &entry.kind {
            app.actions.push(Action::PlayItem(item.kind.clone()));
        }
    } else if resp.clicked() {
        match &entry.kind {
            EntryKind::Folder { id, .. } => app.actions.push(Action::OpenFolder(Some(*id))),
            EntryKind::Item(item) => app.actions.push(Action::Navigate(item.route.clone())),
        }
    }
    if resp.double_clicked() && entry.is_playable() {
        if let EntryKind::Item(item) = &entry.kind {
            app.actions.push(Action::PlayItem(item.kind.clone()));
        }
    }

    draw_context_menu(app, ui, resp, entry);
}

fn draw_context_menu(app: &mut App, ui: &Ui, resp: &Response, entry: &Entry) {
    let mut pending_rename_folder: Option<(u64, String)> = None;
    let mut unfollow_ytm: Option<String> = None;
    let mut copy_text: Option<String> = None;
    let mut open_and_create_playlist: Option<u64> = None;

    let t_play = app.t("Phát", "Play");
    let t_pause = app.t("Tạm dừng", "Pause");
    let t_queue = app.t("Thêm vào danh sách chờ", "Add to queue");
    let t_pin = app.t("Ghim", "Pin");
    let t_unpin = app.t("Bỏ ghim", "Unpin");
    let t_edit = app.t("Chỉnh sửa thông tin", "Edit details");
    let t_create_folder = app.t("Tạo thư mục", "Create folder");
    let t_move_folder = app.t("Chuyển vào thư mục", "Move to folder");
    let t_remove_folder = app.t("Xóa khỏi thư mục", "Remove from folder");
    let t_rename = app.t("Đổi tên", "Rename");
    let t_delete = app.t("Xóa", "Delete");
    let t_unfollow = app.t("Xóa khỏi Thư viện", "Remove from Your Library");
    let t_copy_link = app.t("Sao chép liên kết", "Copy link");
    let t_create_pl = app.t("Tạo danh sách phát mới", "Create playlist");

    let folders: Vec<(u64, String)> = app
        .data
        .folders
        .iter()
        .map(|f| (f.id, f.name.clone()))
        .collect();
    let live = is_entry_playing(app, entry);
    let tracks = match &entry.kind {
        EntryKind::Item(item) => app.tracks_of(&item.kind).2,
        EntryKind::Folder { .. } => Vec::new(),
    };

    let _ = ui;
    resp.context_menu(|ui| {
        ui.set_min_width(196.0);
        match &entry.kind {
            EntryKind::Folder { id, name, .. } => {
                let fid = *id;
                if ui.button(t_create_pl).clicked() {
                    open_and_create_playlist = Some(fid);
                    ui.close();
                }
                if ui.button(t_create_folder).clicked() {
                    app.actions.push(Action::CreateFolder(None));
                    ui.close();
                }
                ui.separator();
                let pin_label = if entry.pinned { t_unpin } else { t_pin };
                if ui.button(pin_label).clicked() {
                    app.actions.push(Action::ToggleFolderPin(fid));
                    ui.close();
                }
                if ui.button(t_rename).clicked() {
                    pending_rename_folder = Some((fid, name.clone()));
                    ui.close();
                }
                if ui.button(t_delete).clicked() {
                    app.actions.push(Action::RequestConfirm(ConfirmAction::DeleteFolder(fid)));
                    ui.close();
                }
            }
            EntryKind::Item(item) => {
                if entry.is_playable() {
                    if ui.button(if live { t_pause } else { t_play }).clicked() {
                        app.actions.push(Action::PlayItem(item.kind.clone()));
                        ui.close();
                    }
                    if !tracks.is_empty() && ui.button(t_queue).clicked() {
                        for tid in &tracks {
                            app.actions.push(Action::Enqueue(*tid));
                        }
                        ui.close();
                    }
                    ui.separator();
                }

                let pin_label = if entry.pinned { t_unpin } else { t_pin };
                if ui.button(pin_label).clicked() {
                    app.actions.push(Action::TogglePin(item.kind.clone()));
                    ui.close();
                }

                match &item.kind {
                    RecentItem::Playlist(pid) => {
                        let pid = *pid;
                        let current_folder = app.data.playlist(pid).and_then(|p| p.folder_id);
                        if ui.button(t_edit).clicked() {
                            app.actions.push(Action::BeginEditPlaylist(pid));
                            ui.close();
                        }
                        if ui.button(t_rename).clicked() {
                            app.actions.push(Action::BeginRename(pid));
                            ui.close();
                        }
                        if ui.button(t_create_folder).clicked() {
                            app.actions.push(Action::CreateFolder(None));
                            ui.close();
                        }
                        ui.menu_button(t_move_folder, |ui| {
                            if current_folder.is_some() && ui.button(t_remove_folder).clicked() {
                                app.actions.push(Action::MovePlaylistToFolder(pid, None));
                                ui.close();
                            }
                            for (fid, fname) in &folders {
                                let label = if current_folder == Some(*fid) {
                                    format!("✓ {fname}")
                                } else {
                                    fname.clone()
                                };
                                if ui.button(label).clicked() {
                                    app.actions.push(Action::MovePlaylistToFolder(pid, Some(*fid)));
                                    ui.close();
                                }
                            }
                            if !folders.is_empty() {
                                ui.separator();
                            }
                            if ui.button(t_create_folder).clicked() {
                                app.actions.push(Action::CreateFolder(None));
                                ui.close();
                            }
                        });
                        if ui.button(t_delete).clicked() {
                            app.actions.push(Action::RequestConfirm(ConfirmAction::DeletePlaylist(pid)));
                            ui.close();
                        }
                    }
                    RecentItem::FollowedYouTube(yt_id) => {
                        if ui.button(t_unfollow).clicked() {
                            unfollow_ytm = Some(yt_id.clone());
                            ui.close();
                        }
                    }
                    RecentItem::Liked | RecentItem::Album(_) | RecentItem::Artist(_) => {}
                }

                ui.separator();
                if ui.button(t_copy_link).clicked() {
                    copy_text = Some(entity_link(&item.kind));
                    ui.close();
                }
            }
        }
    });

    if let Some((fid, fname)) = pending_rename_folder {
        app.rename_folder = Some(RenameState {
            id: fid,
            name: fname,
            focus: true,
        });
    }
    if let Some(fid) = open_and_create_playlist {
        app.opened_folder = Some(fid);
        app.actions.push(Action::CreatePlaylist(None));
    }
    if let Some(yt_id) = unfollow_ytm {
        app.update_followed_ytm(&yt_id, None);
    }
    if let Some(link) = copy_text {
        ui.ctx().copy_text(link);
        app.toast = Some((
            app.t("Đã sao chép liên kết", "Link copied to clipboard").into(),
            Instant::now(),
        ));
    }
}

fn entity_link(kind: &RecentItem) -> String {
    match kind {
        RecentItem::Liked => "https://open.spotify.com/collection/tracks".to_string(),
        RecentItem::Playlist(id) => format!("https://open.spotify.com/playlist/{id:016x}"),
        RecentItem::Album(key) => format!("https://open.spotify.com/album/{}", url_slug(key)),
        RecentItem::Artist(name) => format!("https://open.spotify.com/artist/{}", url_slug(name)),
        RecentItem::FollowedYouTube(id) => format!("https://music.youtube.com/playlist?list={id}"),
    }
}

fn url_slug(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn collect_entries(app: &App) -> Vec<Entry> {
    let filter = app.lib_filter;
    let creator_filter = app.lib_creator_filter;
    let q = fold(app.lib_query.trim());
    let opened_folder = app.opened_folder;
    let you_label = app.t("Bạn", "You").to_string();
    let playlist_word = app.t("Danh sách phát", "Playlist");
    let folder_word = app.t("Thư mục", "Folder");

    let rank_of = |k: &RecentItem| -> usize {
        if *k == RecentItem::Liked {
            return 0;
        }
        app.data
            .recent
            .iter()
            .position(|r| r == k)
            .map_or(usize::MAX, |p| p + 1)
    };

    let mut entries: Vec<Entry> = Vec::new();

    // Folders: shown in All or Playlists when creator filter is All.
    let show_folders = opened_folder.is_some()
        || (matches!(filter, LibraryFilter::All | LibraryFilter::Playlists)
            && creator_filter == LibraryCreatorFilter::All);
    if show_folders {
        for folder in &app.data.folders {
            if folder.parent_id != opened_folder {
                continue;
            }
            let pl_count = app
                .data
                .playlists
                .iter()
                .filter(|p| p.folder_id == Some(folder.id))
                .count();
            let sub_count = app
                .data
                .folders
                .iter()
                .filter(|f| f.parent_id == Some(folder.id))
                .count();
            let count = pl_count + sub_count;
            let sub = format!(
                "{folder_word} • {count} {}",
                app.t("danh sách phát", "playlists")
            );
            entries.push(Entry {
                kind: EntryKind::Folder {
                    id: folder.id,
                    name: folder.name.clone(),
                    count,
                },
                title: folder.name.clone(),
                sub,
                pinned: folder.pinned,
                added: folder.created,
                recent_rank: usize::MAX,
                creator: you_label.clone(),
            });
        }
    }

    let mut kinds: Vec<RecentItem> = Vec::new();
    if let Some(fid) = opened_folder {
        for p in &app.data.playlists {
            if p.folder_id != Some(fid) {
                continue;
            }
            if matches_creator_filter(p.creator.as_deref(), creator_filter) {
                kinds.push(RecentItem::Playlist(p.id));
            }
        }
    } else {
        if matches!(filter, LibraryFilter::All | LibraryFilter::Playlists) {
            if creator_filter != LibraryCreatorFilter::BySpotify {
                kinds.push(RecentItem::Liked);
            }
            for p in &app.data.playlists {
                if p.folder_id.is_none() && matches_creator_filter(p.creator.as_deref(), creator_filter) {
                    kinds.push(RecentItem::Playlist(p.id));
                }
            }
            for f in &app.data.followed_ytm {
                if f.id.starts_with("album:") {
                    continue;
                }
                let include = match creator_filter {
                    LibraryCreatorFilter::All => true,
                    LibraryCreatorFilter::ByYou | LibraryCreatorFilter::BySpotify => false,
                };
                if include {
                    kinds.push(RecentItem::FollowedYouTube(f.id.clone()));
                }
            }
        }
        if matches!(filter, LibraryFilter::All | LibraryFilter::Albums) {
            for f in &app.data.followed_ytm {
                if f.id.starts_with("album:") {
                    kinds.push(RecentItem::FollowedYouTube(f.id.clone()));
                }
            }
            for a in &app.lib.albums {
                let is_local_file = a.tracks.iter().any(|tid| {
                    app.lib.track(*tid).is_some_and(|t| {
                        !t.path.starts_with("yt:") && !t.path.starts_with("demo:")
                    })
                });
                if app.lib.demo || is_local_file || app.data.saved_albums.contains(&a.key) {
                    kinds.push(RecentItem::Album(a.key.clone()));
                }
            }
        }
        if matches!(filter, LibraryFilter::All | LibraryFilter::Artists) {
            let mut seen_artists = std::collections::HashSet::new();
            for a in &app.lib.artists {
                let is_local_file = a.tracks.iter().any(|tid| {
                    app.lib.track(*tid).is_some_and(|t| {
                        !t.path.starts_with("yt:") && !t.path.starts_with("demo:")
                    })
                });
                if app.lib.demo || is_local_file || app.data.followed_artists.contains(&a.name) {
                    seen_artists.insert(fold(&a.name));
                    kinds.push(RecentItem::Artist(a.name.clone()));
                }
            }
            let mut extra_followed: Vec<&String> = app.data.followed_artists.iter().collect();
            extra_followed.sort_by(|a, b| fold(a).cmp(&fold(b)));
            for name in extra_followed {
                if seen_artists.insert(fold(name)) {
                    kinds.push(RecentItem::Artist(name.clone()));
                }
            }
        }
    }

    for k in &kinds {
        let Some(mut item) = item_of(app, k) else {
            continue;
        };
        let (added, creator) = match k {
            RecentItem::Liked => {
                let count = app
                    .data
                    .liked
                    .iter()
                    .filter(|e| app.lib.tracks.contains_key(&e.id))
                    .count();
                item.sub = format!(
                    "{playlist_word} • {count} {}",
                    app.t("bài hát", "songs")
                );
                let latest = app.data.liked.iter().map(|e| e.added).max().unwrap_or(0);
                (latest, you_label.clone())
            }
            RecentItem::Playlist(id) => {
                let p = app.data.playlist(*id);
                let creator_name = p
                    .and_then(|pl| pl.creator.clone())
                    .unwrap_or_else(|| you_label.clone());
                if let Some(pl) = p {
                    if let Some(c) = &pl.creator {
                        item.sub = format!("{playlist_word} • {c}");
                    }
                }
                (p.map_or(0, |pl| pl.created), creator_name)
            }
            RecentItem::Album(key) => {
                let a = app.lib.album(key);
                (
                    a.map_or(0, |al| al.added),
                    a.map(|al| al.artist.clone()).unwrap_or_default(),
                )
            }
            RecentItem::Artist(name) => (0, name.clone()),
            RecentItem::FollowedYouTube(id) => {
                let f = app.data.followed_ytm.iter().find(|list| &list.id == id);
                let is_album = id.starts_with("album:");
                let by = f
                    .and_then(|list| list.snapshot.subtitle.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "YouTube Music".to_string());
                if !is_album {
                    item.sub = format!("{playlist_word} • {by}");
                }
                (f.map_or(0, |list| list.added), by)
            }
        };

        let pinned = app.data.is_pinned(k);
        let title = item.title.clone();
        let sub = item.sub.clone();
        entries.push(Entry {
            kind: EntryKind::Item(item),
            title,
            sub,
            pinned,
            added,
            recent_rank: rank_of(k),
            creator,
        });
    }

    if !q.is_empty() {
        entries.retain(|e| fold(&e.title).contains(&q) || fold(&e.sub).contains(&q));
    }

    let sort_by = app.data.settings.effective_library_sort();
    entries.sort_by(|a, b| {
        let pin_cmp = Reverse(a.pinned).cmp(&Reverse(b.pinned));
        if pin_cmp != std::cmp::Ordering::Equal {
            return pin_cmp;
        }
        match sort_by {
            LibrarySortBy::Recents => (a.recent_rank, Reverse(a.added), fold(&a.title))
                .cmp(&(b.recent_rank, Reverse(b.added), fold(&b.title))),
            LibrarySortBy::RecentlyAdded => {
                (Reverse(a.added), fold(&a.title)).cmp(&(Reverse(b.added), fold(&b.title)))
            }
            LibrarySortBy::Alphabetical => fold(&a.title).cmp(&fold(&b.title)),
            LibrarySortBy::Creator => {
                (fold(&a.creator), fold(&a.title)).cmp(&(fold(&b.creator), fold(&b.title)))
            }
        }
    });

    entries
}

fn matches_creator_filter(creator: Option<&str>, filter: LibraryCreatorFilter) -> bool {
    match filter {
        LibraryCreatorFilter::All => true,
        LibraryCreatorFilter::ByYou => creator.is_none_or(|c| !c.eq_ignore_ascii_case("spotify")),
        LibraryCreatorFilter::BySpotify => creator.is_some_and(|c| c.eq_ignore_ascii_case("spotify")),
    }
}

fn paint_entry_cover(app: &mut App, ui: &Ui, rect: Rect, entry: &Entry, radius: f32) {
    match &entry.kind {
        EntryKind::Folder { .. } => {
            ui.painter().rect_filled(rect, radius, CARD_HOVER);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.center(), Vec2::splat(rect.width() * 0.46)),
                Icon::Folder,
                SUBTLE,
            );
        }
        EntryKind::Item(item) if item.kind == RecentItem::Liked => {
            let tex = liked_gradient_texture(ui.ctx());
            cover(ui, rect, &tex, radius);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(rect.center(), Vec2::splat(rect.width() * 0.42)),
                Icon::HeartFilled,
                TEXT,
            );
        }
        EntryKind::Item(item) => {
            let mut key = item.cover_key.clone();
            let fallback_artist_art = if item.cover_path.is_none() && !app.lib.demo {
                if let RecentItem::Artist(name) = &item.kind {
                    let wake = ui.ctx().clone();
                    let fetched = crate::ytm::artist_profile_cached(name, move || wake.request_repaint())
                        .and_then(|p| p.image);
                    if let Some(ref url) = fetched {
                        if app.data.record_artist_avatar(name, url) {
                            app.dirty = true;
                        }
                        key = crate::art::cover_key(url);
                    }
                    fetched
                } else {
                    None
                }
            } else {
                None
            };
            let source = item
                .cover_path
                .as_deref()
                .or(fallback_artist_art.as_deref());
            let tex = app.art.get(ui.ctx(), &key, source);
            cover(
                ui,
                rect,
                &tex,
                if item.circle { rect.width() * 0.5 } else { radius },
            );
        }
    }
}

fn liked_gradient_texture(ctx: &egui::Context) -> TextureHandle {
    let id = Id::unique("nhip-liked-gradient-450af5-c4efd9");
    ctx.data_mut(|d| d.get_temp::<TextureHandle>(id))
        .unwrap_or_else(|| {
            const SIDE: usize = 64;
            let mut pixels = Vec::with_capacity(SIDE * SIDE);
            for y in 0..SIDE {
                for x in 0..SIDE {
                    let t = (x + y) as f32 / (2.0 * (SIDE - 1) as f32);
                    let r = egui::lerp(
                        LIKED_GRADIENT_TOP_LEFT.r() as f32..=LIKED_GRADIENT_BOTTOM_RIGHT.r() as f32,
                        t,
                    )
                    .round() as u8;
                    let g = egui::lerp(
                        LIKED_GRADIENT_TOP_LEFT.g() as f32..=LIKED_GRADIENT_BOTTOM_RIGHT.g() as f32,
                        t,
                    )
                    .round() as u8;
                    let b = egui::lerp(
                        LIKED_GRADIENT_TOP_LEFT.b() as f32..=LIKED_GRADIENT_BOTTOM_RIGHT.b() as f32,
                        t,
                    )
                    .round() as u8;
                    pixels.push(Color32::from_rgb(r, g, b));
                }
            }
            let image = ColorImage::new([SIDE, SIDE], pixels);
            let tex = ctx.load_texture("liked-gradient", image, TextureOptions::LINEAR);
            ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
            tex
        })
}

fn is_entry_selected(app: &App, entry: &Entry) -> bool {
    match &entry.kind {
        EntryKind::Folder { id, .. } => app.opened_folder == Some(*id),
        EntryKind::Item(item) => match (&app.route, &item.route) {
            (Route::YouTubeMusicItem(open), Route::YouTubeMusicItem(list)) => open.id == list.id,
            (open, list) => open == list,
        },
    }
}

fn is_entry_playing(app: &App, entry: &Entry) -> bool {
    let context = entry.context();
    context != ContextRef::None && app.player.context_ref == context && app.player.current.is_some()
}

fn sort_name(app: &App, sort: LibrarySortBy) -> &'static str {
    match sort {
        LibrarySortBy::Recents => app.t("Gần đây", "Recents"),
        LibrarySortBy::RecentlyAdded => app.t("Mới thêm", "Recently added"),
        LibrarySortBy::Alphabetical => app.t("Theo tên", "Alphabetical"),
        LibrarySortBy::Creator => app.t("Người tạo", "Creator"),
    }
}

fn view_name(app: &App, view: LibraryViewMode) -> &'static str {
    match view {
        LibraryViewMode::CompactList => app.t("Danh sách thu gọn", "Compact list"),
        LibraryViewMode::DefaultList => app.t("Danh sách mặc định", "Default list"),
        LibraryViewMode::CompactGrid => app.t("Lưới thu gọn", "Compact grid"),
        LibraryViewMode::DefaultGrid => app.t("Lưới mặc định", "Default grid"),
    }
}

fn compact_tag(app: &App, entry: &Entry) -> &'static str {
    match &entry.kind {
        EntryKind::Folder { .. } => app.t("Thư mục", "Folder"),
        EntryKind::Item(item) => match &item.kind {
            RecentItem::Liked | RecentItem::Playlist(_) => app.t("Playlist", "Playlist"),
            RecentItem::FollowedYouTube(id) if id.starts_with("album:") => "Album",
            RecentItem::FollowedYouTube(_) => app.t("Playlist", "Playlist"),
            RecentItem::Album(_) => "Album",
            RecentItem::Artist(_) => app.t("Nghệ sĩ", "Artist"),
        },
    }
}

fn format_added_date(ts: u64) -> String {
    if ts == 0 {
        return "—".to_string();
    }
    Local
        .timestamp_opt(ts as i64, 0)
        .single()
        .map(|dt| dt.format("%d/%m/%Y").to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn context_of(kind: &RecentItem) -> ContextRef {
    match kind {
        RecentItem::Liked => ContextRef::Liked,
        RecentItem::Playlist(id) => ContextRef::Playlist(*id),
        RecentItem::Album(k) => ContextRef::Album(k.clone()),
        RecentItem::Artist(n) => ContextRef::Artist(n.clone()),
        RecentItem::FollowedYouTube(_) => ContextRef::None,
    }
}

pub fn route_of(app: &App, kind: &RecentItem) -> Option<Route> {
    Some(match kind {
        RecentItem::Liked => Route::Liked,
        RecentItem::Playlist(id) => Route::Playlist(*id),
        RecentItem::Album(k) => Route::Album(k.clone()),
        RecentItem::Artist(n) => Route::Artist(n.clone()),
        RecentItem::FollowedYouTube(id) => {
            let playlist = app.data.followed_ytm.iter().find(|list| list.id == *id)?;
            let mut item = playlist.item();
            if let Some(album_id) = id.strip_prefix("album:") {
                item.id = crate::ytm::ItemId::Album(album_id.to_string());
            }
            Route::YouTubeMusicItem(item)
        }
    })
}
