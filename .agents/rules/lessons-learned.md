# Lessons Learned & Mistake-Prevention Ledger (`nhip`)

> **Mandatory Protocol:** Every agent working in this repository MUST review relevant entries before editing and MUST append a new `LL-NNN` entry whenever a bug, test failure, build breakage, visual regression, or user correction happens. **Never repeat a logged mistake.**

---

## LL-001 — `cfg(test)` in `nhip-core` does not apply to `nhip` binary tests
- **Symptom:** `app::tests::toggle_play_on_youtube_track_stays_on_current_and_toggles_playing` failed after splitting `nhip-core` because `Engine::start()` opened a real WASAPI `cpal` device instead of `Output::timer`.
- **Root Cause:** Cargo compiles library workspace members without `cfg(test)` when testing a downstream binary crate.
- **Prevention Rule:** Always use `Engine::start_silent(...)` in `src/app.rs` `test_app()` and any downstream tests. Never replace `Engine::start(...)` in production `App::new()`. Always verify `edit_file` replacement counts when two similar call sites exist.

## LL-002 — Keep `crates/nhip-core` strictly free of UI crates
- **Symptom:** Adding `egui`, `eframe`, `image`, or `lofty` to `crates/nhip-core` breaks the compile boundary and forces full backend rebuilds on UI changes.
- **Root Cause:** Sharing UI constants (`LEFT_DEFAULT`, `RIGHT_WIDTH`) or helper functions (`cover_key`) in the wrong direction.
- **Prevention Rule:** Place pure domain constants and helpers in `nhip-core` (`mechanics.rs` / `library.rs`) and re-export them in `src/theme.rs` / `src/art.rs`. Periodically confirm zero `unused_dependencies` warnings in `crates/nhip-core/Cargo.toml`.

## LL-003 — Do not introduce a second HTTP/TLS stack (`ureq` / `rustls` / `ring`)
- **Symptom:** Keeping `ureq` alongside `reqwest` pulled in `ring` (C/asm compilation via `cc`) + `rustls` + ~15 extra crates for no benefit.
- **Root Cause:** Separate modules independently chose `ureq` vs `reqwest`.
- **Prevention Rule:** Use `reqwest` (`native-tls`, `blocking`) for all synchronous and asynchronous HTTP calls. After editing `Cargo.toml`, verify `cargo tree -i ureq` and `cargo tree -i ring` return nothing on Windows.

## LL-004 — `ytmapi-rs = "=0.3.4"` fails on `rustc 1.101-nightly`
- **Symptom:** `cargo +nightly check` fails inside `ytmapi-rs-0.3.4/src/parse/playlist.rs:277` with `E0283` (`JsonCrawler` type inference).
- **Root Cause:** Type-inference change in `rustc 1.101.0-nightly` against pinned `ytmapi-rs 0.3.4`.
- **Prevention Rule:** Keep `stable` (`rustc 1.100+`) as the supported build/test toolchain. Do not commit nightly-only scripts until `ytmapi-rs` is upgraded and verified green on nightly.

## LL-005 — PowerShell strips double quotes inside native CLI arguments
- **Symptom:** Passing `--config 'profile.dev.codegen-backend="cranelift"'` from PowerShell fails TOML parsing because PowerShell strips `"`.
- **Root Cause:** PowerShell argument marshaling to native Windows executables removes unescaped double quotes.
- **Prevention Rule:** In PowerShell scripts invoking `cargo --config`, always use TOML single-quoted literal strings (`'...'`) inside PowerShell double-quoted strings, e.g. `"key='value'"`.

## LL-006 — Never hardcode temporary or external paths in tests
- **Symptom:** Ignored live tests in `ytm.rs` wrote to `../.scratch/rea-spotify/...`, which breaks outside the old Craft Anything directory layout.
- **Root Cause:** Leftover paths from external research scripts.
- **Prevention Rule:** Always use `std::env::temp_dir()` for scratch outputs in tests and workspace-relative paths (`CARGO_MANIFEST_DIR` / `$PSScriptRoot`) for project assets.

## LL-007 — Do not force `sccache` during local incremental Windows dev
- **Symptom:** `cargo build` after a 1-file UI edit took `10.2s` with `sccache` vs `7.19s` without it.
- **Root Cause:** `sccache` cannot cache incremental binary crates (`nhip.exe`) and adds ~3s IPC overhead per invocation on Windows.
- **Prevention Rule:** Keep `rustc-wrapper = "sccache"` commented out in global `~/.cargo/config.toml` for daily dev; opt in via `$env:RUSTC_WRAPPER = "sccache"` only for clean builds or CI.

