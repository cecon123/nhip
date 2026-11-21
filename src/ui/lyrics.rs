use std::time::Duration;

use eframe::egui::{
    self, Align, Color32, CornerRadius, Layout, Mesh, Pos2, Rect, RichText, Stroke, TextFormat, Ui,
    Vec2,
};

use super::icons::{self, Icon, SpicyIcon};
use super::widgets::{
    cover, cover_shadow, icon_button_sized, marquee_text, mmss, skeleton_glass_rect, text,
    text_width,
};
use crate::app::{Action, App, Route, TtmlImportMode};
use crate::library::{fold, imported_lyrics_file, split_artists, Track, TrackId};
use crate::lrc::{LyricLine, Lyrics};
use crate::queue::Repeat;
use crate::theme::{
    body, bold, marginal, marginal_small, semi, ACCENT, CARD, ELEVATED, HAIRLINE, HOVER, MUTED,
    POPUP, RADIUS_MODAL, SUBTLE, TEXT,
};

const CARD_GLASS_BG: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);

// Reference: spicy-lyrics-f000506.js 6.4.70, LF SHA-256 805c63a94121b748f99f91d5936c419d0810d8d318b204e13850c98ec20a6abc.
// Measurements and selectors: .scratch/rea-spotify/pages-and-lyrics-flows.md section 5.
const ACTIVE_UNSUNG_WHITE: Color32 = Color32::from_rgba_premultiplied(112, 112, 112, 112); // ~0.44
const PAST_SUNG_WHITE: Color32 = Color32::from_rgba_premultiplied(102, 102, 102, 102); // ~0.40
const MANUAL_SCROLL_RESUME_SECS: f64 = 0.75;

#[derive(Clone, Default)]
struct LyricsHoverPillState {
    track: Option<TrackId>,
    /// Animated pill bounds in scroll-content coordinates (`y - content_origin_y`).
    rect: Option<Rect>,
    opacity: f32,
}

impl LyricsHoverPillState {
    fn step(&mut self, target: Option<Rect>, dt: f32, motion_on: bool) -> bool {
        let mut animating = false;
        if let Some(t) = target {
            if self.rect.is_none() || self.opacity < 0.02 || !motion_on {
                self.rect = Some(t);
            } else if let Some(cur) = self.rect {
                let k = 1.0 - (-dt * 18.0).exp();
                let next = Rect::from_min_max(
                    Pos2::new(
                        cur.min.x + (t.min.x - cur.min.x) * k,
                        cur.min.y + (t.min.y - cur.min.y) * k,
                    ),
                    Pos2::new(
                        cur.max.x + (t.max.x - cur.max.x) * k,
                        cur.max.y + (t.max.y - cur.max.y) * k,
                    ),
                );
                let delta = (next.min.x - t.min.x)
                    .abs()
                    .max((next.min.y - t.min.y).abs())
                    .max((next.max.x - t.max.x).abs())
                    .max((next.max.y - t.max.y).abs());
                if delta < 0.35 {
                    self.rect = Some(t);
                } else {
                    self.rect = Some(next);
                    animating = true;
                }
            }
            if !motion_on || self.opacity > 0.99 {
                self.opacity = 1.0;
            } else {
                let k = 1.0 - (-dt * 16.0).exp();
                self.opacity = (self.opacity + (1.0 - self.opacity) * k).min(1.0);
                animating = true;
            }
        } else if self.opacity > 0.01 {
            if !motion_on {
                self.opacity = 0.0;
                self.rect = None;
            } else {
                let k = 1.0 - (-dt * 14.0).exp();
                self.opacity *= 1.0 - k;
                if self.opacity < 0.01 {
                    self.opacity = 0.0;
                    self.rect = None;
                } else {
                    animating = true;
                }
            }
        }
        animating
    }
}

#[derive(Clone, Default)]
struct LyricsFollowState {
    track: Option<TrackId>,
    last_active: Option<usize>,
    manual_scroll_at: Option<f64>,
    active_visible: bool,
    last_position: Option<u64>,
    last_playing: Option<bool>,
    last_scroll_y: f32,
    anim_target_y: Option<f32>,
    programmatic_y: Option<f32>,
}

impl LyricsFollowState {
    fn resume_follow(&mut self, now: f64, position: u64, playing: bool, manual_input: bool) -> bool {
        // Bundle rb()/eb(): a >1000ms seek or playback change releases manual browsing.
        let transport_changed = self.last_position.is_some_and(|p| p.abs_diff(position) > 1000)
            || self.last_playing.is_some_and(|p| p != playing);
        self.last_position = Some(position);
        self.last_playing = Some(playing);
        if manual_input {
            self.manual_scroll_at = Some(now);
            self.anim_target_y = None;
        } else if self.manual_scroll_at.is_some_and(|t| {
            transport_changed || (now - t >= MANUAL_SCROLL_RESUME_SECS && self.active_visible)
        }) {
            self.manual_scroll_at = None;
            self.last_active = None;
            return true;
        }
        false
    }
}

#[derive(Clone)]
struct BackdropState {
    track: Option<TrackId>,
    from: [Color32; 4],
    current: [Color32; 4],
    target: [Color32; 4],
    prev_tex: Option<egui::TextureHandle>,
    cur_tex: Option<egui::TextureHandle>,
    blend: f32,
    phase: f32,
}

impl BackdropState {
    fn set_target(&mut self, track: Option<TrackId>, target: [Color32; 4], texture: Option<egui::TextureHandle>, motion_on: bool) {
        let tex_changed = self.cur_tex.as_ref().map(|t| t.id()) != texture.as_ref().map(|t| t.id());
        if self.track == track && self.target == target && !tex_changed { return; }
        if tex_changed {
            self.prev_tex = self.cur_tex.take();
            self.cur_tex = texture;
        }
        self.track = track;
        self.from = self.current;
        self.target = target;
        // Tracks sharing a cover must not fade that same texture up from transparent.
        self.blend = if motion_on && (tex_changed || self.cur_tex.is_none()) { 0.0 } else { 1.0 };
    }

    fn advance_blend(&mut self, dt: f32, motion_on: bool) -> f32 {
        self.blend = if motion_on { (self.blend + dt / 0.5).min(1.0) } else { 1.0 };
        // Bundle Yw: 500ms cosine crossfade, also snap an in-flight fade when motion is disabled.
        let t = 0.5 - 0.5 * (self.blend * std::f32::consts::PI).cos();
        for i in 0..4 {
            self.current[i] = lerp_color(self.from[i], self.target[i], t);
        }
        if self.blend >= 1.0 {
            self.prev_tex = None;
        }
        t
    }
}

#[derive(Clone)]
struct SavedTtmlCache {
    refreshed_at: f64,
    tracks: Vec<TrackId>,
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    let bounds = ui.max_rect();
    let motion_on = app.data.settings.effective_lyrics_motion();
    let minimized = ui.ctx().input(|i| i.viewport().minimized.unwrap_or(false));
    let lyrics_visible = app.route == Route::Lyrics && !minimized;
    let playing = app.is_playing();
    let position = app.position_ms();
    let track = app.current_track();
    let track_id = track.as_ref().map(|t| t.id);

    // Animated Kawarp-style blurred cover backdrop with smooth track crossfade
    let (target_colors, backdrop_tex) = if let Some(ref t) = track {
        let key = t.cover_key();
        let bd = app.art.backdrop_texture(ui.ctx(), &key, t.art_source());
        (app.art.lyric_colors(&key), Some(bd))
    } else {
        (
            [
                Color32::from_rgb(36, 44, 62),
                Color32::from_rgb(24, 34, 48),
                Color32::from_rgb(18, 22, 32),
                Color32::from_rgb(16, 16, 20),
            ],
            None,
        )
    };
    let animating_backdrop = draw_animated_backdrop(
        ui,
        bounds,
        track_id,
        target_colors,
        backdrop_tex,
        motion_on && lyrics_visible,
        playing,
    );

    if track_id.is_some() {
        let _ = app.current_lyrics();
    }

    let cinema_mode = app.lyrics_cinema || app.lyrics_fullscreen.is_some();

    if cinema_mode || app.lyrics_split_nowbar {
        draw_cinema_stage(app, ui, bounds, track.as_ref(), position, motion_on, cinema_mode);
    } else {
        draw_center_lyrics(app, ui, bounds, track_id, position, motion_on, false);
    }

    if !cinema_mode {
        draw_view_controls(app, ui, bounds, false, 1.0, motion_on);
    }

    // Repaint scheduling: keep Kawarp dynamic backdrop & lyric sweep fluid at ~60fps without spinning 100% CPU.
    let synced_playing = playing
        && app
            .current_lyrics()
            .is_some_and(|l| l.synced);
    if lyrics_visible && (animating_backdrop || synced_playing || motion_on) {
        if motion_on && (animating_backdrop || synced_playing) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(42));
        }
    }

    // TTML Manager modal/drawer
    if app.lyrics_manager_open {
        draw_ttml_manager_modal(app, ui, track.as_ref());
    }
}

pub fn card(app: &mut App, ui: &mut Ui) {
    // Suppress duplicate Lyrics card when Route::Lyrics is already active in center
    if app.route == Route::Lyrics {
        return;
    }

    let expanded = app.lyrics_card_expanded;
    let collapsed = app.lyrics_card_collapsed && !expanded;
    let playing = app.is_playing();
    let motion_on = app.data.settings.effective_lyrics_motion();
    let position = app.position_ms();
    let track_id = app.player.current;

    ui.horizontal(|ui| {
        let inset = if expanded { 0.0 } else { 16.0 };
        ui.add_space(inset);
        let width = (ui.available_width() - inset).max(0.0);
        let frame = if expanded {
            egui::Frame::new()
                .fill(Color32::TRANSPARENT)
                .inner_margin(egui::Margin::ZERO)
        } else {
            egui::Frame::new()
                .fill(CARD_GLASS_BG)
                .corner_radius(CornerRadius::same(16))
                .inner_margin(egui::Margin::ZERO)
        };

        frame.show(ui, |ui| {
            let inner_w = width;
            ui.vertical(|ui| {
                ui.set_width(inner_w);
                ui.spacing_mut().item_spacing.y = 0.0;
                let card_bounds = ui.max_rect();
                let card_hovered = ui.rect_contains_pointer(card_bounds);
                // live-home-measurements.json (1304x724, DPR 1.25): 301 = 8 + 38 + 255 CSS px.
                if !expanded {
                    ui.add_space(8.0);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(16, 0))
                        .show(ui, |ui| {
                            draw_card_header(app, ui, expanded, collapsed, motion_on, card_hovered)
                        });
                }

                if collapsed {
                    ui.add_space(8.0);
                    return;
                }

                let body_height = if expanded {
                    ui.available_height().max(1.0)
                } else {
                    255.0
                };

                let missing = if track_id.is_some() {
                    app.t("Bài này chưa có lời", "No lyrics for this song")
                } else {
                    app.t("Chọn bài hát để xem lời", "Play a song to see its lyrics")
                };
                let ended_label = app.t("Đã hết lời", "End of lyrics");
                let sync_label = app.t("Đồng bộ", "Sync");

                let _ = app.current_lyrics();
                let is_loading = app.lyrics_loading();
                let cur_artist = app.current_track().map(|t| t.artist.clone()).unwrap_or_default();
                let mut seek_action: Option<u64> = None;
                let mut repaint_ms: Option<u64> = None;

                {
                    let lyrics_opt = app.current_lyrics();
                    ui.allocate_ui_with_layout(
                        Vec2::new(inner_w, body_height),
                        Layout::top_down(Align::Min),
                        |ui| {
                            ui.set_clip_rect(ui.max_rect().intersect(ui.clip_rect()));
                            let Some(lyrics) = lyrics_opt else {
                                if is_loading {
                                    draw_lyrics_skeleton(ui, inner_w, body_height, 16.0, 18.0);
                                } else {
                                    ui.add_space((body_height * 0.22).min(46.0));
                                    ui.vertical_centered(|ui| {
                                        ui.label(
                                            RichText::new(missing)
                                                .font(semi(14.5))
                                                .color(SUBTLE),
                                        );
                                    });
                                }
                                return;
                            };

                            if playing && lyrics.synced {
                                repaint_ms = Some(if motion_on { 24 } else { 48 });
                            }

                            // `NPVLyrics.css:492` (`clamp(1.85rem, 6cqw, 2.8rem)`) & `.Expanded` (`NPVLyrics.css:366`).
                            let font_size = if expanded {
                                (inner_w * 0.088).clamp(27.0, 36.0)
                            } else {
                                (inner_w * 0.082).clamp(24.5, 29.6)
                            };
                            let line_inset = if expanded {
                                (inner_w * 0.07).clamp(16.0, 32.0)
                            } else {
                                16.0
                            };
                            let credits = expanded
                                .then(|| resolve_lyrics_credits(lyrics, &cur_artist))
                                .flatten();
                            seek_action = render_lyrics_scroll_body(
                                ui,
                                "npv-lyrics-card-scroll",
                                lyrics,
                                track_id,
                                position,
                                playing,
                                motion_on,
                                font_size,
                                inner_w,
                                body_height,
                                line_inset,
                                ended_label,
                                sync_label,
                                true,
                                0.0,
                                credits,
                            );
                        },
                    );
                }
                if let Some(ms) = repaint_ms {
                    ui.ctx().request_repaint_after(Duration::from_millis(ms));
                }
                if let Some(ms) = seek_action {
                    app.actions.push(Action::SeekTo(ms));
                }
                if expanded {
                    let header_rect = Rect::from_center_size(
                        Pos2::new(card_bounds.center().x, card_bounds.top() + 35.0),
                        Vec2::new(80.0, 38.0),
                    );
                    let opacity = if ui.rect_contains_pointer(header_rect) { 1.0 }
                        else if ui.rect_contains_pointer(card_bounds) { 0.7 } else { 0.0 };
                    let mut header = ui.new_child(egui::UiBuilder::new().max_rect(header_rect));
                    header.set_opacity(opacity);
                    draw_card_header(app, &mut header, expanded, collapsed, motion_on, true);
                }
            });
        });
    });
    if !expanded {
        ui.add_space(12.0);
    }
}

fn draw_card_header(
    app: &mut App,
    ui: &mut Ui,
    expanded: bool,
    collapsed: bool,
    motion_on: bool,
    card_hovered: bool,
) {
    ui.horizontal(|ui| {
        ui.set_height(38.0);
        ui.spacing_mut().item_spacing.x = 4.0;
        if !expanded {
            ui.label(
                RichText::new("Lyrics")
                    .font(semi(16.5))
                    .color(Color32::from_white_alpha(240)),
            );
        }
        // Compact NPV card header buttons are always visible at rest (checks/original/window.png);
        // expanded mode reveals its floating pill on hover.
        let show_btns = !expanded || collapsed || card_hovered;
        let btn_alpha = if motion_on {
            ui.ctx().animate_bool_with_time(
                ui.scope_id().with("npv-card-hdr-btns"),
                show_btns,
                0.16,
            )
        } else if show_btns {
            1.0
        } else {
            0.0
        };
        if btn_alpha > 0.01 {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.set_opacity(btn_alpha);
                for (id, icon, tip, selected, action) in [
                    (
                        "npv-lyrics-toggle",
                        if collapsed {
                            SpicyIcon::CardToggleDown
                        } else {
                            SpicyIcon::CardToggleUp
                        },
                        if collapsed {
                            app.t("Hiện lời bài hát", "Show lyrics")
                        } else {
                            app.t("Ẩn lời bài hát", "Hide lyrics")
                        },
                        false,
                        Action::ToggleLyricsCardCollapse,
                    ),
                    (
                        "npv-lyrics-expand",
                        if expanded {
                            SpicyIcon::CardCollapse
                        } else {
                            SpicyIcon::CardExpand
                        },
                        if expanded {
                            app.t("Thu nhỏ thẻ lời", "Restore lyrics card")
                        } else {
                            app.t("Phóng to thẻ lời", "Expand lyrics card")
                        },
                        expanded,
                        Action::ToggleLyricsCardExpand,
                    ),
                    (
                        "npv-lyrics-open",
                        SpicyIcon::OpenPage,
                        app.t("Mở lời bài hát", "Open lyrics"),
                        false,
                        Action::ToggleLyrics,
                    ),
                ] {
                    if expanded && id == "npv-lyrics-toggle" {
                        continue;
                    }
                    if glass_circle_spicy_button(ui, id, 36.0, icon, tip, selected, motion_on)
                        .clicked()
                    {
                        app.actions.push(action);
                    }
                }
            });
        }
    });
}

fn draw_center_lyrics(
    app: &mut App,
    ui: &mut Ui,
    bounds: Rect,
    track_id: Option<TrackId>,
    position: u64,
    motion_on: bool,
    cinema: bool,
) {
    let missing = if track_id.is_some() {
        app.t("Bài này chưa có lời", "No lyrics for this song")
    } else {
        app.t("Chọn bài hát để xem lời", "Play a song to see its lyrics")
    };
    let loading = app.t("Đang tìm lời bài hát…", "Finding lyrics…");
    let ended_label = app.t("Đã hết lời", "End of lyrics");
    let sync_label = app.t("Đồng bộ", "Sync");

    let _ = app.current_lyrics();
    let is_loading = app.lyrics_loading();
    let cur_artist = app.current_track().map(|t| t.artist.clone()).unwrap_or_default();
    let playing = app.is_playing();
    let mut target = None;

    {
        let Some(lyrics) = app.current_lyrics() else {
            ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
                if is_loading {
                    let pad = if cinema {
                        (bounds.width() * 0.06).clamp(20.0, 52.0)
                    } else {
                        (bounds.width() * 0.18).clamp(32.0, 175.0)
                    };
                    draw_lyrics_skeleton(ui, bounds.width(), bounds.height(), pad, 26.0);
                } else {
                    ui.add_space(bounds.height() * 0.40);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(missing)
                                .font(bold(22.0))
                                .color(TEXT),
                        );
                    });
                }
            });
            let _ = loading;
            return;
        };

        let padding = if cinema {
            (bounds.width() * 0.06).clamp(20.0, 54.0)
        } else {
            // Spicy Lyrics `ContentBox.css:75-76`: `--LyricsLeftSidePadding: 18cqw; --LyricsRightSidePadding: 15.5cqw;`
            (bounds.width() * 0.165).clamp(32.0, 165.0)
        };
        let content_w = (bounds.width() - 2.0 * padding).min(720.0).max(160.0);
        // `Lyrics/main.css:36` & `Mixed.css:263` (`--DefaultLyricsScale: 0.95` -> ~31.8px at 684px panel width, matching `1e971dbd.jpg` word wrap).
        let font_size = if cinema {
            (bounds.width() * 0.052).clamp(28.5, 44.0)
        } else {
            (bounds.width() * 0.0465).clamp(28.5, 42.0)
        };
        let credits = resolve_lyrics_credits(lyrics, &cur_artist);

        ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
            target = render_lyrics_scroll_body(
                ui,
                if cinema {
                    "cinema-lyrics-lines"
                } else {
                    "center-lyrics-lines"
                },
                lyrics,
                track_id,
                position,
                playing,
                motion_on,
                font_size,
                bounds.width(),
                bounds.height().max(120.0),
                (bounds.width() - content_w) * 0.5,
                ended_label,
                sync_label,
                false,
                if cinema { 0.0 } else { 30.0 },
                credits,
            );
        });
    }
    if let Some(ms) = target {
        app.actions.push(Action::SeekTo(ms));
    }
}

fn draw_lyrics_skeleton(ui: &mut Ui, width: f32, height: f32, pad_x: f32, bar_h: f32) {
    let avail = (width - pad_x * 2.0).clamp(80.0, 620.0);
    let top_space = (height * 0.16).clamp(18.0, 96.0);
    ui.add_space(top_space);
    const FRACS: [f32; 7] = [0.58, 0.76, 0.46, 0.84, 0.64, 0.72, 0.42];
    for frac in FRACS {
        let (row, _) = ui.allocate_exact_size(Vec2::new(width, bar_h + 14.0), egui::Sense::hover());
        let bar = Rect::from_min_size(
            Pos2::new(row.left() + pad_x, row.top() + 4.0),
            Vec2::new(avail * frac, bar_h),
        );
        skeleton_glass_rect(ui, bar, bar_h * 0.42);
    }
}

fn resolve_lyrics_credits<'a>(lyrics: &'a Lyrics, fallback_artist: &'a str) -> Option<(&'a str, &'a str)> {
    let writer = lyrics
        .songwriter
        .as_deref()
        .or_else(|| {
            let trimmed = fallback_artist.trim();
            (!trimmed.is_empty()).then_some(trimmed)
        })?;
    let provider = lyrics.provider.unwrap_or("Nhịp");
    Some((writer, provider))
}

fn paint_rounded_triangle(painter: &egui::Painter, a: Pos2, b: Pos2, c: Pos2, corner_r: f32, color: Color32) {
    let verts = [a, b, c];
    let mut poly = Vec::with_capacity(18);
    for i in 0..3 {
        let prev = verts[(i + 2) % 3];
        let curr = verts[i];
        let next = verts[(i + 1) % 3];
        let d_prev = prev - curr;
        let d_next = next - curr;
        let l_prev = d_prev.length().max(0.01);
        let l_next = d_next.length().max(0.01);
        let cut = corner_r.min(l_prev * 0.34).min(l_next * 0.34);
        let p0 = curr + d_prev / l_prev * cut;
        let p2 = curr + d_next / l_next * cut;
        for step in 0..=4 {
            let t = step as f32 / 4.0;
            let u = 1.0 - t;
            poly.push(Pos2::new(
                u * u * p0.x + 2.0 * u * t * curr.x + t * t * p2.x,
                u * u * p0.y + 2.0 * u * t * curr.y + t * t * p2.y,
            ));
        }
    }
    painter.add(egui::Shape::convex_polygon(poly, color, Stroke::new(0.6, color)));
}

fn paint_spicy_double_skip(painter: &egui::Painter, rect: Rect, forward: bool, color: Color32) {
    // Exact Spicy Lyrics `Icons.TrackSkip` (`viewBox="0 0 35 20"`, Icons.ts:20 — plump 1.75:1 rounded double triangles).
    let w = rect.width();
    let h = w * (20.0 / 35.0);
    let c = rect.center();
    let dir = if forward { 1.0 } else { -1.0 };
    let r = (h * 0.14).clamp(1.1, 3.2);
    for offset in [-0.245_f32, 0.255_f32] {
        let cx = c.x + offset * w * dir;
        let tip = Pos2::new(cx + 0.245 * w * dir, c.y);
        let top = Pos2::new(cx - 0.245 * w * dir, c.y - h * 0.485);
        let bot = Pos2::new(cx - 0.245 * w * dir, c.y + h * 0.485);
        if forward {
            paint_rounded_triangle(painter, top, tip, bot, r, color);
        } else {
            paint_rounded_triangle(painter, top, bot, tip, r, color);
        }
    }
}

fn paint_spicy_view_glyph(painter: &egui::Painter, rect: Rect, kind: u8, nowbar_right: bool, fs_active: bool, col: Color32) {
    let icon = match kind {
        2 => SpicyIcon::CinemaView,
        9 => {
            if nowbar_right {
                SpicyIcon::NowBarRight
            } else {
                SpicyIcon::NowBarLeft
            }
        }
        7 => SpicyIcon::LyricsManager,
        6 => {
            if nowbar_right {
                SpicyIcon::NowBarSideSwapRight
            } else {
                SpicyIcon::NowBarSideSwapLeft
            }
        }
        5 => SpicyIcon::UpNext,
        1 => {
            if fs_active {
                SpicyIcon::CloseFullscreen
            } else {
                SpicyIcon::Fullscreen
            }
        }
        4 => SpicyIcon::Settings,
        10 => SpicyIcon::LyricsBubble,
        _ => SpicyIcon::Close,
    };
    icons::paint_spicy(painter, rect, icon, col);
}

/// Renders a Spotify-styled tooltip pill clamped strictly inside `bounds` so edge buttons never clip off-window.
fn paint_clamped_tooltip(ui: &Ui, btn_rect: Rect, label: &str, bounds: Rect, prefer_below: bool) {
    if label.trim().is_empty() {
        return;
    }
    let layer = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Tooltip,
        ui.scope_id().with(("spicy-tip", label)),
    ));
    let galley = layer.layout_no_wrap(label.to_owned(), semi(12.5), TEXT);
    let tip_w = galley.size().x + 18.0;
    let tip_h = 26.0;
    let safe = bounds.intersect(ui.ctx().content_rect());
    let min_cx = safe.left() + tip_w * 0.5 + 4.0;
    let max_cx = (safe.right() - tip_w * 0.5 - 4.0).max(min_cx);
    let cx = btn_rect.center().x.clamp(min_cx, max_cx);
    let cy = if prefer_below && btn_rect.bottom() + 8.0 + tip_h <= safe.bottom() - 4.0 {
        btn_rect.bottom() + 8.0 + tip_h * 0.5
    } else {
        (btn_rect.top() - 8.0 - tip_h * 0.5).max(safe.top() + tip_h * 0.5 + 4.0)
    };
    let tip_r = Rect::from_center_size(Pos2::new(cx, cy), Vec2::new(tip_w, tip_h));
    let sh = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(95),
    };
    layer.add(sh.as_shape(tip_r, CornerRadius::same(6)));
    layer.rect_filled(tip_r, CornerRadius::same(6), POPUP);
    layer.galley(tip_r.center() - galley.size() * 0.5, galley, TEXT);
}

fn cinema_art_rect(bounds: Rect) -> Rect {
    // Spicy NowBar: min(30cqw, 52cqh) * 1.1, centered in the left half.
    let side = (bounds.width() * 0.30).min(bounds.height() * 0.52) * 1.1;
    Rect::from_min_size(
        bounds.min + Vec2::new(
            (bounds.width() * 0.5 - side * 1.25).max(16.0),
            (bounds.height() * 0.5 - side * 0.625).max(24.0),
        ),
        Vec2::splat(side),
    )
}

fn cinema_art_rect_sided(bounds: Rect, right_side: bool) -> Rect {
    let left_rect = cinema_art_rect(bounds);
    if !right_side {
        return left_rect;
    }
    let inset_x = left_rect.left() - bounds.left();
    Rect::from_min_size(
        Pos2::new(bounds.right() - inset_x - left_rect.width(), left_rect.top()),
        left_rect.size(),
    )
}

