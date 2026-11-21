# Numbers read from the original program

Original: Spotify desktop for Windows, version `1.3.9.2055.g806b5b278de8` (re-verified against `1.3.6.1921`).

- `Spotify.exe` only starts the Chromium layer. The engine is `Spotify.dll` (C++, x86_64 PE, sha256 `9991ade34f9986ba60dd1122386f4c6cbdd2a167c85a07ce3c30554b7c38153f`).
- The interface is a web bundle in `Apps/xpui`. Live DOM/computed-style metrics were captured via Chromium CDP (`--remote-debugging-port=9229`) at `1304×724` and `800×600` (DPR 1.25).
- "estimated" means the code was searched and the exact value was not found.

## Playback

| What | Value | Where it came from |
|---|---|---|
| Seek step, music | 5 s | `xpui-modules.js`, `seekOffsetMs`: track `5e3`, episode `15e3` |
| Crossfade | 0 to 12 s, whole seconds | `xpui-routes-desktop-settings.js`: slider max `12e3`, step `1e3` |
| Repeat cycle | off, whole list, one song | i18n keys `enable-repeat`, `enable-repeat-one`, `disable-repeat` |
| Skip back may restart the song | yes | `xpui-modules.js`: `skipPrev({allowSeeking:!0})` |
| First song, Previous | restarts it | `0x180caa728`: with `allow_seeking`, a lone `no_prev_track` restriction doesn't stop the command |
| Previous restarts after | 3 s, estimated | Not located. Both compares against 3000 (`0x18098f4c4`, `0x180ba832c`) are unrelated; none against 3000000 |
| Volume key step | `65535 / device steps`, clamped to 0..65535; count capped at 512 | `0x180eedd98` |
| Device steps for this client | 10, estimated | The count comes from the device record; the place it's set was not located |
| Autoplay after the list ends | yes | `Spotify.dll` 0x1807349f8 (`autoplay` restriction), string `hm://context-resolve/v1/autoplay` |
| Song hand-over | see below | `0x180d56f4c` |
| Shuffle | random order, current song first; list keeps its place when turned off | Exact scoring is server-tuned (`enable_artist_separation_shuffle_scorer` and similar flags), estimated |

## Song hand-over

