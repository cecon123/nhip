//! Small painted controls shared by every page.

use eframe::egui::text::{LayoutJob, TextFormat, TextWrapping};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Image, LayerId, Order, Painter, Pos2, Rect,
    Response, Sense, Shape, Stroke, TextureHandle, Ui, Vec2,
};

use super::icons::{self, Icon};
use crate::theme::*;

/// Draws one line of text cut off with an ellipsis; returns the space it took.
pub fn text(p: &Painter, pos: Pos2, s: &str, font: FontId, color: Color32, max_w: f32, align: Align2) -> Rect {
    let mut job = LayoutJob::single_section(s.to_owned(), TextFormat { font_id: font, color, ..Default::default() });
    job.wrap = TextWrapping::truncate_at_width(max_w.max(1.0));
    let galley = p.layout_job(job);
    let rect = align.anchor_size(pos, galley.size());
    p.galley(rect.min, galley, color);
    rect
}

pub fn text_width(ui: &Ui, s: &str, font: FontId) -> f32 {
    ui.painter().layout_no_wrap(s.to_owned(), font, TEXT).size().x
}

/// Custom-painted controls still need a visible keyboard focus target.
pub fn focus_ring(ui: &Ui, response: &Response, radius: f32) {
    if response.has_focus() {
        ui.painter().rect_stroke(response.rect.expand(3.0), radius, Stroke::new(2.0, TEXT), egui::StrokeKind::Outside);
    }
}

pub fn mmss(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub fn icon_button(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: Icon, base: Color32, tip: &str) -> Response {
    let glyph = match icon {
        Icon::Home | Icon::HomeFilled => 20.0,
        _ => 16.0,
    };
    icon_button_sized(ui, rect, id, icon, base, tip, glyph)
}

pub fn icon_button_sized(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: Icon, base: Color32, tip: &str, glyph: f32) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    let pressed = resp.is_pointer_button_down_on();
    let hovered = resp.hovered();
    let color = if pressed {
        SUBTLE
    } else if hovered {
        if base == ACCENT { ACCENT_HOVER } else { TEXT }
    } else {
        base
    };
    let scale = hover_scale(hovered, pressed);
    let size = rect.height().min(rect.width()).min(glyph) * scale;
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(size)), icon, color);
    focus_ring(ui, &resp, rect.height().min(rect.width()) * 0.5);
    resp
}

pub fn round_button(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, icon: Icon, fill: Color32, fg: Color32, tip: &str) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click()).on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), tip));
    let scale = hover_scale(resp.hovered(), resp.is_pointer_button_down_on());
    let r = rect.width() * 0.5 * scale;
    ui.painter().circle_filled(rect.center(), r, fill);
    let side = (rect.width() * 0.5).clamp(12.0, 24.0);
    icons::paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(side)), icon, fg);
    focus_ring(ui, &resp, r);
    resp
}

pub fn pill(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, label: &str, active: bool) -> Response {
    // Spotify filter chips use `14px` Regular (`encore-text-body-small`, `font-weight: 400`).
    pill_sized(ui, rect, id, label, active, 14.0)
}

pub fn pill_sized(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, label: &str, active: bool, size: f32) -> Response {
    let resp = ui.interact(rect, ui.scope_id().with(id), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    let (bg, fg) = if active {
        (TEXT, Color32::BLACK)
    } else if resp.hovered() {
        (Color32::from_white_alpha(46), TEXT)
    } else {
        (Color32::from_white_alpha(26), TEXT)
    };
    ui.painter().rect_filled(rect, rect.height() * 0.5, bg);
    text(ui.painter(), rect.center(), label, body(size), fg, rect.width() - 8.0, Align2::CENTER_CENTER);
    focus_ring(ui, &resp, rect.height() * 0.5);
    resp
}

pub fn pill_width(ui: &Ui, label: &str) -> f32 {
    text_width(ui, label, body(14.0)) + 24.0
}

pub fn cover(ui: &Ui, rect: Rect, tex: &TextureHandle, radius: f32) {
    Image::new(tex)
        .corner_radius(CornerRadius::same(radius.min(255.0) as u8))
        .paint_at(ui, rect);
}

/// Draws a soft downward-offset ambient shadow beneath album/playlist artwork without dark side halos.
pub fn cover_shadow(p: &Painter, rect: Rect, radius: f32) {
    if rect.width() <= 14.0 || rect.height() <= 14.0 {
        return;
    }
    let r = CornerRadius::same(radius.clamp(0.0, 255.0) as u8);
    let ambient = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(66),
    };
    let contact = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 10,
        spread: 0,
        color: Color32::from_black_alpha(38),
    };
    p.add(ambient.as_shape(rect.shrink(5.0), r));
    p.add(contact.as_shape(rect.shrink(2.5), r));
}