## LL-008 — Keep `Cargo.lock` synced for `--locked` packaging
- **Symptom:** `package-windows.ps1` fails with lockfile drift if `Cargo.toml` is edited without updating `Cargo.lock`.
- **Root Cause:** `package-windows.ps1` enforces `cargo build --locked --release`.
- **Prevention Rule:** After any `Cargo.toml` dependency/feature edit, run `cargo check --workspace` to update `Cargo.lock`, then verify `cargo nt --locked` and `cargo build --locked`.

## LL-009 — Never guess Spotify UI tokens or playback mechanics
- **Symptom:** Hardcoded "eyeballed" padding, colors, or transition thresholds drift from Spotify Desktop behavior.
- **Root Cause:** Implementing UI or audio logic from memory instead of measured evidence.
- **Prevention Rule:** Measure exact values from Spotify Desktop (`xpui.spa` / `Spotify.dll`) using the `spotify-re-parity` skill (`rea` MCP), document provenance in `MECHANICS.md`, then code + test.

## LL-010 — Always verify responsive layout at `800×600`, not only `1304×724`
- **Symptom:** Panels, sidebars, or Vietnamese labels that look fine at `1304×724` can clip, overlap, or push controls off-screen at minimum window size `800×600`.
- **Root Cause:** Testing only at default desktop viewport size.
- **Prevention Rule:** Every UI/UX change must pass `.\scripts\ux-ui-test.ps1` covering both `1304x724` and `800x600`, plus headless layout unit tests in `src/ui/`.

## LL-011 — Keep dependency debuginfo off (`debug = 0`) in dev profile
- **Symptom:** `target/debug/nhip.pdb` bloated to `79.2 MB` because ~490 dependency crates emitted PDB line tables, slowing down every link.
- **Root Cause:** `[profile.dev.package."*"]` inherited `debug` from `[profile.dev]`.
- **Prevention Rule:** Keep `debug = 0` on `[profile.dev.package."*"]` and `debug = "line-tables-only"` only on `[profile.dev]` and `[profile.dev.package.nhip-core]`.

## LL-012 — PowerShell `Start-Process -ArgumentList` breaks paths with spaces (`Craft Anything`)
- **Symptom:** `Start-Process -FilePath nhip.exe -ArgumentList @('--screenshot', $shotPath)` split `Craft Anything` into two args, causing silent screenshot failure and empty `$proc.ExitCode`.
- **Root Cause:** Windows PowerShell 5.1 joins `-ArgumentList` without quoting spaces and does not expose `.ExitCode` unless `.Handle` is cached.
- **Prevention Rule:** Use `System.Diagnostics.ProcessStartInfo` with explicit `"` quoting around path arguments (`'"{0}"' -f $path`) in PowerShell scripts that invoke native executables.

## LL-013 — Do not write obvious, tautological, or overly loose tests
- **Symptom:** The suite contained constant self-checks (`assert_eq!(PLAYER_BAR, 88.0)`), trivial `HashSet`/enum-match/serde-derive tests, dead-code tests (`insert_queue_item`), and a Vietnamese font test that only asserted `galley.size().x > 0.0` (which still passes on missing-glyph tofu boxes `□`).
- **Root Cause:** Writing tests to hit coverage or echo implementation literals instead of checking real failure modes.
- **Prevention Rule:** Only test non-trivial invariants, edge cases, state transitions, parsers, DSP, and real rendering/layout constraints. For typography/visual checks, assert actual glyph resolution (distinct from `\u{FFFD}`) or rendered rect/pixel properties.

## LL-014 — `egui::Ui::allocate_ui_with_layout` shrinks to child content unless `set_min_width` is called
- **Symptom:** In `src/ui/settings.rs` `pref_row`, all right-side dropdowns and toggle switches floated in the middle of the Settings page instead of aligning flush to the right edge of `col_w`.
- **Root Cause:** `ui.allocate_ui_with_layout(Vec2::new(left_w, 0.0), Layout::top_down(Align::Min), ...)` allocates only the *actual* child width (~210px for short labels), leaving the subsequent right-to-left column shifted left.
- **Prevention Rule:** Whenever creating fixed-ratio horizontal columns in `egui`, explicitly call `ui.set_min_width(col_w)` / `ui.set_max_width(col_w)` inside each column closure and guard the invariant with a headless `egui` layout test (`settings::tests::pref_row_flushes_control_to_column_right_edge_for_short_labels`).