`getAudioTransition`, `0x180d56f4c`, checks in this order (ad and episode cases don't apply here):

```
gapless on, same album, same disc, next track number      -> gapless
crossfade on, length 0                                    -> gapless
crossfade on, 2 x length <= both songs' durations         -> crossfade
gapless on (both local files)                             -> gapless
otherwise                                                 -> a stop between songs
```

So the next song on an album never crossfades while gapless is on. The length of the stop (300 ms) is estimated.

## Loudness

`0x180469728` in `Spotify.dll`, read from its machine code, floats at `0x1819e0e8c`..`0x1819e0e98`:

```
gain = -14 - loudness         (no loudness: the file's own gain, 0 for local files)
outside -40..42 dB: gain = 0
gain += level offset
gain += -2 dB for speech      (0x181aad86c; no podcasts here, so unused)
if gain <= 0, or level is Loud, or peak unknown: done
ceiling = 0 dB for three lossless format codes (0x10, 0x16, 0x39), otherwise -1 dB
if peak + gain <= ceiling: done
if peak < ceiling: gain = ceiling - peak
otherwise: gain = 0
```

Defaults: normalization on, level Normal (`0x1811419e0`). Level index 0 quiet, 1 normal, 2 loud (`0x180e47378`).

`0x180d4073c` parses a server setting, `loudness_levels`, shaped `default:a,b,c:speech`, and `0x180d400c0` applies it. The Quiet and Loud offsets therefore aren't in the program at all.

| What | Value | Where it came from |
|---|---|---|
| Normal target | -14 dB | `0x1819e0e94` |
| Accepted gain range | -40 to 42 dB | `0x1819e0e98`, `0x1819e0e8c` |
| Quiet, Loud offsets | -9, +3 dB, estimated | Delivered by Spotify's servers (`0x180d400c0`); these are its published -23 and -11 dB levels |
| Peak ceilings | -1 dB, 0 dB lossless | `0x180469728` |
| Which local formats count as lossless | flac, wav, aiff, caf, estimated | The format codes are read; their names are not in the code |
| Peak limiter | on | `core-peak-limiter-feature`, `PeakLimiter` strings |

The original gets loudness and true peak from Spotify's servers. Local files use ReplayGain tags when they have them. Otherwise the file is measured in the background with a gated energy estimate, close to LUFS but not exact, and its sample peak stands in for true peak.

## Equalizer

`xpui-modules.js`, module 95421. Six bands, each -12 to +12 dB in 0.1 steps:

| Band | Type | Frequency |
|---|---|---|
| 1 | low shelf | 60 Hz |
| 2 | peaking | 150 Hz |
| 3 | peaking | 400 Hz |
| 4 | peaking | 1 kHz |
| 5 | peaking | 2.4 kHz |
| 6 | high shelf | 15 kHz |

The 22 presets come from the same module (`src/mechanics.rs`). The original's "deep" preset lists eight gains for six bands; only the first six can apply.

`0x1814269c0` loads the six gains; each is stored as a 32-bit integer scaled by 12 / 2^31 (double at `0x181c12d80`), which confirms the +/-12 dB range. `0x180470d74` receives filters as type, frequency, Q and gain, but 2400 and 15000 appear in `Spotify.dll` as neither float nor double, so the table isn't there. Filter Q is 1.0, estimated.

## Look

Measured directly from Spotify Desktop `1.3.9.2055` live DOM (`getComputedStyle` & `getBoundingClientRect` at `1304×724` and `800×600`, DPR 1.25):

| What | Value | Where it came from |
|---|---|---|
| Top bar height | `64 px` (`48px` controls) | `.Root__globalNav`, `xpui-snapshot.css` |
| Back / Forward buttons | `24×48 px` slots, `4px` gap (`28px` pitch) | `[data-testid="top-bar-back-button"]` / `forward-button` |
| Search input & Browse divider | `48px` pill, `16px Regular` text, `1px solid #7c7c7c` divider at `right - 48px`, `24px` Browse icon | `[data-testid="search-input"]`, `[data-testid="browse-button"]` |
| Friend Activity button | Visible at `1304×724` and `800×600` (`>= 720px`) | `[data-testid="friend-activity-button"]` |
| Player bar outer / inner height | `88 px` outer (`padding: 8px`), `72 px` inner (`left` has `padding-inline-start: 8px` so cover starts at `x=16px`) | `.Root__now-playing-bar`, `.main-nowPlayingBar-left` |
| Player cover -> title gap | `16 px` | `.main-nowPlayingWidget-trackInfo` |
| Scrubber time labels | `12 px Regular` (`encore-text-marginal`), `50 px` slot width | `[data-testid="playback-position"]` / `playback-duration` |
| Player right cluster | `32×32 px` buttons with `0px` gap; `[data-testid="volume-bar"]` is `125 px` total (`32px` mute + `93px` slider) + `8px` gap to PiP | `.main-nowPlayingBar-right`, `[data-testid="volume-bar"]` |
| Grid layout (`1304×724`) | `303 px` left, `666 px` main, `303 px` right, `8 px` gap & outer padding | `.Root__top-container` `grid-template-columns` |
| Grid layout (`800×600`) | `72 px` left (auto-collapsed), `416 px` main, `280 px` right (docked) | `.Root__top-container` `grid-template-columns` |
| Sidebar header buttons | `35×35 px` circles (`#242424`), collapsed rail header `107 px + 8 px` gap | `.main-yourLibraryX-header` |
| Sidebar / Queue / Tracklist / Card titles | `16 px Regular` (`400`) titles + `14 px Regular` (`#b3b3b3`) subtitles (NOT bold) | `encore-text-body-medium` / `encore-text-body-small` |
| Queue panel header & rows | `64 px` header with `2px #1ed760` active tab underline; `16px Bold` section headers; `64px` rows (`48px` cover) | `[aria-label="Danh sách chờ"]` |
| Filter chips / pills | `32 px` height, `14 px Regular` (`encore-text-body-small`), Vietnamese chip `"Âm nhạc"` | `.encore-chip` |
| Entity hero cover & title | `~150 px` cover at `666 px` main (`4 px` radius), bold fitted title up to `96 px`; Album column shown down to `620 px` main width | `.main-entityHeader-image`, `[role="grid"]` |
| Search Top Result card | `#181818` (`CARD`) idle -> `#282828` (`CARD_HOVER`) hovered; `24 px` side margin at `>= 640px` | `[data-testid="top-result-card"]` |
| Settings (`/preferences`) | `max-width: 900 px`, `32 px` pad; `h1` `32px Bold`; `h2` `16px Bold` (no hairline); labels `14px Regular #b3b3b3`; `<select>` `32px`, `#242424`, `4px` rad, `14px`; toggle `42×24 px`, `#1db954` ON / `#535353` OFF; controls right-aligned | `/preferences` DOM computed styles |
| Context / Popover menus | `#282828` bg, `4px` rad, `4px` pad, `40 px` row height, `8px 8px 8px 12px` pad, `2px` item hover radius | `[data-testid="context-menu"]` |
| Core CSS color tokens | `--background-base: #121212`, `--background-elevated-base: #242424`, `--background-elevated-highlight: #2a2a2a`, `--background-elevated-press: #191919`, `--essential-subdued: #7c7c7c`, `--decorative-subdued: #292929`, `--text-bright-accent: #1ed760`, `--text-subdued: #b3b3b3` | Live `:root` computed CSS variables (`1.3.9.2055`) |

## Keys

From the shortcut table in `xpui-modules.js` (module `31300`, desktop Windows forms, verified in `1.3.9.2055`):

| Action | Key |
|---|---|
| Play or pause | Space |
| Next, previous | Ctrl+Right, Ctrl+Left |
| Seek 5 s | Shift+Right, Shift+Left |
| Volume | Ctrl+Up, Ctrl+Down |
| Mute / Unmute | M |
| Shuffle, repeat | Ctrl+S, Ctrl+R |
| Quick search / Go to search | Ctrl+K, Ctrl+L |
| Filter in view / Library text filter | Ctrl+F, Ctrl+Alt+F |
| New playlist / New folder | Ctrl+N, Ctrl+Shift+N |
| History back / forward | Ctrl+Alt+Left / Alt+Left, Ctrl+Alt+Right / Alt+Right |
| Settings | Ctrl+, |
| Shortcut list | Ctrl+/ or ? |
| Like | Alt+Shift+B |
| Home, liked songs, queue, now playing, recents, friend activity | Alt+Shift+H, S, Q, J, Y, U |
| Library / Playlists / Artists / Albums | Alt+Shift+0, 1, 3, 4 |
| Collapse left panel, toggle right panel | Alt+Shift+L, R |
| Resize panels | Alt+Shift+arrows |

## Spotify account

Everything goes through the official Web API with the person's own developer app. The original's private login, tokens, internal endpoints and Client ID are not used.

- Sign-in: Authorization Code with PKCE (S256) in the system browser, a one-shot listener on `127.0.0.1:8898`, a random `state` that is checked. The refresh token is kept in Windows Credential Manager.
- Rules from Spotify's February 2026 change for apps in development mode: the app's owner needs Premium; library changes use `PUT/DELETE /me/library`; playlist songs use `/playlists/{id}/items` and only exist for playlists the person owns or collaborates on; search returns at most 10 per type; batch fetches, browse, new releases and artist top tracks are gone.
- Endpoints used: `/me`, `/me/tracks`, `/me/albums`, `/me/playlists`, `/playlists/{id}`, `/playlists/{id}/items`, `/albums/{id}`, `/search`, `/me/library`, `/me/player` and its `play`, `pause`, `next`, `previous`, `seek`, `shuffle`, `repeat`, `volume`, `queue`, `devices`.
- Player commands need Premium; Spotify answers 403 otherwise and the app says so.
- An artist page for someone not in the library is built from a search, because the top-tracks endpoint is gone.

Icons are drawn from plain geometry to match the original's set, not copied from its artwork, and its logo and name are not used.

## Left out

- Playing Spotify's catalogue inside this app: its audio is protected by DRM, which was not touched. Songs play on the person's Spotify devices and this app controls them.
- Lyrics, friends' activity, home recommendations, new releases, browse categories and top tracks of an artist: no public API offers them to this kind of app.
- Podcasts and audiobooks: the API has them, but they aren't built yet.
- Playback speed exists in the original for podcasts and audiobooks, which this app does not have.
