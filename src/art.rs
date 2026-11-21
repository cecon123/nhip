//! Cover pictures: read off a worker thread, generated placeholders until they arrive.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use lofty::prelude::*;

const SIZE: u32 = 544;

pub use crate::library::cover_key;

struct Decoded {
    key: String,
    image: Option<ColorImage>,
    backdrop: Option<ColorImage>,
    colors: Option<[Color32; 4]>,
}

pub struct ArtCache {
    textures: HashMap<String, TextureHandle>,
    backdrops: HashMap<String, TextureHandle>,
    colors: HashMap<String, [Color32; 4]>,
    placeholders: HashMap<String, TextureHandle>,
    placeholder_backdrops: HashMap<String, TextureHandle>,
    requested: HashSet<String>,
    pending: usize,
    jobs: Sender<(String, String)>,
    results: Receiver<Decoded>,
}

impl ArtCache {
    pub fn new(ctx: egui::Context) -> Self {
        let (jobs, rx) = mpsc::channel::<(String, String)>();
        let (tx, results) = mpsc::channel();
        std::thread::Builder::new()
            .name("art".into())
            .spawn(move || {
                while let Ok((key, path)) = rx.recv() {
                    let image = read_cover(&path);
                    let (backdrop, colors) = match &image {
                        Some(img) => (Some(spicy_backdrop_image(img)), Some(cover_colors(img))),
                        None => (None, None),
                    };
                    if tx.send(Decoded { key, image, backdrop, colors }).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                }
            })
            .expect("art thread");
        Self {
            textures: HashMap::new(),
            backdrops: HashMap::new(),
            colors: HashMap::new(),
            placeholders: HashMap::new(),
            placeholder_backdrops: HashMap::new(),
            requested: HashSet::new(),
            pending: 0,
            jobs,
            results,
        }
    }

    pub fn pump(&mut self, ctx: &egui::Context) {
        const MAX_UPLOADS_PER_FRAME: usize = 4;
        let mut uploaded = 0usize;
        while uploaded < MAX_UPLOADS_PER_FRAME {
            let Ok(d) = self.results.try_recv() else { break };
            self.pending = self.pending.saturating_sub(1);
            if let Some(img) = d.image {
                let colors = d.colors.unwrap_or_else(|| cover_colors(&img));
                let bd_img = d.backdrop.unwrap_or_else(|| spicy_backdrop_image(&img));
                self.insert_precomputed(ctx, &d.key, img, bd_img, colors);
                uploaded += 1;
            }
        }
        if uploaded == MAX_UPLOADS_PER_FRAME {
            ctx.request_repaint();
        }
    }

    fn insert_precomputed(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        img: ColorImage,
        bd_img: ColorImage,
        colors: [Color32; 4],
    ) {
        self.colors.insert(key.to_string(), colors);
        let bd_tex = ctx.load_texture(format!("backdrop:{key}"), bd_img, TextureOptions::LINEAR);
        self.backdrops.insert(key.to_string(), bd_tex);
        let tex = ctx.load_texture(format!("cover:{key}"), img, TextureOptions::LINEAR);
        self.textures.insert(key.to_string(), tex);
    }

    pub fn inject_image(&mut self, ctx: &egui::Context, key: &str, img: ColorImage) {
        let colors = cover_colors(&img);
        let bd_img = spicy_backdrop_image(&img);
        self.insert_precomputed(ctx, key, img, bd_img, colors);
    }

    pub fn loading(&self) -> bool {
        self.pending > 0
    }

    /// Starts decoding/fetching artwork on the background art thread without allocating a placeholder texture.
    pub fn prefetch(&mut self, key: &str, path: Option<&str>) {
        if self.textures.contains_key(key) {
            return;
        }
        if let Some(p) = path {
            if self.requested.insert(key.to_string()) && self.jobs.send((key.to_string(), p.to_string())).is_ok() {
                self.pending += 1;
            }
        }
    }

