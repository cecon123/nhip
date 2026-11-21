//! Colours and measurements read from the original's style sheets (see MECHANICS.md).

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, Vec2};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0x00, 0x00, 0x00);
pub const PANEL: Color32 = Color32::from_rgb(0x12, 0x12, 0x12);
pub const ELEVATED: Color32 = Color32::from_rgb(0x24, 0x24, 0x24);
pub const ELEVATED_PRESS: Color32 = Color32::from_rgb(0x19, 0x19, 0x19);
pub const HOVER: Color32 = Color32::from_rgb(0x2a, 0x2a, 0x2a);
pub const CARD: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
pub const CARD_HOVER: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
pub const POPUP: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
pub const TAB_ACTIVE: Color32 = Color32::from_rgb(0x33, 0x33, 0x33);
pub const TEXT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);
pub const SUBTLE: Color32 = Color32::from_rgb(0xb3, 0xb3, 0xb3);
pub const MUTED: Color32 = Color32::from_rgb(0x7c, 0x7c, 0x7c);
pub const DECORATIVE_SUBDUED: Color32 = Color32::from_rgb(0x29, 0x29, 0x29);
pub const DISABLED: Color32 = Color32::from_rgb(0x53, 0x53, 0x53);
pub const ACCENT: Color32 = Color32::from_rgb(0x1e, 0xd7, 0x60);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x3b, 0xe4, 0x77);
pub const ACCENT_ALT: Color32 = Color32::from_rgb(0x1d, 0xb9, 0x54);
pub const ACCENT_ACTIVE: Color32 = Color32::from_rgb(0x1a, 0xbc, 0x54);
pub const TRACK_BG: Color32 = Color32::from_rgb(0x4d, 0x4d, 0x4d);
pub const TRACK_ALPHA: Color32 = Color32::from_rgba_premultiplied(0x4d, 0x4d, 0x4d, 0x4d);
pub const TINT_BASE: Color32 = Color32::from_rgba_premultiplied(0x1a, 0x1a, 0x1a, 0x1a);
pub const TINT_HOVER: Color32 = Color32::from_rgba_premultiplied(0x24, 0x24, 0x24, 0x24);
pub const TINT_PRESS: Color32 = Color32::from_rgba_premultiplied(0x36, 0x36, 0x36, 0x36);
pub const HAIRLINE: Color32 = Color32::from_rgba_premultiplied(0x1a, 0x1a, 0x1a, 0x1a);
pub const SCROLLBAR_IDLE: Color32 = Color32::from_rgba_premultiplied(0x4d, 0x4d, 0x4d, 0x4d);
pub const SCROLLBAR_HOVER: Color32 = Color32::from_rgba_premultiplied(0x80, 0x80, 0x80, 0x80);
pub const SCROLLBAR_ACTIVE: Color32 = Color32::from_rgba_premultiplied(0xb3, 0xb3, 0xb3, 0xb3);

pub const GAP: f32 = 8.0;
pub const TOP_BAR: f32 = 64.0;
// Measured in the user's Spicetify runtime at 1304×724 CSS px (DPR 1.25).
pub const PLAYER_BAR: f32 = 88.0;
pub use crate::mechanics::{LEFT_DEFAULT, RIGHT_WIDTH};
pub const LEFT_COLLAPSED: f32 = 72.0;
pub const LEFT_MIN: f32 = 280.0;
pub const LEFT_EXPANDED_MIN: f32 = 584.0;
pub const LEFT_MAX: f32 = 420.0;
pub const RIGHT_MIN: f32 = 260.0;
pub const RIGHT_MAX: f32 = 480.0;
pub const MIN_MAIN_WIDTH: f32 = 416.0;
pub const RADIUS: u8 = 8;
pub const RADIUS_SMALLER: f32 = 2.0;
pub const RADIUS_BASE: f32 = 4.0;
pub const RADIUS_LARGER: f32 = 6.0;
pub const RADIUS_CARD: f32 = 8.0;
pub const RADIUS_MODAL: f32 = 16.0;
pub const RADIUS_PILL: f32 = 9999.0;

// Verified Spicy Lyrics GlassCard & Now Playing sidebar tokens (`tokens.css`, `NPVLyrics.css`, `default.css`)
pub const GLASS_CARD_FILL: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18); // rgba(255,255,255,0.07)
pub const GLASS_CARD_BORDER: Color32 = Color32::from_rgba_premultiplied(24, 24, 24, 24); // subtle glass hairline
pub const GLASS_CARD_RADIUS_LG: u8 = 16; // --radius-lg (`NPVLyrics.css:12`)
pub const GLASS_CARD_RADIUS_MD: u8 = 12; // --radius-md (`tokens.css:88`)
pub const NPV_SIDE_INSET: f32 = 16.0;
pub const NPV_SECTION_GAP: f32 = 12.0;
pub const ROW_H: f32 = 56.0;
pub const OPACITY_DISABLED: f32 = 0.3;
pub const OPACITY_ACTIVE: f32 = 0.7;
pub const HOVER_SCALE: f32 = 1.04;

