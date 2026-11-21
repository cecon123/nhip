---
name: egui-nhip-arch
description: >-
  Architectural and performance conventions for developing Nhip's Rust workspace
  (crates/nhip-core + root nhip egui/eframe UI). Use when adding modules, dependencies,
  new UI screens, custom widgets, or refactoring code across the workspace.
---

# Nhip Rust & `egui` Architecture Guide

## Workspace Boundary Rules
- **`crates/nhip-core` (Backend):**
  - Owns: `audio`, `library`, `queue`, `ytm`, `lrc`, `ttml`, `lrclib`, `mechanics`.
  - Allowed deps: audio/DSP (`cpal`, `rtrb`, `symphonia`, `timestretch`), network/data (`reqwest`, `tokio`, `ytmapi-rs`, `serde`, `serde_json`, `roxmltree`, `chrono`, `dirs`, `rand`).
  - **Forbidden:** `egui`, `eframe`, `glow`, `image`, `lofty`, `rfd`, `souvlaki`.
- **`src/` (`nhip` UI binary):**
  - Owns: `main.rs`, `app.rs`, `art.rs`, `media.rs`, `theme.rs`, `ui/*`.
  - Consumes `nhip-core` via root re-imports in `src/main.rs`.
  - Keep business logic (queue math, audio state, API parsing, lyrics parsing) in `nhip-core` so UI edits never trigger backend recompilation.

## `egui` Performance & Maintainability
1. **Avoid Per-Frame Heap Churn:** Cache decoded textures (`ArtCache`), compiled GL shaders (`backdrop.rs`), and pre-sorted/filtered lists where possible; avoid allocating large `Vec`/`String` buffers on every repaint.
2. **Smart Repaints:** Use `ctx.request_repaint_after(...)` only while animating, playing lyrics, or polling background tasks; do not spin 100% CPU when paused/idle.
3. **Reuse `theme.rs` & `ui/widgets.rs`:** Never hardcode raw RGB values or font sizes inline when a token exists in `src/theme.rs`.
4. **Action Pattern:** Route user interactions through `Action` enum in `src/app.rs` so shortcuts, buttons, context menus, and unit tests all exercise the exact same state transitions.

## Build-Speed Hygiene
- Keep `Cargo.toml` profile settings intact (`opt-level = 0` + `line-tables-only` for `nhip`, `debug = 0` for external deps, `rust-lld.exe` linker in `.cargo/config.toml`).
- Prefer explicit feature flags (`default-features = false`) on any new dependency.
- Never add a duplicate HTTP or TLS crate (stick to `reqwest` + `native-tls`).
- Run `cargo nt --locked` and `cargo build --locked` after dependency or module changes.
