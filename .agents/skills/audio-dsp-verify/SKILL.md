---
name: audio-dsp-verify
description: >-
  Test and verify Nhip's audio engine, DSP (6-band EQ, peak limiter, loudness
  normalization), gapless/crossfade/automix transitions, and YouTube HTTP range
  streaming deterministically without hardware audio devices. Use when editing
  crates/nhip-core/src/audio/ or playback mechanics.
---

# Deterministic Audio & DSP Verification

## Core Rules
1. **No Real Hardware Audio in Tests:**
   - Inside `nhip-core` unit tests, `Engine::start` automatically selects `Output::timer`.
   - From downstream `nhip` tests (`src/app.rs`), always call `Engine::start_silent(...)` (never `Engine::start(...)`) so WASAPI devices are not opened during parallel test runs.
   - When constructing `Worker::new(...)` directly in tests, pass `silent = true`.
2. **Use Synthetic Fixtures:**
   - Generate deterministic PCM/WAV fixtures (e.g. `440 Hz` sine wave helpers in `audio/mod.rs`) and `SourceSpec::Demo` seeds instead of relying on local user audio files or live internet access.
   - For HTTP range streaming tests, spin up a local `TcpListener` with `test_agent()` (`reqwest::blocking::Client::builder().no_proxy()`) to test 206 partial content, retries, 403/410 re-resolution, and cancellation.

## Verification Commands
```powershell
# Fast targeted audio/DSP test run
cargo nt -p nhip-core audio

# Full workspace verification
cargo nt --locked
```

## DSP Invariants to Preserve
- Peak limiter must keep output samples strictly below full scale (`<= 1.0` / ceiling per `MECHANICS.md`).
- Gapless album transitions must not crossfade when gapless is enabled.
- Cancel signals (`CancelSignal`) must abort in-flight HTTP range reads and automix preparation promptly without leaking audio threads.
- Keep `symphonia`, `timestretch`, and `cpal` at `opt-level = 3` in `[profile.dev.package.*]` so local `cargo run` playback stays glitch-free.