pub fn hover_white(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

pub fn body(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold(size: f32) -> FontId {
    let family = if size >= 48.0 { "extrabold" } else if size >= 24.0 { "title" } else { "bold" };
    FontId::new(size, FontFamily::Name(family.into()))
}

pub fn semi(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("semi".into()))
}

pub fn marginal_small() -> FontId {
    body(11.0)
}

pub fn marginal() -> FontId {
    body(12.0)
}

pub fn body_small() -> FontId {
    body(14.0)
}

pub fn body_medium() -> FontId {
    body(16.0)
}

pub fn title_small() -> FontId {
    bold(16.0)
}

pub fn title_medium() -> FontId {
    bold(24.0)
}

pub fn title_large() -> FontId {
    bold(32.0)
}

pub fn headline_large() -> FontId {
    bold(48.0)
}

pub fn pill_radius() -> f32 {
    RADIUS_PILL
}

pub fn pill_corner_radius() -> egui::CornerRadius {
    egui::CornerRadius::same(u8::MAX)
}

pub fn card_radius(radius: u8) -> egui::CornerRadius {
    egui::CornerRadius::same(radius)
}

pub fn hover_scale(hovered: bool, pressed: bool) -> f32 {
    if hovered && !pressed {
        HOVER_SCALE
    } else {
        1.0
    }
}

pub fn hover_scale_rect(rect: egui::Rect, hovered: bool, pressed: bool) -> egui::Rect {
    let scale = hover_scale(hovered, pressed);
    if (scale - 1.0).abs() < f32::EPSILON {
        rect
    } else {
        egui::Rect::from_center_size(rect.center(), rect.size() * scale)
    }
}

pub fn accent_color(theme: crate::library::AccentTheme) -> Color32 {
    let [r, g, b] = theme.rgb();
    Color32::from_rgb(r, g, b)
}

/// Keeps `egui`'s root `CentralPanel` fill transparent whenever native Windows Mica / Acrylic is active
/// (otherwise `eframe::App::ui` paints opaque `#121212` over the entire transparent GL framebuffer!),
/// and updates interactive accent highlights to match the user's chosen `AccentTheme`.
pub fn sync_surface_and_accent(
    ctx: &egui::Context,
    surface: crate::library::SurfaceStyle,
    accent: crate::library::AccentTheme,
) {
    let ac = accent_color(accent);
    let panel_fill = if surface == crate::library::SurfaceStyle::Standard {
        PANEL
    } else {
        Color32::TRANSPARENT
    };
    ctx.global_style_mut(|style| {
        style.visuals.panel_fill = panel_fill;
        style.visuals.hyperlink_color = ac;
        style.visuals.selection.bg_fill = ac.linear_multiply(0.35);
        style.visuals.selection.stroke = Stroke::new(1.0, ac);
    });
}

pub fn popup_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::new()
        .fill(POPUP)
        .stroke(Stroke::new(1.0, HAIRLINE))
        .corner_radius(card_radius(RADIUS_CARD as u8))
        .inner_margin(egui::Margin::same(5))
        .shadow(style.visuals.popup_shadow)
}

pub fn menu_frame(style: &egui::Style) -> egui::Frame {
    egui::Frame::new()
        .fill(POPUP)
        .stroke(Stroke::new(1.0, HAIRLINE))
        .corner_radius(card_radius(RADIUS_BASE as u8))
        .inner_margin(egui::Margin::same(3))
        .shadow(style.visuals.popup_shadow)
}

/// Match the installed client's type metrics; system fonts cover remaining scripts.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_string());
    let load = |file: &str| std::fs::read(format!("{dir}\\Fonts\\{file}")).ok();

    let mut add = |name: &str, file: &str| -> bool {
        if let Some(bytes) = load(file) {
            fonts.font_data.insert(name.to_string(), Arc::new(FontData::from_owned(bytes)));
            true
        } else {
            false
        }
    };
    let fallbacks: Vec<String> = [("system-ui", "segoeui.ttf"), ("cjk", "msyh.ttc"), ("korean", "malgun.ttf"), ("symbols", "seguisym.ttf")]
        .iter()
        .filter(|(n, f)| add(n, f))
        .map(|(n, _)| n.to_string())
        .collect();

    for (name, bytes) in [
        ("ui", include_bytes!("../assets/fonts/SpotifyMixUI-Regular.ttf").as_slice()),
        ("ui-bold", include_bytes!("../assets/fonts/SpotifyMixUI-Bold.ttf").as_slice()),
        ("ui-title", include_bytes!("../assets/fonts/SpotifyMixUITitle-Bold.ttf").as_slice()),
        ("ui-extrabold", include_bytes!("../assets/fonts/SpotifyMixUITitle-Extrabold.ttf").as_slice()),
        ("spicy-lyrics", include_bytes!("../assets/fonts/SpicyLyrics-Bold.ttf").as_slice()),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }

    let base = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: Option<&str>| -> Vec<String> {
        let mut v: Vec<String> = first.map(str::to_string).into_iter().collect();
        v.extend(base.iter().cloned());
        v.extend(fallbacks.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, chain(Some("ui")));
    fonts.families.insert(FontFamily::Name("bold".into()), chain(Some("ui-bold")));
    fonts.families.insert(FontFamily::Name("semi".into()), chain(Some("ui-bold")));
    fonts.families.insert(FontFamily::Name("title".into()), chain(Some("ui-title")));
    fonts.families.insert(FontFamily::Name("extrabold".into()), chain(Some("ui-extrabold")));
    fonts.families.insert(FontFamily::Name("lyrics".into()), chain(Some("spicy-lyrics")));
    ctx.set_fonts(fonts);
}