    /// The best picture available for `key` right now. `path` is a file whose tags may hold art.
    pub fn get(&mut self, ctx: &egui::Context, key: &str, path: Option<&str>) -> TextureHandle {
        if let Some(t) = self.textures.get(key) {
            return t.clone();
        }
        if let Some(p) = path {
            if self.requested.insert(key.to_string()) && self.jobs.send((key.to_string(), p.to_string())).is_ok() {
                self.pending += 1;
            }
        }
        if let Some(t) = self.placeholders.get(key) {
            return t.clone();
        }
        let tex = ctx.load_texture(format!("ph:{key}"), placeholder(key), TextureOptions::LINEAR);
        self.placeholders.insert(key.to_string(), tex.clone());
        tex
    }

    pub fn lyric_colors(&self, key: &str) -> [Color32; 4] {
        self.colors.get(key).copied().unwrap_or_else(|| {
            let (a, b) = palette(key);
            let (c, d) = palette(&format!("lyrics:{key}"));
            [a, c, b, d]
        })
    }

    /// Raw blurred album cover; Kawarp and CSS grading run in the GPU paint callback.
    pub fn backdrop_texture(&mut self, ctx: &egui::Context, key: &str, path: Option<&str>) -> TextureHandle {
        let _ = self.get(ctx, key, path);
        if let Some(t) = self.backdrops.get(key) {
            return t.clone();
        }
        if let Some(t) = self.placeholder_backdrops.get(key) {
            return t.clone();
        }
        let bd_img = spicy_backdrop_image(&placeholder(key));
        let tex = ctx.load_texture(format!("ph-bd:{key}"), bd_img, TextureOptions::LINEAR);
        self.placeholder_backdrops.insert(key.to_string(), tex.clone());
        tex
    }
}

/// Precomputes the raw blurred album texture once per cover.
pub fn spicy_backdrop_image(img: &ColorImage) -> ColorImage {
    const N: usize = 64;
    let [sw, sh] = img.size;
    let mut buf = vec![[0.08_f32, 0.09_f32, 0.14_f32]; N * N];
    if sw > 0 && sh > 0 && img.pixels.len() == sw * sh {
        for y in 0..N {
            for x in 0..N {
                let mut acc = [0.0f32; 3];
                for sy in 0..3 {
                    for sx in 0..3 {
                        let u = (x as f32 + (sx as f32 + 0.5) / 3.0) / N as f32;
                        let v = (y as f32 + (sy as f32 + 0.5) / 3.0) / N as f32;
                        let ix = ((u * sw as f32) as usize).min(sw - 1);
                        let iy = ((v * sh as f32) as usize).min(sh - 1);
                        let [r, g, b, _] = img.pixels[iy * sw + ix].to_srgba_unmultiplied();
                        acc[0] += r as f32 / 255.0;
                        acc[1] += g as f32 / 255.0;
                        acc[2] += b as f32 / 255.0;
                    }
                }
                buf[y * N + x] = [acc[0] / 9.0, acc[1] / 9.0, acc[2] / 9.0];
            }
        }
    }

    // 4 separable box-blur passes (r = 3 on 64x64) yield silky broad bokeh zones with zero
    // harsh cover creases while preserving rich multi-hue pockets for Kawarp's GPU bokeh pass.
    let mut scratch = vec![[0.0f32; 3]; N * N];
    let r: isize = 3;
    let span = (r * 2 + 1) as f32;
    for _pass in 0..4 {
        for y in 0..N {
            for x in 0..N {
                let mut acc = [0.0f32; 3];
                for k in -r..=r {
                    let sx = (x as isize + k).clamp(0, N as isize - 1) as usize;
                    let p = buf[y * N + sx];
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                }
                scratch[y * N + x] = [acc[0] / span, acc[1] / span, acc[2] / span];
            }
        }
        for y in 0..N {
            for x in 0..N {
                let mut acc = [0.0f32; 3];
                for k in -r..=r {
                    let sy = (y as isize + k).clamp(0, N as isize - 1) as usize;
                    let p = scratch[sy * N + x];
                    acc[0] += p[0];
                    acc[1] += p[1];
                    acc[2] += p[2];
                }
                buf[y * N + x] = [acc[0] / span, acc[1] / span, acc[2] / span];
            }
        }
    }

    // Grading follows the moving warp in the GPU output pass, never before it.
    let pixels = buf.into_iter().map(|rgb| {
        let [r, g, b] = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        Color32::from_rgb(r, g, b)
    }).collect();
    ColorImage::new([N, N], pixels)
}

