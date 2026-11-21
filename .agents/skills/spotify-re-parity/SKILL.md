---
name: spotify-re-parity
description: >-
  Reverse engineer Spotify Desktop (xpui.spa UI bundle and Spotify.dll native engine)
  using the REA MCP server and Chrome DevTools to extract exact visual tokens, layout
  metrics, shortcuts, and audio/playback behavior for Nhip. Use whenever cloning or
  verifying a Spotify feature, spacing, color, interaction, or audio rule.
---

# Spotify Reverse-Engineering & Parity Workflow

## Prerequisites
- Check `MECHANICS.md` first — many playback, loudness, EQ, and layout values are already measured and documented.
- Use the `rea` MCP server (`reverse-engineer-anything` skill) for shipped Spotify artifacts. Never guess values from memory.

## Procedure

### 1. Choose the Right Target
- **UI tokens, colors, typography, spacing, breakpoints, shortcuts, routes:**
  - Target Spotify's web bundle (`Apps/xpui.spa` or unpacked `xpui` assets) or live Spotify Web/CEF via `rea` (`analyze_web_bundle`, `recover_javascript_sources`, `inspect_web_page`) or `chrome-devtools` MCP.
  - Extract exact CSS custom properties (`--spice-*`, `--encore-*`), pixel dimensions, border radii, hover/active opacity, transition durations, and responsive breakpoints.
- **Audio playback, transitions, loudness, shuffle, queue, volume curve:**
  - Target `Spotify.dll` using `rea` (`open_binary`, `search_strings`, `find_xrefs_to_name`, `procedure_pseudo_code`) or runtime observation (`instrument_with_frida`, `observe_native_calls`).
- **Live visual behavior (scroll, hover, popovers, animations):**
  - Capture real Spotify UI with `rea` (`capture_native_ui_scenario` / `capture_web_screenshot`) and compare against Nhip screenshots (`compare_web_screenshots`).

### 2. Record Provenance Before Coding
Update `MECHANICS.md` with:
- **What** was measured (exact value + unit).
- **Where** it came from (file/module/symbol/offset or CSS variable name).
- Mark anything not directly located as `estimated`.

### 3. Implement in the Right Layer
- Pure domain/playback constants → `crates/nhip-core/src/mechanics.rs`.
- Visual tokens (colors, spacing, radii, typography) → `src/theme.rs`.
- Behavior logic → `crates/nhip-core/src/` (audio/queue/library) or `src/app.rs` / `src/ui/` (interaction/layout).

### 4. Lock Parity With Tests
- Add a unit test asserting the exact constant/formula or interaction rule.
- For UI parity, capture a screenshot via `.\scripts\ux-ui-test.ps1` and compare side-by-side against the Spotify reference capture.
- Run `cargo nt --locked`.