pub fn apply_style(ctx: &egui::Context) {
    ctx.set_visuals(egui::Visuals::dark());
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = Vec2::new(8.0, 4.0);
        style.spacing.button_padding = Vec2::new(10.0, 6.0);
        style.spacing.menu_margin = egui::Margin::same(4);
        style.spacing.scroll = egui::style::ScrollStyle {
            floating: true,
            bar_width: 6.0,
            floating_width: 5.0,
            handle_min_length: 30.0,
            foreground_color: false,
            dormant_handle_opacity: 0.0,
            active_handle_opacity: 0.55,
            interact_handle_opacity: 0.85,
            dormant_background_opacity: 0.0,
            active_background_opacity: 0.0,
            interact_background_opacity: 0.0,
            fade: egui::style::ScrollFadeStyle {
                strength: 0.0,
                size: 0.0,
            },
            ..Default::default()
        };
        style.text_styles.insert(egui::TextStyle::Body, body(14.0));
        style.text_styles.insert(egui::TextStyle::Button, body(14.0));
        style.text_styles.insert(egui::TextStyle::Small, body(12.0));
        style.text_styles.insert(egui::TextStyle::Heading, bold(24.0));

        let v = &mut style.visuals;
        v.panel_fill = PANEL;
        v.window_fill = POPUP;
        v.window_stroke = Stroke::NONE;
        v.popup_shadow = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(105),
        };
        v.window_corner_radius = card_radius(RADIUS_LARGER as u8);
        v.menu_corner_radius = card_radius(RADIUS_BASE as u8);
        v.override_text_color = Some(TEXT);
        v.hyperlink_color = ACCENT;
        v.selection.bg_fill = ACCENT.linear_multiply(0.35);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_edit_bg_color = Some(HOVER);
        v.widgets.noninteractive.bg_stroke = Stroke::NONE;
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = card_radius(RADIUS_BASE as u8);
            w.bg_stroke = Stroke::NONE;
        }
        v.widgets.inactive.weak_bg_fill = TAB_ACTIVE;
        v.widgets.inactive.bg_fill = SCROLLBAR_IDLE;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0, TINT_BASE);
        v.widgets.hovered.weak_bg_fill = TINT_HOVER;
        v.widgets.hovered.bg_fill = SCROLLBAR_HOVER;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, TINT_HOVER);
        v.widgets.active.weak_bg_fill = TINT_PRESS;
        v.widgets.active.bg_fill = SCROLLBAR_ACTIVE;
        v.widgets.active.bg_stroke = Stroke::new(1.0, TINT_PRESS);
        v.widgets.open.weak_bg_fill = TINT_HOVER;
        v.widgets.open.bg_fill = POPUP;
        v.widgets.open.bg_stroke = Stroke::new(1.0, TINT_HOVER);
        v.interact_cursor = Some(egui::CursorIcon::PointingHand);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_typefaces_render_vietnamese_headings_and_body() {
        let ctx = egui::Context::default();
        install_fonts(&ctx);
        let sample = "Nhịp Nghệ sĩ yêu thích Đặng ướt";
        let mut output = ctx.run_ui(Default::default(), |ui| {
            for font in [body(16.0), semi(14.0), bold(24.0), bold(72.0), FontId::new(36.0, FontFamily::Name("lyrics".into()))] {
                let tofu = ui.painter().layout_no_wrap("\u{FFFD}".into(), font.clone(), TEXT);
                let tofu_uv = tofu.rows.first().and_then(|r| r.glyphs.first()).map(|g| (g.uv_rect.min, g.uv_rect.max));

                let galley = ui.painter().layout_no_wrap(sample.into(), font, TEXT);
                let glyphs = &galley.rows[0].glyphs;
                assert_eq!(glyphs.len(), sample.chars().count(), "expected one glyph per Unicode scalar");
                for g in glyphs.iter().filter(|g| !g.chr.is_whitespace()) {
                    assert!(!g.uv_rect.is_nothing(), "missing rasterized glyph for '{}'", g.chr);
                    if let Some(bad_uv) = tofu_uv {
                        assert_ne!((g.uv_rect.min, g.uv_rect.max), bad_uv, "Vietnamese char '{}' fell back to replacement glyph", g.chr);
                    }
                }
            }
        });
        output.textures_delta.clear();
    }
}