fn draw_cinema_stage(
    app: &mut App,
    ui: &mut Ui,
    bounds: Rect,
    track: Option<&std::sync::Arc<Track>>,
    position: u64,
    motion_on: bool,
    full_window_cinema: bool,
) {
    let right_side = app.lyrics_nowbar_right;
    let art = cinema_art_rect_sided(bounds, right_side);
    let art_radius = (bounds.height() * 0.016).clamp(8.0, 16.0);

    // Track pointer motion inside Cinema stage (Fullscreen.ts 750ms idle fade).
    let now = ui.input(|i| i.time);
    let ptr_pos = ui.input(|i| i.pointer.hover_pos());
    let move_state_id = ui.scope_id().with("cinema-ptr-move");
    let (prev_pos, mut last_move_at) = ui
        .ctx()
        .data(|d| d.get_temp::<(Option<Pos2>, f64)>(move_state_id))
        .unwrap_or((None, -10.0));
    if let Some(p) = ptr_pos.filter(|p| bounds.contains(*p)) {
        if prev_pos.is_none_or(|old| old.distance(p) > 1.5) {
            last_move_at = now;
        }
    }
    ui.ctx()
        .data_mut(|d| d.insert_temp(move_state_id, (ptr_pos, last_move_at)));

    let force_hover = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("lyrics-force-cover-hover")))
        .unwrap_or(false);
    let vol_drag_id = ui.scope_id().with("cinema-vol-drag");
    let was_vol_dragging = ui.ctx().data(|d| d.get_temp::<bool>(vol_drag_id)).unwrap_or(false);
    let art_hovered = force_hover
        || was_vol_dragging
        || app.lyrics_manager_open
        || ui.rect_contains_pointer(art.expand(4.0));
    let recent_page_move = ptr_pos.is_some_and(|p| bounds.contains(p)) && (now - last_move_at) < 0.75;
    if recent_page_move && !art_hovered {
        ui.ctx().request_repaint_after(Duration::from_millis(760));
    }

    let target_controls = if art_hovered {
        1.0
    } else if recent_page_move {
        0.65
    } else {
        0.0
    };
    let controls_alpha = if motion_on {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with("cinema-controls-opacity"),
            target_controls,
            0.22,
        )
    } else {
        target_controls
    };

    if let Some(t) = track {
        let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
        cover_shadow(ui.painter(), art, art_radius);
        cover(ui, art, &tex, art_radius);
        if controls_alpha > 0.01 {
            ui.painter().rect_filled(
                art,
                CornerRadius::same(art_radius.round() as u8),
                Color32::from_black_alpha((112.0 * controls_alpha).round() as u8),
            );
            paint_cinema_heart(app, ui, art, t.id, controls_alpha, motion_on);
        }
    } else {
        ui.painter().rect_filled(art, art_radius, CARD);
    }

    if controls_alpha > 0.01 {
        if full_window_cinema {
            draw_view_controls(app, ui, art, true, controls_alpha, motion_on);
        }

        // Bottom 5 transport controls inside cover overlay.
        let cy = art.bottom() - art.height() * 0.105;
        let strip_rect = Rect::from_center_size(
            Pos2::new(art.center().x, cy),
            Vec2::new(art.width() * 0.72, art.height() * 0.12),
        );
        let strip_hovered = ui.rect_contains_pointer(strip_rect);
        let buttons = [
            (Icon::Shuffle, Action::ToggleShuffle, app.player.shuffle, "Phát ngẫu nhiên", "Shuffle", 0.052),
            (Icon::Previous, Action::Previous, false, "Bài trước", "Previous", 0.068),
            (
                if app.is_playing() { Icon::Pause } else { Icon::Play },
                Action::TogglePlay,
                false,
                "Phát / tạm dừng",
                "Play / pause",
                0.082,
            ),
            (Icon::Next, Action::Next, false, "Bài tiếp theo", "Next", 0.068),
            (
                if app.player.repeat == Repeat::Track { Icon::RepeatOne } else { Icon::Repeat },
                Action::CycleRepeat,
                app.player.repeat != Repeat::Off,
                "Lặp lại",
                "Repeat",
                0.052,
            ),
        ];
        for (i, (icon, action, active, vi, en, glyph_frac)) in buttons.into_iter().enumerate() {
            let r = Rect::from_center_size(
                Pos2::new(art.left() + art.width() * (0.22 + i as f32 * 0.14), cy),
                Vec2::splat(art.width() * 0.10),
            );
            let btn_hov = ui.rect_contains_pointer(r);
            let dim = if strip_hovered && !btn_hov { 0.55 } else { 1.0 };
            let alpha = ((if active || btn_hov { 255.0 } else { 228.0 }) * dim * controls_alpha)
                .round()
                .clamp(0.0, 255.0) as u8;
            if active {
                ui.painter().circle_filled(
                    r.center(),
                    art.width() * 0.036,
                    Color32::from_white_alpha((36.0 * controls_alpha).round() as u8),
                );
            }
            if matches!(icon, Icon::Previous | Icon::Next) {
                let resp = ui
                    .interact(r, ui.scope_id().with(("cinema-transport", i)), egui::Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(app.t(vi, en));
                let draw_col = Color32::from_white_alpha(if resp.hovered() {
                    (255.0 * controls_alpha).round() as u8
                } else {
                    alpha
                });
                paint_spicy_double_skip(
                    ui.painter(),
                    Rect::from_center_size(r.center(), Vec2::splat(art.width() * glyph_frac * 1.15)),
                    icon == Icon::Next,
                    draw_col,
                );
                if resp.clicked() {
                    app.actions.push(action);
                }
            } else if icon_button_sized(
                ui,
                r,
                ("cinema-transport", i),
                icon,
                Color32::from_white_alpha(alpha),
                app.t(vi, en),
                art.width() * glyph_frac,
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            {
                app.actions.push(action);
            }
        }

        // Right vertical frosted volume capsule + % scrub preview pill.
        draw_cinema_volume_capsule(app, ui, art, controls_alpha, motion_on, vol_drag_id);
    } else {
        ui.ctx().data_mut(|d| d.insert_temp(vol_drag_id, false));
    }

    // Spicy Lyrics glass capsule timeline + scrub preview tooltip below cover.
    let duration = app.duration_ms();
    let frac = if duration > 0 { position as f32 / duration as f32 } else { 0.0 };
    let shown = app.seek_drag.unwrap_or(frac).clamp(0.0, 1.0);
    let time_col = Color32::from_white_alpha(205);
    let timeline = Rect::from_min_size(
        art.left_bottom() + Vec2::new(54.0, 18.0),
        Vec2::new((art.width() - 108.0).max(40.0), 20.0),
    );
    spicy_capsule_timeline(app, ui, timeline, shown, duration, motion_on);

    ui.painter().text(
        timeline.left_center() - Vec2::new(10.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        mmss((shown * duration as f32) as u64),
        semi(13.5),
        time_col,
    );
    let remaining = app.show_remaining_time();
    let time_label = if remaining {
        format!("-{}", mmss(duration.saturating_sub((shown * duration as f32) as u64)))
    } else {
        mmss(duration)
    };
    let time_rect = ui.painter().text(
        timeline.right_center() + Vec2::new(10.0, 0.0),
        egui::Align2::LEFT_CENTER,
        time_label,
        semi(13.5),
        time_col,
    );
    if ui
        .interact(
            time_rect.expand(4.0),
            ui.scope_id().with("cinema-time-mode"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        app.actions.push(Action::ToggleRemainingTime);
    }

    if let Some(t) = track {
        let max_title_w = (art.width() * 0.86).max(80.0);
        let title_rect = Rect::from_center_size(
            Pos2::new(art.center().x, timeline.bottom() + 22.0),
            Vec2::new(max_title_w, 32.0),
        );
        let font = bold((art.width() * 0.056).clamp(18.0, 28.0));
        let title_col = Color32::from_white_alpha(244);
        let galley = ui.painter().layout_no_wrap(t.title.clone(), font.clone(), title_col);
        if galley.size().x <= title_rect.width() {
            ui.painter()
                .galley(title_rect.center() - galley.size() * 0.5, galley, title_col);
        } else {
            marquee_text(ui, title_rect, "cinema-title", &t.title, font, title_col);
        }
        if ui
            .interact(title_rect, ui.scope_id().with("cinema-title-link"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
        {
            app.actions.push(Action::Navigate(super::right_panel::track_route(t)));
        }
        let artist_parts = split_artists(&t.artist);
        let artist_font_sz = (art.width() * 0.039).clamp(13.5, 19.5);
        let sep_w = text_width(ui, ", ", body(artist_font_sz));
        let raw_artist_w: f32 = artist_parts
            .iter()
            .map(|n| text_width(ui, n, body(artist_font_sz)))
            .sum::<f32>()
            + sep_w * artist_parts.len().saturating_sub(1) as f32
            + 6.0;
        let artist_w = raw_artist_w.min(art.width() * 0.90);
        let artist_rect = Rect::from_center_size(
            Pos2::new(art.center().x, title_rect.bottom() + 13.0),
            Vec2::new(artist_w.max(24.0), 24.0),
        );
        super::right_panel::draw_artist_links(
            app,
            ui,
            artist_rect,
            &t.artist,
            artist_font_sz,
            "cinema-artists",
        );
    }

    let content_rect = if right_side {
        Rect::from_min_max(
            bounds.min,
            Pos2::new(bounds.left() + bounds.width() * 0.54, bounds.bottom()),
        )
    } else {
        Rect::from_min_max(
            Pos2::new(bounds.left() + bounds.width() * 0.45, bounds.top()),
            bounds.max,
        )
    };

    if full_window_cinema && app.lyrics_cinema_queue {
        draw_cinema_queue(app, ui, content_rect);
    } else {
        draw_center_lyrics(
            app,
            ui,
            content_rect,
            track.map(|t| t.id),
            position,
            motion_on,
            true,
        );
    }

    if full_window_cinema {
        draw_cinema_mode_pill(app, ui, bounds);
    }
}

fn paint_cinema_heart(
    app: &mut App,
    ui: &mut Ui,
    art: Rect,
    track_id: TrackId,
    controls_alpha: f32,
    motion_on: bool,
) {
    let liked = app.data.is_liked(track_id);
    let heart_box = Rect::from_center_size(art.center(), art.size() * 0.54);
    let resp = ui
        .interact(
            heart_box,
            ui.scope_id().with("cinema-save"),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(app.t("Lưu / bỏ lưu bài hát", "Save / remove song"));
    let target_scale = if resp.is_pointer_button_down_on() {
        0.90
    } else if resp.hovered() {
        1.05
    } else {
        1.0
    };
    let scale = if motion_on {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with("cinema-heart-scale"),
            target_scale,
            0.14,
        )
    } else {
        target_scale
    };
    // Exact Spicy Lyrics `Icons.Heart` cubic Bezier path in 0..1000 viewBox (Icons.ts:116).
    // Ordered as two smooth halves: top cleft (500.5, 190.1) -> right lobe -> bottom tip (500.5, 936)
    // -> left lobe -> top cleft, so unliked strokes use round joins at the 2 corners without miter spikes.
    const RIGHT_HALF: [([f32; 2], [f32; 2], [f32; 2], [f32; 2]); 4] = [
        ([500.496, 190.119], [545.998, 114.641], [628.493, 64.0], [723.02, 64.0]),
        ([723.02, 64.0], [866.598, 64.0], [982.995, 180.45], [982.995, 324.097]),
        ([982.995, 324.097], [982.995, 356.935], [976.659, 388.226], [965.562, 417.155]),
        ([965.562, 417.155], [879.595, 664.536], [500.499, 936.0], [500.499, 936.0]),
    ];
    const LEFT_HALF: [([f32; 2], [f32; 2], [f32; 2], [f32; 2]); 4] = [
        ([500.499, 936.0], [500.499, 936.0], [121.399, 664.536], [35.436, 417.155]),
        ([35.436, 417.155], [24.3385, 388.224], [18.0, 356.935], [18.0, 324.097]),
        ([18.0, 324.097], [18.0, 180.45], [134.394, 64.0], [277.973, 64.0]),
        ([277.973, 64.0], [372.499, 64.0], [454.992, 114.641], [500.496, 190.119]),
    ];
    let draw_size = heart_box.width() * 0.88 * scale;
    let origin = heart_box.center() - Vec2::splat(draw_size * 0.5);
    let map_pt = |p: [f32; 2]| -> Pos2 {
        origin + Vec2::new(p[0] / 1000.0 * draw_size, p[1] / 1000.0 * draw_size)
    };
    let sample_half = |segs: &[([f32; 2], [f32; 2], [f32; 2], [f32; 2]); 4]| -> Vec<Pos2> {
        const STEPS: usize = 16;
        let mut out = Vec::with_capacity(segs.len() * STEPS + 1);
        for (si, (p0, p1, p2, p3)) in segs.iter().enumerate() {
            let start = if si == 0 { 0 } else { 1 };
            for step in start..=STEPS {
                let t = step as f32 / STEPS as f32;
                let u = 1.0 - t;
                let x = u * u * u * p0[0]
                    + 3.0 * u * u * t * p1[0]
                    + 3.0 * u * t * t * p2[0]
                    + t * t * t * p3[0];
                let y = u * u * u * p0[1]
                    + 3.0 * u * u * t * p1[1]
                    + 3.0 * u * t * t * p2[1]
                    + t * t * t * p3[1];
                out.push(map_pt([x, y]));
            }
        }
        out
    };
    let right_pts = sample_half(&RIGHT_HALF);
    let left_pts = sample_half(&LEFT_HALF);
    let mut ring = Vec::with_capacity(right_pts.len() + left_pts.len());
    ring.extend_from_slice(&right_pts);
    ring.extend_from_slice(&left_pts[1..left_pts.len() - 1]);
    // Remove degenerate coincident cusp points so miter normals at the bottom tip and top cleft never flip.
    ring.dedup_by(|a, b| a.distance(*b) < 0.15);
    if ring.len() > 2 && ring.first().zip(ring.last()).is_some_and(|(a, b)| a.distance(*b) < 0.15) {
        ring.pop();
    }
    let n = ring.len();
    let alpha = ((if liked { 252.0 } else { 196.0 }) * controls_alpha)
        .round()
        .clamp(0.0, 255.0) as u8;
    let col = Color32::from_white_alpha(alpha);
    if liked {
        let fan_center = map_pt([500.5, 420.0]);
        let mut mesh = Mesh::default();
        mesh.colored_vertex(fan_center, col);
        for &p in &ring {
            mesh.colored_vertex(p, col);
        }
        for i in 0..n {
            mesh.add_triangle(0, 1 + i as u32, 1 + ((i + 1) % n) as u32);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
        let aa = Stroke::new(1.2, col);
        ui.painter().add(egui::Shape::closed_line(ring, aa));
    } else if draw_size <= 36.0 && n >= 4 {
        // Crisp small outline heart for Popup Lyrics (no broken cusp miter gap at small icon sizes).
        let thin = (draw_size * 0.10).clamp(1.75, 2.4);
        ui.painter().add(egui::Shape::closed_line(ring, Stroke::new(thin, col)));
    } else if n >= 4 {
        // Non-overlapping closed triangle-strip ribbon for large Cinema cover heart.
        let stroke_w = (draw_size * 0.032).clamp(3.0, 6.8);
        let half_w = stroke_w * 0.5;
        let mut mesh = Mesh::default();
        for i in 0..n {
            let prev = ring[(i + n - 1) % n];
            let curr = ring[i];
            let next = ring[(i + 1) % n];
            let d0 = (curr - prev).normalized();
            let d1 = (next - curr).normalized();
            let n0 = Vec2::new(d0.y, -d0.x);
            let n1 = Vec2::new(d1.y, -d1.x);
            let bis = n0 + n1;
            let normal = if bis.length_sq() > 0.25 { bis.normalized() } else { n0 };
            let miter = (1.0 / normal.dot(n0).abs().max(0.75)).min(1.28);
            let off = normal * (half_w * miter);
            mesh.colored_vertex(curr - off, col);
            mesh.colored_vertex(curr + off, col);
        }
        for i in 0..n {
            let a0 = (i * 2) as u32;
            let a1 = a0 + 1;
            let b0 = (((i + 1) % n) * 2) as u32;
            let b1 = b0 + 1;
            mesh.add_triangle(a0, a1, b0);
            mesh.add_triangle(a1, b1, b0);
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    }
    if resp.clicked() {
        app.actions.push(Action::ToggleLike(track_id));
    }
}

fn draw_cinema_volume_capsule(
    app: &mut App,
    ui: &mut Ui,
    art: Rect,
    controls_alpha: f32,
    motion_on: bool,
    vol_drag_id: egui::Id,
) {
    let hit_rect = Rect::from_center_size(
        art.right_center() - Vec2::new(art.width() * 0.062, 0.0),
        Vec2::new(26.0, art.height() * 0.34),
    );
    let response = ui
        .interact(
            hit_rect,
            ui.scope_id().with("cinema-volume"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let dragging = response.dragged() || response.is_pointer_button_down_on();
    ui.ctx().data_mut(|d| d.insert_temp(vol_drag_id, dragging));
    let engaged = response.hovered() || dragging;
    let target_w = if engaged { 22.0 } else { 18.0 };
    let w = if motion_on {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with("cinema-vol-width"),
            target_w,
            0.16,
        )
    } else {
        target_w
    };
    let volume_rect = Rect::from_center_size(hit_rect.center(), Vec2::new(w, hit_rect.height()));
    let radius = (w * 0.5).round() as u8;

    let mut cur_vol = if app.data.settings.muted {
        0.0
    } else {
        app.data.settings.volume.clamp(0.0, 1.0)
    };
    if let Some(pos) = response.interact_pointer_pos() {
        if dragging || response.clicked() {
            cur_vol = ((volume_rect.bottom() - pos.y) / volume_rect.height()).clamp(0.0, 1.0);
            app.actions.push(Action::SetVolume(cur_vol));
        }
    }
    if engaged {
        let delta = ui.input(|i| i.smooth_scroll_delta.y);
        if delta.abs() > 0.0 {
            cur_vol = (cur_vol + delta / 1200.0).clamp(0.0, 1.0);
            app.actions.push(Action::SetVolume(cur_vol));
        }
    }
    response.widget_info(|| {
        egui::WidgetInfo::slider(ui.is_enabled(), cur_vol as f64, "Volume")
    });

    ui.painter().rect_filled(
        volume_rect,
        CornerRadius::same(radius),
        Color32::from_black_alpha((64.0 * controls_alpha).round() as u8),
    );
    if cur_vol > 0.005 {
        let fill_top = volume_rect.bottom() - volume_rect.height() * cur_vol;
        let fill_rect = Rect::from_min_max(Pos2::new(volume_rect.left(), fill_top), volume_rect.max);
        let mut clip_p = ui.painter().clone();
        clip_p.set_clip_rect(fill_rect.intersect(ui.clip_rect()));
        clip_p.rect_filled(
            volume_rect,
            CornerRadius::same(radius),
            Color32::from_white_alpha((238.0 * controls_alpha).round() as u8),
        );
    }
    ui.painter().rect_stroke(
        volume_rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, Color32::from_white_alpha((96.0 * controls_alpha).round() as u8)),
        egui::StrokeKind::Inside,
    );

    let spk_icon = if cur_vol <= 0.005 {
        Icon::Mute
    } else if cur_vol < 0.45 {
        Icon::VolumeLow
    } else {
        Icon::Volume
    };
    let spk_col = if cur_vol > 0.14 {
        Color32::from_rgba_unmultiplied(20, 22, 26, (215.0 * controls_alpha).round() as u8)
    } else {
        Color32::from_white_alpha((220.0 * controls_alpha).round() as u8)
    };
    icons::paint(
        ui.painter(),
        Rect::from_center_size(
            Pos2::new(volume_rect.center().x, volume_rect.bottom() - 11.0),
            Vec2::splat(11.0),
        ),
        spk_icon,
        spk_col,
    );

    if engaged {
        let hover_y = ui
            .input(|i| i.pointer.hover_pos())
            .map(|p| p.y.clamp(volume_rect.top(), volume_rect.bottom()))
            .unwrap_or(volume_rect.bottom() - volume_rect.height() * cur_vol);
        let pct_val = if dragging {
            cur_vol
        } else {
            ((volume_rect.bottom() - hover_y) / volume_rect.height()).clamp(0.0, 1.0)
        };
        let label = format!("{}%", (pct_val * 100.0).round() as u32);
        let lw = text_width(ui, &label, semi(12.5)) + 14.0;
        let pill_r = Rect::from_center_size(
            Pos2::new(volume_rect.left() - 8.0 - lw * 0.5, hover_y),
            Vec2::new(lw, 24.0),
        );
        ui.painter().rect_filled(
            pill_r,
            CornerRadius::same(7),
            Color32::from_rgba_unmultiplied(18, 18, 22, 204),
        );
        ui.painter().rect_stroke(
            pill_r,
            CornerRadius::same(7),
            Stroke::new(1.0, Color32::from_white_alpha(46)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            pill_r.center(),
            egui::Align2::CENTER_CENTER,
            label,
            semi(12.5),
            Color32::WHITE,
        );
    }
}

fn spicy_capsule_timeline(
    app: &mut App,
    ui: &mut Ui,
    hit_rect: Rect,
    shown: f32,
    duration_ms: u64,
    motion_on: bool,
) {
    let resp = ui
        .interact(
            hit_rect,
            ui.scope_id().with("cinema-scrubber"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let dragging = resp.dragged() || resp.is_pointer_button_down_on();
    let engaged = resp.hovered() || dragging;
    let target_h = if engaged { 11.0 } else { 7.0 };
    let bar_h = if motion_on {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with("cinema-timeline-h"),
            target_h,
            0.16,
        )
    } else {
        target_h
    };
    let bar_rect = Rect::from_center_size(hit_rect.center(), Vec2::new(hit_rect.width(), bar_h));
    let radius = CornerRadius::same((bar_h * 0.5).round() as u8);

    let mut cur = shown;
    if let Some(pos) = resp.interact_pointer_pos() {
        let f = ((pos.x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0);
        if dragging {
            app.seek_drag = Some(f);
            cur = f;
        }
        if resp.drag_stopped() || resp.clicked() {
            let target = app.seek_drag.take().unwrap_or(f);
            if app.can_play_in_app() && duration_ms > 0 {
                app.actions
                    .push(Action::SeekTo((target * duration_ms as f32).round() as u64));
            }
        }
    } else if resp.drag_stopped() {
        if let Some(target) = app.seek_drag.take() {
            if app.can_play_in_app() && duration_ms > 0 {
                app.actions
                    .push(Action::SeekTo((target * duration_ms as f32).round() as u64));
            }
        }
    }

    ui.painter()
        .rect_filled(bar_rect, radius, Color32::from_white_alpha(40));
    if cur > 0.002 {
        let fill_right = bar_rect.left() + bar_rect.width() * cur;
        let fill_clip = Rect::from_min_max(bar_rect.min, Pos2::new(fill_right, bar_rect.bottom()));
        let mut p = ui.painter().clone();
        p.set_clip_rect(fill_clip.intersect(ui.clip_rect()));
        p.rect_filled(bar_rect, radius, Color32::WHITE);
    }
    ui.painter().rect_stroke(
        bar_rect,
        radius,
        Stroke::new(1.0, Color32::from_white_alpha(26)),
        egui::StrokeKind::Inside,
    );

    if engaged && duration_ms > 0 {
        if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
            let clamped_x = ptr.x.clamp(bar_rect.left(), bar_rect.right());
            let hover_f = if dragging {
                cur
            } else {
                ((clamped_x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0)
            };
            let label = mmss((hover_f * duration_ms as f32).round() as u64);
            let lw = text_width(ui, &label, semi(12.5)) + 14.0;
            let cx = clamped_x.clamp(bar_rect.left() + lw * 0.5, bar_rect.right() - lw * 0.5);
            let pill_r = Rect::from_center_size(
                Pos2::new(cx, bar_rect.top() - 18.0),
                Vec2::new(lw, 24.0),
            );
            ui.painter().rect_filled(
                pill_r,
                CornerRadius::same(7),
                Color32::from_rgba_unmultiplied(18, 18, 22, 204),
            );
            ui.painter().rect_stroke(
                pill_r,
                CornerRadius::same(7),
                Stroke::new(1.0, Color32::from_white_alpha(46)),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                pill_r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                semi(12.5),
                Color32::WHITE,
            );
        }
    }
}

fn draw_cinema_queue(app: &mut App, ui: &mut Ui, bounds: Rect) {
    let inset_x = (bounds.width() * 0.05).clamp(20.0, 48.0);
    let inner = Rect::from_min_max(
        Pos2::new(bounds.left() + inset_x, bounds.top() + 56.0),
        Pos2::new(bounds.right() - inset_x, bounds.bottom() - 68.0),
    );
    let mut col_ui = ui.new_child(egui::UiBuilder::new().id_salt("cinema-upnext").max_rect(inner));
    col_ui.set_clip_rect(inner.intersect(ui.clip_rect()));

    col_ui.label(
        RichText::new(app.t("Tiếp theo", "Up Next"))
            .font(bold(30.0))
            .color(Color32::WHITE),
    );
    col_ui.add_space(6.0);
    let sub_label = if !app.player.queue.is_empty() {
        app.t("Danh sách chờ", "Queue")
    } else {
        app.t("Tự động phát", "Autoplay")
    };
    col_ui.label(
        RichText::new(sub_label)
            .font(semi(14.0))
            .color(Color32::from_white_alpha(180)),
    );
    col_ui.add_space(14.0);

    let mut items: Vec<(TrackId, Option<usize>)> = app
        .player
        .queue
        .iter()
        .enumerate()
        .map(|(qi, &id)| (id, Some(qi)))
        .collect();
    for id in app.player.upcoming(25) {
        items.push((id, None));
    }

    let list_w = col_ui.available_width().max(160.0);
    let list_h = col_ui.available_height().max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("cinema-upnext-scroll")
        .max_height(list_h)
        .auto_shrink([false, false])
        .show(&mut col_ui, |ui| {
            if items.is_empty() {
                ui.add_space(32.0);
                ui.label(
                    RichText::new(app.t(
                        "Chưa có bài tiếp theo trong hàng chờ",
                        "No upcoming songs in queue",
                    ))
                    .font(semi(15.0))
                    .color(Color32::from_white_alpha(165)),
                );
                return;
            }
            for (row_idx, (tid, q_idx)) in items.into_iter().enumerate() {
                let Some(t) = app.track(tid) else { continue };
                let (row_rect, resp) =
                    ui.allocate_exact_size(Vec2::new(list_w, 58.0), egui::Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                let hov = resp.hovered();
                if hov {
                    ui.painter().rect_filled(
                        row_rect,
                        CornerRadius::same(10),
                        Color32::from_white_alpha(24),
                    );
                    ui.painter().rect_stroke(
                        row_rect,
                        CornerRadius::same(10),
                        Stroke::new(1.0, Color32::from_white_alpha(30)),
                        egui::StrokeKind::Inside,
                    );
                }
                let cov_r = Rect::from_center_size(
                    Pos2::new(row_rect.left() + 30.0, row_rect.center().y),
                    Vec2::splat(44.0),
                );
                let tex = app.art.get(ui.ctx(), &t.cover_key(), t.art_source());
                cover(ui, cov_r, &tex, 6.0);
                if hov {
                    ui.painter()
                        .rect_filled(cov_r, 6.0, Color32::from_black_alpha(120));
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(cov_r.center(), Vec2::splat(18.0)),
                        Icon::Play,
                        Color32::WHITE,
                    );
                }
                let text_x = cov_r.right() + 14.0;
                let right_x = row_rect.right() - 14.0;
                let text_max = (right_x - text_x - 56.0).max(40.0);
                text(
                    ui.painter(),
                    Pos2::new(text_x, row_rect.center().y - 9.0),
                    &t.title,
                    semi(15.0),
                    Color32::WHITE,
                    text_max,
                    egui::Align2::LEFT_CENTER,
                );
                text(
                    ui.painter(),
                    Pos2::new(text_x, row_rect.center().y + 10.0),
                    &t.artist,
                    body(13.0),
                    Color32::from_white_alpha(175),
                    text_max,
                    egui::Align2::LEFT_CENTER,
                );

                let liked = app.data.is_liked(tid);
                let mut like_clicked = false;
                if hov || liked {
                    let hr = Rect::from_center_size(
                        Pos2::new(right_x - if hov { 10.0 } else { 44.0 }, row_rect.center().y),
                        Vec2::splat(26.0),
                    );
                    let h_resp = ui
                        .interact(
                            hr,
                            ui.scope_id().with(("cinema-q-like", row_idx)),
                            egui::Sense::click(),
                        )
                        .on_hover_cursor(egui::CursorIcon::PointingHand);
                    icons::paint(
                        ui.painter(),
                        Rect::from_center_size(hr.center(), Vec2::splat(15.0)),
                        if liked { Icon::Saved } else { Icon::Save },
                        if liked {
                            ACCENT
                        } else if h_resp.hovered() {
                            Color32::WHITE
                        } else {
                            Color32::from_white_alpha(190)
                        },
                    );
                    if h_resp.clicked() {
                        like_clicked = true;
                        app.actions.push(Action::ToggleLike(tid));
                    }
                }
                if !hov && t.duration_ms > 0 {
                    text(
                        ui.painter(),
                        Pos2::new(right_x, row_rect.center().y),
                        &mmss(t.duration_ms),
                        semi(13.0),
                        Color32::from_white_alpha(175),
                        48.0,
                        egui::Align2::RIGHT_CENTER,
                    );
                }
                if resp.clicked() && !like_clicked {
                    if let Some(qi) = q_idx {
                        app.actions.push(Action::PlayQueued(qi));
                    } else {
                        app.actions.push(Action::JumpUpcoming(tid));
                    }
                }
            }
        });
}

fn draw_cinema_mode_pill(app: &mut App, ui: &mut Ui, bounds: Rect) {
    let pill_w = 78.0;
    let pill_h = 38.0;
    let pill_rect = Rect::from_min_size(
        Pos2::new(bounds.right() - pill_w - 20.0, bounds.bottom() - pill_h - 18.0),
        Vec2::new(pill_w, pill_h),
    );
    let r = CornerRadius::same((pill_h * 0.5) as u8);
    ui.painter()
        .rect_filled(pill_rect, r, Color32::from_black_alpha(64));
    ui.painter()
        .rect_filled(pill_rect, r, Color32::from_white_alpha(18));
    ui.painter().rect_stroke(
        pill_rect,
        r,
        Stroke::new(1.0, Color32::from_white_alpha(48)),
        egui::StrokeKind::Inside,
    );

    let tabs = [
        (false, 10u8, app.t("Lời bài hát", "Lyrics")),
        (true, 5u8, app.t("Tiếp theo", "Queue")),
    ];
    for (idx, (want_queue, glyph_kind, tip)) in tabs.into_iter().enumerate() {
        let cx = pill_rect.left() + 20.0 + idx as f32 * 38.0;
        let btn_r = Rect::from_center_size(Pos2::new(cx, pill_rect.center().y), Vec2::splat(32.0));
        let active = app.lyrics_cinema_queue == want_queue;
        let resp = ui
            .interact(
                btn_r,
                ui.scope_id().with(("cinema-mode-tab", idx)),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tip);
        if active || resp.hovered() {
            ui.painter().circle_filled(
                btn_r.center(),
                15.0,
                Color32::from_white_alpha(if active { 42 } else { 24 }),
            );
        }
        paint_spicy_view_glyph(
            ui.painter(),
            Rect::from_center_size(btn_r.center(), Vec2::splat(17.0)),
            glyph_kind,
            false,
            false,
            if active || resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_white_alpha(180)
            },
        );
        if resp.clicked() {
            app.lyrics_cinema_queue = want_queue;
        }
    }
}

fn draw_view_controls(
    app: &mut App,
    ui: &mut Ui,
    anchor: Rect,
    cinema_mode: bool,
    parent_alpha: f32,
    motion_on: bool,
) {
    // Unified frosted glass capsule matching Spicy Lyrics 6.4+ `.ViewControls` (f_009.png & f_030.png).
    let items: Vec<(&str, Icon, &str, bool, u8)> = if cinema_mode {
        vec![
            (
                "viewctl-upnext",
                Icon::Queue,
                app.t("Danh sách phát tiếp theo", "Up Next"),
                app.lyrics_cinema_queue,
                5,
            ),
            (
                "viewctl-side",
                if app.lyrics_nowbar_right {
                    Icon::LibraryCollapse
                } else {
                    Icon::LibraryExpand
                },
                app.t("Đổi bên ảnh bìa (Trái / Phải)", "Swap cover side"),
                app.lyrics_nowbar_right,
                6,
            ),
            (
                "viewctl-fullscreen",
                if app.lyrics_fullscreen.is_some() {
                    Icon::FullscreenExit
                } else {
                    Icon::Fullscreen
                },
                if app.lyrics_fullscreen.is_some() {
                    app.t("Thoát toàn màn hình", "Exit fullscreen")
                } else {
                    app.t("Toàn màn hình", "Fullscreen")
                },
                app.lyrics_fullscreen.is_some(),
                1,
            ),
            (
                "viewctl-copy",
                Icon::OpenLyrics,
                app.t("Sao chép lời bài hát", "Copy lyrics"),
                false,
                7,
            ),
            (
                "viewctl-ttml",
                Icon::Gear,
                app.t("Cài đặt & Quản lý TTML", "Lyrics settings & TTML"),
                app.lyrics_manager_open,
                4,
            ),
            (
                "viewctl-close-cinema",
                Icon::Close,
                app.t("Thoát chế độ rạp", "Exit cinema view"),
                false,
                8,
            ),
        ]
    } else {
        vec![
            (
                "viewctl-cinema",
                Icon::Miniplayer,
                app.t("Chế độ rạp toàn cửa sổ", "Cinema view (full window)"),
                app.lyrics_cinema,
                2,
            ),
            (
                "viewctl-split",
                Icon::NowPlaying,
                app.t("Hiện / ẩn ảnh bìa cạnh lời", "Toggle cover beside lyrics"),
                app.lyrics_split_nowbar,
                9,
            ),
            (
                "viewctl-copy",
                Icon::OpenLyrics,
                app.t("Sao chép lời bài hát", "Copy lyrics"),
                false,
                7,
            ),
            (
                "viewctl-ttml",
                Icon::Gear,
                app.t("Cài đặt & Quản lý TTML", "Lyrics settings & TTML"),
                app.lyrics_manager_open,
                4,
            ),
            (
                "viewctl-close",
                Icon::Close,
                app.t("Đóng lời bài hát", "Close lyrics"),
                false,
                0,
            ),
        ]
    };

    let btn_sz = if cinema_mode {
        (anchor.width() * 0.082).clamp(28.0, 34.0)
    } else {
        32.0
    };
    let gap = if cinema_mode { 5.0 } else { 8.0 };
    let pad_x = 12.0;
    let pad_y = 4.0;
    let bar_w = items.len() as f32 * btn_sz + (items.len() as f32 - 1.0) * gap + pad_x * 2.0;
    let bar_h = btn_sz + pad_y * 2.0;
    let top_y = anchor.top() + if cinema_mode { anchor.height() * 0.06 } else { 20.0 };
    let bar_rect = Rect::from_min_size(
        Pos2::new(anchor.center().x - bar_w * 0.5, top_y),
        Vec2::new(bar_w, bar_h),
    );

    let target_opacity = if cinema_mode {
        parent_alpha
    } else {
        let force_top = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("lyrics-force-top-hover")))
            .unwrap_or(false);
        let top_hover_zone = Rect::from_min_size(anchor.min, Vec2::new(anchor.width(), 96.0));
        let bar_hovered = force_top
            || ui.rect_contains_pointer(bar_rect.expand(12.0))
            || app.lyrics_manager_open;
        if bar_hovered {
            1.0
        } else if ui.rect_contains_pointer(top_hover_zone) {
            0.86
        } else {
            0.0
        }
    };
    let opacity = if motion_on && !cinema_mode {
        ui.ctx().animate_value_with_time(
            ui.scope_id().with("viewcontrols-vis"),
            target_opacity,
            0.22,
        )
    } else {
        target_opacity
    };
    if opacity <= 0.01 {
        return;
    }

    let radius = CornerRadius::same((bar_h * 0.5).round() as u8);
    // Clean frosted glass capsule (`rgba(255,255,255,0.09)` + subtle specular border, matching `668563dd.png`).
    let pill_shadow = egui::epaint::Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha((26.0 * opacity).round() as u8),
    };
    ui.painter().add(pill_shadow.as_shape(bar_rect.shrink(1.0), radius));
    ui.painter().rect_filled(
        bar_rect,
        radius,
        Color32::from_black_alpha((16.0 * opacity).round() as u8),
    );
    ui.painter().rect_filled(
        bar_rect,
        radius,
        Color32::from_white_alpha((24.0 * opacity).round() as u8),
    );
    ui.painter().rect_stroke(
        bar_rect,
        radius,
        Stroke::new(1.0, Color32::from_white_alpha((46.0 * opacity).round() as u8)),
        egui::StrokeKind::Inside,
    );

    for (idx, (id, _icon, tip, active, kind)) in items.iter().enumerate() {
        let cx = bar_rect.left() + pad_x + btn_sz * 0.5 + idx as f32 * (btn_sz + gap);
        let r = Rect::from_center_size(Pos2::new(cx, bar_rect.center().y), Vec2::splat(btn_sz));
        let resp = ui
            .interact(r, ui.scope_id().with(*id), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let hov = resp.hovered() || resp.has_focus();
        if resp.hovered() {
            paint_clamped_tooltip(ui, r, tip, anchor, true);
        }
        if *active || hov {
            ui.painter().circle_filled(
                r.center(),
                btn_sz * 0.46,
                Color32::from_white_alpha(
                    ((if hov { 40.0 } else { 26.0 }) * opacity).round() as u8,
                ),
            );
        }
        let icon_alpha = ((if *active || hov { 255.0 } else { 226.0 }) * opacity)
            .round()
            .clamp(0.0, 255.0) as u8;
        let icon_scale = crate::theme::hover_scale(hov, resp.is_pointer_button_down_on());
        let icon_sz = (btn_sz * 0.56).clamp(16.0, 20.0) * icon_scale;
        paint_spicy_view_glyph(
            ui.painter(),
            Rect::from_center_size(r.center(), Vec2::splat(icon_sz)),
            *kind,
            app.lyrics_nowbar_right,
            app.lyrics_fullscreen.is_some(),
            Color32::from_white_alpha(icon_alpha),
        );
        if resp.clicked() {
            match kind {
                0 => app.actions.push(Action::ToggleLyrics),
                1 => app.actions.push(Action::ToggleLyricsFullscreen),
                2 => app.actions.push(Action::ToggleLyricsCinema),
                4 => {
                    app.lyrics_manager_open = !app.lyrics_manager_open;
                    app.lyrics_delete_confirm_for = None;
                }
                5 => app.lyrics_cinema_queue = !app.lyrics_cinema_queue,
                6 => app.lyrics_nowbar_right = !app.lyrics_nowbar_right,
                7 => {
                    if let Some(lyr) = app.current_lyrics() {
                        let full_text = lyr
                            .lines
                            .iter()
                            .map(|l| l.text.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !full_text.is_empty() {
                            ui.ctx().copy_text(full_text);
                            app.toast = Some((
                                app.t(
                                    "Đã sao chép lời bài hát",
                                    "Lyrics copied to clipboard",
                                )
                                .to_string(),
                                std::time::Instant::now(),
                            ));
                        }
                    }
                }
                8 => {
                    if app.lyrics_fullscreen.is_some() {
                        app.actions.push(Action::ToggleLyricsFullscreen);
                    }
                    app.lyrics_cinema = false;
                    app.lyrics_cinema_queue = false;
                }
                9 => app.lyrics_split_nowbar = !app.lyrics_split_nowbar,
                _ => {}
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_lyrics_scroll_body(
    ui: &mut Ui,
    salt: &str,
    lyrics: &Lyrics,
    track_id: Option<TrackId>,
    position: u64,
    playing: bool,
    motion_on: bool,
    font_size: f32,
    total_width: f32,
    height: f32,
    left_margin: f32,
    ended_label: &str,
    sync_label: &str,
    compact_card: bool,
    follow_offset: f32,
    credits: Option<(&str, &str)>,
) -> Option<u64> {
    let is_popup = salt == "spicy-popup-scroll";
    let is_line_only = lyrics.synced && lyrics.lines.iter().all(|l| l.spans.is_empty());
    let eff_pos = if playing && is_line_only {
        position.saturating_sub(85)
    } else {
        position
    };
    let active = lyrics.display_current(eff_pos);
    let follow_id = ui.scope_id().with(salt).with("follow");
    let now = ui.input(|i| i.time);
    let dt = ui.input(|i| i.unstable_dt).clamp(0.001, 0.05);

    let mut follow = ui
        .data_mut(|d| d.get_temp::<LyricsFollowState>(follow_id))
        .filter(|s| s.track == track_id)
        .unwrap_or(LyricsFollowState {
            track: track_id,
            active_visible: true,
            ..Default::default()
        });

    let area_rect = ui.available_rect_before_wrap();
    let area_rect = Rect::from_min_size(area_rect.min, Vec2::new(total_width, height));
    let fade_rect = area_rect.intersect(ui.clip_rect());

    // Expand hit zone by 28px horizontally so dragging/clicking the right scrollbar is ALWAYS treated as manual scroll!
    let pointer_in_area = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| area_rect.expand2(Vec2::new(28.0, 4.0)).contains(p));
    let manual_input = pointer_in_area
        && ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::MouseWheel { .. }
                    | egui::Event::Touch { phase: egui::TouchPhase::Move, .. }))
                || (i.pointer.primary_down() && i.pointer.is_decidedly_dragging())
        });
    let force_manual_offscreen = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("lyrics-force-manual-scroll")))
        .unwrap_or(false);
    // Process timeline seek / playback resume BEFORE deciding scroll target so scrubbing syncs immediately!
    let resumed_now = follow.resume_follow(now, position, playing, manual_input && lyrics.synced);
    if force_manual_offscreen && lyrics.synced {
        follow.manual_scroll_at = Some(now);
        follow.anim_target_y = None;
    }

    let first_vocal_ms = lyrics
        .lines
        .iter()
        .find(|l| !l.background && !is_instrumental_line(&l.text))
        .and_then(|l| l.time_ms);
    let in_intro = lyrics.synced && first_vocal_ms.is_some_and(|ms| eff_pos < ms);

    let manual_paused = follow.manual_scroll_at.is_some();
    let hide_blur = manual_paused;
    // ONLY reset scroll to 0.0 during a true intro (before first vocal line) — NEVER mid-song!
    let needs_intro_reset = !manual_paused && in_intro && follow.last_active.is_some();
    let wants_recenter = !manual_paused
        && lyrics.synced
        && active.is_some()
        && (resumed_now || active != follow.last_active || !follow.active_visible);
    if needs_intro_reset {
        follow.anim_target_y = Some(0.0);
    }
    follow.last_active = active;

    let mut seek_target: Option<u64> = None;
    let mut sync_clicked = false;
    let mut active_rect_seen: Option<Rect> = None;
    let mut active_content_center_y: Option<f32> = None;

    // Advance silky critically-damped exponential scroll glide toward `anim_target_y` (matches Spotify's ~450ms ease).
    let mut applied_scroll_y: Option<f32> = None;
    if !manual_paused {
        if let Some(target_y) = follow.anim_target_y {
            let cur_y = follow.last_scroll_y;
            let diff = target_y - cur_y;
            if !motion_on || diff.abs() < 0.75 {
                applied_scroll_y = Some(target_y);
                follow.anim_target_y = None;
            } else {
                let step = 1.0 - (-dt * 8.5).exp();
                let next_y = (cur_y + diff * step).max(0.0);
                applied_scroll_y = Some(next_y);
                ui.ctx().request_repaint();
            }
        }
    }
    follow.programmatic_y = applied_scroll_y;

    // Key the ScrollArea by `track_id` so switching songs NEVER inherits the previous song's bottom scroll offset!
    let mut scroll_area = egui::ScrollArea::vertical()
        .id_salt((salt, track_id))
        .max_height(height)
        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded)
        .scroll_source(egui::scroll_area::ScrollSource::ALL)
        .auto_shrink([false, false])
        .animated(false);
    if let Some(y) = applied_scroll_y {
        scroll_area = scroll_area.vertical_scroll_offset(y);
    }
    let scroll_out = scroll_area.show(ui, |ui| {
            let content_origin_y = ui.cursor().top();
            let lead_space = if !lyrics.synced {
                if compact_card { 12.0 } else { 64.0 }
            } else if is_popup {
                (height * 0.24).clamp(26.0, 68.0)
            } else if compact_card {
                if font_size >= 35.2 { height * 0.267 } else { 24.0 }
            } else {
                (height * 0.36).max(68.0)
            };
            ui.add_space(lead_space);

            let line_w = (total_width - left_margin.max(0.0) * 2.0)
                .clamp(120.0, 760.0);
            let centered_left = ((total_width - line_w) * 0.5).max(0.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if centered_left > 0.0 {
                    ui.add_space(centered_left);
                }
                ui.vertical(|ui| {
                    ui.set_width(line_w);
                    // Comfortable Spotify/Spicy Lyrics vertical rhythm (`~0.68em` main gap, tight `0.18em` for background ad-libs).
                    let row_gap = if is_popup {
                        (font_size * 0.54).clamp(12.0, 20.0)
                    } else if compact_card {
                        (font_size * 0.62).clamp(15.5, 22.0)
                    } else {
                        (font_size * 0.68).clamp(18.0, 30.0)
                    };
                    let (pad_h, fade_h) = if is_popup {
                        (6.0, (height * 0.14).clamp(26.0, 44.0))
                    } else if compact_card {
                        (6.0, (height * 0.14).clamp(32.0, 48.0))
                    } else {
                        // Generous soft vertical vignette matching Spotify Desktop (~16% top/bottom fade).
                        (8.0, (height * 0.16).clamp(68.0, 118.0))
                    };
                    if lyrics.synced {
                        if let Some(first_ms) = first_vocal_ms {
                            if eff_pos < first_ms
                                && should_render_synthetic_intro_dots(&lyrics.lines, first_ms)
                            {
                                let dots_r = paint_interlude_dots(
                                    ui,
                                    font_size,
                                    eff_pos,
                                    0,
                                    first_ms,
                                    true,
                                    motion_on,
                                    fade_rect,
                                    fade_h,
                                );
                                active_rect_seen = Some(dots_r);
                                active_content_center_y = Some(dots_r.center().y - content_origin_y);
                                ui.add_space(if compact_card { 8.0 } else { 14.0 });
                            }
                        }
                    }

                    let has_duet = lyrics.lines.iter().any(|l| !l.right_align)
                        && lyrics.lines.iter().any(|l| l.right_align);
                    let wrap_w = if has_duet && line_w >= 380.0 {
                        (line_w * 0.84).max(260.0)
                    } else {
                        line_w
                    };
                    // Reserve two painter slots strictly behind all lyric lines for the single morphing hover pill.
                    let pill_fill_slot = ui.painter().add(egui::Shape::Noop);
                    let pill_stroke_slot = ui.painter().add(egui::Shape::Noop);
                    let mut hovered_content_rect: Option<Rect> = None;

                    for (index, line) in lyrics.lines.iter().enumerate() {
                        // Detect if a mid-song instrumental interlude gap (>= 3.5s) follows this main line,
                        // suppressing synthetic dots whenever an explicit instrumental line already covers the break.
                        let interlude_gap = synthetic_mid_interlude_gap(&lyrics.lines, index);
                        let in_following_gap = interlude_gap
                            .is_some_and(|(gs, ge)| eff_pos >= gs && eff_pos < ge);
                        let is_focus = active == Some(index) && !in_following_gap;
                        let is_singing = is_focus
                            || (line.background && lyrics.is_line_singing(index, eff_pos));
                        let next_is_bg = lyrics.lines.get(index + 1).is_some_and(|n| n.background);
                        let line_right = has_duet && line.right_align;
                        let line_font_sz = if line.background {
                            (font_size * 0.72).clamp(13.0, 24.0)
                        } else {
                            font_size
                        };
                        let line_id = ui.scope_id().with(salt).with(("line-hov", index));
                        let prev_hovered = ui.ctx().data(|d| d.get_temp::<bool>(line_id)).unwrap_or(false);
                        let hover_t = if !prev_hovered && !motion_on {
                            0.0
                        } else if motion_on {
                            ui.ctx().animate_bool_with_time(line_id.with("t"), prev_hovered, 0.18)
                        } else {
                            1.0
                        };
                        let dist = active.map(|cur| index.abs_diff(cur)).unwrap_or(0);
                        let (mut color, lift_px, scale, mut blur_px) = line_style(
                            ui,
                            salt,
                            index,
                            lyrics.synced,
                            is_line_only,
                            if is_singing { Some(index) } else if in_following_gap { None } else { active },
                            hide_blur,
                            motion_on,
                            dist,
                        );
                        if in_following_gap {
                            color = PAST_SUNG_WHITE;
                            blur_px = if hide_blur { 0.0 } else { 1.1 };
                        }
                        if line.background && !is_singing {
                            color = Color32::from_white_alpha((color.a() as f32 * 0.82).round() as u8);
                            blur_px *= 0.85;
                        }
                        let eff_blur = blur_px * (1.0 - hover_t);
                        let eff_alpha = egui::lerp(color.a() as f32..=255.0, hover_t).round() as u8;
                        let eff_color = Color32::from_white_alpha(eff_alpha);
                        let word_pos = is_singing.then_some(eff_pos);
                        let line_end_ms = lyrics.lines.get(index + 1).and_then(|n| n.time_ms);

                        if lyrics.synced && is_instrumental_line(&line.text) && line.time_ms.is_some() {
                            // Collapse consecutive instrumental/blank lines into a single `• • •` interlude.
                            if let Some((start_ms, end_ms, run_end_idx)) =
                                explicit_instrumental_span(&lyrics.lines, index)
                            {
                                let run_active = active
                                    .is_some_and(|cur| cur >= index && cur <= run_end_idx);
                                let dots_r = paint_interlude_dots(
                                    ui,
                                    font_size,
                                    eff_pos,
                                    start_ms,
                                    end_ms,
                                    run_active,
                                    motion_on,
                                    fade_rect,
                                    fade_h,
                                );
                                if run_active {
                                    active_rect_seen = Some(dots_r);
                                    active_content_center_y = Some(dots_r.center().y - content_origin_y);
                                }
                                ui.add_space(row_gap);
                            }
                            continue;
                        }

                        let (resp, text_rect, seek) = line_widget(
                            ui,
                            line,
                            line_font_sz,
                            wrap_w,
                            eff_color,
                            word_pos,
                            line_end_ms,
                            motion_on,
                            lift_px,
                            scale,
                            eff_blur,
                            fade_rect,
                            pad_h,
                            fade_h,
                            line_right,
                        );
                        let now_hovered = resp.hovered();
                        if now_hovered != prev_hovered {
                            ui.ctx().data_mut(|d| d.insert_temp(line_id, now_hovered));
                            ui.ctx().request_repaint();
                        }
                        if now_hovered && line.time_ms.is_some() {
                            // Optically center Vietnamese ascenders/descenders inside the rounded hover pill.
                            let shifted = text_rect.translate(Vec2::new(0.0, lift_px - content_origin_y));
                            let pill = Rect::from_min_max(
                                Pos2::new(shifted.min.x - 16.0, shifted.min.y - 10.0),
                                Pos2::new(shifted.max.x + 16.0, shifted.max.y + 6.0),
                            );
                            hovered_content_rect = Some(pill);
                        }
                        if is_focus {
                            active_rect_seen = Some(resp.rect);
                            active_content_center_y = Some(resp.rect.center().y - content_origin_y);
                        }
                        if let Some(ms) = seek {
                            seek_target = Some(ms);
                            follow.manual_scroll_at = None;
                            follow.last_active = None;
                            follow.anim_target_y = None;
                        }
                        let gap_after = if next_is_bg {
                            (row_gap * 0.24).clamp(4.0, 8.0)
                        } else {
                            row_gap
                        };
                        ui.add_space(gap_after);

                        // Smoothly expand animated 3-dot interlude (`• • •`) during mid-song instrumental solos (>= 3.5s).
                        if let Some((gs, ge)) = interlude_gap {
                            let gap_anim_id = ui.scope_id().with(salt).with(("mid-gap", index));
                            let gap_t = if motion_on {
                                ui.ctx().animate_bool_with_time(gap_anim_id, in_following_gap, 0.32)
                            } else if in_following_gap {
                                1.0
                            } else {
                                0.0
                            };
                            if gap_t > 0.02 {
                                let dots_r = paint_interlude_dots(
                                    ui,
                                    font_size * (0.65 + 0.35 * gap_t),
                                    eff_pos,
                                    gs,
                                    ge,
                                    in_following_gap,
                                    motion_on,
                                    fade_rect,
                                    fade_h,
                                );
                                if in_following_gap {
                                    active_rect_seen = Some(dots_r);
                                    active_content_center_y = Some(dots_r.center().y - content_origin_y);
                                }
                                ui.add_space(row_gap * 0.55 * gap_t);
                            }
                        }
                    }

                    // Smoothly glide & morph the single shared Spotify frosted hover pill between lyric lines!
                    let pill_id = ui.scope_id().with(salt).with("hover-pill");
                    let mut pill_state = ui
                        .ctx()
                        .data(|d| d.get_temp::<LyricsHoverPillState>(pill_id))
                        .filter(|s| s.track == track_id)
                        .unwrap_or(LyricsHoverPillState {
                            track: track_id,
                            ..Default::default()
                        });
                    if pill_state.step(hovered_content_rect, dt, motion_on) {
                        ui.ctx().request_repaint();
                    }
                    if let Some(content_pill) = pill_state.rect {
                        if pill_state.opacity > 0.01 {
                            let screen_pill = content_pill.translate(Vec2::new(0.0, content_origin_y));
                            let cy = screen_pill.center().y;
                            let edge_fade = if fade_h <= 0.5 {
                                1.0
                            } else {
                                (smoothstep(pad_h, pad_h + fade_h, cy - fade_rect.top())
                                    * smoothstep(pad_h, pad_h + fade_h, fade_rect.bottom() - cy))
                                .clamp(0.0, 1.0)
                            };
                            let a = (pill_state.opacity * edge_fade).clamp(0.0, 1.0);
                            if a > 0.01 && screen_pill.intersects(fade_rect.expand(12.0)) {
                                let radius = CornerRadius::same(12);
                                ui.painter().set(
                                    pill_fill_slot,
                                    egui::Shape::rect_filled(
                                        screen_pill,
                                        radius,
                                        Color32::from_white_alpha((34.0 * a).round() as u8),
                                    ),
                                );
                                ui.painter().set(
                                    pill_stroke_slot,
                                    egui::Shape::rect_stroke(
                                        screen_pill,
                                        radius,
                                        Stroke::new(1.0, Color32::from_white_alpha((14.0 * a).round() as u8)),
                                        egui::StrokeKind::Inside,
                                    ),
                                );
                            }
                        }
                    }
                    ui.ctx().data_mut(|d| d.insert_temp(pill_id, pill_state));

                    if let Some((writer, provider)) = credits {
                        // Spicy Lyrics `Exp_RefinedCredits` (`.SongCredits`, Mixed.css:1145-1209):
                        // dimmed 0.55 until the last lyric line is active or finished (`LinesAllSung`).
                        let last_active =
                            !lyrics.lines.is_empty() && active == Some(lyrics.lines.len() - 1);
                        let all_sung =
                            !lyrics.synced || hide_blur || last_active || lyrics.ended(eff_pos);
                        let dim: f32 = if all_sung { 1.0 } else { 0.55 };
                        let credit_sz = (font_size * 0.52).clamp(12.6, 15.5);
                        let prov_sz = (credit_sz * 0.90).clamp(11.6, 13.8);
                        ui.add_space((font_size * 0.16).clamp(3.0, 6.0));

                        let mut w_job = egui::text::LayoutJob::default();
                        w_job.wrap.max_width = line_w;
                        w_job.append(
                            "Written by ",
                            0.0,
                            TextFormat {
                                font_id: semi(credit_sz),
                                color: Color32::from_white_alpha((168.0 * dim).round() as u8),
                                ..Default::default()
                            },
                        );
                        w_job.append(
                            writer,
                            0.0,
                            TextFormat {
                                font_id: bold(credit_sz),
                                color: Color32::from_white_alpha((224.0 * dim).round() as u8),
                                ..Default::default()
                            },
                        );
                        let w_galley = ui.fonts_mut(|f| f.layout_job(w_job));
                        let (w_rect, _) = ui.allocate_exact_size(w_galley.size(), egui::Sense::hover());
                        if w_rect.intersects(fade_rect) {
                            let masked_w = mask_galley_vertical(&w_galley, w_rect.min, fade_rect, pad_h, fade_h, None);
                            ui.painter().galley(w_rect.min, masked_w, Color32::WHITE);
                        }

                        ui.add_space(3.5);
                        let prov_text = format!("Provided by {provider}");
                        let p_galley = ui.painter().layout(
                            prov_text,
                            semi(prov_sz),
                            Color32::from_white_alpha((142.0 * dim).round() as u8),
                            line_w,
                        );
                        let (p_rect, _) = ui.allocate_exact_size(p_galley.size(), egui::Sense::hover());
                        if p_rect.intersects(fade_rect) {
                            let masked_p = mask_galley_vertical(&p_galley, p_rect.min, fade_rect, pad_h, fade_h, None);
                            ui.painter().galley(p_rect.min, masked_p, Color32::WHITE);
                        }
                    } else if active.is_none() && lyrics.ended(position) {
                        ui.add_space(8.0);
                        ui.label(RichText::new(ended_label).font(semi(font_size * 0.65)).color(PAST_SUNG_WHITE));
                    }
                });
            });

            let trail_space = if is_popup { 52.0 } else { lead_space };
            ui.add_space(trail_space);
        });

    let clip_rect = scroll_out.inner_rect;
    let cur_scroll_y = scroll_out.state.clamped_offset().y;
    let max_scroll_y = (scroll_out.content_size.y - clip_rect.height()).max(0.0);

    // Detect ANY manual scroll (including scrollbar thumb drag, track click, wheel inertia) by comparing actual offset vs expected.
    let unexpected_scroll = lyrics.synced
        && match follow.programmatic_y {
            Some(py) => (cur_scroll_y - py).abs() > 3.5,
            None => (cur_scroll_y - follow.last_scroll_y).abs() > 2.0,
        };
    follow.last_scroll_y = cur_scroll_y;
    if unexpected_scroll && !resumed_now {
        follow.manual_scroll_at = Some(now);
        follow.anim_target_y = None;
    }

    let anchor_y = if compact_card && !is_popup {
        (height * 0.24).min(64.0)
    } else if is_popup {
        height * 0.44
    } else {
        // Spotify Desktop places the active line cleanly at ~40% from top so 3-4 upcoming lines remain visible below.
        height * 0.40 + follow_offset
    };

    let manual_paused_now = follow.manual_scroll_at.is_some();
    // Active line is considered "in view" for the sync button if visible, and "in reading band" for auto-follow resume.
    let active_in_view = !force_manual_offscreen
        && active_rect_seen.is_some_and(|r| {
            let cy = r.center().y;
            cy >= clip_rect.top() + height * 0.10 && cy <= clip_rect.bottom() - height * 0.10
        });
    let active_in_reading_band = !force_manual_offscreen
        && active_rect_seen.is_some_and(|r| (r.center().y - (clip_rect.top() + anchor_y)).abs() <= height * 0.20);

    if let Some(content_cy) = active_content_center_y {
        let desired_y = (content_cy - anchor_y).clamp(0.0, max_scroll_y);
        if !manual_paused_now && lyrics.synced && (wants_recenter || (desired_y - cur_scroll_y).abs() > height * 0.18) {
            if follow.anim_target_y.is_none_or(|t| (t - desired_y).abs() > 1.5) {
                follow.anim_target_y = Some(desired_y);
                ui.ctx().request_repaint();
            }
        }
    }

    follow.active_visible = if manual_paused_now {
        active_in_reading_band
    } else {
        active_in_view
    };
    if resumed_now {
        ui.ctx().request_repaint();
    }
    if let Some(t) = follow.manual_scroll_at {
        if follow.active_visible {
            ui.ctx().request_repaint_after(Duration::from_secs_f64(
                (MANUAL_SCROLL_RESUME_SECS - (now - t)).max(0.0),
            ));
        }
    }

    // Floating `.ScrollToActive` pure frosted glass circle (NO muddy dark drop shadow — crisp translucent ring matching Spotify).
    if lyrics.synced && manual_paused_now && !active_in_view && active.is_some() {
        let _ = sync_label;
        let (btn_d, inset_r, inset_b, icon_d) = if compact_card {
            (32.0, 24.0, 24.0, 12.0)
        } else {
            (38.0, 28.0, 28.0, 14.0)
        };
        let btn_r = Rect::from_center_size(
            Pos2::new(clip_rect.right() - inset_r, clip_rect.bottom() - inset_b),
            Vec2::splat(btn_d),
        );
        let resp = ui
            .interact(
                btn_r,
                ui.scope_id().with(salt).with("sync-pill"),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let force_sync_hov = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("lyrics-force-sync-hover")))
            .unwrap_or(false);
        let hov = resp.hovered() || force_sync_hov;
        let pressed = resp.is_pointer_button_down_on();
        let rad = btn_d * 0.5;
        let fill_a = if pressed { 44 } else if hov { 34 } else { 18 };
        ui.painter()
            .circle_filled(btn_r.center(), rad, Color32::from_white_alpha(fill_a));
        ui.painter().circle_stroke(
            btn_r.center(),
            rad - 0.5,
            Stroke::new(1.0, Color32::from_white_alpha(if hov { 86 } else { 48 })),
        );
        // `ScrollToActive.css:124-126` & `ScrollToActiveLine.ts:189`: `Above` (`^`) vs `Below` (`v`)
        let force_below = ui
            .ctx()
            .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("lyrics-force-sync-below")))
            .unwrap_or(false);
        let active_is_above = !force_below
            && active_rect_seen.is_none_or(|r| r.center().y <= clip_rect.center().y);
        let chev_icon = if active_is_above {
            SpicyIcon::SyncChevron
        } else {
            SpicyIcon::SyncChevronDown
        };
        let ic_scale = if pressed { 0.95 } else if hov { 1.05 } else { 1.0 };
        icons::paint_spicy(
            ui.painter(),
            Rect::from_center_size(btn_r.center(), Vec2::splat(icon_d * ic_scale)),
            chev_icon,
            Color32::from_white_alpha(if hov { 255 } else { 235 }),
        );
        if resp.clicked() {
            sync_clicked = true;
        }
    }

    if sync_clicked {
        follow.manual_scroll_at = None;
        follow.last_active = None;
        if let Some(content_cy) = active_content_center_y {
            let desired_y = (content_cy - anchor_y).clamp(0.0, max_scroll_y);
            follow.anim_target_y = Some(desired_y);
        }
        ui.ctx().request_repaint();
    }

    ui.data_mut(|d| d.insert_temp(follow_id, follow));
    seek_target
}

