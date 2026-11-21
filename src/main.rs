#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod art;
#[cfg(windows)]
mod media;
mod theme;
mod ui;

use nhip_core::{audio, library, lrc, mechanics, queue, spicy, ttml, ytm};

use eframe::egui;
use std::path::PathBuf;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);

    let page = args.iter().position(|a| a == "--page").and_then(|i| args.get(i + 1)).cloned();

    // `--size 800x600` is for checking how the layout copes with small windows.
    let size = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.split_once('x'))
        .and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))
        .unwrap_or([1304.0, 724.0]);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Nhịp")
            .with_inner_size(size)
            .with_min_inner_size([800.0, 600.0])
            .with_decorations(false)
            .with_transparent(true)
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native("Nhịp", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, screenshot, page)))))
}

/// Renders the Nhịp "Mèo Sóng Âm" (Waveform Equalizer Cat) Liquid Glass vector emblem as RGBA bytes (`size x size`).
pub(crate) fn render_nhip_icon_rgba(size: usize) -> Vec<u8> {
    let mut rgba = vec![0u8; size * size * 4];
    let half = size as f32 * 0.5;
    let inv_half = 1.0 / half;

    // Continuous G² Apple squircle signed distance approximation (`|x|^p + |y|^p = r^p`, p = 4.6).
    let sd_squircle = |px: f32, py: f32, r: f32| -> f32 {
        let p = 4.6_f32;
        let ax = px.abs().max(1e-6);
        let ay = py.abs().max(1e-6);
        (ax.powf(p) + ay.powf(p)).powf(1.0 / p) - r
    };

    // Capsule SDF between points A and B with radius r.
    let sd_capsule = |px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32| -> f32 {
        let pax = px - ax;
        let pay = py - ay;
        let bax = bx - ax;
        let bay = by - ay;
        let h = ((pax * bax + pay * bay) / (bax * bax + bay * bay).max(1e-6)).clamp(0.0, 1.0);
        (pax - bax * h).hypot(pay - bay * h) - r
    };

    // Exact 2D triangle SDF for the subtle dark-emerald cat nose on the center equalizer bar.
    let sd_triangle = |px: f32, py: f32, p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)| -> f32 {
        let e0 = (p1.0 - p0.0, p1.1 - p0.1);
        let e1 = (p2.0 - p1.0, p2.1 - p1.1);
        let e2 = (p0.0 - p2.0, p0.1 - p2.1);
        let v0 = (px - p0.0, py - p0.1);
        let v1 = (px - p1.0, py - p1.1);
        let v2 = (px - p2.0, py - p2.1);
        let seg = |v: (f32, f32), e: (f32, f32)| -> f32 {
            let t = ((v.0 * e.0 + v.1 * e.1) / (e.0 * e.0 + e.1 * e.1).max(1e-6)).clamp(0.0, 1.0);
            let dx = v.0 - e.0 * t;
            let dy = v.1 - e.1 * t;
            dx * dx + dy * dy
        };
        let d2 = seg(v0, e0).min(seg(v1, e1)).min(seg(v2, e2));
        let s = (e0.0 * e2.1 - e0.1 * e2.0).signum();
        let c0 = s * (v0.0 * e0.1 - v0.1 * e0.0);
        let c1 = s * (v1.0 * e1.1 - v1.1 * e1.0);
        let c2 = s * (v2.0 * e2.1 - v2.1 * e2.0);
        let inside = c0.min(c1).min(c2) >= 0.0;
        if inside { -d2.sqrt() } else { d2.sqrt() }
    };

    // Waveform Equalizer Cat ("Mèo Sóng Âm"): 7 symmetric equalizer pill bars sculpting a geometric
    // cat head with tall pointed ears and negative-space eye cutouts, flanked by 2 subtle whisker pulses.
    let sd_main_bars = |px: f32, py: f32| -> f32 {
        let ax = px.abs();
        let r = 0.0625_f32;
        // Outer cheek bars (±0.508)
        let cheek = sd_capsule(ax, py, 0.508, -0.109, 0.508, 0.344, r);
        // Tall cat ear peak bars (±0.336)
        let ear = sd_capsule(ax, py, 0.336, -0.453, 0.336, 0.422, r);
        // Eye-split bars (±0.164): negative-space gap between y=-0.0625 and y=0.250 forms cute cat eyes
        let eye_top = sd_capsule(ax, py, 0.164, -0.1875, 0.164, -0.0625, r);
        let eye_bot = sd_capsule(ax, py, 0.164, 0.250, 0.164, 0.469, r);
        // Center crown & chin bar (0.0)
        let center = sd_capsule(ax, py, 0.0, -0.125, 0.0, 0.484, r);
        cheek.min(ear).min(eye_top).min(eye_bot).min(center)
    };
    let sd_whisker_bars = |px: f32, py: f32| -> f32 {
        sd_capsule(px.abs(), py, 0.684, 0.098, 0.684, 0.215, 0.0508)
    };
    let sd_emblem = |px: f32, py: f32| -> f32 {
        sd_main_bars(px, py).min(sd_whisker_bars(px, py))
    };
    let sd_nose = |px: f32, py: f32| -> f32 {
        sd_triangle(px, py, (-0.045, 0.236), (0.045, 0.236), (0.0, 0.292)) - 0.012
    };

    let samples = if size >= 128 { 3 } else { 4 };
    let inv_samples = 1.0 / samples as f32;
    let aa_px = 1.6 * inv_half;

    for y in 0..size {
        for x in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..samples {
                for sx in 0..samples {
                    let px = ((x as f32 + (sx as f32 + 0.5) * inv_samples) - half) * inv_half;
                    let py = ((y as f32 + (sy as f32 + 0.5) * inv_samples) - half) * inv_half;

                    let d_tile = sd_squircle(px, py, 0.93);
                    if d_tile > aa_px {
                        continue;
                    }
                    let tile_a = ((-d_tile) / aa_px + 0.5).clamp(0.0, 1.0);

                    // Liquid Glass emerald-to-teal jewel gradient (`#20EC7E -> #0EB368 -> #05644C`).
                    let vy = ((py + 0.93) / 1.86).clamp(0.0, 1.0);
                    let vx = ((px + 0.93) / 1.86).clamp(0.0, 1.0);
                    let mut r = 32.0 * (1.0 - vy) + 5.0 * vy + 8.0 * (1.0 - vx) * (1.0 - vy);
                    let mut g = 236.0 * (1.0 - vy) + 100.0 * vy + 16.0 * (1.0 - vx) * (1.0 - vy);
                    let mut b = 126.0 * (1.0 - vy) + 76.0 * vy + 18.0 * vx * vy;

                    // Overhead Liquid Glass dome sheen (soft curved top reflection).
                    let dome_d = ((px * 0.85).powi(2) + ((py + 0.72) * 1.35).powi(2)).sqrt();
                    let dome = (1.0 - dome_d).clamp(0.0, 1.0).powf(2.2) * 0.30;
                    r += (255.0 - r) * dome;
                    g += (255.0 - g) * dome;
                    b += (255.0 - b) * dome;

                    // Subtle bottom-right inner emerald bounce light (Liquid Glass caustics).
                    let bounce_d = ((px - 0.38).hypot(py - 0.48) / 0.82).clamp(0.0, 1.0);
                    let bounce = (1.0 - bounce_d).powf(2.0) * 0.22;
                    r += 26.0 * bounce;
                    g += 116.0 * bounce;
                    b += 82.0 * bounce;

                    // Crisp Liquid Glass outer refractive rim (brightest top-left & bottom-right).
                    let edge_band = (1.0 - (d_tile + 0.026).abs() / 0.026).clamp(0.0, 1.0);
                    let rim_dir = ((-px - py) * 0.55).max((px + py) * 0.38).clamp(0.0, 1.0);
                    let rim = edge_band * (0.16 + 0.56 * rim_dir);
                    r += (255.0 - r) * rim;
                    g += (255.0 - g) * rim;
                    b += (255.0 - b) * rim;

                    // Soft vertical drop shadow under the white equalizer cat emblem.
                    let d_shadow = sd_emblem(px, py - 0.048);
                    if d_shadow < 0.16 {
                        let sh = ((-d_shadow.max(0.0) * 15.0).exp() * 0.26).clamp(0.0, 0.30);
                        r *= 1.0 - sh;
                        g *= 1.0 - sh;
                        b *= 1.0 - sh;
                    }

                    // Pure frosted-white equalizer cat bars + slightly softer outer whisker pulses.
                    let d_main = sd_main_bars(px, py);
                    let d_whisker = sd_whisker_bars(px, py);
                    let main_cov = ((-d_main) / aa_px + 0.5).clamp(0.0, 1.0);
                    let whisker_cov = ((-d_whisker) / aa_px + 0.5).clamp(0.0, 1.0) * 0.84;
                    let mark_cov = main_cov.max(whisker_cov);
                    if mark_cov > 0.0 {
                        let shine = ((0.45 - py) * 0.55).clamp(0.0, 1.0);
                        let mr = 238.0 + 17.0 * shine;
                        let mg = 250.0 + 5.0 * shine;
                        let mb = 244.0 + 11.0 * shine;
                        r = r * (1.0 - mark_cov) + mr * mark_cov;
                        g = g * (1.0 - mark_cov) + mg * mark_cov;
                        b = b * (1.0 - mark_cov) + mb * mark_cov;
                    }

                    // Crisp dark-emerald cat nose accent centered on middle bar.
                    let nose_cov = ((-sd_nose(px, py)) / aa_px + 0.5).clamp(0.0, 1.0);
                    if nose_cov > 0.0 {
                        r = r * (1.0 - nose_cov) + 6.0 * nose_cov;
                        g = g * (1.0 - nose_cov) + 107.0 * nose_cov;
                        b = b * (1.0 - nose_cov) + 70.0 * nose_cov;
                    }

                    acc[0] += r * tile_a;
                    acc[1] += g * tile_a;
                    acc[2] += b * tile_a;
                    acc[3] += tile_a;
                }
            }
            let total_s = (samples * samples) as f32;
            let a = (acc[3] / total_s).clamp(0.0, 1.0);
            let i = (y * size + x) * 4;
            if a > 1e-4 {
                let inv = 1.0 / acc[3];
                rgba[i] = (acc[0] * inv).round().clamp(0.0, 255.0) as u8;
                rgba[i + 1] = (acc[1] * inv).round().clamp(0.0, 255.0) as u8;
                rgba[i + 2] = (acc[2] * inv).round().clamp(0.0, 255.0) as u8;
                rgba[i + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    rgba
}

fn app_icon() -> egui::IconData {
    const N: usize = 256;
    egui::IconData {
        rgba: render_nhip_icon_rgba(N),
        width: N as u32,
        height: N as u32,
    }
}

#[cfg(test)]
mod icon_tests {
    use super::render_nhip_icon_rgba;

    #[test]
    fn waveform_cat_icon_renders_symmetric_ears_eye_cutouts_and_nose() {
        let n = 64usize;
        let rgba = render_nhip_icon_rgba(n);
        let px = |u: f32, v: f32| -> [u8; 4] {
            let x = (((u + 1.0) * 0.5) * n as f32).clamp(0.0, (n - 1) as f32) as usize;
            let y = (((v + 1.0) * 0.5) * n as f32).clamp(0.0, (n - 1) as f32) as usize;
            let i = (y * n + x) * 4;
            [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
        };
        // Corner outside squircle is transparent.
        assert_eq!(rgba[3], 0);
        // Both tall cat ear peaks (±0.336, -0.35) are crisp bright white.
        let ear_l = px(-0.336, -0.35);
        let ear_r = px(0.336, -0.35);
        assert!(ear_l[0] > 220 && ear_l[1] > 235 && ear_l[2] > 225, "left ear={ear_l:?}");
        assert!(ear_r[0] > 220 && ear_r[1] > 235 && ear_r[2] > 225, "right ear={ear_r:?}");
        // Eye cutouts (±0.164, 0.08) show emerald background (low red, dominant green) instead of white bar.
        let eye_l = px(-0.164, 0.08);
        let eye_r = px(0.164, 0.08);
        assert!(eye_l[0] < 90 && eye_l[1] >= 125 && eye_l[1] > eye_l[0] * 3, "left eye cutout={eye_l:?}");
        assert!(eye_r[0] < 90 && eye_r[1] >= 125 && eye_r[1] > eye_r[0] * 3, "right eye cutout={eye_r:?}");
        // Cat nose at center bar (0.0, 0.26) is deep emerald accent (distinct from surrounding white bar).
        let nose = px(0.0, 0.26);
        let forehead = px(0.0, 0.0);
        assert!(forehead[0] > 220, "center bar={forehead:?}");
        assert!(nose[0] < 60 && nose[1] < 140, "nose accent={nose:?}");
    }
}