## LL-015 — Verify container vs child widths in Spotify DOM (`volume-bar` is `125px` *including* the `32px` mute button)
- **Symptom:** Giving Nhịp's volume slider `125px` (plus a `32px` mute button and gaps) made the right player-bar cluster ~40px too wide and pushed controls leftward compared to Spotify Desktop.
- **Root Cause:** Reading `[data-testid="volume-bar"]` width (`125px`) as the slider track width alone instead of `32px` mute button + `93px` slider track.
- **Prevention Rule:** Always measure both the wrapper element and its immediate child elements via `getBoundingClientRect()` when extracting Spotify layout metrics, and lock right-cluster desktop/compact behavior with `player_bar::tests::right_cluster_shows_inline_93px_volume_at_desktop_and_collapses_at_compact`.

## LL-016 — Spotify uses `16px Regular` (not Bold) for list/card/tracklist titles
- **Symptom:** Sidebar items, Queue rows, Shelf card titles, and Tracklist song titles looked heavier and more cramped in Nhịp than in Spotify Desktop.
- **Root Cause:** Using `semi(...)` (which maps to `SpotifyMixUI-Bold` in `src/theme.rs`) instead of `body_medium()` (`16px Regular` = `encore-text-body-medium`) for list titles.
- **Prevention Rule:** Check `getComputedStyle(el).fontWeight` on live Spotify DOM before choosing `body_*` vs `semi`/`bold`: `400` (`body_medium`/`body_small`) for tracklist/sidebar/queue/card titles and descriptions; `700` (`bold`/`title_*`) for section headings, hero titles, and buttons.

## LL-017 — Tracklist responsive column breakpoints must account for `666px` default main width at `1304×724`
- **Symptom:** At default `1304×724` (`303px` left + `666px` main + `303px` right), Playlist and Liked Songs hid the `Album` column even though Spotify Desktop shows `# | Tiêu đề | Album | 🕒`.
- **Root Cause:** `Columns::for_width` required `width >= 760px` to display the Album column, which exceeded the `666px` default main pane width.
- **Prevention Rule:** Calibrate responsive breakpoints against Spotify's actual main-pane widths (`666px` at `1304×724`, `416px` at `800×600`) and keep unit test `pages::tests::columns_hide_album_below_620_and_date_below_960` aligned with both viewports.

## LL-018 — Always audit live YouTube Music UI paths (`src/ui/ytm.rs`), not only local demo pages (`src/ui/pages.rs`)
- **Symptom:** Fixing Artist Follow button color, layout hierarchy, and tracklist columns in `pages.rs` left the live online Artist page in `ytm.rs` broken (green Follow button, wrong Verified badge above Name, missing Preview pill, blank right half with no play counts/durations, English `"views"`).
- **Root Cause:** Local demo mode renders `src/ui/pages.rs` while real live YouTube Music usage renders `src/ui/ytm.rs`.
- **Prevention Rule:** Every UI parity fix for Home, Search, Album, Artist, or Playlist MUST be checked and applied identically in both `src/ui/pages.rs` and `src/ui/ytm.rs`, and never ship third-party extension names (`"Spicy"`) in user-facing Nhịp labels.

## LL-019 — Never stamp a shared collab song's cover art onto an Artist's sidebar avatar
- **Symptom:** Both `Low G` and `Thắng` displayed the identical `An Thần` track cover in the Left Sidebar because `item_of(RecentItem::Artist)` returned `app.lib.artist(name).cover_path`, which prevented `sidebar.rs` from calling `artist_profile_cached`.
- **Root Cause:** `Library::artist(name)` derives `cover_path` from the artist's first track in `app.lib`, colliding whenever two artists collaborate on a track.
- **Prevention Rule:** Persist real artist portraits in `UserData::artist_avatars` keyed by `fold(name)` and always prefer `artist_avatar` (or `None` to trigger `artist_profile_cached`) over YouTube track covers. Guard with `pages::tests::artist_sidebar_item_prefers_artist_avatar_and_never_collides_on_shared_yt_track`.