fn line_style(
    ui: &Ui,
    salt: &str,
    index: usize,
    synced: bool,
    is_line_only: bool,
    active: Option<usize>,
    hide_blur: bool,
    motion_on: bool,
    dist: usize,
) -> (Color32, f32, f32, f32) {
    if !synced {
        return (Color32::from_rgba_unmultiplied(255, 255, 255, 232), 0.0, 1.0, 0.0);
    }

    let is_active = active == Some(index);
    let act_t = if motion_on {
        ui.ctx().animate_bool_with_time(
            ui.scope_id().with(salt).with(("line-act", index)),
            is_active,
            0.22,
        )
    } else if is_active {
        1.0
    } else {
        0.0
    };

    let base_color = match active {
        None => ACTIVE_UNSUNG_WHITE,
        Some(cur) if index == cur => Color32::WHITE,
        Some(_) => {
            if hide_blur {
                Color32::from_rgba_unmultiplied(255, 255, 255, 136)
            } else {
                // `Mixed.css:232-270`: `.line.Sung` (`0.497 * 0.85 = 0.422`), `.line.NotSung` (`0.51`)
                let a = match dist {
                    0 | 1 => 118,
                    2 => 104,
                    3 => 88,
                    _ => 72,
                };
                Color32::from_rgba_unmultiplied(255, 255, 255, a)
            }
        }
    };

    // `Animator/Shared.ts:5`: `BlurMultiplier = 1.25`
    let blur_px = if !hide_blur && active.is_some() && !is_active {
        (dist as f32 * 1.25).min(6.4)
    } else {
        0.0
    };
    let lift_px = if motion_on { -1.2 * act_t } else { 0.0 };
    // `Mixed.css:706`: `.line.Active` scales up (`1.04` on Line-synced, `1.02` on Syllable-synced)
    let scale_add = if is_line_only { 0.04 } else { 0.02 };
    let scale = if motion_on { 1.0 + scale_add * act_t } else { 1.0 };
    (base_color, lift_px, scale, blur_px)
}