/// Animated Spotify-style dark shimmer skeleton placeholder (`#222222` -> `#343434` sweep).
pub fn skeleton_rect(ui: &Ui, rect: Rect, radius: f32) {
    if !rect.is_positive() {
        return;
    }
    let t = (ui.input(|i| i.time) * 0.85 + 0.42).fract() as f32;
    let r = CornerRadius::same(radius.clamp(0.0, 127.0) as u8);
    let base = Color32::from_rgb(0x3a, 0x3a, 0x3a);
    let peak = Color32::from_rgb(0x50, 0x50, 0x50);
    let full_w = ui.ctx().content_rect().width().max(400.0);
    let band_x = egui::lerp(-220.0..=(full_w + 220.0), t);
    let dist = ((rect.center().x - band_x).abs() / 240.0).clamp(0.0, 1.0);
    let pulse = (1.0 - dist * dist * (3.0 - 2.0 * dist)).clamp(0.0, 1.0);
    ui.painter().rect_filled(rect, r, base.lerp_to_gamma(peak, pulse));
    if !ui.input(|i| i.viewport().minimized.unwrap_or(false)) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// Translucent glass shimmer skeleton bar (used on colored Spicy Lyrics / Hero backdrops).
pub fn skeleton_glass_rect(ui: &Ui, rect: Rect, radius: f32) {
    if !rect.is_positive() {
        return;
    }
    let t = (ui.input(|i| i.time) * 0.85).fract() as f32;
    let r = CornerRadius::same(radius.clamp(0.0, 127.0) as u8);
    let full_w = ui.ctx().content_rect().width().max(400.0);
    let band_x = egui::lerp(-220.0..=(full_w + 220.0), t);
    let dist = ((rect.center().x - band_x).abs() / 260.0).clamp(0.0, 1.0);
    let pulse = (1.0 - dist * dist * (3.0 - 2.0 * dist)).clamp(0.0, 1.0);
    let alpha = (28.0 + 34.0 * pulse).round() as u8;
    ui.painter().rect_filled(rect, r, Color32::from_white_alpha(alpha));
    if !ui.input(|i| i.viewport().minimized.unwrap_or(false)) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
}

/// A horizontal progress or volume bar. Returns the fraction while the user is dragging, plus
/// whether the drag ended on this frame.
pub struct BarResult {
    pub dragging: Option<f32>,
    pub released: bool,
}

pub fn bar(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash + std::fmt::Debug, value: f32) -> BarResult {
    spotify_scrubber(ui, rect, id, value, None)
}

/// Spotify-style 4px progress/volume scrubber with hover green (`#1db954`), 12x12 white knob, and
/// optional `#2a2a2a` floating timestamp tooltip when `duration_ms` is `Some`.
pub fn spotify_scrubber(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: f32,
    duration_ms: Option<u64>,
) -> BarResult {
    let hit = rect.expand2(Vec2::new(0.0, 6.0));
    let resp = ui.interact(hit, ui.scope_id().with(id), Sense::click_and_drag());
    let keyboard = if resp.has_focus() && ui.is_enabled() {
        ui.input_mut(|input| {
            [egui::Key::ArrowLeft, egui::Key::ArrowRight, egui::Key::Home, egui::Key::End]
                .into_iter()
                .find(|key| input.consume_key(egui::Modifiers::NONE, *key))
                .map(|key| scrubber_key_value(value, duration_ms, key))
        })
    } else {
        None
    };
    resp.widget_info(|| egui::WidgetInfo::slider(ui.is_enabled(), value as f64, if duration_ms.is_some() { "Playback position" } else { "Volume" }));
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 4.0));
    let width = rect.width().max(1.0);
    let pointer = resp
        .interact_pointer_pos()
        .map(|p| ((p.x - rect.left()) / width).clamp(0.0, 1.0));
    let dragging = resp.dragged() || resp.is_pointer_button_down_on();
    let active = resp.hovered() || dragging || resp.has_focus();
    let shown = if let Some(value) = keyboard {
        value
    } else if dragging {
        pointer.unwrap_or(value)
    } else {
        value
    }
    .clamp(0.0, 1.0);
    let filled = Rect::from_min_max(
        track.min,
        Pos2::new(track.left() + track.width() * shown, track.bottom()),
    );

    let p = ui.painter();
    p.rect_filled(track, 2.0, TRACK_ALPHA);
    p.rect_filled(filled, 2.0, if active { ACCENT_ALT } else { TEXT });
    if active {
        let knob_center = Pos2::new(filled.right(), track.center().y);
        let knob_rect = Rect::from_center_size(knob_center, Vec2::splat(12.0));
        let knob_shadow = egui::epaint::Shadow {
            offset: [0, 2],
            blur: 4,
            spread: 0,
            color: Color32::from_black_alpha(0x80),
        };
        p.add(knob_shadow.as_shape(knob_rect, pill_corner_radius()));
        p.circle_filled(knob_center, 6.0, TEXT);

        if let Some(total_ms) = duration_ms {
            let hover_frac = if dragging {
                shown
            } else {
                resp.hover_pos()
                    .map(|pos| ((pos.x - rect.left()) / width).clamp(0.0, 1.0))
                    .unwrap_or(shown)
            };
            let tip_ms = (hover_frac as f64 * total_ms as f64).round() as u64;
            let tip_label = mmss(tip_ms);
            let galley = p.layout_no_wrap(tip_label, bold(12.0), TEXT);
            let tip_w = galley.size().x + 16.0;
            let tip_h = 24.0;
            let raw_x = rect.left() + track.width() * hover_frac;
            let tip_x = if rect.width() >= tip_w {
                raw_x.clamp(rect.left() + tip_w * 0.5, rect.right() - tip_w * 0.5)
            } else {
                rect.center().x
            };
            let tip_rect = Rect::from_center_size(
                Pos2::new(tip_x, track.top() - 8.0 - tip_h * 0.5),
                Vec2::new(tip_w, tip_h),
            );
            let tip_layer = ui
                .ctx()
                .layer_painter(LayerId::new(Order::Tooltip, resp.id.with("scrubber-tip")));
            tip_layer.add(knob_shadow.as_shape(tip_rect, card_radius(RADIUS_BASE as u8)));
            tip_layer.rect_filled(tip_rect, RADIUS_BASE, HOVER);
            let text_pos = Pos2::new(
                tip_rect.center().x - galley.size().x * 0.5,
                tip_rect.center().y - galley.size().y * 0.5,
            );
            tip_layer.galley(text_pos, galley, TEXT);
        }
    }
    focus_ring(ui, &resp, 4.0);
    BarResult {
        dragging: keyboard.or(if dragging { pointer } else { None }),
        released: keyboard.is_some() || resp.drag_stopped() || resp.clicked(),
    }
}