## LL-020 — In `egui` child viewports, `ScrollArea` steals OS window drag events if placed after the drag strip
- **Symptom:** The frameless Always-On-Top Popup Lyrics window stayed frozen in place and could not be dragged across the desktop.
- **Root Cause:** `drag_strip` was registered before `lyrics_rect` (`ScrollArea` starting at `rect.min`), so `ScrollArea` intercepted every mouse drag, and `send_viewport_cmd` was not explicitly targeted with `send_viewport_cmd_to(vp_id, ...)`.
- **Prevention Rule:** Register child-viewport window drag handles AFTER scroll areas (excluding interactive button pills) and explicitly target the popup viewport with `ctx.send_viewport_cmd_to(vp_id, ViewportCommand::StartDrag)`.

## LL-021 — Proactive Auto-Queue & Next-Track Deck Preloading must run while playing, never after `Event::Finished`
- **Symptom:** Playing a single song from Home/Search left Queue / Cinema Up-Next completely empty (`Chưa có bài tiếp theo trong hàng chờ`), Next did nothing, and Gapless/Crossfade stalled on synchronous HTTPS stream resolution.
- **Root Cause:** `request_autoplay()` only ran after `Event::Finished` stopped playback, and `Worker::advance_to_next` only preloaded decks for `Transition::AutoMix`.
- **Prevention Rule:** Proactively prefetch radio recommendations (`maybe_prefetch_autoplay` -> `Player::append_context`) whenever `< 3` upcoming tracks remain, cache resolved YouTube streams (`STREAM_CACHE`), and run a cancellable background `audio-preload` worker for `Cmd::SetNext` across Gapless, Crossfade, and manual Next. Guard with `audio::tests::set_next_preloads_gapless_deck_and_reuses_it_on_manual_load` and `app::tests::proactive_autoplay_appends_to_live_queue_and_ranks_by_user_taste`.

### LL-022: Small Outline Heart Ribbon Miter Breaks at Bottom Cusp
- **Symptom:** The unliked outline heart icon in Popup Lyrics rendered with a visible vertical split/notch at its bottom tip.
- **Root Cause:** `paint_cinema_heart` built a quad-strip miter ribbon with clamped `3.0px` stroke even at `21px` popup size, and adjacent cubic curves met at identical cusp coordinates (`[500.499, 936.0]`), producing a zero-length segment whose normal flipped the miter quad.
- **Prevention Rule:** Always deduplicate coincident contour vertices (`dedup_by`) before computing stroke normals, and use `egui::Shape::closed_line` with proportional stroke (`(draw_size * 0.088).clamp(1.75, 2.4)`) for small outline hearts (`draw_size <= 36.0`).

### LL-023: Lyrics ScrollArea Inherited Previous Track's Scroll Offset & Stalled Sync
- **Symptom:** Switching to a new song or playing through an intro left the Lyrics viewport pinned near the bottom/credits instead of auto-following the active lyric line; scrubbing the timeline also lagged behind.
- **Root Cause:** `ScrollArea::vertical().id_salt(salt)` did not include `track_id`, so `egui` reused the previous track's scroll offset; `follow.resume_follow(...)` ran after `ScrollArea` instead of before; `active.is_none()` intros never reset scroll to `0.0`; and slight drift didn't trigger re-centering.
- **Prevention Rule:** Key every lyrics `ScrollArea` by `(salt, track_id)`, call `follow.resume_follow` before evaluating `scroll_to_active`, reset scroll to `0.0` during track intros when `auto_follow` is active, and re-center whenever the active line drifts `> height * 0.24` from the viewport target.

### LL-024: Over-Blurred 64×64 Backdrops Wash Out Monochrome Covers & 16ms Timer Jitters Motion
- **Symptom:** Album covers with tight hue palettes (e.g., green or deep blue) washed into flat near-solid green/gray backgrounds in Lyrics/NPV, and backdrop motion felt stuttery on high-refresh monitors.
- **Root Cause:** `spicy_backdrop_image` applied 6 box-blur passes at `r=4` on a `64×64` texture (destroying local light/dark pockets before Kawarp warped it), and `request_repaint_after(16ms)` hit coarse Windows timer resolution (~15.6ms jitter) instead of vsync.
- **Prevention Rule:** Limit CPU pre-blur on `64×64` backdrop textures to 3 passes at `r=2` so Kawarp's 3-octave GPU domain warp + high-contrast grading (`output.frag`) creates rich luminous pockets, and use vsync-locked `ctx.request_repaint()` when motion is active.

