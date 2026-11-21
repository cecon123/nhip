---
name: nhip-ux-ui-audit
description: >-
  Run automated UX/UI verification, visual regression testing, and design-system parity
  audits for Nhip across desktop and compact viewports (1304x724 and 800x600). Use after
  any UI change, layout tweak, theme update, or before shipping a release.
---

# Nhip UX/UI Audit & Automated Visual Testing

## 3-Layer Verification Pipeline

### Layer 1 — Automated Logic, Shortcut & Headless Layout Gate
Run all unit and `egui` headless layout/interaction tests:
```powershell
cargo nt --locked
```
Confirm 100% pass across both `nhip-core` and `nhip` (keyboard shortcuts, focus guards, breakpoints, spring animations, Vietnamese fonts, icon coverage).

### Layer 2 — Automated Screenshot Matrix & Pixel Diff
Run the automated visual QA script from workspace root:
```powershell
.\scripts\ux-ui-test.ps1            # Full matrix (home, search, album, artist, playlist, queue, lyrics, settings @ 1304x724 + home @ 800x600)
.\scripts\ux-ui-test.ps1 -Quick     # Fast 3-view smoke test (home 1304x724, home 800x600, lyrics 1304x724)
```
- Captures deterministic offline demo screenshots using Nhip's built-in `--page`, `--size`, and `--screenshot` CLI flags.
- Compares each capture against `tests/ui-baselines/*.png` and writes red-highlighted diff images to `target/ui-qa/diffs/` plus a machine-readable report at `target/ui-qa/report.json`.
- Only update baselines intentionally after visually approving changes:
  ```powershell
  .\scripts\ux-ui-test.ps1 -UpdateBaselines
  ```

### Layer 3 — Visual & UX Parity Inspection
Inspect captured PNGs (via `read_file`) and diff images against this checklist:
1. **Layout & Overflow:** No overlapping panels, no horizontal clipping/spill at `800x600` or `1304x724`, player bar always visible and clickable.
2. **Spotify Token Fidelity:** Surface hierarchy, green accent (`#1ed760`), radii, 8px spacing rhythm, and icon alignment match `src/theme.rs` and `MECHANICS.md`.
3. **Vietnamese Typography:** Diacritics render cleanly with no clipped ascenders/descenders or broken line wrapping; long titles truncate/marquee gracefully.
4. **Hit Targets & Interaction Clarity:** Interactive controls have comfortable hit areas (≥ 32px for primary actions), clear hover/active states, and keyboard focus guards when typing.
5. **External Pattern Cross-Check (when redesigning a screen):**
   - Use **Mobbin** (`search_screens` / `redesign-screen`) to compare flows against shipped Spotify/music-app patterns.
   - Use **Figma** (`get_design_context` / `get_variable_defs`) if a Figma reference is linked.
   - Use **REA** (`compare_web_screenshots`) when comparing directly against a Spotify Desktop capture.

### On Any Failure or Visual Bug
1. Fix the UI code in `src/ui/` or `src/theme.rs`.
2. Add a headless layout test in `src/ui/` so the regression is caught in Layer 1.
3. Log any non-obvious pitfall in `.agents/rules/lessons-learned.md`.