#[derive(Clone, Copy, Debug)]
struct CharAnim {
    alpha: f32,
    glow: f32,
    blur_px: f32,
    y_offset: f32,
}

fn eval_spline(stops: &[(f32, f32)], t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    for w in stops.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if t <= t1 {
            let span = (t1 - t0).max(1e-5);
            let u = ((t - t0) / span).clamp(0.0, 1.0);
            let s = u * u * (3.0 - 2.0 * u);
            return v0 + (v1 - v0) * s;
        }
    }
    stops.last().map(|s| s.1).unwrap_or(0.0)
}

// Spicy Lyrics Animator curves (LyricsAnimator.ts GlowRange / YOffsetRange)
fn spicy_glow_spline(t: f32) -> f32 {
    eval_spline(&[(0.0, 0.0), (0.15, 1.0), (0.60, 1.0), (1.0, 0.0)], t)
}

fn spicy_y_offset_spline(t: f32) -> f32 {
    eval_spline(&[(0.0, 0.01), (0.90, -0.0167), (1.0, 0.0)], t)
}

fn is_instrumental_line(text: &str) -> bool {
    let t = text.trim();
    t.is_empty() || matches!(t, "♪" | "♫" | "♬" | "..." | "…" | "• • •")
}

/// Returns true if synthetic intro `• • •` dots should be inserted before `first_vocal_ms`,
/// suppressing them whenever the lyrics already contain an explicit instrumental line beforehand.
fn should_render_synthetic_intro_dots(lines: &[LyricLine], first_vocal_ms: u64) -> bool {
    if first_vocal_ms < 2_500 {
        return false;
    }
    !lines
        .iter()
        .any(|l| is_instrumental_line(&l.text) && l.time_ms.is_some_and(|t| t < first_vocal_ms))
}

/// Returns a synthetic mid-song `(gap_start_ms, gap_end_ms)` interlude after `lines[index]`
/// only when there is a >= 3.5s gap to the next main vocal line AND no explicit instrumental line
/// already covers that break (preventing double `• • •` dots).
fn synthetic_mid_interlude_gap(lines: &[LyricLine], index: usize) -> Option<(u64, u64)> {
    let line = lines.get(index)?;
    if line.background || is_instrumental_line(&line.text) {
        return None;
    }
    let end = line.end_ms?;
    let mut next_main_idx = None;
    for (offset, cand) in lines[index + 1..].iter().enumerate() {
        let cand_idx = index + 1 + offset;
        if is_instrumental_line(&cand.text) && cand.time_ms.is_some() {
            // An explicit instrumental/blank timed line already handles this interlude.
            return None;
        }
        if !cand.background {
            next_main_idx = Some(cand_idx);
            break;
        }
    }
    let next_start = lines.get(next_main_idx?)?.time_ms?;
    (next_start >= end.saturating_add(3_500)).then_some((end, next_start))
}

/// Collapses a run of one or more consecutive explicit instrumental/blank timed lines into a single
/// `(start_ms, end_ms, run_end_index)` span on the first line of the run (returning `None` for
/// subsequent lines in the run or sub-2s empty LRC spacers).
fn explicit_instrumental_span(lines: &[LyricLine], index: usize) -> Option<(u64, u64, usize)> {
    let line = lines.get(index)?;
    let start_ms = line.time_ms?;
    if !is_instrumental_line(&line.text) {
        return None;
    }
    // Suppress duplicate dots when the immediately preceding timed line was already instrumental.
    if index > 0 {
        let prev = &lines[index - 1];
        if is_instrumental_line(&prev.text) && prev.time_ms.is_some() {
            return None;
        }
    }
    let mut run_end = index;
    let mut explicit_symbol = !line.text.trim().is_empty();
    while let Some(next) = lines.get(run_end + 1) {
        if is_instrumental_line(&next.text) && next.time_ms.is_some() {
            explicit_symbol |= !next.text.trim().is_empty();
            run_end += 1;
        } else {
            break;
        }
    }
    let next_vocal_ms = lines.get(run_end + 1).and_then(|n| n.time_ms);
    let end_ms = lines[run_end]
        .end_ms
        .or(next_vocal_ms)
        .unwrap_or(start_ms + 4_000)
        .max(start_ms + 500);
    // Ignore tiny (< 2.2s) blank timestamped spacer lines that aren't real musical interludes.
    if !explicit_symbol && end_ms.saturating_sub(start_ms) < 2_200 {
        return None;
    }
    Some((start_ms, end_ms, run_end))
}

#[allow(clippy::too_many_arguments)]
fn paint_interlude_dots(
    ui: &mut Ui,
    font_size: f32,
    pos_ms: u64,
    start_ms: u64,
    end_ms: u64,
    is_active: bool,
    motion_on: bool,
    viewport: Rect,
    fade_h: f32,
) -> Rect {
    let row_h = (font_size * 1.15).max(24.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(font_size * 3.0, row_h), egui::Sense::click());
    if resp.clicked() {
        // Handled by caller if needed; interlude dots sit at start_ms.
    }
    let cy = rect.center().y;
    let edge_fade = if fade_h <= 1.0 {
        1.0
    } else {
        (smoothstep(0.0, fade_h, cy - viewport.top())
            * smoothstep(0.0, fade_h, viewport.bottom() - cy))
        .clamp(0.0, 1.0)
    };
    let dur = end_ms.saturating_sub(start_ms).max(1) as f32;
    let overall = if is_active {
        ((pos_ms.saturating_sub(start_ms)) as f32 / dur).clamp(0.0, 1.0)
    } else if pos_ms >= end_ms {
        1.0
    } else {
        0.0
    };
    let base_r = (font_size * 0.23).clamp(5.0, 10.0);
    let spacing = base_r * 3.1;
    let time_s = ui.input(|i| i.time) as f32;
    for i in 0..3 {
        let seg_start = i as f32 / 3.0;
        let local_p = ((overall - seg_start) * 3.0).clamp(0.0, 1.0);
        let idle_pulse = if is_active && motion_on {
            0.5 + 0.5 * (time_s * 3.2 - i as f32 * 0.9).sin()
        } else {
            0.5
        };
        let scale = if !is_active {
            0.78
        } else {
            eval_spline(&[(0.0, 0.82), (0.70, 1.15), (1.0, 1.0)], local_p)
                + 0.06 * idle_pulse * (1.0 - local_p)
        };
        let y_off = if is_active && motion_on {
            eval_spline(&[(0.0, 0.0), (0.65, -0.15), (1.0, 0.0)], local_p) * font_size
                - idle_pulse * 2.2 * (1.0 - local_p)
        } else {
            0.0
        };
        let alpha_f = if !is_active {
            0.38
        } else {
            eval_spline(&[(0.0, 0.48), (0.55, 1.0), (1.0, 0.94)], local_p)
                .max(0.46 + 0.18 * idle_pulse)
        } * edge_fade;
        let glow_f = if is_active {
            eval_spline(&[(0.0, 0.0), (0.55, 1.0), (1.0, 0.55)], local_p) * edge_fade
        } else {
            0.0
        };
        let center = Pos2::new(rect.left() + base_r * 1.2 + i as f32 * spacing, cy + y_off);
        if glow_f > 0.04 {
            ui.painter().circle_filled(
                center,
                base_r * scale * 2.1,
                Color32::from_white_alpha((32.0 * glow_f).round() as u8),
            );
            ui.painter().circle_filled(
                center,
                base_r * scale * 1.45,
                Color32::from_white_alpha((68.0 * glow_f).round() as u8),
            );
        }
        ui.painter().circle_filled(
            center,
            base_r * scale,
            Color32::from_white_alpha((255.0 * alpha_f).round().clamp(0.0, 255.0) as u8),
        );
    }
    rect
}