fn scrubber_key_value(value: f32, duration_ms: Option<u64>, key: egui::Key) -> f32 {
    let step = duration_ms.map_or(0.01, |duration| 5000.0 / duration.max(1) as f32);
    match key {
        egui::Key::Home => 0.0,
        egui::Key::End => 1.0,
        egui::Key::ArrowLeft => (value - step).clamp(0.0, 1.0),
        egui::Key::ArrowRight => (value + step).clamp(0.0, 1.0),
        _ => value,
    }
}

/// Renders single-line text clipped to `rect`, ping-pong scrolling at 12 px/s with 1500 ms pauses
/// at both ends whenever the text exceeds `rect.width()`.
pub fn marquee_text(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    s: &str,
    font: FontId,
    color: Color32,
) {
    let galley = ui.painter().layout_no_wrap(s.to_owned(), font, color);
    let overflow = (galley.size().x - rect.width()).max(0.0);
    let offset = if overflow > 0.5 {
        let now = ui.ctx().input(|i| i.time);
        let state_id = ui.scope_id().with(id).with("marquee");
        let start_t = ui.ctx().data_mut(|d| {
            let entry = d.get_temp_mut_or_insert_with(state_id, || (s.to_owned(), now));
            if entry.0 != s {
                *entry = (s.to_owned(), now);
            }
            entry.1
        });
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(16));
        marquee_offset((now - start_t).max(0.0), overflow)
    } else {
        0.0
    };
    let y = rect.center().y - galley.size().y * 0.5;
    let draw_pos = Pos2::new(rect.left() - offset, y);
    let galley = if overflow > 0.5 {
        let mut g = (*galley).clone();
        let fade_l = if offset > 0.5 { 16.0 } else { 0.0 };
        let fade_r = if offset < overflow - 0.5 { 20.0 } else { 0.0 };
        let smooth = |t: f32| {
            let u = t.clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };
        for placed in &mut g.rows {
            let row = std::sync::Arc::make_mut(&mut placed.row);
            for v in &mut row.visuals.mesh.vertices {
                let vx = draw_pos.x + placed.pos.x + v.pos.x;
                let al = if fade_l > 0.0 {
                    smooth((vx - rect.left()) / fade_l)
                } else {
                    1.0
                };
                let ar = if fade_r > 0.0 {
                    smooth((rect.right() - vx) / fade_r)
                } else {
                    1.0
                };
                let a = (al * ar).clamp(0.0, 1.0);
                v.color = v.color.gamma_multiply(a);
            }
        }
        std::sync::Arc::new(g)
    } else {
        galley
    };
    ui.painter()
        .with_clip_rect(rect)
        .galley(draw_pos, galley, color);
}

