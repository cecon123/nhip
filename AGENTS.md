# AGENTS.md — Nhịp (Rust Desktop Music Player)

## Project Architecture
- **Workspace layout:**
  - `crates/nhip-core/`: Pure backend library (`audio`, `library`, `queue`, `ytm`, `lrc`, `ttml`, `lrclib`, `mechanics`). **Zero UI dependencies** — never add `egui`, `eframe`, `glow`, `image`, `lofty`, `rfd`, or `souvlaki` here.
  - `src/`: Binary crate `nhip` (`main.rs`, `app.rs`, `art.rs`, `media.rs`, `theme.rs`, `ui/*`). Imports backend modules via `use nhip_core::{...};` in `src/main.rs`.
  - `assets/fonts/`: Embedded fonts loaded via `include_bytes!("../assets/fonts/...")` in `src/theme.rs`.
  - `MECHANICS.md`: Single source of truth for measurements and behavior reverse-engineered from Spotify Desktop.
  - `scripts/ux-ui-test.ps1`: Automated 3-layer UX/UI test & visual regression runner.
  - `package-windows.ps1`: Windows portable release packager (`cargo build --locked --release`).

## Mandatory Workflow Rules
1. **Check Past Mistakes First:** Before modifying architecture, Cargo config, audio/engine startup, HTTP/TLS, or UI layout, read `.agents/rules/lessons-learned.md` and strictly follow every `LL-NNN` rule.
2. **Never Repeat a Mistake (Self-Learning Rule):** Whenever a bug, test failure, build regression, visual glitch, or user correction occurs:
   - Fix the root cause (never a fragile workaround).
   - Append a concise entry (`LL-NNN`: Symptom, Root Cause, Prevention Rule) to `.agents/rules/lessons-learned.md`.
   - Add a regression test (`cargo nt`) or visual check (`scripts/ux-ui-test.ps1`) that would catch it automatically next time.
3. **Never Guess Spotify Specs:** Before changing spacing, colors, shortcuts, playback transitions, EQ, or loudness rules, verify against `MECHANICS.md` or measure directly from Spotify Desktop using the `spotify-re-parity` skill (`rea` MCP).
4. **Write High-Signal Tests Only (No Trivial/Tautological Tests):**
   - Only test real invariants, edge cases, non-trivial state machines, parsers, DSP math, backwards-compatible deserialization, layout/rendering bounds, and regression tests for real bugs.
   - **Forbidden:** testing constants against themselves (`assert_eq!(CONST, literal)`), trivial `bool`/setter/HashSet toggles, default `#[derive(Serialize, Deserialize)]` round-trips, dead code, or weak assertions like `size.x > 0.0` that pass even when broken (e.g. tofu glyphs).
5. **Always Verify Before Finishing:**
   - Code changes: `cargo nt --locked` (must pass 100% across both `nhip-core` and `nhip`).
   - Dependency/`Cargo.toml` changes: verify `cargo build --locked` and `cargo nt --locked`.
   - UI/UX changes: run `.\scripts\ux-ui-test.ps1` (or `-Quick` for focused UI edits) and inspect any visual diffs at both `1304x724` and `800x600`.
6. **Preserve Build Speed:**
   - Keep `rust-lld.exe` + `-C linker-flavor=lld-link` in `.cargo/config.toml`.
   - Keep `reqwest` (`native-tls`, `blocking`) as the sole HTTP/TLS stack — do not re-add `ureq`, `rustls`, or `ring`.
   - Keep `symphonia` features explicit (no `features = ["all"]`).

## Available Project Skills
- `spotify-re-parity`: Reverse engineer Spotify Desktop (`xpui.spa` / `Spotify.dll`) with `rea` MCP and record provenance in `MECHANICS.md`.
- `nhip-ux-ui-audit`: Run automated visual regression (`scripts/ux-ui-test.ps1`) and UX/UI parity checks (with Mobbin/Figma/REA references).
- `egui-nhip-arch`: Workspace architecture, `egui` performance/layout conventions, and fast-compile discipline.
- `audio-dsp-verify`: Deterministic headless audio, DSP, automix, and streaming tests using `Engine::start_silent` / `Output::timer`.