fn compute_active_char_anims(
    line: &LyricLine,
    display: &str,
    pos: u64,
    next_line_start_ms: Option<u64>,
    font_size: f32,
    motion_on: bool,
) -> Vec<CharAnim> {
    let char_count = display.chars().count().max(1);
    let mut anims = vec![
        CharAnim {
            alpha: 0.51,
            glow: 0.0,
            blur_px: 4.0,
            y_offset: 0.0,
        };
        char_count
    ];

    if !line.spans.is_empty() {
        let mut byte_to_char = vec![char_count; display.len() + 1];
        for (ci, (bi, _)) in display.char_indices().enumerate() {
            byte_to_char[bi] = ci;
        }
        let mut last = char_count;
        for b in (0..=display.len()).rev() {
            if byte_to_char[b] != char_count {
                last = byte_to_char[b];
            } else {
                byte_to_char[b] = last;
            }
        }

        let mut prev_end = 0usize;
        for span in &line.spans {
            if span.bytes.end > display.len() || span.bytes.start > span.bytes.end {
                continue;
            }
            let c_start = byte_to_char[span.bytes.start].min(char_count);
            let c_end = byte_to_char[span.bytes.end].min(char_count);

            // Fill spaces / punctuation between previous span and this span
            if c_start > prev_end {
                let gap_sung = pos >= span.start_ms;
                for item in &mut anims[prev_end..c_start] {
                    item.alpha = if gap_sung { 1.0 } else { 0.51 };
                    item.glow = if gap_sung { 0.14 } else { 0.0 };
                    item.blur_px = 4.5;
                }
            }

            let word_chars = c_end.saturating_sub(c_start);
            if word_chars > 0 {
                if pos < span.start_ms {
                    for item in &mut anims[c_start..c_end] {
                        item.alpha = 0.51;
                        item.glow = 0.0;
                    }
                } else if pos >= span.end_ms {
                    let tail = ((pos - span.end_ms) as f32 / 420.0).clamp(0.0, 1.0);
                    let glow = 0.42 * (1.0 - tail) + 0.44;
                    for item in &mut anims[c_start..c_end] {
                        item.alpha = 1.0;
                        item.glow = glow;
                        item.blur_px = 5.5 + 5.5 * glow;
                    }
                } else {
                    let dur = (span.end_ms - span.start_ms).max(1);
                    let p = ((pos - span.start_ms) as f32 / dur as f32).clamp(0.0, 1.0);
                    let is_held = dur >= 780 && word_chars >= 2;
                    for k in 0..word_chars {
                        let ci = c_start + k;
                        if is_held {
                            let lp = ((p * (word_chars as f32 + 0.65) - k as f32) / 1.15)
                                .clamp(0.0, 1.0);
                            let cursor_f = p * word_chars as f32;
                            let dist = (cursor_f - (k as f32 + 0.45)).abs();
                            let prox = (-dist * 0.55).exp();
                            let warm_trail = if lp > 0.0 { 0.30 * (1.0 - p * 0.45) } else { 0.0 };
                            let glow = (spicy_glow_spline(lp) * 0.96)
                                .max(0.48 * prox)
                                .max(warm_trail)
                                .clamp(0.0, 1.0);
                            anims[ci] = CharAnim {
                                alpha: 0.51 + 0.49 * lp,
                                glow,
                                blur_px: 4.0 + 11.5 * glow,
                                y_offset: if motion_on {
                                    spicy_y_offset_spline(lp) * font_size * 1.45
                                } else {
                                    0.0
                                },
                            };
                        } else {
                            let char_sweep =
                                ((p * (word_chars as f32 + 0.75) - (k as f32 + 0.30)) / 0.95)
                                    .clamp(0.0, 1.0);
                            let word_glow = spicy_glow_spline(p).max(0.30);
                            let glow = (word_glow * (0.55 + 0.45 * char_sweep)).clamp(0.0, 1.0);
                            anims[ci] = CharAnim {
                                alpha: 0.51 + 0.49 * char_sweep,
                                glow,
                                blur_px: 4.0 + 6.5 * glow,
                                y_offset: if motion_on {
                                    spicy_y_offset_spline(p) * font_size
                                } else {
                                    0.0
                                },
                            };
                        }
                    }
                }
            }
            prev_end = c_end;
        }

        if prev_end < char_count {
            let tail_sung = line.spans.last().is_some_and(|s| pos >= s.end_ms);
            for item in &mut anims[prev_end..char_count] {
                item.alpha = if tail_sung { 1.0 } else { 0.51 };
                item.glow = if tail_sung { 0.14 } else { 0.0 };
                item.blur_px = 4.5;
            }
        }
        return anims;
    }

    // Line-synced LRC: sweep Spicy Lyrics' ~0.55em gradient & glow smoothly character-by-character
    // across natural vocal phrase duration (weighting punctuation pauses so commas pause naturally).
    let start = line.time_ms.unwrap_or(pos);
    let raw_end = line
        .end_ms
        .or(next_line_start_ms)
        .unwrap_or(start + 3_600)
        .max(start + 1);
    let raw_dur = (raw_end - start) as f32;

    let mut weights = Vec::with_capacity(char_count);
    let mut total_weight = 0.0_f32;
    for ch in display.chars() {
        let w = if ch.is_whitespace() {
            0.55
        } else if matches!(ch, ',' | ';' | '、' | '，') {
            2.6
        } else if matches!(ch, '.' | '!' | '?' | '…' | '。' | '！' | '？') {
            3.6
        } else {
            1.0
        };
        weights.push(w);
        total_weight += w;
    }
    total_weight = total_weight.max(1.0);

    // Cap singing duration so an instrumental tail after a short line doesn't crawl indefinitely,
    // while allowing slow ballad phrases to sweep at natural vocal pace.
    let natural_cap_ms = (total_weight * 145.0 + 750.0).max(1_350.0);
    let vocal_dur = (raw_dur * 0.85).min(natural_cap_ms).max(500.0).min(raw_dur);
    let elapsed = pos.saturating_sub(start) as f32;
    let sweep_p = (elapsed / vocal_dur).clamp(0.0, 1.0);
    let fade_w = 5.2_f32; // Smooth 20% gradient transition band (`Mixed.css:119-125`)
    let cursor_w = sweep_p * (total_weight + fade_w);
    let post_phrase_decay = if elapsed > vocal_dur {
        ((elapsed - vocal_dur) / 600.0).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let mut cum_w = 0.0_f32;
    for (ci, &w) in weights.iter().enumerate() {
        let mid_w = cum_w + w * 0.5;
        cum_w += w;
        let char_p = ((cursor_w - mid_w + fade_w * 0.5) / fade_w).clamp(0.0, 1.0);
        let dist_behind = cursor_w - mid_w;
        let head_glow = if dist_behind >= -fade_w * 0.7 && dist_behind <= fade_w * 2.5 {
            let norm = (dist_behind / (fade_w * 1.45)).abs();
            (1.0 - norm * norm * 0.55).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let sung_trail = if char_p >= 0.95 {
            (0.82 * (1.0 - post_phrase_decay * 0.25)).max(0.62)
        } else {
            0.34 + 0.42 * char_p
        };
        let glow = head_glow.max(sung_trail).clamp(0.0, 1.0);
        let lift = if motion_on && char_p > 0.0 && char_p < 1.0 {
            -(char_p * std::f32::consts::PI).sin() * (font_size * 0.042).min(2.2)
        } else {
            0.0
        };
        anims[ci] = CharAnim {
            alpha: 0.53 + 0.47 * char_p,
            glow,
            blur_px: 6.5 + 8.0 * glow,
            y_offset: lift,
        };
    }
    anims
}

fn map_galley_glyphs_to_chars(
    galley: &egui::Galley,
    display: &str,
) -> Vec<Vec<usize>> {
    let chars: Vec<char> = display.chars().collect();
    let max_ci = chars.len().saturating_sub(1);
    let mut cursor = 0usize;
    galley
        .rows
        .iter()
        .map(|placed| {
            placed
                .row
                .glyphs
                .iter()
                .map(|g| {
                    let found = (cursor..chars.len().min(cursor + 6))
                        .find(|&idx| chars[idx] == g.chr);
                    if let Some(idx) = found {
                        cursor = (idx + 1).min(chars.len());
                        idx
                    } else {
                        cursor.min(max_ci)
                    }
                })
                .collect()
        })
        .collect()
}

/// Applies per-vertex vertical CSS `mask-image` alpha fade (`pad_px` transparent margin + `fade_px` smoothstep)
/// so neither wrapped multi-line lyrics nor Gaussian blur taps are ever sliced by `glScissor`.
fn mask_galley_vertical(
    base: &std::sync::Arc<egui::Galley>,
    origin: Pos2,
    viewport: Rect,
    pad_px: f32,
    fade_px: f32,
    tint: Option<Color32>,
) -> std::sync::Arc<egui::Galley> {
    let mut g = (**base).clone();
    for placed in &mut g.rows {
        let row = std::sync::Arc::make_mut(&mut placed.row);
        for v in &mut row.visuals.mesh.vertices {
            let vy = origin.y + placed.pos.y + v.pos.y;
            let mask_a = if fade_px <= 0.5 {
                1.0
            } else {
                let top_a = smoothstep(pad_px, pad_px + fade_px, vy - viewport.top());
                let bot_a = smoothstep(pad_px, pad_px + fade_px, viewport.bottom() - vy);
                (top_a * bot_a).clamp(0.0, 1.0)
            };
            let c = tint.unwrap_or(v.color);
            let [r, gr, b, a] = c.to_srgba_unmultiplied();
            let out_a = ((a as f32) * mask_a).round().clamp(0.0, 255.0) as u8;
            v.color = Color32::from_rgba_unmultiplied(r, gr, b, out_a);
        }
    }
    std::sync::Arc::new(g)
}

#[allow(clippy::too_many_arguments)]
fn make_tinted_galley(
    base: &egui::Galley,
    row_char_map: &[Vec<usize>],
    char_anims: &[CharAnim],
    dir: (f32, f32),
    ring_alpha_scale: f32,
    halo_pass: bool,
    origin: Pos2,
    viewport: Rect,
    pad_px: f32,
    fade_px: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut g = base.clone();
    for (ri, placed) in g.rows.iter_mut().enumerate() {
        let Some(glyph_chars) = row_char_map.get(ri) else {
            continue;
        };
        let row_y = origin.y + placed.pos.y;
        let row = std::sync::Arc::make_mut(&mut placed.row);
        let v_end = row.visuals.glyph_vertex_range.end.min(row.visuals.mesh.vertices.len());
        for (gi, glyph) in row.glyphs.iter().enumerate() {
            let ci = glyph_chars.get(gi).copied().unwrap_or(0);
            let anim = char_anims.get(ci).copied().unwrap_or(CharAnim {
                alpha: 1.0,
                glow: 0.0,
                blur_px: 4.0,
                y_offset: 0.0,
            });
            let first = glyph.first_vertex as usize;
            let next = row
                .glyphs
                .get(gi + 1)
                .map(|nx| nx.first_vertex as usize)
                .unwrap_or(v_end)
                .min(v_end);
            if first >= next {
                continue;
            }
            if halo_pass {
                if anim.glow <= 0.03 {
                    for v in &mut row.visuals.mesh.vertices[first..next] {
                        v.color = Color32::TRANSPARENT;
                    }
                    continue;
                }
                let r = anim.blur_px.min(14.0) * ring_alpha_scale;
                let dx = dir.0 * r;
                let dy = dir.1 * r + anim.y_offset;
                let base_a = if ring_alpha_scale <= 0.16 {
                    34.0
                } else if ring_alpha_scale <= 0.28 {
                    22.0
                } else {
                    12.0
                };
                for v in &mut row.visuals.mesh.vertices[first..next] {
                    v.pos.x += dx;
                    v.pos.y += dy;
                    let vy = row_y + v.pos.y;
                    let mask_a = if fade_px <= 0.5 {
                        1.0
                    } else {
                        smoothstep(pad_px, pad_px + fade_px, vy - viewport.top())
                            * smoothstep(pad_px, pad_px + fade_px, viewport.bottom() - vy)
                    };
                    let a = (anim.glow * base_a * mask_a).round().clamp(0.0, 255.0) as u8;
                    v.color = Color32::from_rgba_unmultiplied(255, 255, 255, a);
                }
            } else {
                for v in &mut row.visuals.mesh.vertices[first..next] {
                    v.pos.y += anim.y_offset;
                    let vy = row_y + v.pos.y;
                    let mask_a = if fade_px <= 0.5 {
                        1.0
                    } else {
                        smoothstep(pad_px, pad_px + fade_px, vy - viewport.top())
                            * smoothstep(pad_px, pad_px + fade_px, viewport.bottom() - vy)
                    };
                    let a = (anim.alpha * 255.0 * mask_a).round().clamp(0.0, 255.0) as u8;
                    v.color = Color32::from_rgba_unmultiplied(255, 255, 255, a);
                }
            }
        }
    }
    std::sync::Arc::new(g)
}

fn lyric_font(ui: &Ui, size: f32) -> egui::FontId {
    // Runtime #SpicyLyricsPage: SpicyLyrics 700 (fonts.spikerko.org/spicy-lyrics/LyricsBold.woff2).
    let family = egui::FontFamily::Name("lyrics".into());
    if ui.fonts(|f| f.families().contains(&family)) {
        egui::FontId::new(size, family)
    } else {
        bold(size)
    }
}

fn line_seek_time(line: &LyricLine, char_index: Option<usize>) -> Option<u64> {
    let byte = char_index.and_then(|i| line.text.char_indices().nth(i).map(|(b, _)| b));
    byte.and_then(|b| line.spans.iter().find(|s| s.bytes.contains(&b)).map(|s| s.start_ms))
        .or(line.time_ms)
}

#[derive(Clone, Default)]
struct BalancedWidthCache {
    entries: std::collections::HashMap<u64, f32>,
}

/// Lays out a lyric line at constant base font size with CSS `text-wrap: balance` semantics
/// so wrapped lines never leave lonely 1-word orphans and never re-wrap when scaled.
/// Memoizes the solved balanced width per `(text, size, wrap_w, right_align)` so scrolling at 60fps
/// shapes each line once per frame instead of 7 times.
fn layout_balanced_lyric(
    ui: &Ui,
    display: &str,
    size: f32,
    wrap_w: f32,
    right_align: bool,
) -> std::sync::Arc<egui::Galley> {
    use std::hash::{Hash, Hasher};
    let font = lyric_font(ui, size);
    let make = |w: f32| {
        let mut job = egui::text::LayoutJob::single_section(
            display.to_owned(),
            TextFormat {
                font_id: font.clone(),
                line_height: Some(size * 1.16),
                color: Color32::WHITE,
                ..Default::default()
            },
        );
        job.wrap.max_width = w;
        job.halign = if right_align { Align::RIGHT } else { Align::LEFT };
        ui.painter().layout_job(job)
    };
    if display.len() > 240 {
        return make(wrap_w);
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    display.hash(&mut hasher);
    (size.round() as i32).hash(&mut hasher);
    (wrap_w.round() as i32).hash(&mut hasher);
    right_align.hash(&mut hasher);
    let cache_key = hasher.finish();
    let cache_id = egui::Id::NULL.with("lyrics-balanced-w-cache");
    if let Some(cached_w) = ui
        .ctx()
        .data(|d| d.get_temp::<BalancedWidthCache>(cache_id).and_then(|c| c.entries.get(&cache_key).copied()))
    {
        return make(cached_w);
    }

    let galley = make(wrap_w);
    let row_count = galley.rows.len();
    let mut solved_w = wrap_w;
    let mut best = galley;
    if row_count >= 2 {
        let max_row_w = best
            .rows
            .iter()
            .map(|r| r.size.x)
            .fold(0.0_f32, f32::max);
        let last_row_w = best.rows.last().map(|r| r.size.x).unwrap_or(max_row_w);
        // Re-balance whenever the last wrapped line is disproportionately short (< 50% of widest row).
        if max_row_w > 1.0 && last_row_w < max_row_w * 0.50 {
            let mut lo = (max_row_w * 0.54).max(120.0);
            let mut hi = max_row_w;
            for _ in 0..6 {
                let mid = (lo + hi) * 0.5;
                let cand = make(mid);
                if cand.rows.len() <= row_count {
                    best = cand;
                    solved_w = mid;
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
        }
    }
    ui.ctx().data_mut(|d| {
        let mut cache = d.get_temp::<BalancedWidthCache>(cache_id).unwrap_or_default();
        if cache.entries.len() >= 512 {
            cache.entries.clear();
        }
        cache.entries.insert(cache_key, solved_w);
        d.insert_temp(cache_id, cache);
    });
    best
}

/// Scales a laid-out `Galley` in place around `(0.0, mid_y)` (left edge for `v1`, right edge for `v2`),
/// matching CSS `transform: scale(...)` with zero text re-wrapping!
fn scale_galley_in_place(galley: std::sync::Arc<egui::Galley>, scale: f32) -> std::sync::Arc<egui::Galley> {
    if (scale - 1.0).abs() < 0.001 {
        return galley;
    }
    let mid_y = galley.size().y * 0.5;
    let mut g = (*galley).clone();
    for placed in &mut g.rows {
        placed.pos.x *= scale;
        placed.pos.y = mid_y + (placed.pos.y - mid_y) * scale;
        let row = std::sync::Arc::make_mut(&mut placed.row);
        for v in &mut row.visuals.mesh.vertices {
            v.pos.x *= scale;
            v.pos.y *= scale;
        }
    }
    std::sync::Arc::new(g)
}

/// Spicy Lyrics line-synced 180° top-to-bottom vertical illumination wave (`linear-gradient(180deg, ...)`).
#[allow(clippy::too_many_arguments)]
fn make_vertical_line_galley(
    base: &egui::Galley,
    progress: f32,
    halo_pass: bool,
    dir: (f32, f32),
    ring_r: f32,
    ring_alpha: f32,
    origin: Pos2,
    viewport: Rect,
    pad_px: f32,
    fade_px: f32,
) -> std::sync::Arc<egui::Galley> {
    let total_h = base.size().y.max(1.0);
    // Soft vertical gradient band (~40% of line height) sweeping cleanly from top to bottom.
    let band = (total_h * 0.42).max(16.0);
    let front_y = progress * (total_h + band * 1.15) - band * 0.15;
    let mut g = base.clone();
    for placed in &mut g.rows {
        let row_local_y = placed.pos.y;
        let row_screen_y = origin.y + placed.pos.y;
        let row = std::sync::Arc::make_mut(&mut placed.row);
        let dx = if halo_pass { dir.0 * ring_r } else { 0.0 };
        let dy = if halo_pass { dir.1 * ring_r } else { 0.0 };
        for v in &mut row.visuals.mesh.vertices {
            let local_y = row_local_y + v.pos.y;
            let fill = ((front_y - local_y) / band + 0.5).clamp(0.0, 1.0);
            let smooth_fill = fill * fill * (3.0 - 2.0 * fill);
            if halo_pass {
                v.pos.x += dx;
                v.pos.y += dy;
            }
            let vy = row_screen_y + v.pos.y;
            let mask_a = if fade_px <= 0.5 {
                1.0
            } else {
                smoothstep(pad_px, pad_px + fade_px, vy - viewport.top())
                    * smoothstep(pad_px, pad_px + fade_px, viewport.bottom() - vy)
            };
            let a = if halo_pass {
                (smooth_fill * ring_alpha * mask_a).round().clamp(0.0, 255.0) as u8
            } else {
                ((0.52 + 0.48 * smooth_fill) * 255.0 * mask_a)
                    .round()
                    .clamp(0.0, 255.0) as u8
            };
            v.color = Color32::from_rgba_unmultiplied(255, 255, 255, a);
        }
    }
    std::sync::Arc::new(g)
}

#[allow(clippy::too_many_arguments)]
fn line_widget(
    ui: &mut Ui,
    line: &LyricLine,
    size: f32,
    wrap_w: f32,
    line_color: Color32,
    word_position: Option<u64>,
    next_line_start_ms: Option<u64>,
    motion_on: bool,
    lift_px: f32,
    scale: f32,
    blur_px: f32,
    viewport: Rect,
    pad_h: f32,
    fade_h: f32,
    right_align: bool,
) -> (egui::Response, Rect, Option<u64>) {
    let display = if line.text.is_empty() {
        " "
    } else {
        &line.text
    };
    let col_w = ui.available_width().max(60.0);
    let eff_wrap_w = wrap_w.min(col_w).max(60.0);

    let seek = |resp: &egui::Response, galley: &egui::Galley, origin: Pos2| {
        resp.clicked().then(|| {
            let index = resp.interact_pointer_pos().map(|p| galley.cursor_from_pos(p - origin).index.0);
            line_seek_time(line, index)
        }).flatten()
    };
    let alloc_aligned = |ui: &mut Ui, galley_sz: Vec2, clickable: bool| -> (Rect, Rect, Pos2, egui::Response) {
        let sense = if clickable { egui::Sense::click() } else { egui::Sense::hover() };
        let row_w = if right_align { col_w } else { galley_sz.x.min(col_w) };
        let (row_rect, mut resp) = ui.allocate_exact_size(Vec2::new(row_w, galley_sz.y), sense);
        let text_rect = if right_align {
            Rect::from_min_size(
                Pos2::new((row_rect.right() - galley_sz.x).max(row_rect.left()), row_rect.top()),
                Vec2::new(galley_sz.x.min(col_w), galley_sz.y),
            )
        } else {
            row_rect
        };
        if clickable {
            // Re-interact on generous pill hit bounds so gliding pointer up/down across lines never flickers.
            let hit = text_rect.expand2(Vec2::new(14.0, 8.0));
            resp = ui
                .interact(hit, resp.id.with("pill"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        let origin = if right_align {
            Pos2::new(row_rect.right(), row_rect.top() + lift_px)
        } else {
            Pos2::new(row_rect.left(), row_rect.top() + lift_px)
        };
        (row_rect, text_rect, origin, resp)
    };

    // Always lay out at constant base `size` (zero re-wrap!) and apply `scale` as a pure vertex transform.
    let base_galley = layout_balanced_lyric(ui, display, size, eff_wrap_w, right_align);
    let (rect, text_rect, origin, resp) = alloc_aligned(ui, base_galley.size(), line.time_ms.is_some());
    let target = seek(&resp, &base_galley, origin);

    if let Some(pos) = word_position.filter(|_| !display.trim().is_empty()) {
        if !rect.intersects(viewport.expand(18.0)) {
            return (resp, text_rect, target);
        }
        let scaled_base = scale_galley_in_place(base_galley, scale);
        // Line-synced mode (no word/syllable spans): authentic Spotify / Spicy Lyrics 180° top-to-bottom illumination wave!
        if line.spans.is_empty() {
            let start = line.time_ms.unwrap_or(pos);
            let raw_end = line
                .end_ms
                .or(next_line_start_ms)
                .unwrap_or(start + 2_600)
                .max(start + 400);
            let sweep_dur = ((raw_end - start) as f32 * 0.65).clamp(480.0, 1_850.0);
            let progress = if motion_on {
                ((pos.saturating_sub(start)) as f32 / sweep_dur).clamp(0.0, 1.0)
            } else {
                1.0
            };
            const OCT_DIRS: [(f32, f32); 8] = [
                (1.0, 0.0),
                (-1.0, 0.0),
                (0.0, 1.0),
                (0.0, -1.0),
                (0.707, 0.707),
                (-0.707, 0.707),
                (0.707, -0.707),
                (-0.707, -0.707),
            ];
            for dir in OCT_DIRS {
                let halo = make_vertical_line_galley(
                    &scaled_base,
                    progress,
                    true,
                    dir,
                    1.6,
                    24.0,
                    origin,
                    viewport,
                    pad_h,
                    fade_h,
                );
                ui.painter().galley(origin, halo, Color32::WHITE);
            }
            for dir in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                let halo = make_vertical_line_galley(
                    &scaled_base,
                    progress,
                    true,
                    dir,
                    3.4,
                    14.0,
                    origin,
                    viewport,
                    pad_h,
                    fade_h,
                );
                ui.painter().galley(origin, halo, Color32::WHITE);
            }
            let fg = make_vertical_line_galley(
                &scaled_base,
                progress,
                false,
                (0.0, 0.0),
                0.0,
                0.0,
                origin,
                viewport,
                pad_h,
                fade_h,
            );
            ui.painter().galley(origin, fg, Color32::WHITE);
            return (resp, text_rect, target);
        }
        let char_anims = compute_active_char_anims(
            line,
            display,
            pos,
            next_line_start_ms,
            size,
            motion_on,
        );
        let row_char_map = map_galley_glyphs_to_chars(&scaled_base, display);
        if char_anims.iter().any(|a| a.glow > 0.03) {
            // Soft glyph-shaped text-shadow halo rings (Spicy Lyrics main.css:24)
            const OCT_DIRS: [(f32, f32); 8] = [
                (1.0, 0.0),
                (-1.0, 0.0),
                (0.0, 1.0),
                (0.0, -1.0),
                (0.707, 0.707),
                (-0.707, 0.707),
                (0.707, -0.707),
                (-0.707, -0.707),
            ];
            for dir in OCT_DIRS {
                let halo = make_tinted_galley(
                    &scaled_base,
                    &row_char_map,
                    &char_anims,
                    dir,
                    0.16,
                    true,
                    origin,
                    viewport,
                    pad_h,
                    fade_h,
                );
                ui.painter().galley(origin, halo, Color32::WHITE);
            }
            for dir in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                let halo = make_tinted_galley(
                    &scaled_base,
                    &row_char_map,
                    &char_anims,
                    dir,
                    0.30,
                    true,
                    origin,
                    viewport,
                    pad_h,
                    fade_h,
                );
                ui.painter().galley(origin, halo, Color32::WHITE);
            }
        }
        let fg = make_tinted_galley(
            &scaled_base,
            &row_char_map,
            &char_anims,
            (0.0, 0.0),
            0.0,
            false,
            origin,
            viewport,
            pad_h,
            fade_h,
        );
        ui.painter().galley(origin, fg, Color32::WHITE);
        return (resp, text_rect, target);
    }

    if blur_px > 0.35 && word_position.is_none() {
        if line_color.a() > 0 && rect.intersects(viewport.expand(18.0)) {
            let scaled = scale_galley_in_place(base_galley, scale);
            let a_target = (line_color.a() as f32 / 255.0).clamp(0.0, 0.999);
            let tap_alpha = |w: f32| -> u8 {
                (255.0 * (1.0 - (1.0 - a_target).powf(w))).round().clamp(1.0, 255.0) as u8
            };
            // 2-ring isotropic Gaussian disc kernel (13 radial taps, per-vertex Y-masked, `BlurMultiplier = 1.25`)
            let g0 = mask_galley_vertical(
                &scaled,
                origin,
                viewport,
                pad_h,
                fade_h,
                Some(Color32::from_white_alpha(tap_alpha(0.30))),
            );
            ui.painter().galley(origin, g0, Color32::WHITE);

            let r1 = blur_px * 0.30;
            let g1 = mask_galley_vertical(
                &scaled,
                origin,
                viewport,
                pad_h,
                fade_h,
                Some(Color32::from_white_alpha(tap_alpha(0.075))),
            );
            for k in 0..6 {
                let (s, c) = (k as f32 * std::f32::consts::TAU / 6.0).sin_cos();
                ui.painter().galley(origin + Vec2::new(c * r1, s * r1), g1.clone(), Color32::WHITE);
            }

            let r2 = blur_px * 0.62;
            let g2 = mask_galley_vertical(
                &scaled,
                origin,
                viewport,
                pad_h,
                fade_h,
                Some(Color32::from_white_alpha(tap_alpha(0.042))),
            );
            for k in 0..6 {
                let (s, c) = (k as f32 * std::f32::consts::TAU / 6.0 + 0.52).sin_cos();
                ui.painter().galley(origin + Vec2::new(c * r2, s * r2), g2.clone(), Color32::WHITE);
            }
        }
        return (resp, text_rect, target);
    }

    if rect.intersects(viewport.expand(12.0)) {
        let scaled = scale_galley_in_place(base_galley, scale);
        let masked = mask_galley_vertical(&scaled, origin, viewport, pad_h, fade_h, Some(line_color));
        ui.painter().galley(origin, masked, Color32::WHITE);
    }
    (resp, text_rect, target)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0).max(1e-5)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn draw_animated_backdrop(
    ui: &Ui,
    bounds: Rect,
    track_id: Option<TrackId>,
    target: [Color32; 4],
    backdrop_tex: Option<egui::TextureHandle>,
    motion_on: bool,
    playing: bool,
) -> bool {
    let id = ui.scope_id().with("lyrics-backdrop-state");
    let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.05);

    let mut state = ui
        .data_mut(|d| d.get_temp::<BackdropState>(id))
        .unwrap_or(BackdropState {
            track: track_id,
            from: target,
            current: target,
            target,
            prev_tex: None,
            cur_tex: backdrop_tex.clone(),
            blend: 1.0,
            phase: 0.0,
        });

    state.set_target(track_id, target, backdrop_tex, motion_on);

    let smooth_t = state.advance_blend(dt, motion_on);

    if motion_on {
        let speed = if playing {
            1.40 + 0.35 * (state.phase * 0.7).sin()
        } else {
            0.22
        };
        state.phase = (state.phase + dt * speed) % (std::f32::consts::TAU * 64.0);
    }

    let rect = bounds;
    if let Some(ref cur_tex) = state.cur_tex {
        if smooth_t < 1.0 {
            if let Some(ref prev_tex) = state.prev_tex {
                super::backdrop::paint(ui, rect, prev_tex, state.phase, 1.0, false, None);
            }
        }
        super::backdrop::paint(ui, rect, cur_tex, state.phase, smooth_t, false, None);
    } else {
        const N: u32 = 16;
        let mut mesh = Mesh::default();
        for y in 0..=N {
            for x in 0..=N {
                let (u, v) = (x as f32 / N as f32, y as f32 / N as f32);
                let mut rgb = [0.0f32; 3];
                let mut sum = 0.0f32;
                for (index, color) in state.current.iter().enumerate() {
                    let p = state.phase * (0.75 + index as f32 * 0.14) + index as f32 * 1.7;
                    let base_x = [0.14, 0.84, 0.24, 0.90][index];
                    let base_y = [0.16, 0.12, 0.86, 0.78][index];
                    let cx = if motion_on { base_x + 0.13 * p.sin() } else { base_x };
                    let cy = if motion_on { base_y + 0.11 * p.cos() } else { base_y };
                    let dist2 = (u - cx).powi(2) + (v - cy).powi(2);
                    let weight = 1.0 / (0.045 + dist2).powi(2);
                    rgb[0] += weight * color.r() as f32;
                    rgb[1] += weight * color.g() as f32;
                    rgb[2] += weight * color.b() as f32;
                    sum += weight;
                }
                let avg = [rgb[0] / sum, rgb[1] / sum, rgb[2] / sum];
                let luma = 0.2126 * avg[0] + 0.7152 * avg[1] + 0.0722 * avg[2];
                let sat_boost = 1.65;
                let vignette = 0.62 * (1.0 - 0.24 * smoothstep(0.60, 1.0, v));
                let [r, g, b] = avg.map(|c| ((luma + (c - luma) * sat_boost) * vignette).clamp(8.0, 232.0) as u8);
                mesh.colored_vertex(rect.lerp_inside(Vec2::new(u, v)), Color32::from_rgb(r, g, b));
                if x < N && y < N {
                    let n = y * (N + 1) + x;
                    mesh.add_triangle(n, n + 1, n + N + 1);
                    mesh.add_triangle(n + 1, n + N + 2, n + N + 1);
                }
            }
        }
        ui.painter().add(egui::Shape::mesh(mesh));
    }

    let crossfading = state.blend < 1.0;
    ui.data_mut(|d| d.insert_temp(id, state));
    motion_on || crossfading
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let lerp = |x: u8, y: u8| -> u8 { (x as f32 + (y as f32 - x as f32) * t).round() as u8 };
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

#[allow(clippy::too_many_arguments)]
fn glass_circle_spicy_button(
    ui: &mut Ui,
    salt: &str,
    size: f32,
    icon: SpicyIcon,
    tip: &str,
    active: bool,
    motion_on: bool,
) -> egui::Response {
    // `NPVLyrics.css:42-78`: 36-38px circular `.CardControl`, `18px` SVG, `:hover { scale: 1.05; background: rgba(255,255,255,0.12) }`, `:active { scale: 0.94 }`.
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let resp = ui
        .interact(rect, ui.scope_id().with(salt), egui::Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), active, tip));
    let hovered = resp.hovered() || resp.has_focus();
    let scale = if !motion_on {
        1.0
    } else if resp.is_pointer_button_down_on() {
        0.94
    } else if hovered {
        1.05
    } else {
        1.0
    };
    let draw_rect = Rect::from_center_size(rect.center(), rect.size() * scale);
    let center = draw_rect.center();
    let radius = draw_rect.width() * 0.5;
    ui.painter()
        .circle_filled(center, radius, Color32::from_black_alpha(32));
    ui.painter().circle_filled(
        center,
        radius,
        Color32::from_white_alpha(if active || hovered { 34 } else { 20 }),
    );
    ui.painter().circle_stroke(
        center,
        radius - 0.5,
        Stroke::new(
            1.0,
            Color32::from_white_alpha(if active || hovered { 72 } else { 44 }),
        ),
    );
    let icon_size = 17.5 * scale;
    let icon_rect = Rect::from_center_size(draw_rect.center(), Vec2::splat(icon_size));
    icons::paint_spicy(
        ui.painter(),
        icon_rect,
        icon,
        if hovered { Color32::WHITE } else { Color32::from_white_alpha(238) },
    );
    resp
}

/// Closed-form analytical spring solver ported from Spicy Lyrics `Spring.ts` (Fraktality MIT).
#[derive(Clone, Copy, Debug)]
struct Spring {
    d: f64,
    f: f64,
    g: f64,
    p: f64,
    v: f64,
}

impl Spring {
    const SLEEP_OFFSET_SQ: f64 = (1.0 / 3840.0) * (1.0 / 3840.0);
    const SLEEP_VEL_SQ: f64 = 1e-4;
    const EPS: f64 = 1e-5;

    fn new(start: f64, frequency_hz: f64, damping: f64) -> Self {
        Self {
            d: damping,
            f: frequency_hz,
            g: start,
            p: start,
            v: 0.0,
        }
    }

    fn step(&mut self, dt: f64) -> f64 {
        let dt = dt.clamp(0.0, 0.25);
        if dt <= 0.0 {
            return self.p;
        }
        let d = self.d;
        let f = self.f * (2.0 * std::f64::consts::PI);
        let g = self.g;
        let mut p = self.p;
        let mut v = self.v;
        if (d - 1.0).abs() <= 1e-7 {
            let q = (-f * dt).exp();
            let w = dt * q;
            let c0 = q + w * f;
            let c2 = q - w * f;
            let c3 = w * f * f;
            let o = p - g;
            p = o * c0 + v * w + g;
            v = v * c2 - o * c3;
        } else if d < 1.0 {
            let q = (-d * f * dt).exp();
            let c = (1.0 - d * d).max(0.0).sqrt();
            let i = (dt * f * c).cos();
            let j = (dt * f * c).sin();
            let z = if c > Self::EPS {
                j / c
            } else {
                let a = dt * f;
                a + ((a * a) * (c * c) * (c * c) / 20.0 - c * c) * (a * a * a) / 6.0
            };
            let y = if f * c > Self::EPS {
                j / (f * c)
            } else {
                let b = f * c;
                dt + ((dt * dt) * (b * b) * (b * b) / 20.0 - b * b) * (dt * dt * dt) / 6.0
            };
            let o = p - g;
            p = (o * (i + z * d) + v * y) * q + g;
            v = (v * (i - z * d) - o * (z * f)) * q;
        } else {
            let c = (d * d - 1.0).max(1e-12).sqrt();
            let r1 = -f * (d + c);
            let r2 = -f * (d - c);
            let ec1 = (r1 * dt).exp();
            let ec2 = (r2 * dt).exp();
            let o = p - g;
            let co2 = (v - o * r1) / (2.0 * f * c);
            let co1 = ec1 * (o - co2);
            p = co1 + co2 * ec2 + g;
            v = co1 * r1 + co2 * ec2 * r2;
        }
        self.p = p;
        self.v = v;
        p
    }

    fn can_sleep(&self) -> bool {
        if self.v * self.v > Self::SLEEP_VEL_SQ {
            return false;
        }
        let off = self.p - self.g;
        off * off <= Self::SLEEP_OFFSET_SQ
    }

    fn goal(&self) -> f64 {
        self.g
    }

    fn set_goal(&mut self, goal: f64, snap: bool) {
        self.g = goal;
        if snap {
            self.p = goal;
            self.v = 0.0;
        }
    }
}

/// Exact CSS custom properties from Spicy Lyrics `PopupDock.css:15-53, 104-110`.
#[derive(Clone, Copy, Debug)]
struct PopupDockMetrics {
    gutter: f32,
    bottom: f32,
    gap: f32,
    meta_h: f32,
    title_sz: f32,
    artist_sz: f32,
    bar_row: f32,
    bar_h: f32,
    time_sz: f32,
    show_times: bool,
    timeline_h: f32,
    play_h: f32,
    skip_h: f32,
    toggle_h: f32,
    controls_h: f32,
    control_gap: f32,
    heart_w: f32,
    vol_w: f32,
    vol_h: f32,
    vol_icon: f32,
    view_size: f32,
    view_inset: f32,
    reserve: f32,
    box_h: f32,
    idle_h: f32,
    art_gap: f32,
    art_side: f32,
    compact_bar: bool,
}

impl PopupDockMetrics {
    fn compute(w: f32, h: f32) -> Self {
        let u = (w * 0.01).min(h * 0.012);
        let gutter = (u * 5.0).clamp(12.0, 28.0);
        let bottom = (u * 3.5).clamp(8.0, 20.0);
        let gap = (u * 1.5).clamp(3.0, 8.0);
        let meta_h = (u * 10.0).clamp(30.0, 52.0);
        let title_sz = (u * 4.2).clamp(12.5, 20.0);
        let artist_sz = (u * 3.4).clamp(11.0, 15.0);
        let bar_row = (u * 2.4).clamp(8.0, 12.0);
        let bar_h = (u * 1.25).clamp(3.5, 6.0);
        let time_sz = (u * 2.85).clamp(9.5, 12.0);
        let show_times = h > 230.0;
        let timeline_h = if show_times {
            bar_row + 3.0 + time_sz * 1.3
        } else {
            bar_row
        };
        let play_h = (u * 4.4).clamp(12.0, 24.0);
        let skip_h = (u * 3.2).clamp(9.0, 17.0);
        let toggle_h = (u * 3.0).clamp(9.0, 16.0);
        let controls_h = play_h * 1.6;
        let control_gap = (u * 2.0).clamp(2.0, 14.0);
        let heart_w = (u * 4.4).clamp(13.0, 24.0);
        let vol_w = (u * 14.0).clamp(40.0, 96.0);
        let vol_h = (u * 1.3).clamp(4.0, 7.0);
        let vol_icon = (u * 3.4).clamp(11.0, 18.0);
        let view_size = (u * 7.0).clamp(22.0, 34.0);
        let view_inset = (u * 2.5).clamp(6.0, 12.0);
        let reserve = controls_h + gap;
        let box_h = gap + timeline_h + reserve;
        let idle_h = bottom + timeline_h + gap + meta_h;
        let art_gap = (u * 3.0).clamp(8.0, 16.0);
        let art_side = (h - gutter - art_gap - meta_h - box_h - bottom)
            .min(w - 2.0 * gutter)
            .max(48.0);
        Self {
            gutter,
            bottom,
            gap,
            meta_h,
            title_sz,
            artist_sz,
            bar_row,
            bar_h,
            time_sz,
            show_times,
            timeline_h,
            play_h,
            skip_h,
            toggle_h,
            controls_h,
            control_gap,
            heart_w,
            vol_w,
            vol_h,
            vol_icon,
            view_size,
            view_inset,
            reserve,
            box_h,
            idle_h,
            art_gap,
            art_side,
            compact_bar: h < 170.0,
        }
    }
}

#[derive(Clone, Copy)]
struct PopupSpringState {
    controls_opacity: Spring,
    dock_lift: Spring,
    last_ts: Option<f64>,
    last_move_at: f64,
    prev_ptr: Option<Pos2>,
    drag_lock: bool,
}

impl Default for PopupSpringState {
    fn default() -> Self {
        Self {
            controls_opacity: Spring::new(0.0, 2.5, 1.0),
            dock_lift: Spring::new(0.0, 2.5, 1.0),
            last_ts: None,
            last_move_at: -10.0,
            prev_ptr: None,
            drag_lock: false,
        }
    }
}

impl PopupSpringState {
    /// Steps `controlsOpacitySpring` & `dockLiftSpring` (`Fullscreen.ts:87-155`).
    /// Returns `(controls_opacity, dock_lift, animating)`.
    fn advance(
        &mut self,
        now: f64,
        page_hover: bool,
        dock_or_pill_hover: bool,
        ptr_moved: bool,
        motion_on: bool,
    ) -> (f32, f32, bool) {
        if ptr_moved {
            self.last_move_at = now;
        }
        if !self.drag_lock {
            let idle_expired = (now - self.last_move_at) >= 0.75;
            let goal = if dock_or_pill_hover {
                0.985
            } else if page_hover && !idle_expired {
                0.65
            } else {
                0.0
            };
            self.controls_opacity.set_goal(goal, !motion_on);
        }
        if !motion_on {
            // PopupDock.css:487-500 (`prefers-reduced-motion: reduce`): rows stay raised, only controls fade.
            self.dock_lift.set_goal(1.0, true);
            self.last_ts = Some(now);
            return (self.controls_opacity.p as f32, 1.0, false);
        }
        let dt = self.last_ts.map(|prev| (now - prev).clamp(0.0, 0.1)).unwrap_or(1.0 / 60.0);
        self.last_ts = Some(now);
        let co = self.controls_opacity.step(dt);
        if self.controls_opacity.goal() > 0.0 {
            self.dock_lift.set_goal(1.0, false);
        } else if co <= 0.005 {
            self.dock_lift.set_goal(0.0, false);
        }
        let dl = self.dock_lift.step(dt);
        let sleeping = self.controls_opacity.can_sleep() && self.dock_lift.can_sleep();
        let waiting_idle = page_hover && !dock_or_pill_hover && (now - self.last_move_at) < 0.75;
        (co.clamp(0.0, 1.0) as f32, dl.clamp(0.0, 1.0) as f32, !sleeping || waiting_idle)
    }
}

const POPUP_WINDOW_TITLE: &str = "Nhịp — Lời bài hát";

#[cfg(windows)]
fn apply_win11_dwm_popup_rounding() {
    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(lpClassName: *const u16, lpWindowName: *const u16) -> isize;
        fn GetWindowThreadProcessId(hWnd: isize, lpdwProcessId: *mut u32) -> u32;
    }
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            dwAttribute: u32,
            pvAttribute: *const i32,
            cbAttribute: u32,
        ) -> i32;
    }
    static APPLIED: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
    let title: Vec<u16> = format!("{POPUP_WINDOW_TITLE}\0").encode_utf16().collect();
    // SAFETY: Win32 window lookup + DWM corner preference (`DWMWA_WINDOW_CORNER_PREFERENCE = 33`, `DWMWCP_ROUND = 2`)
    // only touches our own process's HWND to grant native Windows 11 rounded corners and OS drop shadow.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd != 0 && APPLIED.load(std::sync::atomic::Ordering::Relaxed) != hwnd {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == std::process::id() {
                let round: i32 = 2; // DWMWCP_ROUND
                let _ = DwmSetWindowAttribute(hwnd, 33, &round, 4);
                APPLIED.store(hwnd, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

pub fn draw_spicy_popup(app: &mut App, ui: &mut Ui, full: Rect) {
    let ctx = ui.ctx().clone();
    let saved = app.data.settings.miniplayer_size.unwrap_or([390.0, 379.0]);
    let default_w = saved[0].clamp(260.0, 960.0);
    let default_h = saved[1].clamp(112.0, 960.0);

    // Deterministic in-canvas host for `--screenshot` pixel & multi-frame animation verification;
    // real OS Always-On-Top frameless resizable PiP viewport (`PopupLyrics.ts` / `xpui-pip-mini-player`) in live use!
    if app.screenshot.is_some() {
        let staged_size = ctx
            .data(|d| d.get_temp::<[f32; 2]>(egui::Id::NULL.with("spicy-popup-size")))
            .unwrap_or([default_w, default_h]);
        let w = staged_size[0].min(full.width() - 20.0).max(240.0);
        let h = staged_size[1].min(full.height() - 24.0).max(112.0);
        let pos = Pos2::new(
            (full.right() - w - 18.0).max(8.0),
            (full.bottom() - h - 18.0).max(8.0),
        );
        egui::Area::new(egui::Id::unique("spicy-popup-lyrics-shot"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(&ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
                cover_shadow(ui.painter(), rect, 8.0);
                render_popup_contents(app, ui, rect, None);
            });
        return;
    }

    let vp_id = egui::ViewportId::from_hash_of("nhip-spicy-popup-viewport");
    let builder = egui::ViewportBuilder::default()
        .with_title(POPUP_WINDOW_TITLE)
        .with_inner_size([default_w, default_h])
        .with_min_inner_size([260.0, 112.0])
        .with_decorations(false)
        .with_always_on_top();

    ctx.show_viewport_immediate(vp_id, builder, |vp_ui, _class| {
        let vp_ctx = vp_ui.ctx().clone();
        if vp_ctx.input(|i| i.viewport().close_requested()) {
            app.miniplayer_open = false;
            return;
        }
        // Keyboard transport & Escape inside the PiP window
        vp_ctx.input_mut(|i| {
            use egui::{Key, KeyboardShortcut as S, Modifiers as M};
            if i.consume_shortcut(&S::new(M::NONE, Key::Escape)) {
                app.actions.push(Action::ToggleMiniplayer);
            }
            if i.consume_shortcut(&S::new(M::NONE, Key::Space)) {
                app.actions.push(Action::TogglePlay);
            }
            if i.consume_shortcut(&S::new(M::NONE, Key::M)) {
                app.actions.push(Action::ToggleMute);
            }
            if i.consume_shortcut(&S::new(M::CTRL, Key::ArrowRight)) {
                app.actions.push(Action::Next);
            }
            if i.consume_shortcut(&S::new(M::CTRL, Key::ArrowLeft)) {
                app.actions.push(Action::Previous);
            }
            if i.consume_shortcut(&S::new(M::SHIFT, Key::ArrowRight)) {
                app.actions.push(Action::SeekBy(5_000));
            }
            if i.consume_shortcut(&S::new(M::SHIFT, Key::ArrowLeft)) {
                app.actions.push(Action::SeekBy(-5_000));
            }
        });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(8, 14, 38)))
            .show(vp_ui, |ui| {
                #[cfg(windows)]
                apply_win11_dwm_popup_rounding();
                let rect = ui.max_rect();
                let cur_sz = [rect.width(), rect.height()];
                if app
                    .data
                    .settings
                    .miniplayer_size
                    .is_none_or(|old| (old[0] - cur_sz[0]).abs() > 1.0 || (old[1] - cur_sz[1]).abs() > 1.0)
                {
                    app.data.settings.miniplayer_size = Some(cur_sz);
                    app.dirty = true;
                }
                render_popup_contents(app, ui, rect, Some(vp_id));
                super::resize_edges(ui, rect);
            });
    });
}

fn render_popup_contents(app: &mut App, ui: &mut Ui, rect: Rect, native_vp: Option<egui::ViewportId>) {
    let native_viewport = native_vp.is_some();
    let ctx = ui.ctx().clone();
    let w = rect.width();
    let h = rect.height();
    let motion_on = app.data.settings.effective_lyrics_motion();
    let m = PopupDockMetrics::compute(w, h);

    let playing = app.is_playing();
    let position = app.position_ms();
    let track = app.current_track();
    let track_id = track.as_ref().map(|t| t.id);
    let (target_colors, backdrop_tex) = if let Some(ref t) = track {
        let key = t.cover_key();
        let bd = app.art.backdrop_texture(&ctx, &key, t.art_source());
        (app.art.lyric_colors(&key), Some(bd))
    } else {
        (
            [
                Color32::from_rgb(18, 40, 126),
                Color32::from_rgb(12, 28, 92),
                Color32::from_rgb(8, 18, 66),
                Color32::from_rgb(6, 12, 42),
            ],
            None,
        )
    };

    let corner_r = if native_viewport { 6 } else { 8 };
    let mut card_ui = ui.new_child(egui::UiBuilder::new().id_salt("spicy-popup-root").max_rect(rect));
    card_ui.set_clip_rect(rect);
    card_ui
        .painter()
        .rect_filled(rect, CornerRadius::same(corner_r), Color32::from_rgb(8, 18, 62));
    let animating_bd = draw_animated_backdrop(
        &card_ui,
        rect,
        track_id,
        target_colors,
        backdrop_tex,
        motion_on,
        playing,
    );

    if m.compact_bar {
        render_compact_pip_bar(app, &mut card_ui, rect, track.as_ref(), position, playing, motion_on, native_vp);
        if animating_bd || playing {
            if motion_on {
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_millis(42));
            }
        }
        return;
    }

    // Spring state & pointer-stillness auto-hide (`Fullscreen.ts` 750ms)
    let spring_id = egui::Id::NULL.with("spicy-popup-springs");
    let mut sp = ctx.data(|d| d.get_temp::<PopupSpringState>(spring_id)).unwrap_or_default();
    let now = card_ui.input(|i| i.time);
    let ptr = card_ui.input(|i| i.pointer.hover_pos());
    let page_hover = ptr.is_some_and(|p| rect.contains(p));
    let ptr_moved = page_hover && sp.prev_ptr.is_none_or(|old| ptr.is_some_and(|p| old.distance(p) > 1.2));
    if page_hover {
        sp.prev_ptr = ptr;
    } else {
        sp.prev_ptr = None;
    }

    let pill_w = m.view_size * 2.45;
    let pill_h = m.view_size * 1.16;
    let pill_hover_zone = Rect::from_min_size(
        rect.min + Vec2::new(m.view_inset, m.view_inset),
        Vec2::new(pill_w + 6.0, pill_h + 8.0),
    );
    let cur_drop_est = if motion_on {
        (1.0 - sp.dock_lift.p as f32) * m.reserve
    } else {
        0.0
    };
    let dock_hover_zone = Rect::from_min_max(
        Pos2::new(rect.left(), rect.bottom() - m.bottom - m.box_h - m.meta_h + cur_drop_est),
        rect.max,
    );
    let dock_or_pill_hover = ptr.is_some_and(|p| dock_hover_zone.contains(p) || pill_hover_zone.contains(p));

    // Staged animation frames for deterministic `--screenshot` verification
    let staged_mode = ctx.data(|d| d.get_temp::<&'static str>(egui::Id::NULL.with("spicy-popup-stage")));
    let force_hov = ctx
        .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("spicy-popup-force-hover")))
        .unwrap_or(false);
    let (controls_opacity, dock_lift, spring_busy) = if let Some(stage) = staged_mode {
        match stage {
            "idle" => (0.0, 0.0, false),
            "f1" => {
                let mut s = PopupSpringState::default();
                s.advance(0.0, true, false, true, true);
                s.advance(0.085, true, false, false, true)
            }
            "f2" => {
                let mut s = PopupSpringState::default();
                s.advance(0.0, true, false, true, true);
                s.advance(0.18, true, false, false, true)
            }
            "sleep1" => {
                let mut s = PopupSpringState::default();
                s.controls_opacity.set_goal(0.65, true);
                s.dock_lift.set_goal(1.0, true);
                s.last_ts = Some(0.0);
                s.last_move_at = -1.0;
                s.advance(0.14, true, false, false, true)
            }
            _ => (0.985, 1.0, false),
        }
    } else if force_hov {
        (0.92, 1.0, false)
    } else {
        sp.advance(now, page_hover, dock_or_pill_hover, ptr_moved, motion_on)
    };

    let pd_drop = if motion_on {
        (1.0 - dock_lift) * m.reserve
    } else {
        0.0
    };
    let pd_transport_drop = if motion_on {
        (1.0 - dock_lift) * m.bottom
    } else {
        0.0
    };

    // `.LyricsContainer` keeps a CONSTANT height (`100% - --PD_IdleH`, PopupDock.css:58) so lyrics NEVER jump on hover!
    let lyrics_h = if motion_on {
        (h - m.idle_h).max(64.0)
    } else {
        (h - m.idle_h - m.reserve).max(64.0)
    };
    let lyrics_rect = Rect::from_min_size(rect.min, Vec2::new(w, lyrics_h));
    let is_loading = app.lyrics_loading();
    let lyr_opt = if track_id.is_some() {
        app.current_lyrics().cloned()
    } else {
        None
    };
    let cover_mode = app.data.settings.miniplayer_cover_mode
        || (track_id.is_some() && !is_loading && lyr_opt.is_none());

    if cover_mode {
        // `.ContentBox.LyricsHidden` (`PopupDock.css:104-131`): square cover sitting on caption gutter, rising with `--PD_Drop`.
        let art_bottom = rect.bottom() - m.bottom - m.box_h - m.meta_h - m.art_gap + pd_drop;
        let art_r = Rect::from_min_size(
            Pos2::new(rect.left() + m.gutter, art_bottom - m.art_side),
            Vec2::splat(m.art_side),
        );
        let art_rad = (m.art_side * 0.045).clamp(6.0, 14.0);
        if let Some(ref t) = track {
            let tex = app.art.get(&ctx, &t.cover_key(), t.art_source());
            cover_shadow(card_ui.painter(), art_r, art_rad);
            cover(&card_ui, art_r, &tex, art_rad);
        } else {
            card_ui.painter().rect_filled(art_r, art_rad, CARD);
        }
    } else if track_id.is_some() {
        let ended_label = app.t("Bài hát đã kết thúc", "Song ended").to_owned();
        let sync_label = app.t("Đồng bộ", "Sync").to_owned();
        let cur_artist = track.as_ref().map(|t| t.artist.clone()).unwrap_or_default();
        // `main.css:112` popup font formula (`min(max(5.5cqw, 1.64rem), min(8cqh, 8cqw))` -> ~26.2px at 390x379)
        let pop_font = (w * 0.055)
            .max(26.0)
            .min(h * 0.072)
            .min(w * 0.072)
            .clamp(17.5, 34.0);
        let pop_inset = (w * 0.07).clamp(12.0, 64.0);
        let cur_meta_top = rect.bottom() - m.bottom - m.box_h - m.meta_h + pd_drop;
        let vis_lyrics_rect = Rect::from_min_max(
            rect.min,
            Pos2::new(rect.right(), (cur_meta_top - 3.0).max(rect.top() + 36.0)),
        );
        let mut lyr_ui = card_ui.new_child(egui::UiBuilder::new().id_salt("spicy-popup-lyr-box").max_rect(lyrics_rect));
        lyr_ui.set_clip_rect(vis_lyrics_rect.intersect(rect));
        // Slim 4px translucent right scrollbar matching `9a4a97ec.jpg` & `02e48b2e.jpg`
        let show_scroll_thumb = page_hover
            || staged_mode == Some("hover")
            || staged_mode == Some("f2")
            || ctx
                .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("spicy-popup-show-scrollbar")))
                .unwrap_or(false);
        {
            let sc = &mut lyr_ui.spacing_mut().scroll;
            sc.floating = true;
            sc.bar_width = 4.0;
            sc.bar_inner_margin = 0.0;
            sc.bar_outer_margin = 5.0;
            sc.floating_width = 4.0;
            sc.dormant_background_opacity = 0.0;
            sc.active_background_opacity = 0.0;
            sc.interact_background_opacity = 0.0;
            sc.dormant_handle_opacity = if show_scroll_thumb { 0.38 } else { 0.0 };
            sc.active_handle_opacity = 0.52;
            sc.interact_handle_opacity = 0.68;
        }
        if let Some(ref lyrics) = lyr_opt {
            let credits = resolve_lyrics_credits(lyrics, &cur_artist);
            if let Some(ms) = render_lyrics_scroll_body(
                &mut lyr_ui,
                "spicy-popup-scroll",
                lyrics,
                track_id,
                position,
                playing,
                motion_on,
                pop_font,
                w,
                lyrics_h,
                pop_inset,
                &ended_label,
                &sync_label,
                true,
                -10.0,
                credits,
            ) {
                app.actions.push(Action::SeekTo(ms));
            }
        } else if is_loading {
            draw_lyrics_skeleton(&mut lyr_ui, w, lyrics_h, pop_inset, 21.0);
        }
    }

    // `Header::before` bottom scrim (`PopupDock.css:84-94`): smooth cosine falloff translating by `+pd_drop`
    // (at idle `dock_lift == 0` it stays subtle so vivid cobalt/amber covers shine cleanly with zero kink or seam).
    let scrim_h = m.bottom + m.box_h + m.meta_h + 36.0;
    let scrim_top = (rect.bottom() - scrim_h + pd_drop).max(rect.top());
    let scrim_bot_virtual = rect.bottom() + pd_drop;
    if rect.bottom() > scrim_top + 2.0 {
        const BANDS: usize = 10;
        let max_a = 88.0 + 38.0 * dock_lift;
        let mut smesh = Mesh::default();
        let span_virtual = (scrim_bot_virtual - scrim_top).max(1.0);
        for b in 0..=BANDS {
            let y = scrim_top + (rect.bottom() - scrim_top) * (b as f32 / BANDS as f32);
            let u = ((y - scrim_top) / span_virtual).clamp(0.0, 1.0);
            let curve = u * u * (3.0 - 2.0 * u);
            let c = Color32::from_black_alpha((max_a * curve).round().clamp(0.0, 255.0) as u8);
            smesh.colored_vertex(Pos2::new(rect.left(), y), c);
            smesh.colored_vertex(Pos2::new(rect.right(), y), c);
        }
        for b in 0..BANDS as u32 {
            let i = b * 2;
            smesh.add_triangle(i, i + 1, i + 3);
            smesh.add_triangle(i, i + 3, i + 2);
        }
        card_ui.painter().add(egui::Shape::mesh(smesh));
    }

    let mut any_slider_dragging = false;

    // `.Metadata` (`PopupDock.css:148-188`): Title + Artist on `--PD_Gutter`, translated by `--PD_Drop`
    let media_box_bottom = rect.bottom() - m.bottom + pd_drop;
    let media_box_top = media_box_bottom - m.box_h;
    let meta_bottom = media_box_top;
    let meta_top = meta_bottom - m.meta_h;
    if let Some(ref t) = track {
        let meta_w = (w - m.gutter * 2.0).max(40.0);
        let title_r = Rect::from_min_size(
            Pos2::new(rect.left() + m.gutter, meta_top + 1.0),
            Vec2::new(meta_w, m.title_sz * 1.28),
        );
        marquee_text(
            &mut card_ui,
            title_r,
            "spicy-popup-title",
            &t.title,
            bold(m.title_sz),
            Color32::WHITE,
        );
        let t_resp = card_ui
            .interact(title_r, egui::Id::unique("spicy-popup-title-click"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if t_resp.clicked() {
            app.actions.push(Action::Navigate(super::right_panel::track_route(t)));
        }

        let artist_r = Rect::from_min_size(
            Pos2::new(rect.left() + m.gutter, title_r.bottom() + 1.0),
            Vec2::new(meta_w, m.artist_sz * 1.35),
        );
        let a_resp = card_ui
            .interact(artist_r, egui::Id::unique("spicy-popup-artist"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        text(
            card_ui.painter(),
            Pos2::new(artist_r.left(), artist_r.center().y),
            &t.artist,
            body(m.artist_sz),
            Color32::from_white_alpha(if a_resp.hovered() { 255 } else { 196 }),
            artist_r.width(),
            egui::Align2::LEFT_CENTER,
        );
        if a_resp.clicked() {
            app.actions.push(Action::Navigate(Route::Artist(t.artist.clone())));
        }

        // `.Timeline` (`PopupDock.css:237-301`): outlined white capsule + below-bar ScrubPreview
        let dur_ms = app.duration_ms().max(1);
        let frac = app
            .seek_drag
            .unwrap_or_else(|| (position as f32 / dur_ms as f32).clamp(0.0, 1.0));
        let tl_top = media_box_top + m.gap;
        let bar_cy = tl_top + m.bar_row * 0.5;
        let tl_hit = Rect::from_center_size(
            Pos2::new(rect.center().x, bar_cy),
            Vec2::new(meta_w, (m.bar_row + 6.0).max(14.0)),
        );
        let tl_dragging = draw_popup_capsule_timeline(
            app,
            &mut card_ui,
            tl_hit,
            frac,
            dur_ms,
            m.bar_h,
            m.time_sz,
            motion_on,
        );
        any_slider_dragging |= tl_dragging;

        if m.show_times {
            let shown_f = app.seek_drag.unwrap_or(frac).clamp(0.0, 1.0);
            let time_y = tl_top + m.bar_row + 3.0 + m.time_sz * 0.62;
            let p_lbl = mmss((shown_f * dur_ms as f32).round() as u64);
            let rem = app.show_remaining_time();
            let d_lbl = if rem {
                format!("-{}", mmss(dur_ms.saturating_sub((shown_f * dur_ms as f32).round() as u64)))
            } else {
                mmss(dur_ms)
            };
            card_ui.painter().text(
                Pos2::new(tl_hit.left(), time_y),
                egui::Align2::LEFT_CENTER,
                p_lbl,
                semi(m.time_sz),
                Color32::from_white_alpha(192),
            );
            let d_rect = card_ui.painter().text(
                Pos2::new(tl_hit.right(), time_y),
                egui::Align2::RIGHT_CENTER,
                d_lbl,
                semi(m.time_sz),
                Color32::from_white_alpha(192),
            );
            if card_ui
                .interact(
                    d_rect.expand(3.0),
                    egui::Id::unique("spicy-popup-rem-toggle"),
                    egui::Sense::click(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
            {
                app.actions.push(Action::ToggleRemainingTime);
            }
        }

        // Row 2: `[Heart | Shuffle << Play >> Loop | Speaker + Volume]` (`PopupDock.css:303-485`)
        if controls_opacity > 0.01 {
            let co = controls_opacity;
            let ctrl_cy = tl_top + m.timeline_h + m.gap + m.controls_h * 0.5 + pd_transport_drop;
            let narrow_330 = w <= 330.0;
            let narrow_280 = w <= 280.0;

            if !narrow_330 {
                // `paint_cinema_heart` draws at `0.54 * 0.88 = 0.475` of box width -> size virtual box so glyph == `m.heart_w`
                let heart_box_sz = m.heart_w / 0.475;
                let heart_art_box = Rect::from_center_size(
                    Pos2::new(rect.left() + m.gutter + m.heart_w * 0.5, ctrl_cy),
                    Vec2::splat(heart_box_sz),
                );
                paint_cinema_heart(app, &mut card_ui, heart_art_box, t.id, co * 0.82, motion_on);
            }

            let btn_hit_w = m.play_h * 1.45;
            let step_x = btn_hit_w + m.control_gap * 0.45;
            let n_btns = if narrow_280 { 3 } else { 5 };
            let cluster_w = n_btns as f32 * step_x;
            let cluster_cx = if narrow_330 {
                rect.left() + m.gutter + cluster_w * 0.5
            } else {
                rect.center().x - 6.0
            };

            let fg_hi = Color32::from_white_alpha((255.0 * co).round() as u8);
            let fg_play = Color32::from_white_alpha((236.0 * co).round() as u8);
            let fg_skip = Color32::from_white_alpha((218.0 * co).round() as u8);
            let fg_sub = Color32::from_white_alpha((186.0 * co).round() as u8);

            if !narrow_280 {
                let shuf_r = Rect::from_center_size(
                    Pos2::new(cluster_cx - 2.0 * step_x, ctrl_cy),
                    Vec2::new(btn_hit_w, m.controls_h),
                );
                let shuf_col = if app.player.shuffle {
                    ACCENT.gamma_multiply(co)
                } else {
                    fg_sub
                };
                if icon_button_sized(
                    &mut card_ui,
                    shuf_r,
                    "spicy-popup-shuf",
                    Icon::Shuffle,
                    shuf_col,
                    app.t("Trộn bài", "Shuffle"),
                    m.toggle_h * 1.18,
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
                {
                    app.actions.push(Action::ToggleShuffle);
                }
            }

            let prev_off = -step_x;
            let prev_r = Rect::from_center_size(
                Pos2::new(cluster_cx + prev_off, ctrl_cy),
                Vec2::new(btn_hit_w, m.controls_h),
            );
            let prev_resp = card_ui
                .interact(prev_r, egui::Id::unique("spicy-popup-prev"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(app.t("Bài trước", "Previous"));
            paint_spicy_double_skip(
                card_ui.painter(),
                Rect::from_center_size(prev_r.center(), Vec2::splat(m.skip_h * 1.75)),
                false,
                if prev_resp.hovered() { fg_hi } else { fg_skip },
            );
            if prev_resp.clicked() {
                app.actions.push(Action::Previous);
            }

            let play_r = Rect::from_center_size(
                Pos2::new(cluster_cx, ctrl_cy),
                Vec2::new(btn_hit_w, m.controls_h),
            );
            let play_resp = card_ui
                .interact(play_r, egui::Id::unique("spicy-popup-play"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(app.t("Phát / tạm dừng", "Play / pause"));
            let play_col = if play_resp.hovered() { fg_hi } else { fg_play };
            let play_sz = m.play_h * if play_resp.hovered() { 1.06 } else { 1.0 };
            if playing {
                icons::paint(
                    card_ui.painter(),
                    Rect::from_center_size(play_r.center(), Vec2::splat(play_sz * 1.05)),
                    Icon::Pause,
                    play_col,
                );
            } else {
                // Plump rounded Play triangle matching `02e48b2e.jpg`
                let pc = play_r.center();
                let ph = play_sz;
                let pw = ph * 0.88;
                paint_rounded_triangle(
                    card_ui.painter(),
                    Pos2::new(pc.x - pw * 0.44, pc.y - ph * 0.5),
                    Pos2::new(pc.x + pw * 0.56, pc.y),
                    Pos2::new(pc.x - pw * 0.44, pc.y + ph * 0.5),
                    (ph * 0.12).clamp(1.2, 2.8),
                    play_col,
                );
            }
            if play_resp.clicked() {
                app.actions.push(Action::TogglePlay);
            }

            let next_r = Rect::from_center_size(
                Pos2::new(cluster_cx + step_x, ctrl_cy),
                Vec2::new(btn_hit_w, m.controls_h),
            );
            let next_resp = card_ui
                .interact(next_r, egui::Id::unique("spicy-popup-next"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(app.t("Bài tiếp theo", "Next"));
            paint_spicy_double_skip(
                card_ui.painter(),
                Rect::from_center_size(next_r.center(), Vec2::splat(m.skip_h * 1.75)),
                true,
                if next_resp.hovered() { fg_hi } else { fg_skip },
            );
            if next_resp.clicked() {
                app.actions.push(Action::Next);
            }

            if !narrow_280 {
                let rep_on = app.player.repeat != Repeat::Off;
                let rep_r = Rect::from_center_size(
                    Pos2::new(cluster_cx + 2.0 * step_x, ctrl_cy),
                    Vec2::new(btn_hit_w, m.controls_h),
                );
                let rep_col = if rep_on {
                    ACCENT.gamma_multiply(co)
                } else {
                    fg_sub
                };
                if icon_button_sized(
                    &mut card_ui,
                    rep_r,
                    "spicy-popup-rep",
                    if app.player.repeat == Repeat::Track {
                        Icon::RepeatOne
                    } else {
                        Icon::Repeat
                    },
                    rep_col,
                    app.t("Lặp lại", "Repeat"),
                    m.toggle_h * 1.18,
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .clicked()
                {
                    app.actions.push(Action::CycleRepeat);
                }
            }

            // Right Volume capsule (`PopupDock.css:347-423`, stretches when `w <= 330px`)
            let vol_right = rect.right() - m.gutter;
            let vol_w = if narrow_330 {
                (vol_right - (cluster_cx + cluster_w * 0.5 + m.vol_icon + 16.0)).clamp(36.0, 110.0)
            } else {
                m.vol_w
            };
            let vol_hit = Rect::from_min_max(
                Pos2::new(vol_right - vol_w, ctrl_cy - 8.0),
                Pos2::new(vol_right, ctrl_cy + 8.0),
            );
            let spk_r = Rect::from_center_size(
                Pos2::new(vol_hit.left() - 6.0 - m.vol_icon * 0.5, ctrl_cy),
                Vec2::splat(m.vol_icon + 6.0),
            );
            let vol_val = if app.data.settings.muted {
                0.0
            } else {
                app.data.settings.volume.clamp(0.0, 1.0)
            };
            let spk_ic = if vol_val <= 0.005 {
                Icon::Mute
            } else if vol_val < 0.45 {
                Icon::VolumeLow
            } else {
                Icon::Volume
            };
            if icon_button_sized(
                &mut card_ui,
                spk_r,
                "spicy-popup-mute",
                spk_ic,
                fg_sub,
                app.t("Âm lượng", "Volume"),
                m.vol_icon * 0.88,
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            {
                app.actions.push(Action::ToggleMute);
            }

            let v_resp = card_ui
                .interact(vol_hit, egui::Id::unique("spicy-popup-vol"), egui::Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            let v_drag = v_resp.dragged() || v_resp.is_pointer_button_down_on();
            any_slider_dragging |= v_drag;
            let v_engaged = v_resp.hovered() || v_drag;
            let target_vh = if v_engaged { m.vol_h * 1.5 } else { m.vol_h };
            let cur_vh = if motion_on {
                ctx.animate_value_with_time(egui::Id::unique("spicy-popup-vh"), target_vh, 0.14)
            } else {
                target_vh
            };
            let v_bar = Rect::from_center_size(vol_hit.center(), Vec2::new(vol_w, cur_vh));
            let v_rad = CornerRadius::same((cur_vh * 0.5).round() as u8);
            card_ui.painter().rect_filled(
                v_bar,
                v_rad,
                Color32::from_white_alpha((18.0 * co).round() as u8),
            );
            card_ui.painter().rect_stroke(
                v_bar,
                v_rad,
                Stroke::new(1.0, Color32::from_white_alpha((78.0 * co).round() as u8)),
                egui::StrokeKind::Inside,
            );
            if vol_val > 0.005 {
                let fw = (v_bar.width() * vol_val).max(cur_vh.min(v_bar.width()));
                let fill_r = Rect::from_min_size(v_bar.min, Vec2::new(fw, cur_vh));
                card_ui.painter().rect_filled(
                    fill_r,
                    v_rad,
                    Color32::from_white_alpha((228.0 * co).round() as u8),
                );
            }
            if v_drag || v_resp.clicked() {
                if let Some(p) = v_resp.interact_pointer_pos() {
                    let nv = ((p.x - v_bar.left()) / v_bar.width().max(1.0)).clamp(0.0, 1.0);
                    app.actions.push(Action::SetVolume(nv));
                }
            }
            if v_engaged || spk_r.contains(ptr.unwrap_or(Pos2::new(-999.0, -999.0))) {
                let wheel = card_ui.input(|i| i.smooth_scroll_delta.y);
                if wheel.abs() > 0.0 {
                    app.actions
                        .push(Action::SetVolume((vol_val + wheel / 1200.0).clamp(0.0, 1.0)));
                }
            }
        }
    }

    // Top-left frosted ViewControls pill (`[X | LyricsManager]`, `02e48b2e.jpg` + `PopupDock.css:428-441`)
    if controls_opacity > 0.01 {
        let co = controls_opacity;
        let pill_slide_y = if motion_on {
            -(1.0 - dock_lift) * 8.0
        } else {
            0.0
        };
        let pill_r = Rect::from_min_size(
            rect.min + Vec2::new(m.view_inset, m.view_inset + pill_slide_y),
            Vec2::new(pill_w, pill_h),
        );
        let pill_rad = CornerRadius::same((pill_h * 0.5).round() as u8);
        let pill_sh = egui::epaint::Shadow {
            offset: [0, 6],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha((88.0 * co).round() as u8),
        };
        card_ui.painter().add(pill_sh.as_shape(pill_r.shrink(1.0), pill_rad));
        card_ui.painter().rect_filled(
            pill_r,
            pill_rad,
            Color32::from_black_alpha((72.0 * co).round() as u8),
        );
        card_ui.painter().rect_filled(
            pill_r,
            pill_rad,
            Color32::from_white_alpha((22.0 * co).round() as u8),
        );
        card_ui.painter().rect_stroke(
            pill_r,
            pill_rad,
            Stroke::new(1.0, Color32::from_white_alpha((54.0 * co).round() as u8)),
            egui::StrokeKind::Inside,
        );

        let close_r = Rect::from_center_size(
            Pos2::new(pill_r.left() + pill_w * 0.28, pill_r.center().y),
            Vec2::splat(m.view_size * 0.92),
        );
        let c_resp = card_ui
            .interact(close_r, egui::Id::unique("spicy-popup-close"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if c_resp.hovered() {
            paint_clamped_tooltip(&card_ui, close_r, app.t("Đóng", "Close"), rect, true);
        }
        icons::paint_spicy(
            card_ui.painter(),
            Rect::from_center_size(close_r.center(), Vec2::splat(m.view_size * 0.54)),
            SpicyIcon::Close,
            Color32::from_white_alpha(if c_resp.hovered() {
                (255.0 * co).round() as u8
            } else {
                (226.0 * co).round() as u8
            }),
        );
        if c_resp.clicked() {
            app.actions.push(Action::ToggleMiniplayer);
        }

        let open_r = Rect::from_center_size(
            Pos2::new(pill_r.right() - pill_w * 0.28, pill_r.center().y),
            Vec2::splat(m.view_size * 0.92),
        );
        let o_resp = card_ui
            .interact(open_r, egui::Id::unique("spicy-popup-expand"), egui::Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if o_resp.hovered() {
            paint_clamped_tooltip(
                &card_ui,
                open_r,
                app.t("Mở trang Lời bài hát", "Open lyrics page"),
                rect,
                true,
            );
        }
        paint_spicy_view_glyph(
            card_ui.painter(),
            Rect::from_center_size(open_r.center(), Vec2::splat(m.view_size * 0.58)),
            7,
            false,
            false,
            Color32::from_white_alpha(if o_resp.hovered() {
                (255.0 * co).round() as u8
            } else {
                (226.0 * co).round() as u8
            }),
        );
        if o_resp.clicked() {
            app.actions.push(Action::Navigate(Route::Lyrics));
        } else if o_resp.secondary_clicked() {
            app.data.settings.miniplayer_cover_mode = !app.data.settings.miniplayer_cover_mode;
            app.dirty = true;
        }
    }

    // Place the OS window drag handle AFTER `ScrollArea` / cover art so the ScrollArea never steals window-move drags!
    if let Some(vp_id) = native_vp {
        let drag_left = if controls_opacity > 0.05 {
            pill_hover_zone.right() + 4.0
        } else {
            rect.left() + 8.0
        };
        let drag_bottom = if cover_mode {
            (rect.bottom() - m.bottom - m.box_h - m.meta_h + pd_drop - 6.0).max(rect.top() + 40.0)
        } else {
            (rect.top() + 40.0).min(rect.bottom() - m.idle_h)
        };
        let drag_strip = Rect::from_min_max(
            Pos2::new(drag_left, rect.top() + 5.0),
            Pos2::new(rect.right() - 8.0, drag_bottom),
        );
        if drag_strip.width() > 16.0 && drag_strip.height() > 8.0 {
            let d_resp = card_ui
                .interact(
                    drag_strip,
                    egui::Id::unique("popup-os-drag-top"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::Grab);
            if d_resp.drag_started()
                || (d_resp.is_pointer_button_down_on() && card_ui.input(|i| i.pointer.is_moving()))
            {
                ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::StartDrag);
            }
        }
        if controls_opacity > 0.05 {
            let grip = Rect::from_center_size(Pos2::new(rect.center().x, rect.top() + 10.0), Vec2::new(36.0, 3.5));
            card_ui.painter().rect_filled(
                grip,
                2.0,
                Color32::from_white_alpha((115.0 * controls_opacity).round() as u8),
            );
        }
    }

    // Spotify Desktop Miniplayer 9x9 bottom-right diagonal resize grip (`mnQ67ihgNmGZ9lAZVD68`, xpui-pip-mini-player.css)
    if controls_opacity > 0.05 {
        let g_col = Color32::from_white_alpha((128.0 * controls_opacity).round() as u8);
        let br = rect.max - Vec2::splat(4.0);
        card_ui.painter().line_segment(
            [Pos2::new(br.x - 7.0, br.y), Pos2::new(br.x, br.y - 7.0)],
            Stroke::new(1.2, g_col),
        );
        card_ui.painter().line_segment(
            [Pos2::new(br.x - 3.5, br.y), Pos2::new(br.x, br.y - 3.5)],
            Stroke::new(1.2, g_col),
        );
    }

    // Crisp outer 1px window rim (`7a34afb4.jpg` & `02e48b2e.jpg`)
    card_ui.painter().rect_stroke(
        rect,
        CornerRadius::same(corner_r),
        Stroke::new(1.0, Color32::from_white_alpha(32)),
        egui::StrokeKind::Inside,
    );

    sp.drag_lock = any_slider_dragging;
    ctx.data_mut(|d| d.insert_temp(spring_id, sp));
    if animating_bd || playing || spring_busy {
        if motion_on {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(42));
        }
    }
}

/// Outlined white capsule timeline for Spicy Popup Lyrics (`PopupDock.css:252-291`, `9a4a97ec.jpg` & `02e48b2e.jpg`).
/// Returns `true` while being actively dragged (`controlsDragLock`).
#[allow(clippy::too_many_arguments)]
fn draw_popup_capsule_timeline(
    app: &mut App,
    ui: &mut Ui,
    hit_rect: Rect,
    shown: f32,
    duration_ms: u64,
    rest_h: f32,
    preview_font_sz: f32,
    motion_on: bool,
) -> bool {
    let resp = ui
        .interact(
            hit_rect,
            ui.scope_id().with("popup-capsule-scrubber"),
            egui::Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    let dragging = resp.dragged() || resp.is_pointer_button_down_on();
    let engaged = resp.hovered() || dragging;
    let target_h = if engaged { rest_h * 1.8 } else { rest_h };
    let bar_h = if motion_on {
        ui.ctx()
            .animate_value_with_time(ui.scope_id().with("popup-tl-h"), target_h, 0.14)
    } else {
        target_h
    };
    let bar_rect = Rect::from_center_size(hit_rect.center(), Vec2::new(hit_rect.width(), bar_h));
    let radius = CornerRadius::same((bar_h * 0.5).round() as u8);

    let mut cur = shown.clamp(0.0, 1.0);
    if let Some(pos) = resp.interact_pointer_pos() {
        let f = ((pos.x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0);
        if dragging {
            app.seek_drag = Some(f);
            cur = f;
        }
        if resp.drag_stopped() || resp.clicked() {
            let target = app.seek_drag.take().unwrap_or(f);
            if app.can_play_in_app() && duration_ms > 0 {
                app.actions
                    .push(Action::SeekTo((target * duration_ms as f32).round() as u64));
            }
        }
    } else if resp.drag_stopped() {
        if let Some(target) = app.seek_drag.take() {
            if app.can_play_in_app() && duration_ms > 0 {
                app.actions
                    .push(Action::SeekTo((target * duration_ms as f32).round() as u64));
            }
        }
    }

    // Track + subtle white capsule outline (`9a4a97ec.jpg` & `02e48b2e.jpg`)
    ui.painter()
        .rect_filled(bar_rect, radius, Color32::from_white_alpha(18));
    ui.painter().rect_stroke(
        bar_rect,
        radius,
        Stroke::new(1.0, Color32::from_white_alpha(78)),
        egui::StrokeKind::Inside,
    );
    if cur > 0.002 {
        let fw = (bar_rect.width() * cur).max(bar_h.min(bar_rect.width()));
        let fill_r = Rect::from_min_size(bar_rect.min, Vec2::new(fw, bar_h));
        ui.painter().rect_filled(fill_r, radius, Color32::WHITE);
    }

    // `PopupDock.css:286`: ScrubPreview hangs BELOW the timeline bar (`rgba(18, 18, 22, 0.78)`)
    if engaged && duration_ms > 0 {
        if let Some(ptr) = ui.input(|i| i.pointer.hover_pos()) {
            let clamped_x = ptr.x.clamp(bar_rect.left(), bar_rect.right());
            let hover_f = if dragging {
                cur
            } else {
                ((clamped_x - bar_rect.left()) / bar_rect.width().max(1.0)).clamp(0.0, 1.0)
            };
            let label = mmss((hover_f * duration_ms as f32).round() as u64);
            let lw = text_width(ui, &label, semi(preview_font_sz)) + 12.0;
            let cx = clamped_x.clamp(bar_rect.left() + lw * 0.5, bar_rect.right() - lw * 0.5);
            let pill_r = Rect::from_center_size(
                Pos2::new(cx, bar_rect.bottom() + 14.0),
                Vec2::new(lw, 21.0),
            );
            ui.painter().rect_filled(
                pill_r,
                CornerRadius::same(6),
                Color32::from_rgba_unmultiplied(18, 18, 22, 200),
            );
            ui.painter().rect_stroke(
                pill_r,
                CornerRadius::same(6),
                Stroke::new(1.0, Color32::from_white_alpha(46)),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                pill_r.center(),
                egui::Align2::CENTER_CENTER,
                label,
                semi(preview_font_sz),
                Color32::WHITE,
            );
        }
    }
    dragging
}

/// Spotify Desktop compact horizontal miniplayer bar (`xpui-pip-mini-player.css`, `@media (height <= 170px)`).
#[allow(clippy::too_many_arguments)]
fn render_compact_pip_bar(
    app: &mut App,
    ui: &mut Ui,
    rect: Rect,
    track: Option<&std::sync::Arc<Track>>,
    position: u64,
    playing: bool,
    motion_on: bool,
    native_vp: Option<egui::ViewportId>,
) {
    let native_viewport = native_vp.is_some();
    let ctx = ui.ctx().clone();
    let corner_r = if native_viewport { 6 } else { 8 };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(corner_r), Color32::from_black_alpha(96));

    let hovered = ui
        .input(|i| i.pointer.hover_pos())
        .is_some_and(|p| rect.contains(p))
        || ctx
            .data(|d| d.get_temp::<bool>(egui::Id::NULL.with("spicy-popup-force-hover")))
            .unwrap_or(false);
    let hov_t = if motion_on {
        ctx.animate_bool_with_time(egui::Id::unique("pip-compact-hov"), hovered, 0.16)
    } else if hovered {
        1.0
    } else {
        0.0
    };

    if let Some(vp_id) = native_vp {
        let drag_r = Rect::from_min_max(
            rect.min + Vec2::new(6.0, 4.0),
            Pos2::new((rect.right() - 128.0).max(rect.left() + 48.0), rect.bottom() - 10.0),
        );
        let d_resp = ui
            .interact(drag_r, egui::Id::unique("pip-compact-drag"), egui::Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::Grab);
        if d_resp.drag_started()
            || (d_resp.is_pointer_button_down_on() && ui.input(|i| i.pointer.is_moving()))
        {
            ctx.send_viewport_cmd_to(vp_id, egui::ViewportCommand::StartDrag);
        }
    }

    let pad = 12.0;
    let cov_sz = (rect.height() - pad * 2.0 - 4.0).clamp(44.0, 88.0);
    let cov_r = Rect::from_center_size(
        Pos2::new(rect.left() + pad + cov_sz * 0.5, rect.center().y - 1.0),
        Vec2::splat(cov_sz),
    );
    if let Some(t) = track {
        let tex = app.art.get(&ctx, &t.cover_key(), t.art_source());
        cover_shadow(ui.painter(), cov_r, 6.0);
        cover(ui, cov_r, &tex, 6.0);

        let controls_w = if hov_t > 0.02 { 118.0 * hov_t } else { 0.0 };
        let text_left = cov_r.right() + 12.0;
        let text_right = (rect.right() - pad - controls_w - 4.0).max(text_left + 32.0);
        let text_w = text_right - text_left;
        let title_r = Rect::from_min_size(
            Pos2::new(text_left, rect.center().y - 18.0),
            Vec2::new(text_w, 19.0),
        );
        marquee_text(ui, title_r, "pip-bar-title", &t.title, bold(14.5), Color32::WHITE);
        text(
            ui.painter(),
            Pos2::new(text_left, rect.center().y + 10.0),
            &t.artist,
            body(12.5),
            Color32::from_white_alpha(190),
            text_w,
            egui::Align2::LEFT_CENTER,
        );

        if hov_t > 0.03 {
            let cy = rect.center().y + 2.0;
            let a = (255.0 * hov_t).round() as u8;
            let sub = Color32::from_white_alpha((195.0 * hov_t).round() as u8);
            let liked = app.data.is_liked(t.id);
            let like_r = Rect::from_center_size(Pos2::new(rect.right() - 110.0, cy), Vec2::splat(24.0));
            if icon_button_sized(
                ui,
                like_r,
                "pip-bar-like",
                if liked { Icon::Saved } else { Icon::Save },
                if liked { ACCENT.gamma_multiply(hov_t) } else { sub },
                app.t("Lưu", "Save"),
                14.5,
            )
            .clicked()
            {
                app.actions.push(Action::ToggleLike(t.id));
            }

            let prev_r = Rect::from_center_size(Pos2::new(rect.right() - 82.0, cy), Vec2::splat(24.0));
            let pr = ui
                .interact(prev_r, egui::Id::unique("pip-bar-prev"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            paint_spicy_double_skip(
                ui.painter(),
                Rect::from_center_size(prev_r.center(), Vec2::splat(15.5)),
                false,
                if pr.hovered() { Color32::from_white_alpha(a) } else { sub },
            );
            if pr.clicked() {
                app.actions.push(Action::Previous);
            }

            let play_r = Rect::from_center_size(Pos2::new(rect.right() - 51.0, cy), Vec2::splat(30.0));
            ui.painter()
                .circle_filled(play_r.center(), 14.5, Color32::from_white_alpha(a));
            let p_resp = ui
                .interact(play_r, egui::Id::unique("pip-bar-play"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            icons::paint(
                ui.painter(),
                Rect::from_center_size(play_r.center(), Vec2::splat(14.5)),
                if playing { Icon::Pause } else { Icon::Play },
                Color32::from_black_alpha(a),
            );
            if p_resp.clicked() {
                app.actions.push(Action::TogglePlay);
            }

            let next_r = Rect::from_center_size(Pos2::new(rect.right() - 21.0, cy), Vec2::splat(24.0));
            let nr = ui
                .interact(next_r, egui::Id::unique("pip-bar-next"), egui::Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            paint_spicy_double_skip(
                ui.painter(),
                Rect::from_center_size(next_r.center(), Vec2::splat(16.5)),
                true,
                if nr.hovered() { Color32::from_white_alpha(a) } else { sub },
            );
            if nr.clicked() {
                app.actions.push(Action::Next);
            }
        }
    }

    if hov_t > 0.05 {
        let close_r = Rect::from_center_size(Pos2::new(rect.right() - 14.0, rect.top() + 13.0), Vec2::splat(20.0));
        if icon_button_sized(
            ui,
            close_r,
            "pip-bar-close",
            Icon::Close,
            Color32::from_white_alpha((220.0 * hov_t).round() as u8),
            app.t("Đóng", "Close"),
            11.5,
        )
        .clicked()
        {
            app.actions.push(Action::ToggleMiniplayer);
        }
    }

    // `PsxS3aJb1w_UFKxu7INQ` (`xpui-pip-mini-player.css`): 2px bottom white progress strip
    let dur_ms = app.duration_ms().max(1);
    let frac = (position as f32 / dur_ms as f32).clamp(0.0, 1.0);
    let strip = Rect::from_min_max(
        Pos2::new(rect.left() + 2.0, rect.bottom() - 2.5),
        Pos2::new(rect.right() - 2.0, rect.bottom() - 0.5),
    );
    ui.painter()
        .rect_filled(strip, 1.0, Color32::from_white_alpha(38));
    if frac > 0.002 {
        let fill = Rect::from_min_size(strip.min, Vec2::new(strip.width() * frac, strip.height()));
        ui.painter().rect_filled(fill, 1.0, Color32::WHITE);
    }

    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(corner_r),
        Stroke::new(1.0, Color32::from_white_alpha(32)),
        egui::StrokeKind::Inside,
    );
}

fn draw_ttml_manager_modal(app: &mut App, ui: &mut Ui, current: Option<&std::sync::Arc<Track>>) {
    let ctx = ui.ctx();
    let screen = ctx.viewport_rect();

    // Backdrop scrim
    egui::Area::new(ui.scope_id().with("ttml-manager-scrim"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let resp = ui.allocate_rect(screen, egui::Sense::click());
            ui.painter()
                .rect_filled(screen, 0.0, Color32::from_black_alpha(168));
            if resp.clicked() {
                app.lyrics_manager_open = false;
                app.lyrics_delete_confirm_for = None;
            }
        });

    let modal_w = (screen.width() - 48.0).clamp(320.0, 560.0);
    let modal_h = (screen.height() - 64.0).clamp(340.0, 590.0);
    let modal_pos = Pos2::new(
        screen.center().x - modal_w * 0.5,
        screen.center().y - modal_h * 0.5,
    );

    egui::Area::new(ui.scope_id().with("ttml-manager-modal"))
        .order(egui::Order::Foreground)
        .fixed_pos(modal_pos)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(ELEVATED)
                .stroke(Stroke::new(1.0, HAIRLINE))
                .corner_radius(CornerRadius::same(RADIUS_MODAL as u8))
                .inner_margin(egui::Margin::same(20))
                .shadow(egui::Shadow {
                    offset: [0, 14],
                    blur: 32,
                    spread: 2,
                    color: Color32::from_black_alpha(150),
                })
                .show(ui, |ui| {
                    ui.set_width(modal_w - 40.0);
                    ui.set_max_height(modal_h - 40.0);

                    // Header
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(app.t("Trình quản lý TTML", "TTML Lyrics Manager"))
                                .font(bold(18.0))
                                .color(TEXT),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::hover());
                            if icon_button_sized(
                                ui,
                                r,
                                "ttml-modal-close",
                                Icon::Close,
                                SUBTLE,
                                app.t("Đóng", "Close"),
                                15.0,
                            )
                            .clicked()
                            {
                                app.lyrics_manager_open = false;
                                app.lyrics_delete_confirm_for = None;
                            }
                        });
                    });

                    ui.add_space(12.0);

                    // Current track import/export/reset/delete card
                    if let Some(track) = current {
                        let id = track.id;
                        let imported = app.lyrics_imported();
                        let session_only = app.lyrics_session_override;
                        let exportable = app.export_lyrics_xml(track).is_some();

                        egui::Frame::new()
                            .fill(POPUP)
                            .stroke(Stroke::new(1.0, HAIRLINE))
                            .corner_radius(CornerRadius::same(10))
                            .inner_margin(egui::Margin::same(12))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let key = track.cover_key();
                                    let tex = app.art.get(ui.ctx(), &key, track.art_source());
                                    let (art_r, _) = ui.allocate_exact_size(
                                        Vec2::splat(44.0),
                                        egui::Sense::hover(),
                                    );
                                    cover(ui, art_r, &tex, 6.0);
                                    ui.add_space(10.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(&track.title).font(semi(14.0)).color(TEXT),
                                        );
                                        ui.label(
                                            RichText::new(&track.artist)
                                                .font(marginal())
                                                .color(SUBTLE),
                                        );
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        let badge = if session_only {
                                            app.t("Chỉ lần này", "Just once")
                                        } else if imported {
                                            app.t("Đã lưu TTML", "Saved TTML")
                                        } else {
                                            app.t("Mặc định / Nhúng", "Auto / Embedded")
                                        };
                                        let badge_color = if session_only || imported {
                                            ACCENT
                                        } else {
                                            SUBTLE
                                        };
                                        egui::Frame::new()
                                            .fill(Color32::from_white_alpha(14))
                                            .corner_radius(CornerRadius::same(10))
                                            .inner_margin(egui::Margin::symmetric(8, 3))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new(badge)
                                                        .font(marginal_small())
                                                        .color(badge_color),
                                                );
                                            });
                                    });
                                });

                                ui.add_space(10.0);

                                // Save vs Just once segmented toggle
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(app.t("Chế độ nhập:", "Import mode:"))
                                            .font(marginal())
                                            .color(SUBTLE),
                                    );
                                    ui.add_space(6.0);
                                    for (mode, label) in [
                                        (TtmlImportMode::Save, app.t("Lưu", "Save")),
                                        (TtmlImportMode::JustOnce, app.t("Chỉ lần này", "Just once")),
                                    ] {
                                        let selected = app.lyrics_ttml_mode == mode;
                                        let btn = egui::Button::new(
                                            RichText::new(label)
                                                .font(semi(12.5))
                                                .color(if selected { Color32::BLACK } else { TEXT }),
                                        )
                                        .fill(if selected { ACCENT } else { HOVER })
                                        .corner_radius(CornerRadius::same(14));
                                        if ui
                                            .add(btn)
                                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                                            .clicked()
                                        {
                                            app.actions.push(Action::SetTtmlImportMode(mode));
                                        }
                                    }
                                });

                                ui.add_space(10.0);

                                // Action buttons: Import, Export, Reset, 2-step Delete
                                ui.horizontal_wrapped(|ui| {
                                    let import_btn = egui::Button::new(
                                        RichText::new(app.t("Nhập .ttml…", "Import .ttml…"))
                                            .font(semi(12.5))
                                            .color(Color32::BLACK),
                                    )
                                    .fill(ACCENT)
                                    .corner_radius(CornerRadius::same(16));
                                    if ui
                                        .add(import_btn)
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        app.actions.push(Action::ImportLyricsWithMode(
                                            id,
                                            app.lyrics_ttml_mode,
                                        ));
                                    }

                                    let export_btn = egui::Button::new(
                                        RichText::new(app.t("Xuất .ttml", "Export .ttml"))
                                            .font(semi(12.5))
                                            .color(if exportable { TEXT } else { MUTED }),
                                    )
                                    .fill(HOVER)
                                    .corner_radius(CornerRadius::same(16));
                                    if ui
                                        .add_enabled(exportable, export_btn)
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        app.actions.push(Action::ExportLyrics(id));
                                    }

                                    let can_reset = imported || session_only;
                                    let reset_btn = egui::Button::new(
                                        RichText::new(app.t("Đặt lại TTML", "Reset TTML"))
                                            .font(semi(12.5))
                                            .color(if can_reset { TEXT } else { MUTED }),
                                    )
                                    .fill(HOVER)
                                    .corner_radius(CornerRadius::same(16));
                                    if ui
                                        .add_enabled(can_reset, reset_btn)
                                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                                        .clicked()
                                    {
                                        app.actions.push(Action::ResetLyrics(id));
                                    }

                                    if imported {
                                        if app.lyrics_delete_confirm_for == Some(id) {
                                            let confirm_btn = egui::Button::new(
                                                RichText::new(app.t("Xác nhận xoá", "Confirm delete"))
                                                    .font(semi(12.5))
                                                    .color(Color32::WHITE),
                                            )
                                            .fill(Color32::from_rgb(208, 56, 56))
                                            .corner_radius(CornerRadius::same(16));
                                            if ui
                                                .add(confirm_btn)
                                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                .clicked()
                                            {
                                                app.actions.push(Action::RequestDeleteTtml(id));
                                                invalidate_ttml_cache(ui.ctx());
                                            }
                                            let cancel_btn = egui::Button::new(
                                                RichText::new(app.t("Huỷ", "Cancel"))
                                                    .font(semi(12.5))
                                                    .color(SUBTLE),
                                            )
                                            .fill(Color32::TRANSPARENT)
                                            .corner_radius(CornerRadius::same(16));
                                            if ui.add(cancel_btn).clicked() {
                                                app.lyrics_delete_confirm_for = None;
                                            }
                                        } else {
                                            let del_btn = egui::Button::new(
                                                RichText::new(app.t("Xoá TTML", "Delete TTML"))
                                                    .font(semi(12.5))
                                                    .color(Color32::from_rgb(245, 118, 118)),
                                            )
                                            .fill(HOVER)
                                            .corner_radius(CornerRadius::same(16));
                                            if ui
                                                .add(del_btn)
                                                .on_hover_cursor(egui::CursorIcon::PointingHand)
                                                .clicked()
                                            {
                                                app.actions.push(Action::ConfirmDeleteTtmlStep(id));
                                            }
                                        }
                                    }
                                });
                            });
                        ui.add_space(12.0);
                    }

                    // Search saved TTML library
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(app.t("Tệp TTML đã lưu", "Saved TTML Library"))
                                .font(semi(13.5))
                                .color(TEXT),
                        );
                    });
                    ui.add_space(6.0);
                    let search_hint = app.t("Tìm bài hát hoặc nghệ sĩ…", "Search songs or artists…");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.lyrics_manager_query)
                            .desired_width(f32::INFINITY)
                            .hint_text(search_hint),
                    );
                    ui.add_space(8.0);

                    let saved_ids = cached_saved_ttml_tracks(ui.ctx(), app);
                    let query = fold(app.lyrics_manager_query.trim());
                    let filtered: Vec<TrackId> = saved_ids
                        .into_iter()
                        .filter(|id| {
                            if query.is_empty() {
                                return true;
                            }
                            app.track(*id).is_some_and(|t| {
                                fold(&t.title).contains(&query) || fold(&t.artist).contains(&query)
                            })
                        })
                        .collect();

                    let list_h = (ui.available_height()).max(110.0);
                    egui::ScrollArea::vertical()
                        .id_salt("ttml-saved-list")
                        .max_height(list_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if filtered.is_empty() {
                                ui.add_space(16.0);
                                ui.label(
                                    RichText::new(app.t(
                                        "Chưa có tệp TTML đã nhập",
                                        "No saved TTML files match",
                                    ))
                                    .font(marginal())
                                    .color(SUBTLE),
                                );
                                return;
                            }
                            for id in filtered {
                                let Some(track) = app.track(id) else {
                                    continue;
                                };
                                ui.horizontal(|ui| {
                                    ui.set_height(42.0);
                                    let key = track.cover_key();
                                    let tex = app.art.get(ui.ctx(), &key, track.art_source());
                                    let (art_r, _) = ui.allocate_exact_size(
                                        Vec2::splat(32.0),
                                        egui::Sense::hover(),
                                    );
                                    cover(ui, art_r, &tex, 4.0);
                                    ui.add_space(8.0);
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(&track.title).font(semi(13.0)).color(TEXT),
                                        );
                                        ui.label(
                                            RichText::new(&track.artist)
                                                .font(marginal_small())
                                                .color(SUBTLE),
                                        );
                                    });
                                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                        if app.lyrics_delete_confirm_for == Some(id) {
                                            let confirm = egui::Button::new(
                                                RichText::new(app.t("Xác nhận", "Confirm"))
                                                    .font(marginal_small())
                                                    .color(Color32::WHITE),
                                            )
                                            .fill(Color32::from_rgb(208, 56, 56))
                                            .corner_radius(CornerRadius::same(12));
                                            if ui.add(confirm).clicked() {
                                                app.actions.push(Action::RequestDeleteTtml(id));
                                                invalidate_ttml_cache(ui.ctx());
                                            }
                                            let cancel = egui::Button::new(
                                                RichText::new(app.t("Huỷ", "Cancel"))
                                                    .font(marginal_small())
                                                    .color(SUBTLE),
                                            )
                                            .fill(Color32::TRANSPARENT);
                                            if ui.add(cancel).clicked() {
                                                app.lyrics_delete_confirm_for = None;
                                            }
                                        } else {
                                            let del = egui::Button::new(
                                                RichText::new(app.t("Xoá", "Delete"))
                                                    .font(marginal_small())
                                                    .color(Color32::from_rgb(245, 118, 118)),
                                            )
                                            .fill(HOVER)
                                            .corner_radius(CornerRadius::same(12));
                                            if ui.add(del).clicked() {
                                                app.actions.push(Action::ConfirmDeleteTtmlStep(id));
                                            }
                                        }
                                        let exp = egui::Button::new(
                                            RichText::new(app.t("Xuất", "Export"))
                                                .font(marginal_small())
                                                .color(TEXT),
                                        )
                                        .fill(HOVER)
                                        .corner_radius(CornerRadius::same(12));
                                        if ui.add(exp).clicked() {
                                            app.actions.push(Action::ExportLyrics(id));
                                        }
                                    });
                                });
                            }
                        });
                });
        });
}