fn marquee_offset(elapsed_s: f64, overflow: f32) -> f32 {
    if overflow <= 0.0 {
        return 0.0;
    }
    const SPEED_PX_S: f64 = 12.0;
    const PAUSE_S: f64 = 1.5;
    let travel_s = overflow as f64 / SPEED_PX_S;
    let half_cycle = PAUSE_S + travel_s;
    let cycle = half_cycle * 2.0;
    let t = elapsed_s.rem_euclid(cycle);
    if t < PAUSE_S {
        0.0
    } else if t < half_cycle {
        ((t - PAUSE_S) * SPEED_PX_S) as f32
    } else if t < half_cycle + PAUSE_S {
        overflow
    } else {
        (overflow as f64 - (t - half_cycle - PAUSE_S) * SPEED_PX_S).max(0.0) as f32
    }
}

/// Spotify green circular play/pause button with lift-in animation, drop shadow, and 1.04x hover scale.
pub fn hover_play_button(
    ui: &mut Ui,
    rect: Rect,
    id: impl std::hash::Hash + std::fmt::Debug,
    playing: bool,
    visible: bool,
    tip: &str,
) -> Response {
    let btn_id = ui.scope_id().with(id);
    let lift = ui.ctx().animate_bool_with_time(btn_id.with("lift"), visible, 0.2);
    if !visible && lift <= 0.01 {
        return ui.interact(Rect::NOTHING, btn_id, Sense::hover());
    }
    let y_offset = (1.0 - lift) * 8.0;
    let shifted = rect.translate(Vec2::new(0.0, y_offset));
    let resp = if visible {
        ui.interact(shifted, btn_id, Sense::click()).on_hover_text(tip)
    } else {
        ui.interact(Rect::NOTHING, btn_id, Sense::hover())
    };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, visible, tip));
    let hovered = visible && resp.hovered();
    let pressed = visible && resp.is_pointer_button_down_on();
    let scaled = hover_scale_rect(shifted, hovered, pressed);
    let fill_base = if pressed {
        ACCENT_ACTIVE
    } else if hovered {
        ACCENT_HOVER
    } else {
        ACCENT
    };
    let alpha = (lift.clamp(0.0, 1.0) * 255.0).round() as u8;
    let fill = fill_base.linear_multiply(lift.clamp(0.0, 1.0));
    let shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha((0x4d as f32 * lift.clamp(0.0, 1.0)).round() as u8),
    };
    let p = ui.painter();
    p.add(shadow.as_shape(scaled, pill_corner_radius()));
    p.circle_filled(scaled.center(), scaled.width() * 0.5, fill);
    let glyph_side = (scaled.width() * 0.46).clamp(14.0, 24.0);
    let icon = if playing { Icon::Pause } else { Icon::Play };
    icons::paint(
        p,
        Rect::from_center_size(scaled.center(), Vec2::splat(glyph_side)),
        icon,
        Color32::from_black_alpha(alpha),
    );
    focus_ring(ui, &resp, scaled.width() * 0.5);
    resp
}