fn cover_colors(img: &ColorImage) -> [Color32; 4] {
    let [w, h] = img.size;
    let mut colors = [Color32::from_rgb(24, 32, 56); 4];
    let mut scores = [0u64; 4];
    for y in (0..h).step_by((h / 16).max(1)) {
        for x in (0..w).step_by((w / 16).max(1)) {
            let color = img.pixels[y * w + x];
            let [r, g, b, a] = color.to_srgba_unmultiplied();
            let max = r.max(g).max(b) as u64;
            let min = r.min(g).min(b) as u64;
            let chroma = max - min;
            // Prefer rich saturated cover pigments (especially magenta/cobalt/emerald over muddy mid-tones)
            let bright_ok = if max < 18 { 1 } else if max > 235 && chroma < 52 { 8 } else { max + 18 };
            let magenta_bonus = if r > 180 && b > 110 && g < 110 { 2 } else { 1 };
            let score = (chroma * chroma + 16) * bright_ok * magenta_bonus * a as u64;
            let i = (2 * x / w) + (2 * y / h) * 2;
            if score > scores[i] {
                colors[i] = color;
                scores[i] = score;
            }
        }
    }
    if let Some((best, _)) = scores.iter().enumerate().max_by_key(|&(_, s)| *s) {
        colors.swap(0, best);
    }
    colors
}

/// Strips symmetric top/bottom black letterbox bars from 4:3 YouTube thumbnails (`hqdefault.jpg` 480x360)
/// and trims uniform low-variance edge border strips so thumbnails fill the square frame edge-to-edge.
pub fn strip_letterbox_bars(img: image::DynamicImage) -> image::DynamicImage {
    use image::GenericImageView;
    let (w, h) = img.dimensions();
    if w < 64 || h < 64 {
        return img;
    }
    let rgba = img.to_rgba8();
    let row_is_black = |y: u32| -> bool {
        let step = (w / 32).max(1);
        let mut dark = 0u32;
        let mut total = 0u32;
        let mut x = 0;
        while x < w {
            let p = rgba.get_pixel(x, y).0;
            if p[0].max(p[1]).max(p[2]) <= 24 {
                dark += 1;
            }
            total += 1;
            x += step;
        }
        dark * 20 >= total * 19
    };
    let max_bar = h / 5;
    let mut top = 0;
    while top < max_bar && row_is_black(top) {
        top += 1;
    }
    let mut bot = 0;
    while bot < max_bar && row_is_black(h - 1 - bot) {
        bot += 1;
    }
    let trim = top.min(bot);
    let img = if trim * 24 >= h && h > trim * 2 + 32 {
        img.crop_imm(0, trim, w, h - trim * 2)
    } else {
        img
    };

    // Also trim any low-variance solid/gradient margin strips along the 4 edges when the interior is vivid.
    let (w, h) = img.dimensions();
    let rgba = img.to_rgba8();
    let row_spread = |y: u32, x0: u32, x1: u32| -> u8 {
        let step = ((x1.saturating_sub(x0)) / 32).max(1);
        let (mut min_c, mut max_c) = ([255u8; 3], [0u8; 3]);
        let mut x = x0;
        while x < x1 {
            let p = rgba.get_pixel(x, y).0;
            for c in 0..3 {
                min_c[c] = min_c[c].min(p[c]);
                max_c[c] = max_c[c].max(p[c]);
            }
            x += step;
        }
        (0..3).map(|c| max_c[c] - min_c[c]).max().unwrap_or(0)
    };
    let col_spread = |x: u32, y0: u32, y1: u32| -> u8 {
        let step = ((y1.saturating_sub(y0)) / 32).max(1);
        let (mut min_c, mut max_c) = ([255u8; 3], [0u8; 3]);
        let mut y = y0;
        while y < y1 {
            let p = rgba.get_pixel(x, y).0;
            for c in 0..3 {
                min_c[c] = min_c[c].min(p[c]);
                max_c[c] = max_c[c].max(p[c]);
            }
            y += step;
        }
        (0..3).map(|c| max_c[c] - min_c[c]).max().unwrap_or(0)
    };
    let mid_row_spread = row_spread(h / 2, w / 6, w - w / 6);
    let mid_col_spread = col_spread(w / 2, h / 6, h - h / 6);
    if mid_row_spread < 130 && mid_col_spread < 130 {
        return img;
    }
    let max_edge_y = h / 8;
    let max_edge_x = w / 8;
    let mut t_edge = 0;
    while t_edge < max_edge_y && row_spread(t_edge, w / 10, w - w / 10) < 60 {
        t_edge += 1;
    }
    let mut b_edge = 0;
    while b_edge < max_edge_y && row_spread(h - 1 - b_edge, w / 10, w - w / 10) < 60 {
        b_edge += 1;
    }
    let y0 = t_edge;
    let y1 = h - b_edge;
    let mut l_edge = 0;
    while l_edge < max_edge_x && col_spread(l_edge, y0.max(h / 10), y1.min(h - h / 10)) < 115 {
        l_edge += 1;
    }
    let mut r_edge = 0;
    while r_edge < max_edge_x && col_spread(w - 1 - r_edge, y0.max(h / 10), y1.min(h - h / 10)) < 115 {
        r_edge += 1;
    }
    if (t_edge > 0 || b_edge > 0 || l_edge > 0 || r_edge > 0)
        && w > l_edge + r_edge + 64
        && h > t_edge + b_edge + 64
    {
        img.crop_imm(l_edge, t_edge, w - l_edge - r_edge, h - t_edge - b_edge)
    } else {
        img
    }
}

