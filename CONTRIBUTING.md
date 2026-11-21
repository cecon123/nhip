# Contributing to Nhịp

Thanks for taking an interest in Nhịp. This guide covers how the repository is structured, how to keep builds fast, and what we look for in pull requests.

## Development setup

1. Install the stable Rust toolchain via [rustup](https://rustup.rs/).
2. (Optional, recommended) Install `cargo-nextest`:
   ```bash
   cargo install cargo-nextest --locked
   ```
   `.cargo/config.toml` defines `cargo nt` as an alias for `cargo nextest run --workspace`.
3. Clone your fork and verify the test suite passes:
   ```bash
   cargo test --workspace --locked
   ```

## Architecture boundaries

The workspace is intentionally split into two crates:

- **`crates/nhip-core`**: Pure backend logic (`audio`, `library`, `queue`, `ytm`, `lrc`, `ttml`, `lrclib`, `mechanics`).
  - **Never add UI or windowing dependencies** (`egui`, `eframe`, `glow`, `image`, `lofty`, `rfd`, `souvlaki`) to `nhip-core`.
  - Keep all playback, DSP, queue, and parser logic testable headlessly without an audio device or GPU context.
- **Root `nhip` (`src/`)**: Desktop UI built with `egui`/`eframe`, artwork decoding, platform media keys, and GLSL shaders.

## Build speed & dependency rules

Nhịp is tuned for fast incremental Rust builds:

- **Single HTTP/TLS stack:** Use `reqwest` (`native-tls`, `blocking`) from `[workspace.dependencies]`. Do not introduce parallel stacks such as `ureq`, `rustls`, or `ring`.
- **Explicit codec features:** Keep `symphonia` features explicit in `crates/nhip-core/Cargo.toml` (avoid `features = ["all"]`).
- **Locked builds:** If you modify `Cargo.toml`, update `Cargo.lock` and verify `cargo build --locked` and `cargo test --workspace --locked` succeed.
- **Windows linker:** `.cargo/config.toml` enables `rust-lld.exe` on `x86_64-pc-windows-msvc` — please keep this intact.

## Testing guidelines

We prioritize high-signal tests over raw test counts:

- **Write tests for real invariants:** state machines (queue, shuffle/unshuffle, repeat), parsers (LRC, TTML, LyricsPlus UTF-8 byte ranges), DSP math (EQ, limiter, loudness normalization), layout/responsive bounds, and regression tests for fixed bugs.
- **Headless audio tests:** Use `Engine::start_silent` / `Output::timer` so tests stay deterministic and run cleanly in CI without audio hardware.
- **Avoid trivial tests:** Do not add tests that merely assert a constant equals itself or test derived `Serialize`/`Deserialize` round-trips without compatibility invariants.
- **Visual regression (Windows):** After UI or shader changes, run:
  ```powershell
  .\scripts\ux-ui-test.ps1
  ```
  Inspect any changed screenshots in `target/ui-qa/captures/` before updating baselines with `-UpdateBaselines`.

## Good areas to contribute

- **Cross-platform support:** Linux (`mpris` media keys, Wayland/X11 frameless window controls) and macOS (`MPNowPlayingInfoCenter`, native window corners/traffic lights).
- **Audio & DSP:** Additional local metadata formats, ReplayGain tag support, output device selector.
- **Lyrics & localization:** Additional open lyrics parsers, right-to-left / CJK typography polish, more UI languages.
- **Accessibility & keyboard navigation:** Focus traversal and screen-reader hooks.

## Pull request checklist

- [ ] `cargo test --workspace --locked` passes with zero failures.
- [ ] `cargo build --locked` succeeds cleanly without introducing new warnings.
- [ ] `crates/nhip-core` has no UI dependencies.
- [ ] UI changes have been checked at both desktop (`1304×724`) and compact (`800×600`) viewports (and `.\scripts\ux-ui-test.ps1` passes on Windows).
- [ ] PR description explains the motivation, root cause (for bug fixes), and before/after screenshots for UI changes.