pub fn vertical_gradient(p: &Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    mesh.colored_vertex_like(rect.left_top(), top, uv);
    mesh.colored_vertex_like(rect.right_top(), top, uv);
    mesh.colored_vertex_like(rect.right_bottom(), bottom, uv);
    mesh.colored_vertex_like(rect.left_bottom(), bottom, uv);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(Shape::mesh(mesh));
}

/// The same colour, darker but still fully opaque.
pub fn darken(c: Color32, k: f32) -> Color32 {
    let f = |x: u8| (x as f32 * k) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// A wash of colour fading down a panel, keeping the panel's rounded corners.

trait MeshExt {
    fn colored_vertex_like(&mut self, pos: Pos2, color: Color32, uv: Pos2);
}

impl MeshExt for egui::epaint::Mesh {
    fn colored_vertex_like(&mut self, pos: Pos2, color: Color32, uv: Pos2) {
        self.vertices.push(egui::epaint::Vertex { pos, uv, color });
    }
}

/// Spotify Settings 42x24 pill toggle switch; returns true when flipped.
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> bool {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool(resp.id, *on);
    let hovered = resp.hovered() && ui.is_enabled();
    let off_bg = if hovered { SUBTLE } else { DISABLED };
    let on_bg = if hovered { ACCENT } else { ACCENT_ALT };
    let mut bg = off_bg.lerp_to_gamma(on_bg, t);
    let mut thumb = TEXT;
    if !ui.is_enabled() {
        bg = bg.linear_multiply(0.4);
        thumb = thumb.linear_multiply(0.4);
    }
    ui.painter().rect_filled(rect, 12.0, bg);
    let x = egui::lerp((rect.left() + 12.0)..=(rect.right() - 12.0), t);
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), 9.0, thumb);
    focus_ring(ui, &resp, 12.0);
    resp.changed()
}

/// A toggle switch; returns true when flipped.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> bool {
    toggle_switch(ui, on)
}

/// Hairline outline in the style of the original's secondary buttons.
pub fn outline_button(ui: &mut Ui, label: &str) -> Response {
    let width = text_width(ui, label, semi(13.0)) + 28.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), Sense::click());
    let color = if resp.hovered() { TEXT } else { MUTED };
    ui.painter().rect_stroke(rect, 16.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
    text(ui.painter(), rect.center(), label, semi(13.0), TEXT, width, Align2::CENTER_CENTER);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    focus_ring(ui, &resp, 16.0);
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_scrubber_seeks_five_seconds_and_volume_one_percent() {
        use egui::Key;
        assert!((scrubber_key_value(0.5, Some(100_000), Key::ArrowRight) - 0.55).abs() < 1e-6);
        assert!((scrubber_key_value(0.5, None, Key::ArrowLeft) - 0.49).abs() < 1e-6);
        assert_eq!(scrubber_key_value(0.99, Some(1000), Key::ArrowRight), 1.0);
        assert_eq!(scrubber_key_value(0.01, Some(1000), Key::ArrowLeft), 0.0);
        assert_eq!(scrubber_key_value(0.5, None, Key::Home), 0.0);
        assert_eq!(scrubber_key_value(0.5, None, Key::End), 1.0);
    }

    #[test]
    fn mmss_formats_minutes_and_hours() {
        assert_eq!(mmss(0), "0:00");
        assert_eq!(mmss(65_000), "1:05");
        assert_eq!(mmss(3_661_000), "1:01:01");
    }

    #[test]
    fn marquee_ping_pongs_with_1500ms_pauses() {
        assert_eq!(marquee_offset(0.0, 0.0), 0.0);
        // 24 px overflow => 2.0s travel + 1.5s pause each side => 7.0s full cycle
        assert_eq!(marquee_offset(0.0, 24.0), 0.0);
        assert_eq!(marquee_offset(1.4, 24.0), 0.0);
        assert!((marquee_offset(2.5, 24.0) - 12.0).abs() < 1e-4);
        assert_eq!(marquee_offset(3.5, 24.0), 24.0);
        assert_eq!(marquee_offset(4.9, 24.0), 24.0);
        assert!((marquee_offset(6.0, 24.0) - 12.0).abs() < 1e-4);
        assert!((marquee_offset(7.0, 24.0) - 0.0).abs() < 1e-4);
    }
}