fn upgrade_art_url(url: &str) -> String {
    if url.contains("googleusercontent.com/") || url.contains("ggpht.com/") {
        if let Some(eq_idx) = url.rfind("=w").or_else(|| url.rfind("=s")) {
            return format!("{}=w544-h544-l90-rj", &url[..eq_idx]);
        }
    }
    url.to_owned()
}

fn download(url: &str) -> Option<ColorImage> {
    use std::io::Read;
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .expect("art HTTP client")
    });
    let hi_url = upgrade_art_url(url);
    let resp = client
        .get(&hi_url)
        .send()
        .ok()
        .filter(|r| r.status().is_success())
        .or_else(|| client.get(url).send().ok().filter(|r| r.status().is_success()))?;
    let mut bytes = Vec::new();
    resp.take(5_000_000).read_to_end(&mut bytes).ok()?;
    let decoded = strip_letterbox_bars(image::load_from_memory(&bytes).ok()?);
    let img = decoded
        .resize_to_fill(SIZE, SIZE, image::imageops::FilterType::CatmullRom)
        .to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([SIZE as usize, SIZE as usize], &img))
}

fn read_cover(path: &str) -> Option<ColorImage> {
    if path.starts_with("https://") {
        return download(path);
    }
    let p = Path::new(path);
    let tagged = lofty::read_from_path(p).ok();
    let embedded = tagged.as_ref().and_then(|t| {
        t.primary_tag()
            .or_else(|| t.first_tag())
            .and_then(|tag| tag.pictures().first())
            .map(|pic| pic.data().to_vec())
    });
    let bytes = embedded.or_else(|| {
        let dir = p.parent()?;
        ["cover.jpg", "cover.png", "folder.jpg", "folder.png", "album.jpg", "front.jpg"]
            .iter()
            .find_map(|n| std::fs::read(dir.join(n)).ok())
    })?;
    let img = strip_letterbox_bars(image::load_from_memory(&bytes).ok()?)
        .resize_to_fill(SIZE, SIZE, image::imageops::FilterType::CatmullRom)
        .to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img))
}

fn hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h = (h ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
    }
    h ^ (h >> 29)
}

/// A colour pair that is stable for a given name.
pub fn palette(key: &str) -> (Color32, Color32) {
    let h = hash(key);
    let hue = (h & 0xff) as f32 / 255.0;
    let top = hsv(hue, 0.65, 0.78);
    let bottom = hsv((hue + 0.12 + ((h >> 8) & 0x3f) as f32 / 255.0).fract(), 0.75, 0.32);
    (top, bottom)
}

pub fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - f * s), v * (1.0 - (1.0 - f) * s));
    let (r, g, b) = match i as i32 % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

fn mix(a: Color32, b: Color32, t: f32) -> [u8; 3] {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    [f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b())]
}