### LL-025: Lyrics Inter-Line `end_ms` Gaps & Unobserved Scrollbar Drags Trigger Jump-to-Top
- **Symptom:** On word-synced tracks where every line has an explicit `end_ms` (e.g., LyricsPlus v2), short breath pauses between lines caused the Lyrics view to snap back to `0.0` (top), and dragging the right scrollbar thumb snapped back on the next active line.
- **Root Cause:** `Lyrics::current(pos)` returned `None` during inter-line gaps (`prev.end_ms <= pos < next.time_ms`), which triggered `needs_intro_reset` (`vertical_scroll_offset(0.0)`); meanwhile, `manual_input` did not cover scrollbar clicks/drags or unexpected `ScrollArea` offset deltas, and `ui.scroll_to_rect` teleported instead of easing.
- **Prevention Rule:** Use `Lyrics::display_current(pos)` to bridge inter-line gaps without dropping to `None`, restrict intro reset strictly to `pos < first_vocal_ms`, detect any unexpected `clamped_offset().y` delta as manual scroll, and drive auto-scroll + sync button returns via smooth exponential easing (`anim_target_y`). Guard with `lrclib::tests::parse_lyricsplus_json_splits_background_vocals_and_right_aligns_second_singer`.

### LL-026: LyricsPlus Fuzzy Wrong-Song Matches & Over-Eager `right_align` on Line-Synced/Group Tracks
- **Symptom:** Playing *Hướng Dương — Thắng* (`3:28`) showed Changg's `3:39` *Hướng Dương* lyrics; playing *Những em bé Mặt Trời* (`type: "Line"`, `v1000` group chorus) mistakenly right-aligned the chorus and ran fake left-to-right sweep instead of top-to-bottom illumination + mid-song interlude dots.
- **Root Cause:** LyricsPlus v2 falls back to title-only matches when an artist is missing; `fetch_word_synced` omitted duration validation against `metadata.totalDuration`, and `parse_lyricsplus_json` right-aligned any singer != `v1` (including `v1000` groups and `"Line"`-synced tracks).
- **Prevention Rule:** Always validate `lyricsplus_matches_duration` (`<= 6.5s` vs `metadata.totalDuration` and `<= 7.5s` past track end), restrict `right_align` strictly to `type == "Word"` non-group opposite vocalists (never `v1000` or `"Line"`), render line-synced tracks with 180° top-to-bottom illumination (`make_vertical_line_galley`), and auto-expand 3-dot interludes for `>= 3.5s` mid-song gaps. Guard with `lrclib::tests::lyricsplus_rejects_wrong_duration_song_and_keeps_line_synced_left_aligned`.


### LL-027 — Artist Popular Row Double Play Icon & Lyrics Double `• • •` Interludes
- **Symptom:** Hovering an indexed Artist Popular track painted two Play triangles simultaneously (one in the `#` column and a second over the 40×40 cover); Lyrics occasionally rendered two stacked `• • •` interlude animations during intros, mid-song breaks, or consecutive blank/instrumental timed lines.
- **Root Cause:** `draw_song_row` (`src/ui/ytm.rs`) painted the cover hover Play overlay unconditionally even when `indexed == true`; `render_lyrics_scroll_body` (`src/ui/lyrics.rs`) inserted synthetic intro/mid-song `• • •` dots without checking for existing `is_instrumental_line` entries or collapsing consecutive instrumental lines.
- **Prevention Rule:** Gate cover Play overlays on `!indexed` in `draw_song_row`. Always deduplicate synthetic vs explicit `• • •` interludes via `should_render_synthetic_intro_dots`, `synthetic_mid_interlude_gap`, and `explicit_instrumental_span`. Guard with `lyrics::tests::interlude_dots_deduplicate_intro_mid_and_consecutive_instrumental_lines`.