fn cached_saved_ttml_tracks(ctx: &egui::Context, app: &App) -> Vec<TrackId> {
    let id = egui::Id::NULL.with("ttml-saved-cache");
    let now = ctx.input(|i| i.time);
    if let Some(cache) = ctx.data(|d| d.get_temp::<SavedTtmlCache>(id)) {
        if now - cache.refreshed_at < 0.75 {
            return cache.tracks;
        }
    }
    let tracks: Vec<TrackId> = app
        .lib
        .tracks
        .values()
        .filter(|t| imported_lyrics_file(t).is_file())
        .map(|t| t.id)
        .collect();
    ctx.data_mut(|d| {
        d.insert_temp(
            id,
            SavedTtmlCache {
                refreshed_at: now,
                tracks: tracks.clone(),
            },
        );
    });
    tracks
}

fn invalidate_ttml_cache(ctx: &egui::Context) {
    ctx.data_mut(|d| {
        d.remove::<SavedTtmlCache>(egui::Id::NULL.with("ttml-saved-cache"));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lrc::TimedSpan;

    #[test]
    fn fullscreen_nowbar_matches_reference_and_fits_supported_sizes() {
        let art = cinema_art_rect(Rect::from_min_size(Pos2::ZERO, Vec2::new(1536.0,816.0)));
        assert!((art.left()-184.56).abs()<1.5, "{art:?}");
        assert!((art.top()-116.28).abs()<1.0, "{art:?}");
        assert!((art.width()-466.752).abs()<1.0);
        for size in [Vec2::new(800.0,600.0),Vec2::new(1304.0,724.0),Vec2::new(1536.0,816.0)] {
            let art = cinema_art_rect(Rect::from_min_size(Pos2::ZERO,size));
            assert!(art.right() < size.x*0.46);
            assert!(art.bottom()+115.0 < size.y);
        }
    }

    #[test]
    fn manual_follow_resumes_visible_lines_after_idle_and_forces_recentering() {
        let mut follow = LyricsFollowState { active_visible: true, last_active: Some(3), ..Default::default() };
        assert!(!follow.resume_follow(1.0, 1000, false, true));
        assert!(!follow.resume_follow(1.749, 1000, false, false));
        assert!(follow.resume_follow(1.75, 1000, false, false));
        assert_eq!(follow.last_active, None);
        assert_eq!(follow.manual_scroll_at, None);
    }

    #[test]
    fn manual_follow_keeps_offscreen_lines_paused_until_seek_or_playback_change() {
        let mut follow = LyricsFollowState::default();
        follow.resume_follow(0.0, 1000, false, true);
        assert!(!follow.resume_follow(2.0, 1000, false, false));
        assert!(follow.resume_follow(2.1, 2001, false, false));
        follow.resume_follow(3.0, 2001, false, true);
        assert!(follow.resume_follow(3.1, 2001, true, false));
        assert_eq!(follow.manual_scroll_at, None);
    }

    #[test]
    fn continuing_drag_prevents_idle_follow_resume() {
        let mut follow = LyricsFollowState { active_visible: true, ..Default::default() };
        follow.resume_follow(0.0, 1000, false, true);
        assert!(!follow.resume_follow(1.0, 1000, false, true));
        assert!(!follow.resume_follow(1.5, 1000, false, false));
        assert!(follow.resume_follow(1.75, 1000, false, false));
    }

    #[test]
    fn word_seek_uses_unicode_byte_spans_and_line_fallback() {
        let line = LyricLine {
            time_ms: Some(1000), end_ms: Some(3000), text: "Anh yêu".into(),
            spans: vec![TimedSpan { bytes: 4..8, start_ms: 1400, end_ms: 2600 }],
            right_align: false, background: false,
        };
        assert_eq!(line_seek_time(&line, Some(5)), Some(1400));
        assert_eq!(line_seek_time(&line, Some(3)), Some(1000));
        assert_eq!(line_seek_time(&line, None), Some(1000));
        assert_eq!(line_seek_time(&line, Some(usize::MAX)), Some(1000));
    }

    #[test]
    fn reduced_motion_finishes_an_in_flight_backdrop_crossfade() {
        let mut state = BackdropState {
            track: None, from: [Color32::BLACK; 4], current: [Color32::BLACK; 4],
            target: [Color32::WHITE; 4], prev_tex: None, cur_tex: None, blend: 0.0, phase: 0.0,
        };
        assert!((state.advance_blend(0.25, true) - 0.5).abs() < 0.0001);
        assert_eq!(state.advance_blend(0.0, false), 1.0);
        assert_eq!(state.current, state.target);
    }

    #[test]
    fn tracks_sharing_a_cover_do_not_fade_from_transparent() {
        let ctx = egui::Context::default();
        let cover = ctx.load_texture("shared-cover", egui::ColorImage::new([1, 1], vec![Color32::WHITE]), egui::TextureOptions::LINEAR);
        let mut state = BackdropState {
            track: Some(1), from: [Color32::BLACK; 4], current: [Color32::WHITE; 4],
            target: [Color32::WHITE; 4], prev_tex: None, cur_tex: Some(cover.clone()), blend: 1.0, phase: 0.0,
        };
        state.set_target(Some(2), [Color32::WHITE; 4], Some(cover), true);
        assert_eq!(state.blend, 1.0);
        assert!(state.prev_tex.is_none());
    }

    #[test]
    fn syllable_and_line_timing_sweep_per_character_with_active_glow() {
        let ttml_line = LyricLine {
            time_ms: Some(1000),
            end_ms: Some(3000),
            text: "Anh yêu".into(),
            spans: vec![
                TimedSpan { bytes: 0..3, start_ms: 1000, end_ms: 1400 },
                TimedSpan { bytes: 4..8, start_ms: 1400, end_ms: 2600 },
            ],
            right_align: false,
            background: false,
        };
        let anims = compute_active_char_anims(&ttml_line, &ttml_line.text, 1900, Some(3200), 28.0, true);
        assert_eq!(anims.len(), ttml_line.text.chars().count());
        assert!((anims[0].alpha - 1.0).abs() < 0.01);
        assert!(anims[5].glow > 0.70 && anims[5].blur_px > 10.0, "active center char should glow brightly: {:?}", anims[5]);
        assert!(anims[4].glow > 0.20, "recently sung char in held word keeps warm glow: {:?}", anims[4]);

        let lrc_line = LyricLine {
            time_ms: Some(1000),
            end_ms: None,
            text: "Chẳng phải phép màu, vậy sao chúng ta gặp nhau?".into(),
            spans: vec![],
            right_align: false,
            background: false,
        };
        let mid_anims = compute_active_char_anims(&lrc_line, &lrc_line.text, 2200, Some(5000), 32.0, true);
        assert!(mid_anims[0].alpha > 0.98, "leading chars already sung: {:?}", mid_anims[0]);
        assert!(mid_anims.last().unwrap().alpha < 0.55, "trailing chars still unsung: {:?}", mid_anims.last());
        assert!(mid_anims.iter().any(|a| a.glow > 0.65), "sweep head must emit strong character glow");
    }

    #[test]
    fn interlude_markers_and_cinema_side_mirroring_behave_as_expected() {
        assert!(is_instrumental_line(""));
        assert!(is_instrumental_line("  ♪  "));
        assert!(is_instrumental_line("..."));
        assert!(!is_instrumental_line("Chẳng phải phép màu"));

        let bounds = Rect::from_min_size(Pos2::ZERO, Vec2::new(1304.0, 724.0));
        let left = cinema_art_rect_sided(bounds, false);
        let right = cinema_art_rect_sided(bounds, true);
        assert!((left.width() - right.width()).abs() < 0.01);
        assert!((left.left() - (bounds.right() - right.right())).abs() < 0.01);
    }

    #[test]
    fn analytical_spring_converges_across_critical_under_and_overdamped_regimes() {
        for damping in [1.0, 0.72, 1.85] {
            let mut sp = Spring::new(0.0, 2.5, damping);
            sp.set_goal(1.0, false);
            for _ in 0..150 {
                sp.step(1.0 / 60.0);
            }
            assert!(
                (sp.p - 1.0).abs() < 0.002,
                "spring d={damping} did not converge: p={}, v={}",
                sp.p,
                sp.v
            );
            assert!(sp.can_sleep(), "spring d={damping} should sleep at rest");
        }
    }

    #[test]
    fn popup_dock_spring_auto_hides_after_750ms_and_holds_lift_until_controls_fade() {
        let mut st = PopupSpringState::default();
        // Wake on pointer move inside lyrics page -> goal 0.65, dock_lift rises toward 1.0
        let (co1, dl1, busy1) = st.advance(0.0, true, false, true, true);
        assert!(busy1 && co1 > 0.0 && dl1 > 0.0);
        for i in 1..40 {
            st.advance(i as f64 / 60.0, true, false, false, true);
        }
        let (co_mid, dl_mid, _) = st.advance(0.70, true, false, false, true);
        assert!((co_mid - 0.65).abs() < 0.02, "expected ~0.65 before 750ms, got {co_mid}");
        assert!(dl_mid > 0.98, "dock should be fully lifted at 0.70s: {dl_mid}");

        // At >=750ms without pointer movement outside dock: controls begin fading, dock_lift STILL holds ~1.0!
        let (co_fade1, dl_fade1, _) = st.advance(0.80, true, false, false, true);
        assert!(co_fade1 < 0.60, "controls should start fading after 750ms: {co_fade1}");
        assert!(
            dl_fade1 > 0.98,
            "dock_lift must hold at 1.0 until controlsOpacity <= 0.005: {dl_fade1}"
        );

        // Step until controls fade below 0.005, then dock_lift smoothly drops to 0.0
        let mut t = 0.80;
        for _ in 0..120 {
            t += 1.0 / 60.0;
            st.advance(t, true, false, false, true);
        }
        let (co_end, dl_end, busy_end) = st.advance(t + 0.02, true, false, false, true);
        assert!(co_end < 0.001 && dl_end < 0.002 && !busy_end, "end=({co_end}, {dl_end}, {busy_end})");
    }

    #[test]
    fn popup_dock_metrics_match_reference_390x379_and_responsive_modes() {
        let m = PopupDockMetrics::compute(390.0, 379.0);
        assert!(!m.compact_bar && m.show_times);
        assert!((m.gutter - 19.5).abs() < 0.05);
        assert!((m.reserve - 33.31).abs() < 0.2, "expected ~33.3px reserve, got {}", m.reserve);
        assert!(379.0 - m.idle_h > 280.0, "lyrics container gets >280px height at 390x379");

        let short = PopupDockMetrics::compute(260.0, 190.0);
        assert!(!short.compact_bar && !short.show_times);

        let bar = PopupDockMetrics::compute(340.0, 118.0);
        assert!(bar.compact_bar);
    }

    #[test]
    fn hover_pill_morphs_smoothly_between_lines_and_fades_out_on_exit() {
        let mut st = LyricsHoverPillState::default();
        let line_a = Rect::from_min_max(Pos2::new(20.0, 100.0), Pos2::new(320.0, 140.0));
        let line_b = Rect::from_min_max(Pos2::new(140.0, 200.0), Pos2::new(500.0, 280.0));

        // Fresh entry starts directly on line A (never flies in from 0,0) and begins fading in.
        assert!(st.step(Some(line_a), 1.0 / 60.0, true));
        assert_eq!(st.rect, Some(line_a));
        assert!(st.opacity > 0.1 && st.opacity < 1.0);
        for _ in 0..30 {
            st.step(Some(line_a), 1.0 / 60.0, true);
        }
        assert!((st.opacity - 1.0).abs() < 0.02);

        // Moving to line B (including across left->right duet alignment) morphs all 4 edges smoothly.
        assert!(st.step(Some(line_b), 1.0 / 60.0, true));
        let mid = st.rect.expect("pill stays visible while morphing");
        assert!(mid.min.x > line_a.min.x && mid.min.x < line_b.min.x);
        assert!(mid.min.y > line_a.min.y && mid.min.y < line_b.min.y);
        assert!(mid.max.x > line_a.max.x && mid.max.x < line_b.max.x);
        assert!(mid.max.y > line_a.max.y && mid.max.y < line_b.max.y);
        for _ in 0..45 {
            st.step(Some(line_b), 1.0 / 60.0, true);
        }
        assert_eq!(st.rect, Some(line_b));

        // Leaving all lines fades opacity smoothly back to 0 and clears rect.
        for _ in 0..45 {
            st.step(None, 1.0 / 60.0, true);
        }
        assert_eq!(st.opacity, 0.0);
        assert!(st.rect.is_none());
    }

    #[test]
    fn interlude_dots_deduplicate_intro_mid_and_consecutive_instrumental_lines() {
        let mk = |t: u64, end: Option<u64>, text: &str| LyricLine {
            time_ms: Some(t),
            end_ms: end,
            text: text.to_string(),
            spans: Vec::new(),
            right_align: false,
            background: false,
        };

        // 1. Intro: explicit `♪` before first vocal suppresses synthetic intro dots; absent -> allows synthetic.
        let with_explicit_intro = vec![mk(0, Some(6_000), "♪"), mk(6_000, Some(10_000), "First vocal")];
        assert!(!should_render_synthetic_intro_dots(&with_explicit_intro, 6_000));
        let without_explicit_intro = vec![mk(6_000, Some(10_000), "First vocal")];
        assert!(should_render_synthetic_intro_dots(&without_explicit_intro, 6_000));

        // 2. Mid-song: explicit `...` between two vocals suppresses synthetic mid-song gap dots.
        let with_mid_explicit = vec![
            mk(1_000, Some(5_000), "Verse 1"),
            mk(6_000, Some(12_000), "..."),
            mk(12_500, Some(16_000), "Verse 2"),
        ];
        assert_eq!(synthetic_mid_interlude_gap(&with_mid_explicit, 0), None);
        assert_eq!(explicit_instrumental_span(&with_mid_explicit, 1), Some((6_000, 12_000, 1)));

        // 3. Consecutive explicit instrumental/blank lines collapse to a SINGLE span on index 0.
        let consecutive = vec![
            mk(10_000, Some(14_000), ""),
            mk(14_000, Some(18_000), "♪"),
            mk(20_000, Some(24_000), "Chorus"),
        ];
        assert_eq!(explicit_instrumental_span(&consecutive, 0), Some((10_000, 18_000, 1)));
        assert_eq!(explicit_instrumental_span(&consecutive, 1), None);

        // 4. Short (< 2.2s) blank LRC spacer does NOT spawn spurious `• • •`.
        let short_spacer = vec![
            mk(10_000, Some(11_000), ""),
            mk(11_200, Some(15_000), "Next vocal"),
        ];
        assert_eq!(explicit_instrumental_span(&short_spacer, 0), None);

        // 5. Pure vocal gap >= 3.5s with NO explicit instrumental line still gets synthetic dots.
        let pure_gap = vec![
            mk(1_000, Some(5_000), "Verse 1"),
            mk(10_000, Some(14_000), "Verse 2"),
        ];
        assert_eq!(synthetic_mid_interlude_gap(&pure_gap, 0), Some((5_000, 10_000)));
    }
}