/// Clean Spotify-grade placeholder artwork:
/// - Artist keys (`ytm-artist*` / `artist*`): matte dark `#282828` with crisp centered Artist silhouette (`#7c7c7c`).
/// - Unloaded live network/playlist keys (`http*`, `yt:*`, `pl:*`): matte dark `#282828` with crisp centered Music Note (`#7c7c7c`).
/// - Local / demo covers without embedded art: smooth organic 3-point ambient mesh gradient (no crude overlapping circles).
fn placeholder(key: &str) -> ColorImage {
    const N: usize = 160;
    let is_artist = key.starts_with("ytm-artist") || key.starts_with("artist-");
    let is_neutral = is_artist
        || key.starts_with("http")
        || key.starts_with("yt:")
        || key.starts_with("pl:")
        || key.starts_with("url:");
    if is_neutral {
        let mut pixels = Vec::with_capacity(N * N);
        let f = N as f32;
        for y in 0..N {
            for x in 0..N {
                let u = (x as f32 + 0.5) / f;
                let v = (y as f32 + 0.5) / f;
                // Subtle matte radial depth (#2d2d2d center -> #242424 edge, Spotify `#282828` placeholder surface).
                let dist = ((u - 0.5).powi(2) + (v - 0.48).powi(2)).sqrt();
                let bg_l = (45.0 - dist * 13.0).clamp(35.0, 46.0);
                let glyph_cov = if is_artist {
                    // Clean Spotify user/artist bust silhouette (head circle + rounded shoulders arc).
                    let head_d = ((u - 0.5).powi(2) + (v - 0.37).powi(2)).sqrt() - 0.135;
                    let body_d = if v >= 0.55 {
                        (((u - 0.5) / 0.27).powi(2) + ((v - 0.78) / 0.21).powi(2)).sqrt() - 1.0
                    } else {
                        1.0
                    };
                    let head_a = (0.5 - head_d * f * 0.55).clamp(0.0, 1.0);
                    let body_a = if v < 0.80 { (0.5 - body_d * f * 0.14).clamp(0.0, 1.0) } else { 0.0 };
                    head_a.max(body_a)
                } else {
                    // Clean Spotify beamed eighth-note (`♫`) silhouette.
                    let n1 = (((u - 0.37) / 0.075).powi(2) + ((v - 0.65) / 0.058).powi(2)).sqrt() - 1.0;
                    let n2 = (((u - 0.61) / 0.075).powi(2) + ((v - 0.61) / 0.058).powi(2)).sqrt() - 1.0;
                    let stem1 = (u - 0.425).abs().max(0.0) - 0.022;
                    let stem1_ok = v >= 0.31 && v <= 0.65;
                    let stem2 = (u - 0.665).abs().max(0.0) - 0.022;
                    let stem2_ok = v >= 0.27 && v <= 0.61;
                    let beam_y = 0.32 - (u - 0.425) * 0.16;
                    let beam = (v - beam_y).abs() - 0.032;
                    let beam_ok = u >= 0.405 && u <= 0.685;
                    let a1 = (0.5 - n1 * f * 0.10).clamp(0.0, 1.0);
                    let a2 = (0.5 - n2 * f * 0.10).clamp(0.0, 1.0);
                    let s1 = if stem1_ok { (0.5 - stem1 * f * 0.6).clamp(0.0, 1.0) } else { 0.0 };
                    let s2 = if stem2_ok { (0.5 - stem2 * f * 0.6).clamp(0.0, 1.0) } else { 0.0 };
                    let bm = if beam_ok { (0.5 - beam * f * 0.5).clamp(0.0, 1.0) } else { 0.0 };
                    a1.max(a2).max(s1).max(s2).max(bm)
                };
                let c = (bg_l + (126.0 - bg_l) * glyph_cov).round().clamp(0.0, 255.0) as u8;
                pixels.push(Color32::from_rgb(c, c, c));
            }
        }
        return ColorImage::new([N, N], pixels);
    }

    // Smooth organic multi-point ambient mesh gradient (no sharp overlapping discs).
    let (top, bottom) = palette(key);
    let h = hash(key);
    let accent = hsv((((h & 0xff) as f32 / 255.0) + 0.08 + (((h >> 16) & 0x1f) as f32 / 180.0)).fract(), 0.72, 0.86);
    let (ax, ay) = (0.25 + ((h >> 24) & 0x3f) as f32 / 126.0, 0.25 + ((h >> 32) & 0x3f) as f32 / 126.0);
    let mut pixels = Vec::with_capacity(N * N);
    for y in 0..N {
        for x in 0..N {
            let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
            let diag = (u * 0.55 + v * 0.45).clamp(0.0, 1.0);
            let smooth = diag * diag * (3.0 - 2.0 * diag);
            let [r0, g0, b0] = mix(top, bottom, smooth);
            let d2 = (u - ax).powi(2) + (v - ay).powi(2);
            let w = (-d2 * 3.8).exp() * 0.42;
            let blend = |base: u8, hi: u8| ((base as f32) * (1.0 - w) + (hi as f32) * w).round().clamp(0.0, 255.0) as u8;
            let vig = 1.0 - (((u - 0.5).powi(2) + (v - 0.5).powi(2)) * 0.28);
            let r = (blend(r0, accent.r()) as f32 * vig).round().clamp(0.0, 255.0) as u8;
            let g = (blend(g0, accent.g()) as f32 * vig).round().clamp(0.0, 255.0) as u8;
            let b = (blend(b0, accent.b()) as f32 * vig).round().clamp(0.0, 255.0) as u8;
            pixels.push(Color32::from_rgb(r, g, b));
        }
    }
    ColorImage::new([N, N], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spicy_backdrop_image_blurs_and_preserves_saturated_cover_colors() {
        // Dark cobalt cover (similar to Bút Chì Bạc) should produce vivid blue tones, not crushed black
        let dark_cover = ColorImage::new([16, 16], vec![Color32::from_rgb(14, 34, 88); 256]);
        let bd_dark = spicy_backdrop_image(&dark_cover);
        assert_eq!(bd_dark.size, [64, 64]);
        let p = bd_dark.pixels[32 * 64 + 32];
        assert!(p.b() > p.r().saturating_add(40), "expected blue richness, got {p:?}");
        assert!((p.b() as i32 - 88).abs() <= 1, "raw cover must be graded by the GPU, got {p:?}");

    }

    #[test]
    fn spicy_backdrop_grays_follow_source_brightness_without_exposure_normalization() {
        for gray in [0, 32, 128, 255] {
            let cover = ColorImage::new([1, 1], vec![Color32::from_gray(gray)]);
            let backdrop = spicy_backdrop_image(&cover);
            let expected = gray;
            assert!(backdrop.pixels.iter().all(|p| *p == Color32::from_gray(expected)));
        }
    }

    #[test]
    fn strip_letterbox_bars_crops_youtube_4_3_black_margins() {
        let mut raw = image::RgbaImage::from_pixel(480, 360, image::Rgba([0, 0, 0, 255]));
        for y in 45..315 {
            for x in 0..480 {
                raw.put_pixel(x, y, image::Rgba([180, 95, 42, 255]));
            }
        }
        let cropped = strip_letterbox_bars(image::DynamicImage::ImageRgba8(raw));
        assert_eq!(cropped.width(), 480);
        assert_eq!(cropped.height(), 270);
    }

    #[test]
    fn artist_and_network_placeholders_use_matte_dark_surface_with_clean_glyph() {
        let artist_img = placeholder("ytm-artist:tam-den");
        let corner = artist_img.pixels[0];
        // Corners must be matte dark neutral gray (~#1e..#2c), never saturated overlapping color discs.
        assert!(corner.r() <= 48 && corner.g() <= 48 && corner.b() <= 48);
        assert!((corner.r() as i16 - corner.b() as i16).abs() <= 4);
        // Center silhouette head/torso must produce brighter neutral pixels (#68..#90).
        let brightest = artist_img.pixels.iter().map(|c| c.r()).max().unwrap_or(0);
        assert!(brightest >= 100 && brightest <= 160, "expected subtle silhouette glyph, got max={brightest}");

        let note_img = placeholder("yt:pending_vid");
        let note_corner = note_img.pixels[0];
        assert!(note_corner.r() <= 48 && note_corner.g() <= 48 && note_corner.b() <= 48);
        let note_bright = note_img.pixels.iter().map(|c| c.r()).max().unwrap_or(0);
        assert!(note_bright >= 100 && note_bright <= 160, "expected music note glyph, got max={note_bright}");
    }
}