### LL-028 — Intermittent No-Audio, Stale 403 Refresh Loop & Stuck/Broken Seeking on YouTube Streams
- **Symptom:** YouTube playback occasionally failed into silence, stopped responding after Windows audio device swaps/headphone switches, or refused to seek (snapping back or doing nothing).
- **Root Cause:** (1) `HttpRangeSource::refresh_stream` called cached `ytm::resolve_stream()`, returning the exact same expired `403` URL from `STREAM_CACHE`, and initial `open_inner` never refreshed on `403`. (2) `FileSource::seek_to_ms` only tried `SeekMode::Accurate` (no `Coarse` fallback or EOF clamp), `Deck::seek` ignored failures without reopening YouTube streams, and `App::seek` dropped seeks when `!engine.has_song()`. (3) Dragging the preview feed scrubber spammed `Cmd::Seek` every frame with no coalescing in `Worker::run`, and `App::seek` re-ran `sync_next()` spawning duplicate prefetch threads. (4) `Output::create` silently fell back to silent `Keepalive::Timer` with no hardware retry, and stalled WASAPI callbacks could hold `waiting_for_flush()` forever.
- **Prevention Rule:** Always evict `STREAM_CACHE` via `ytm::resolve_stream_fresh` on `401/403/410` (initial and mid-stream); fall back `Accurate -> Coarse` in `FileSource::seek_to_ms`; reopen failed YouTube decks fresh on seek; coalesce consecutive `Cmd::Seek` and supersede stale `Cmd::Load` in `Worker::next_command`; load unloaded tracks in `App::seek`; commit scrubber seeks on release; and recover stalled/timer Windows audio output via `Worker::check_output_health`. Guard with `http_range_forces_fresh_resolve_on_initial_403_and_midstream_403` and `engine_coalesces_seek_burst_to_latest_target`.

### LL-029 — Windows "Not Responding" UI Stalls from Main-Thread Blur & Uncached Lyrics Layout
- **Symptom:** App intermittently froze or triggered Windows "Not Responding" while browsing covers or keeping Lyrics open.
- **Root Cause:** (1) `ArtCache::pump` drained all finished covers in one frame and ran CPU `spicy_backdrop_image` (4-pass blur) synchronously on the main UI thread. (2) `layout_balanced_lyric` ran up to 7 `layout_job` shapings per line per frame (~280 shapings/frame) with zero memoization, `line_widget` cloned and scaled active galleys 21× and inactive blurred lines 29× per frame, and `lyrics::draw` called unbounded `request_repaint()` instead of ~60fps pacing.
- **Prevention Rule:** Compute `spicy_backdrop_image` and `cover_colors` on the background `"art"` thread and cap `ArtCache::pump` to `<= 4` texture uploads per frame. Memoize balanced wrap widths in `BalancedWidthCache`, pre-scale active galleys once before halo passes, keep blur/halo tap counts bounded (`13`-tap 2-ring Gaussian / `12`-tap halo), and pace continuous Lyrics repaints at `16ms` (`~60fps`).


### LL-030 — Audio Stuttering / Silence from Synchronous HTTP Chunk Fetches & Over-Eager Watchdog
- **Symptom:** YouTube playback stuttered (`giật giật`) at chunk boundaries or cut out intermittently on Windows devices with Nahimic/Virtual Audio drivers.
- **Root Cause:** (1) `HttpRangeSource` fetched 256 KiB chunks synchronously on the `Worker` audio thread with zero read-ahead while `RING_MS` was only `400ms` — any YouTube CDN range fetch `> 400ms` drained the ring buffer dry. (2) `Worker::check_output_health` checked `free_frames < BLOCK_FRAMES` instead of actual audio consumer progress and didn't reset timers after blocking `start_deck()` loads. (3) `Output::try_device` fell straight back to silent `Keepalive::Timer` if the default surround/virtual Windows config failed without trying stereo `48kHz`/`44.1kHz` fallback configs.
- **Prevention Rule:** Keep `RING_MS >= 1000ms` (`output.rs`), run background read-ahead chunk prefetching (`schedule_prefetch(chunk_idx + 1)` + `MAX_CACHED_CHUNKS = 16`) in `HttpRangeSource`, measure audio output health strictly via `Output::consumed_frames()` progress (`>= 1500ms` stall threshold, reset after blocking `handle()`), and try stereo fallback configs in `Output::try_device`. Guard with `http_range_prefetches_next_chunk_in_background`.

### LL-031 — Legacy `user.json` `"folders": [String]` Silently Locked `can_save_user = false`
- **Symptom:** User's Liked Songs, followed artists, playlists, listening history, and personalization stats were never persisted across restarts (`user.json` stayed untouched since an older release).
- **Root Cause:** Older versions stored local music directory strings (`Vec<String>`) in `"folders"`, while newer versions changed `UserData.folders` to `Vec<PlaylistFolder>`, causing `serde_json` deserialization to fail on load and leaving `can_save_user = false`.
- **Prevention Rule:** Always provide backwards-compatible deserializers when evolving `UserData` field schemas (`deserialize_folders` ignores legacy string entries while preserving `PlaylistFolder` structs). Guard with `library::tests::legacy_user_json_with_string_folders_migrates_without_failing`.
