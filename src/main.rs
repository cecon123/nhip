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

/// A green disc with three sound bars.
fn app_icon() -> egui::IconData {
    const N: usize = 64;
    let mut rgba = vec![0u8; N * N * 4];
    let c = (N as f32 - 1.0) / 2.0;
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 - c, y as f32 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let alpha = ((c - d + 0.5).clamp(0.0, 1.0) * 255.0) as u8;
            let bar = [(-14.0, 8.0), (0.0, 16.0), (14.0, 11.0)]
                .iter()
                .any(|(bx, h): &(f32, f32)| (dx - bx).abs() < 4.0 && dy.abs() < *h);
            let (r, g, b) = if bar { (0, 0, 0) } else { (0x1e, 0xd7, 0x60) };
            let i = (y * N + x) * 4;
            rgba[i..i + 4].copy_from_slice(&[r, g, b, alpha]);
        }
    }
    egui::IconData { rgba, width: N as u32, height: N as u32 }
}
