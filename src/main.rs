#![cfg_attr(not(test), windows_subsystem = "windows")]

mod app;
mod art;
#[cfg(windows)]
mod media;
mod theme;
mod ui;

use nhip_core::{audio, library, lrc, mechanics, queue, ttml, ytm};

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
            .with_icon(app_icon()),
        ..Default::default()
    };
    eframe::run_native("Nhịp", options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, screenshot, page)))))
}

/// Modern supersampled squircle Nhịp icon: obsidian glass tile with luminous emerald rhythm bars.
fn app_icon() -> egui::IconData {
    const N: usize = 128;
    let mut rgba = vec![0u8; N * N * 4];
    let half = N as f32 * 0.5;
    let inv_half = 1.0 / half;

    let sd_rounded_box = |px: f32, py: f32, bx: f32, by: f32, r: f32| -> f32 {
        let qx = px.abs() - bx + r;
        let qy = py.abs() - by + r;
        qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r
    };

    // 4-bar rhythmic pulse emblem (`Nhịp` acoustic waveform mark) in normalized [-1, 1] coords.
    const BARS: [(f32, f32, f32); 4] = [
        (-0.42, 0.24, 0.085),
        (-0.14, 0.48, 0.085),
        (0.14, 0.34, 0.085),
        (0.42, 0.20, 0.085),
    ];

    for y in 0..N {
        for x in 0..N {
            let mut acc = [0.0f32; 4];
            for sy in 0..2 {
                for sx in 0..2 {
                    let px = ((x as f32 + (sx as f32 + 0.5) * 0.5) - half) * inv_half;
                    let py = ((y as f32 + (sy as f32 + 0.5) * 0.5) - half) * inv_half;
                    let d_tile = sd_rounded_box(px, py, 0.92, 0.92, 0.30);
                    if d_tile > 0.02 {
                        continue;
                    }
                    let tile_a = ((-d_tile) / 0.022).clamp(0.0, 1.0);

                    // Deep obsidian glass base with diagonal ambient emerald sheen.
                    let grad_t = ((px * 0.45 - py * 0.55) + 0.5).clamp(0.0, 1.0);
                    let mut r = 11.0 + 10.0 * grad_t;
                    let mut g = 18.0 + 28.0 * grad_t;
                    let mut b = 15.0 + 18.0 * grad_t;

                    // Soft radial emerald aura behind the central waveform bars.
                    let radial = (1.0 - (px * px * 1.1 + (py + 0.05).powi(2) * 1.1).sqrt()).clamp(0.0, 1.0);
                    let aura = radial * radial * 0.55;
                    r += 16.0 * aura;
                    g += 132.0 * aura;
                    b += 58.0 * aura;

                    // Specular frosted rim highlight along top-left edge.
                    let rim = (1.0 - (d_tile + 0.025).abs() / 0.025).clamp(0.0, 1.0);
                    let rim_light = ((-py * 0.65 - px * 0.35) * 0.5 + 0.5).clamp(0.0, 1.0);
                    let rim_add = rim * (14.0 + 46.0 * rim_light);
                    r += rim_add;
                    g += rim_add * 1.25;
                    b += rim_add * 1.05;

                    // Luminous rounded-capsule rhythm bars + soft bloom.
                    let mut min_bar = 10.0f32;
                    for &(bx, bh, bw) in &BARS {
                        let db = sd_rounded_box(px - bx, py, bw, bh, bw);
                        min_bar = min_bar.min(db);
                    }
                    let glow = (-min_bar.max(0.0) * 6.5).exp() * 0.45;
                    r += 28.0 * glow;
                    g += 205.0 * glow;
                    b += 92.0 * glow;

                    let bar_cov = ((-min_bar) / 0.018).clamp(0.0, 1.0);
                    if bar_cov > 0.0 {
                        let bar_v = ((0.5 - py) * 0.9).clamp(0.0, 1.0);
                        let br = 30.0 + 112.0 * bar_v;
                        let bg = 215.0 + 38.0 * bar_v;
                        let bb = 96.0 + 96.0 * bar_v;
                        r = r * (1.0 - bar_cov) + br * bar_cov;
                        g = g * (1.0 - bar_cov) + bg * bar_cov;
                        b = b * (1.0 - bar_cov) + bb * bar_cov;
                    }

                    acc[0] += r * tile_a;
                    acc[1] += g * tile_a;
                    acc[2] += b * tile_a;
                    acc[3] += tile_a;
                }
            }
            let a = (acc[3] * 0.25).clamp(0.0, 1.0);
            let i = (y * N + x) * 4;
            if a > 1e-4 {
                let inv = 1.0 / acc[3];
                rgba[i] = (acc[0] * inv).round().clamp(0.0, 255.0) as u8;
                rgba[i + 1] = (acc[1] * inv).round().clamp(0.0, 255.0) as u8;
                rgba[i + 2] = (acc[2] * inv).round().clamp(0.0, 255.0) as u8;
                rgba[i + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}
