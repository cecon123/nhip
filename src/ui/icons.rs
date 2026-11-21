//! Encore & Spicy Lyrics glyph masks pre-baked from the desktop client, tinted by egui.

use eframe::egui::{
    Color32, ColorImage, Id, Painter, Pos2, Rect, TextureHandle, TextureOptions, Vec2,
};

const COLUMNS: usize = 8;
const CELL_24: usize = 68;
const GLYPH_24: usize = 64;
const CELL_16: usize = 36;
const GLYPH_16: usize = 32;
const PADDING: usize = 2;
const ATLAS_24: &[u8] = include_bytes!("icons.png");
const ATLAS_16: &[u8] = include_bytes!("icons16.png");

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Icon {
    Play,
    Pause,
    Next,
    Previous,
    Shuffle,
    Repeat,
    RepeatOne,
    HeartFilled,
    Save,
    Saved,
    Mute,
    VolumeLow,
    VolumeMedium,
    Volume,
    Queue,
    NowPlaying,
    Lyrics,
    Home,
    HomeFilled,
    Search,
    Library,
    Plus,
    Back,
    Forward,
    Close,
    Clock,
    Dots,
    Gear,
    Expand,
    Shrink,
    ChevronUp,
    ChevronDown,
    OpenLyrics,
    Browse,
    BrowseFilled,
    Bell,
    BellFilled,
    FriendActivity,
    FriendActivityFilled,
    Pin,
    PlayingSpeaker,
    // Preserve glyph indices in the embedded atlases.
    Miniplayer = 42,
    Grid,
    Sort,
    Checkmark,
    Folder,
    VerifiedBadge,
    LibraryCollapse,
    LibraryExpand,
    CardExpand,
    CardCollapse,
    CardToggle,
    Dislike,
    Explicit,
    Info,
    Credits,
    CompactList,
    Fullscreen,
    FullscreenExit,
}

impl Icon {
    pub const ALL: &'static [Icon] = &[
        Icon::Play,
        Icon::Pause,
        Icon::Next,
        Icon::Previous,
        Icon::Shuffle,
        Icon::Repeat,
        Icon::RepeatOne,
        Icon::HeartFilled,
        Icon::Save,
        Icon::Saved,
        Icon::Mute,
        Icon::VolumeLow,
        Icon::VolumeMedium,
        Icon::Volume,
        Icon::Queue,
        Icon::NowPlaying,
        Icon::Lyrics,
        Icon::Home,
        Icon::HomeFilled,
        Icon::Search,
        Icon::Library,
        Icon::Plus,
        Icon::Back,
        Icon::Forward,
        Icon::Close,
        Icon::Clock,
        Icon::Dots,
        Icon::Gear,
        Icon::Expand,
        Icon::Shrink,
        Icon::ChevronUp,
        Icon::ChevronDown,
        Icon::OpenLyrics,
        Icon::Browse,
        Icon::BrowseFilled,
        Icon::Bell,
        Icon::BellFilled,
        Icon::FriendActivity,
        Icon::FriendActivityFilled,
        Icon::Pin,
        Icon::PlayingSpeaker,
        Icon::Miniplayer,
        Icon::Grid,
        Icon::Sort,
        Icon::Checkmark,
        Icon::Folder,
        Icon::VerifiedBadge,
        Icon::LibraryCollapse,
        Icon::LibraryExpand,
        Icon::CardExpand,
        Icon::CardCollapse,
        Icon::CardToggle,
        Icon::Dislike,
        Icon::Explicit,
        Icon::Info,
        Icon::Credits,
        Icon::CompactList,
        Icon::Fullscreen,
        Icon::FullscreenExit,
    ];
}

fn decode_atlas(bytes: &[u8]) -> image::RgbaImage {
    image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
        .expect("embedded icon atlas")
        .into_rgba8()
}

pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let ctx = painter.ctx();
    let scale = ctx.pixels_per_point();
    let logical = rect.width().min(rect.height());
    let optical_16 = logical <= 20.0;
    let max_glyph = if optical_16 { GLYPH_16 } else { GLYPH_24 };
    let side = (logical * scale).round().clamp(1.0, max_glyph as f32) as usize;
    let index = icon as usize;
    let id = Id::unique(("nhip-icon", index, side, optical_16));
    let texture = ctx
        .data_mut(|data| data.get_temp::<TextureHandle>(id))
        .unwrap_or_else(|| {
            let texture = ctx.load_texture(
                "nhip icon",
                rasterize_optical(index, side, optical_16),
                TextureOptions::LINEAR,
            );
            ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    let size = side as f32 / scale;
    let origin = (rect.center() - Vec2::splat(size * 0.5)).to_vec2() * scale;
    let origin = Pos2::new(origin.x.round() / scale, origin.y.round() / scale);
    let pixels = Rect::from_min_size(origin, Vec2::splat(size));
    let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    painter.image(texture.id(), pixels, uv, color);
}

fn rasterize_optical(index: usize, side: usize, optical_16: bool) -> ColorImage {
    static SOURCE_24: std::sync::OnceLock<image::RgbaImage> = std::sync::OnceLock::new();
    static SOURCE_16: std::sync::OnceLock<image::RgbaImage> = std::sync::OnceLock::new();
    let (atlas, cell, glyph_size) = if optical_16 {
        (SOURCE_16.get_or_init(|| decode_atlas(ATLAS_16)), CELL_16, GLYPH_16)
    } else {
        (SOURCE_24.get_or_init(|| decode_atlas(ATLAS_24)), CELL_24, GLYPH_24)
    };
    let x = (index % COLUMNS * cell + PADDING) as u32;
    let y = (index / COLUMNS * cell + PADDING) as u32;
    let glyph = image::imageops::crop_imm(atlas, x, y, glyph_size as u32, glyph_size as u32).to_image();
    let alpha = image::GrayImage::from_fn(glyph_size as u32, glyph_size as u32, |gx, gy| {
        image::Luma([glyph[(gx, gy)][3]])
    });
    let mask = if side == glyph_size {
        alpha
    } else {
        image::imageops::resize(
            &alpha,
            side as u32,
            side as u32,
            image::imageops::FilterType::Triangle,
        )
    };
    let pixels = mask
        .pixels()
        .map(|pixel| Color32::from_white_alpha(pixel[0]))
        .collect();
    ColorImage::new([side, side], pixels)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum SpicyIcon {
    LyricsPage,
    PiPMode,
    CinemaView,
    NowBarLeft,
    NowBarRight,
    LyricsManager,
    NowBarSideSwapLeft,
    NowBarSideSwapRight,
    Fullscreen,
    CloseFullscreen,
    Settings,
    Close,
    SyncChevron,
    SyncChevronDown,
    UpNext,
    LyricsBubble,
    OpenPage,
    CardExpand,
    CardCollapse,
    CardToggleUp,
    CardToggleDown,
}

impl SpicyIcon {
    pub const ALL: &'static [SpicyIcon] = &[
        Self::LyricsPage,
        Self::PiPMode,
        Self::CinemaView,
        Self::NowBarLeft,
        Self::NowBarRight,
        Self::LyricsManager,
        Self::NowBarSideSwapLeft,
        Self::NowBarSideSwapRight,
        Self::Fullscreen,
        Self::CloseFullscreen,
        Self::Settings,
        Self::Close,
        Self::SyncChevron,
        Self::SyncChevronDown,
        Self::UpNext,
        Self::LyricsBubble,
        Self::OpenPage,
        Self::CardExpand,
        Self::CardCollapse,
        Self::CardToggleUp,
        Self::CardToggleDown,
    ];
}

/// Paints an exact `Icons.ts` vector glyph rasterized with 8x8 supersampling at physical pixel resolution.
pub fn paint_spicy(painter: &Painter, rect: Rect, icon: SpicyIcon, color: Color32) {
    let ctx = painter.ctx();
    let scale = ctx.pixels_per_point();
    let logical = rect.width().min(rect.height());
    let side = (logical * scale).round().clamp(8.0, 96.0) as usize;
    let id = Id::unique(("nhip-spicy-icon", icon, side));
    let texture = ctx
        .data_mut(|data| data.get_temp::<TextureHandle>(id))
        .unwrap_or_else(|| {
            let texture = ctx.load_texture(
                "nhip spicy icon",
                rasterize_spicy(icon, side),
                TextureOptions::LINEAR,
            );
            ctx.data_mut(|data| data.insert_temp(id, texture.clone()));
            texture
        });
    let size = side as f32 / scale;
    let origin = (rect.center() - Vec2::splat(size * 0.5)).to_vec2() * scale;
    let origin = Pos2::new(origin.x.round() / scale, origin.y.round() / scale);
    let pixels = Rect::from_min_size(origin, Vec2::splat(size));
    painter.image(
        texture.id(),
        pixels,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        color,
    );
}

fn dist_seg(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let (wx, wy) = (p.0 - a.0, p.1 - a.1);
    let len2 = vx * vx + vy * vy;
    let t = if len2 > 1e-6 {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    ((wx - t * vx).powi(2) + (wy - t * vy).powi(2)).sqrt()
}

fn dist_polyline(p: (f32, f32), pts: &[(f32, f32)]) -> f32 {
    pts.windows(2)
        .map(|w| dist_seg(p, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

fn sd_round_box(p: (f32, f32), x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    let qx = (p.0 - cx).abs() - (w * 0.5 - r);
    let qy = (p.1 - cy).abs() - (h * 0.5 - r);
    let out = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    out + qx.max(qy).min(0.0) - r
}

fn dist_round_rect_stroke(p: (f32, f32), x: f32, y: f32, w: f32, h: f32, r: f32) -> f32 {
    sd_round_box(p, x, y, w, h, r).abs()
}

fn dist_quarter_arc(p: (f32, f32), c: (f32, f32), r: f32, sx: f32, sy: f32) -> f32 {
    let dx = (p.0 - c.0) * sx;
    let dy = (p.1 - c.1) * sy;
    if dx >= 0.0 && dy >= 0.0 {
        ((dx * dx + dy * dy).sqrt() - r).abs()
    } else {
        let e1 = (c.0 + sx * r, c.1);
        let e2 = (c.0, c.1 + sy * r);
        ((p.0 - e1.0).powi(2) + (p.1 - e1.1).powi(2))
            .min((p.0 - e2.0).powi(2) + (p.1 - e2.1).powi(2))
            .sqrt()
    }
}

fn parse_lyrics_page_subpaths() -> Vec<Vec<(f32, f32)>> {
    // Exact verbatim SVG path data from Spicy Lyrics `Icons.LyricsPage` (`Icons.ts:29-33`).
    const PATHS: [&str; 5] = [
        "M18.9962 5.00357C18.5208 4.52802 17.9233 4.19298 17.2696 4.03541C16.6159 3.87784 15.9313 3.90387 15.2915 4.11061C14.6516 4.31735 14.0813 4.69678 13.6433 5.20705C13.2054 5.71733 12.9169 6.33862 12.8097 7.00242L16.9973 11.1897C17.6611 11.0825 18.2824 10.794 18.7927 10.3561C19.303 9.91824 19.6825 9.34793 19.8893 8.7081C20.096 8.06827 20.122 7.38377 19.9645 6.73009C19.8069 6.07641 19.4718 5.47894 18.9962 5.00357ZM15.2227 12.153L11.845 8.77431C10.4947 10.3119 9.14443 11.8495 7.79421 13.3871L4.34436 17.3139C4.06732 17.6308 3.92106 18.0412 3.93518 18.4618C3.94929 18.8825 4.12273 19.2821 4.42039 19.5798C4.71804 19.8774 5.11767 20.0508 5.53838 20.0649C5.9591 20.0791 6.36945 19.9328 6.68639 19.6558L10.6328 16.1894L15.224 12.1543L15.2227 12.153ZM10.8636 6.96374C10.9806 5.9186 11.3904 4.92775 12.0457 4.10518C12.701 3.28261 13.5752 2.66176 14.5678 2.31407C15.5604 1.96638 16.631 1.90598 17.6564 2.13981C18.6818 2.37365 19.6203 2.89222 20.364 3.63586C21.1077 4.3795 21.6263 5.31798 21.8602 6.34331C22.094 7.36865 22.0336 8.43917 21.6859 9.43169C21.3382 10.4242 20.7173 11.2984 19.8947 11.9537C19.0721 12.6089 18.0811 13.0187 17.0359 13.1357L11.9108 17.6402L7.96445 21.1079C7.27835 21.7096 6.38902 22.0279 5.47687 21.9981C4.56473 21.9683 3.69808 21.5926 3.05275 20.9473C2.40742 20.302 2.03174 19.4354 2.00192 18.5234C1.97211 17.6113 2.29039 16.722 2.8922 16.0359L6.34334 12.1092L10.8636 6.96374Z",
        "M8.35932 0.380176C8.40765 0.249583 8.59235 0.249583 8.64068 0.380176L9.15129 1.76009C9.16648 1.80114 9.19886 1.83352 9.23991 1.84871L10.6198 2.35932C10.7504 2.40765 10.7504 2.59235 10.6198 2.64068L9.23991 3.15129C9.19886 3.16648 9.16648 3.19886 9.15129 3.23991L8.64068 4.61982C8.59235 4.75042 8.40765 4.75042 8.35932 4.61982L7.84871 3.23991C7.83352 3.19886 7.80114 3.16648 7.76009 3.15129L6.38018 2.64068C6.24958 2.59235 6.24958 2.40765 6.38018 2.35932L7.76009 1.84871C7.80114 1.83352 7.83352 1.80114 7.84871 1.76009L8.35932 0.380176Z",
        "M19.8593 14.3802C19.9076 14.2496 20.0924 14.2496 20.1407 14.3802L21.0564 16.855C21.0716 16.896 21.104 16.9284 21.145 16.9436L23.6198 17.8593C23.7504 17.9076 23.7504 18.0924 23.6198 18.1407L21.145 19.0564C21.104 19.0716 21.0716 19.104 21.0564 19.145L20.1407 21.6198C20.0924 21.7504 19.9076 21.7504 19.8593 21.6198L18.9436 19.145C18.9284 19.104 18.896 19.0716 18.855 19.0564L16.3802 18.1407C16.2496 18.0924 16.2496 17.9076 16.3802 17.8593L18.855 16.9436C18.896 16.9284 18.9284 16.896 18.9436 16.855L19.8593 14.3802Z",
        "M13.3593 18.3802C13.4076 18.2496 13.5924 18.2496 13.6407 18.3802L14.1513 19.7601C14.1665 19.8011 14.1989 19.8335 14.2399 19.8487L15.6198 20.3593C15.7504 20.4076 15.7504 20.5924 15.6198 20.6407L14.2399 21.1513C14.1989 21.1665 14.1665 21.1989 14.1513 21.2399L13.6407 22.6198C13.5924 22.7504 13.4076 22.7504 13.3593 22.6198L12.8487 21.2399C12.8335 21.1989 12.8011 21.1665 12.7601 21.1513L11.3802 20.6407C11.2496 20.5924 11.2496 20.4076 11.3802 20.3593L12.7601 19.8487C12.8011 19.8335 12.8335 19.8011 12.8487 19.7601L13.3593 18.3802Z",
        "M3.85932 3.38018C3.90765 3.24958 4.09235 3.24958 4.14068 3.38018L5.05643 5.85495C5.07162 5.89601 5.10399 5.92838 5.14505 5.94357L7.61982 6.85932C7.75042 6.90765 7.75042 7.09235 7.61982 7.14068L5.14505 8.05643C5.10399 8.07162 5.07162 8.10399 5.05643 8.14505L4.14068 10.6198C4.09235 10.7504 3.90765 10.7504 3.85932 10.6198L2.94357 8.14505C2.92838 8.10399 2.89601 8.07162 2.85495 8.05643L0.380176 7.14068C0.249583 7.09235 0.249583 6.90765 0.380176 6.85932L2.85495 5.94357C2.89601 5.92838 2.92838 5.89601 2.94357 5.85495L3.85932 3.38018Z",
    ];
    let mut subpaths = Vec::new();
    for d in PATHS {
        let bytes = d.as_bytes();
        let mut i = 0;
        let mut cur = (0.0_f32, 0.0_f32);
        let mut start = (0.0_f32, 0.0_f32);
        let mut poly: Vec<(f32, f32)> = Vec::new();
        let mut cmd = b'M';
        let skip_sep = |idx: &mut usize| {
            while *idx < bytes.len() && matches!(bytes[*idx], b' ' | b',' | b'\t' | b'\n' | b'\r') {
                *idx += 1;
            }
        };
        let read_num = |idx: &mut usize| -> f32 {
            skip_sep(idx);
            let s = *idx;
            if *idx < bytes.len() && matches!(bytes[*idx], b'+' | b'-') {
                *idx += 1;
            }
            let mut dot = false;
            while *idx < bytes.len() {
                let b = bytes[*idx];
                if b.is_ascii_digit() {
                    *idx += 1;
                } else if b == b'.' && !dot {
                    dot = true;
                    *idx += 1;
                } else {
                    break;
                }
            }
            d[s..*idx].parse::<f32>().unwrap_or(0.0)
        };
        while i < bytes.len() {
            skip_sep(&mut i);
            if i >= bytes.len() {
                break;
            }
            if bytes[i].is_ascii_alphabetic() {
                cmd = bytes[i];
                i += 1;
            }
            match cmd {
                b'M' => {
                    if poly.len() >= 3 {
                        if poly.first() != poly.last() {
                            poly.push(start);
                        }
                        subpaths.push(std::mem::take(&mut poly));
                    } else {
                        poly.clear();
                    }
                    cur = (read_num(&mut i), read_num(&mut i));
                    start = cur;
                    poly.push(cur);
                    cmd = b'L';
                }
                b'L' => {
                    cur = (read_num(&mut i), read_num(&mut i));
                    poly.push(cur);
                }
                b'C' => {
                    let p0 = cur;
                    let p1 = (read_num(&mut i), read_num(&mut i));
                    let p2 = (read_num(&mut i), read_num(&mut i));
                    let p3 = (read_num(&mut i), read_num(&mut i));
                    for step in 1..=10 {
                        let t = step as f32 / 10.0;
                        let u = 1.0 - t;
                        let x = u * u * u * p0.0
                            + 3.0 * u * u * t * p1.0
                            + 3.0 * u * t * t * p2.0
                            + t * t * t * p3.0;
                        let y = u * u * u * p0.1
                            + 3.0 * u * u * t * p1.1
                            + 3.0 * u * t * t * p2.1
                            + t * t * t * p3.1;
                        poly.push((x, y));
                    }
                    cur = p3;
                }
                b'Z' | b'z' => {
                    if poly.len() >= 3 {
                        if poly.first() != poly.last() {
                            poly.push(start);
                        }
                        subpaths.push(std::mem::take(&mut poly));
                    } else {
                        poly.clear();
                    }
                    cur = start;
                }
                _ => break,
            }
        }
        if poly.len() >= 3 {
            if poly.first() != poly.last() {
                poly.push(start);
            }
            subpaths.push(poly);
        }
    }
    subpaths
}

fn point_in_subpaths_nonzero(p: (f32, f32), subpaths: &[Vec<(f32, f32)>]) -> bool {
    let mut winding = 0i32;
    for poly in subpaths {
        for w in poly.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a.1 <= p.1 {
                if b.1 > p.1 && (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1) > 0.0 {
                    winding += 1;
                }
            } else if b.1 <= p.1 && (b.0 - a.0) * (p.1 - a.1) - (p.0 - a.0) * (b.1 - a.1) < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

fn settings_gear_poly() -> &'static Vec<(f32, f32)> {
    static GEAR: std::sync::OnceLock<Vec<(f32, f32)>> = std::sync::OnceLock::new();
    GEAR.get_or_init(|| {
        // Exact 8-tooth outline gear from `Icons.Settings` (`Icons.ts:163`, r_outer=8.75, r_inner=6.60).
        const BASE: [(f32, f32, f32); 4] = [
            (10.07, 5.69, 0.0),
            (10.48, 3.38, 8.75),
            (13.52, 3.38, 0.0),
            (13.93, 5.69, 6.60),
        ];
        let mut pts = Vec::with_capacity(65);
        let mut raw_teeth = Vec::with_capacity(32);
        for k in 0..8 {
            let ang = k as f32 * std::f32::consts::FRAC_PI_4;
            let (s, c) = ang.sin_cos();
            for &(bx, by, arc_r) in &BASE {
                let dx = bx - 12.0;
                let dy = by - 12.0;
                raw_teeth.push((12.0 + dx * c - dy * s, 12.0 + dx * s + dy * c, arc_r));
            }
        }
        for idx in 0..raw_teeth.len() {
            let (x0, y0, arc_r) = raw_teeth[idx];
            let (x1, y1, _) = raw_teeth[(idx + 1) % raw_teeth.len()];
            pts.push((x0, y0));
            if arc_r > 0.0 {
                let a0 = (y0 - 12.0).atan2(x0 - 12.0);
                let mut a1 = (y1 - 12.0).atan2(x1 - 12.0);
                if a1 < a0 {
                    a1 += std::f32::consts::TAU;
                }
                for s_i in 1..4 {
                    let a = a0 + (a1 - a0) * (s_i as f32 / 4.0);
                    pts.push((12.0 + arc_r * a.cos(), 12.0 + arc_r * a.sin()));
                }
            }
        }
        if let Some(&first) = pts.first() {
            pts.push(first);
        }
        pts
    })
}

fn stroke_dist_24(icon: SpicyIcon, p: (f32, f32)) -> (f32, f32) {
    // Returns `(distance_to_centerline_in_24_space, half_stroke_width_in_24_space)`.
    match icon {
        SpicyIcon::CinemaView => {
            let d = dist_round_rect_stroke(p, 3.0, 4.5, 18.0, 15.0, 3.0)
                .min(dist_polyline(p, &[(6.6, 10.0), (6.6, 7.5), (9.1, 7.5)]))
                .min(dist_polyline(p, &[(17.4, 14.0), (17.4, 16.5), (14.9, 16.5)]));
            (d, 0.98)
        }
        SpicyIcon::NowBarLeft | SpicyIcon::NowBarRight => {
            let right = icon == SpicyIcon::NowBarRight;
            let (cx, tx, lx) = if right { (12.0, 13.5, 3.0) } else { (3.0, 4.5, 15.5) };
            let d = dist_round_rect_stroke(p, cx, 5.0, 9.0, 9.0, 2.0)
                .min(dist_seg(p, (tx, 18.5), (tx + 6.0, 18.5)))
                .min(dist_seg(p, (lx, 7.5), (lx + 5.0, 7.5)))
                .min(dist_seg(p, (lx, 11.5), (lx + 5.0, 11.5)))
                .min(dist_seg(p, (lx, 15.5), (lx + 3.0, 15.5)));
            (d, 0.98)
        }
        SpicyIcon::LyricsManager => {
            // Back sheet (`M8 3 h8.5 A2.5 2.5 0 0 1 19 5.5 V16`) + Front sheet (`rect x=5 y=6 w=11 h=15 rx=2.5`) + 3 lines
            let back = dist_seg(p, (8.0, 3.0), (16.5, 3.0))
                .min(dist_quarter_arc(p, (16.5, 5.5), 2.5, 1.0, -1.0))
                .min(dist_seg(p, (19.0, 5.5), (19.0, 16.0)));
            let d = back
                .min(dist_round_rect_stroke(p, 5.0, 6.0, 11.0, 15.0, 2.5))
                .min(dist_seg(p, (8.0, 10.5), (13.0, 10.5)))
                .min(dist_seg(p, (8.0, 14.0), (13.0, 14.0)))
                .min(dist_seg(p, (8.0, 17.5), (11.0, 17.5)));
            (d, 0.98)
        }
        SpicyIcon::NowBarSideSwapLeft | SpicyIcon::NowBarSideSwapRight => {
            let right = icon == SpicyIcon::NowBarSideSwapRight;
            let frame = dist_round_rect_stroke(p, 3.0, 4.0, 18.0, 16.0, 3.0);
            let inner = if right {
                dist_round_rect_stroke(p, 13.0, 9.5, 5.0, 5.0, 1.25)
                    .min(dist_seg(p, (10.0, 12.0), (6.0, 12.0)))
                    .min(dist_polyline(p, &[(7.75, 10.25), (6.0, 12.0), (7.75, 13.75)]))
            } else {
                dist_round_rect_stroke(p, 6.0, 9.5, 5.0, 5.0, 1.25)
                    .min(dist_seg(p, (14.0, 12.0), (18.0, 12.0)))
                    .min(dist_polyline(p, &[(16.25, 10.25), (18.0, 12.0), (16.25, 13.75)]))
            };
            (frame.min(inner), 0.96)
        }
        SpicyIcon::Fullscreen => {
            let tl = dist_seg(p, (3.5, 8.5), (3.5, 6.0))
                .min(dist_quarter_arc(p, (6.0, 6.0), 2.5, -1.0, -1.0))
                .min(dist_seg(p, (6.0, 3.5), (8.5, 3.5)));
            let tr = dist_seg(p, (15.5, 3.5), (18.0, 3.5))
                .min(dist_quarter_arc(p, (18.0, 6.0), 2.5, 1.0, -1.0))
                .min(dist_seg(p, (20.5, 6.0), (20.5, 8.5)));
            let br = dist_seg(p, (20.5, 15.5), (20.5, 18.0))
                .min(dist_quarter_arc(p, (18.0, 18.0), 2.5, 1.0, 1.0))
                .min(dist_seg(p, (18.0, 20.5), (15.5, 20.5)));
            let bl = dist_seg(p, (8.5, 20.5), (6.0, 20.5))
                .min(dist_quarter_arc(p, (6.0, 18.0), 2.5, -1.0, 1.0))
                .min(dist_seg(p, (3.5, 18.0), (3.5, 15.5)));
            (tl.min(tr).min(br).min(bl), 1.0)
        }
        SpicyIcon::CloseFullscreen => {
            let tl = dist_seg(p, (8.5, 3.5), (8.5, 6.0))
                .min(dist_quarter_arc(p, (6.0, 6.0), 2.5, 1.0, 1.0))
                .min(dist_seg(p, (6.0, 8.5), (3.5, 8.5)));
            let tr = dist_seg(p, (20.5, 8.5), (18.0, 8.5))
                .min(dist_quarter_arc(p, (18.0, 6.0), 2.5, -1.0, 1.0))
                .min(dist_seg(p, (15.5, 6.0), (15.5, 3.5)));
            let br = dist_seg(p, (15.5, 20.5), (15.5, 18.0))
                .min(dist_quarter_arc(p, (18.0, 18.0), 2.5, -1.0, -1.0))
                .min(dist_seg(p, (18.0, 15.5), (20.5, 15.5)));
            let bl = dist_seg(p, (3.5, 15.5), (6.0, 15.5))
                .min(dist_quarter_arc(p, (6.0, 18.0), 2.5, 1.0, -1.0))
                .min(dist_seg(p, (8.5, 18.0), (8.5, 20.5)));
            (tl.min(tr).min(br).min(bl), 1.0)
        }
        SpicyIcon::Settings => {
            let gear = dist_polyline(p, settings_gear_poly());
            let hub = (((p.0 - 12.0).powi(2) + (p.1 - 12.0).powi(2)).sqrt() - 2.75).abs();
            (gear.min(hub), 0.92)
        }
        SpicyIcon::Close => {
            let d = dist_seg(p, (5.5, 5.5), (18.5, 18.5)).min(dist_seg(p, (18.5, 5.5), (5.5, 18.5)));
            (d, 1.0)
        }
        SpicyIcon::SyncChevron => {
            let d = dist_polyline(p, &[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]);
            (d, 1.05)
        }
        SpicyIcon::SyncChevronDown => {
            let d = dist_polyline(p, &[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]);
            (d, 1.05)
        }
        SpicyIcon::OpenPage => {
            // `Icons.OpenPage` (`Icons.ts:70-72`): `<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M15.5 4v16"/><path d="m10.5 8.5-3.5 3.5 3.5 3.5"/>`
            let frame = dist_round_rect_stroke(p, 3.0, 4.0, 18.0, 16.0, 3.0);
            let bar = dist_seg(p, (15.5, 4.0), (15.5, 20.0));
            let chev = dist_polyline(p, &[(10.5, 8.5), (7.0, 12.0), (10.5, 15.5)]);
            (frame.min(bar).min(chev), 0.98)
        }
        SpicyIcon::CardExpand => {
            // `Icons.CardExpand` (`Icons.ts:126-127`): `<path d="m8 9.5 4-4 4 4"/><path d="m8 14.5 4 4 4-4"/>`
            let top = dist_polyline(p, &[(8.0, 9.5), (12.0, 5.5), (16.0, 9.5)]);
            let bot = dist_polyline(p, &[(8.0, 14.5), (12.0, 18.5), (16.0, 14.5)]);
            (top.min(bot), 1.0)
        }
        SpicyIcon::CardCollapse => {
            // `Icons.CardExpand` inward stack (`Icons.ts:128-129`): `<path d="m8 5.5 4 4 4-4"/><path d="m8 18.5 4-4 4 4"/>`
            let top = dist_polyline(p, &[(8.0, 5.5), (12.0, 9.5), (16.0, 5.5)]);
            let bot = dist_polyline(p, &[(8.0, 18.5), (12.0, 14.5), (16.0, 18.5)]);
            (top.min(bot), 1.0)
        }
        SpicyIcon::CardToggleUp => {
            // `Icons.CardToggle` (`Icons.ts:123`): `<path d="m6 15 6-6 6 6"/>`
            let d = dist_polyline(p, &[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]);
            (d, 1.0)
        }
        SpicyIcon::CardToggleDown => {
            let d = dist_polyline(p, &[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]);
            (d, 1.0)
        }
        SpicyIcon::UpNext => {
            let tri = dist_polyline(p, &[(4.5, 5.8), (8.2, 8.0), (4.5, 10.2), (4.5, 5.8)]);
            let bars = dist_seg(p, (11.0, 8.0), (20.0, 8.0))
                .min(dist_seg(p, (4.5, 13.2), (20.0, 13.2)))
                .min(dist_seg(p, (4.5, 17.8), (15.5, 17.8)));
            (tri.min(bars), 0.98)
        }
        SpicyIcon::LyricsBubble => {
            let body = dist_round_rect_stroke(p, 4.0, 4.5, 16.0, 12.2, 3.8);
            let tail = dist_polyline(p, &[(8.2, 16.7), (6.5, 19.8), (11.2, 16.7)]);
            let lines = dist_seg(p, (8.0, 9.0), (16.0, 9.0)).min(dist_seg(p, (8.0, 12.5), (13.5, 12.5)));
            (body.min(tail).min(lines), 0.96)
        }
        SpicyIcon::LyricsPage | SpicyIcon::PiPMode => (f32::INFINITY, 0.0),
    }
}

fn rasterize_spicy(icon: SpicyIcon, side: usize) -> ColorImage {
    static LYRICS_SUBPATHS: std::sync::OnceLock<Vec<Vec<(f32, f32)>>> = std::sync::OnceLock::new();
    const SS: usize = 8;
    let inv_side = 1.0 / side.max(1) as f32;
    let mut pixels = Vec::with_capacity(side * side);

    match icon {
        SpicyIcon::LyricsPage => {
            let subpaths = LYRICS_SUBPATHS.get_or_init(parse_lyrics_page_subpaths);
            for py in 0..side {
                for px in 0..side {
                    let mut hits = 0usize;
                    for sy in 0..SS {
                        let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * inv_side * 24.0;
                        for sx in 0..SS {
                            let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * inv_side * 24.0;
                            if point_in_subpaths_nonzero((x, y), subpaths) {
                                hits += 1;
                            }
                        }
                    }
                    let a = ((hits as f32 / (SS * SS) as f32) * 255.0).round() as u8;
                    pixels.push(Color32::from_white_alpha(a));
                }
            }
        }
        SpicyIcon::PiPMode => {
            // Exact `Icons.PiPMode` (`Icons.ts:160`, `viewBox="0 0 16 16"`).
            for py in 0..side {
                for px in 0..side {
                    let mut hits = 0usize;
                    for sy in 0..SS {
                        let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * inv_side * 16.0;
                        for sx in 0..SS {
                            let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * inv_side * 16.0;
                            let outer = sd_round_box((x, y), 0.0, 1.0, 16.0, 14.0, 1.45) <= 0.0
                                && !(x >= 1.5 && x <= 14.5 && y >= 2.5 && y <= 13.5)
                                && !(x > 7.007 && y > 7.0);
                            let inset = sd_round_box((x, y), 8.575, 9.007, 7.425, 5.993, 0.75) <= 0.0;
                            if outer || inset {
                                hits += 1;
                            }
                        }
                    }
                    let a = ((hits as f32 / (SS * SS) as f32) * 255.0).round() as u8;
                    pixels.push(Color32::from_white_alpha(a));
                }
            }
        }
        _ => {
            let aa = (24.0 * inv_side * 0.55).max(0.18);
            for py in 0..side {
                let y = (py as f32 + 0.5) * inv_side * 24.0;
                for px in 0..side {
                    let x = (px as f32 + 0.5) * inv_side * 24.0;
                    let (d, half_w) = stroke_dist_24(icon, (x, y));
                    let t = ((half_w + aa - d) / (2.0 * aa)).clamp(0.0, 1.0);
                    let cov = t * t * (3.0 - 2.0 * t);
                    pixels.push(Color32::from_white_alpha((cov * 255.0).round() as u8));
                }
            }
        }
    }

    ColorImage::new([side, side], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_has_a_glyph_for_each_icon() {
        let atlas_24 = decode_atlas(ATLAS_24);
        let atlas_16 = decode_atlas(ATLAS_16);
        assert_eq!(Icon::ALL.len(), Icon::FullscreenExit as usize);
        for &icon in Icon::ALL {
            let index = icon as usize;
            for (atlas, cell, glyph_size, label) in [
                (&atlas_24, CELL_24, GLYPH_24, "24px"),
                (&atlas_16, CELL_16, GLYPH_16, "16px"),
            ] {
                let x = (index % COLUMNS * cell + PADDING) as u32;
                let y = (index / COLUMNS * cell + PADDING) as u32;
                let glyph =
                    image::imageops::crop_imm(atlas, x, y, glyph_size as u32, glyph_size as u32)
                        .to_image();
                assert!(
                    glyph.pixels().any(|pixel| pixel[3] > 0),
                    "missing {label} glyph for {icon:?} (index {index})"
                );
                assert!(
                    glyph.pixels().any(|pixel| pixel[3] == 0),
                    "expected transparent margin in {label} glyph for {icon:?} (index {index})"
                );
            }
        }
    }

    #[test]
    fn small_and_medium_icons_keep_antialiased_edges() {
        for &icon in Icon::ALL {
            for (side, optical_16) in [
                (14, true),
                (16, true),
                (18, true),
                (20, true),
                (24, false),
                (30, false),
                (32, false),
            ] {
                let image = rasterize_optical(icon as usize, side, optical_16);
                assert_eq!(image.size, [side, side]);
                assert!(
                    image.pixels.iter().any(|pixel| (16..240).contains(&pixel.a())),
                    "missing antialiased edge alpha for {icon:?} at {side}px (optical_16={optical_16})"
                );
                assert!(
                    image.pixels.iter().any(|pixel| pixel.a() == 0),
                    "missing transparent background for {icon:?} at {side}px"
                );
            }
        }
    }

    #[test]
    fn repaired_glyphs_have_expected_coverage() {
        // Pause must have two distinct vertical bars with empty center gap
        for (side, optical_16) in [(16, true), (24, false)] {
            let pause = rasterize_optical(Icon::Pause as usize, side, optical_16);
            let mid_x = side / 2;
            let mid_y = side / 2;
            assert_eq!(pause.pixels[mid_y * side + mid_x].a(), 0);
            // VolumeLow has speaker + single wave (fewer lit pixels than Volume)
            let low = rasterize_optical(Icon::VolumeLow as usize, side, optical_16);
            let full = rasterize_optical(Icon::Volume as usize, side, optical_16);
            let lit = |img: &ColorImage| img.pixels.iter().filter(|p| p.a() > 64).count();
            assert!(lit(&low) < lit(&full), "VolumeLow should have fewer lit pixels than Volume");
        }
    }

    #[test]
    fn spicy_svg_icons_rasterize_cleanly_and_connect_arcs() {
        for &ic in SpicyIcon::ALL {
            let img = rasterize_spicy(ic, 24);
            assert_eq!(img.size, [24, 24]);
            assert!(img.pixels.iter().any(|p| p.a() > 200), "missing ink for {ic:?}");
            assert!(img.pixels.iter().any(|p| p.a() == 0), "missing margin for {ic:?}");
            assert!(img.pixels.iter().any(|p| (16..240).contains(&p.a())), "missing AA for {ic:?}");
        }
        // LyricsManager top-right arc `(18.2, 3.8)` must be connected (fixes broken corner gap)
        let mgr = rasterize_spicy(SpicyIcon::LyricsManager, 48);
        let arc_a = mgr.pixels[7 * 48 + 36].a().max(mgr.pixels[8 * 48 + 37].a());
        assert!(arc_a > 140, "expected connected top-right arc on LyricsManager, got alpha={arc_a}");

        // LyricsPage outline microphone (hollow bulb & hollow handle + collar + 4 curved stars)
        let mic = rasterize_spicy(SpicyIcon::LyricsPage, 48);
        for (ux, uy, label) in [
            (21.0, 7.0, "mic bulb rim"),
            (14.0, 9.5, "mic diagonal collar"),
            (3.0, 18.5, "mic handle tip rim"),
            (8.5, 2.5, "top-left small star"),
            (4.0, 7.0, "left star"),
            (20.0, 18.0, "bottom-right star"),
            (13.5, 20.5, "bottom small star"),
        ] {
            let px = (ux * 2.0) as usize;
            let py = (uy * 2.0) as usize;
            assert!(mic.pixels[py * 48 + px].a() > 160, "missing {label} in LyricsPage");
        }
        // Bulb interior `(16.5, 7.5)` and handle interior `(6.0, 18.0)` must remain hollow
        assert_eq!(mic.pixels[15 * 48 + 33].a(), 0, "mic bulb interior should be hollow");
        assert_eq!(mic.pixels[36 * 48 + 12].a(), 0, "mic handle interior should be hollow");
    }
}
