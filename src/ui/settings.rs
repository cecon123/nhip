//! Spotify-style `/preferences` settings page.

use eframe::egui::{
    self, Align, Align2, Color32, Id, Layout, Pos2, Rect, RichText, ScrollArea, Sense, Slider,
    Stroke, TextEdit, Ui, UiBuilder, Vec2,
};

use super::icons::{self, Icon};
use super::widgets::*;
use crate::app::{Action, App};
use crate::library::{data_dir, fold, Lang, NormLevel};
use crate::mechanics::{CROSSFADE_MAX_S, EQ_BANDS, EQ_PRESETS, EQ_RANGE_DB};
use crate::theme::*;

const SEARCH_ID: &str = "settings_search_query";

fn matches_query(query: &str, section: &str, title: &str, desc: &str) -> bool {
    let q = fold(query.trim());
    if q.is_empty() {
        return true;
    }
    fold(section).contains(&q) || fold(title).contains(&q) || fold(desc).contains(&q)
}

fn section_header(ui: &mut Ui, title: &str) {
    ui.add_space(24.0);
    ui.label(RichText::new(title).font(title_small()).color(TEXT));
    ui.add_space(8.0);
}

/// Spotify `2fr 1fr` preference row: title + description subtext on the left, right-aligned control on the right.
fn pref_row(ui: &mut Ui, col_w: f32, title: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let gap = ui.spacing().item_spacing.x;
    let left_w = ((col_w - gap) * 0.64).max(160.0);
    let right_w = (col_w - left_w - gap).max(80.0);
    ui.horizontal(|ui| {
        ui.set_min_width(col_w);
        ui.set_min_height(42.0);
        ui.allocate_ui_with_layout(
            Vec2::new(left_w, 0.0),
            Layout::top_down(Align::Min),
            |ui| {
                ui.set_min_width(left_w);
                ui.set_max_width(left_w);
                ui.add(egui::Label::new(RichText::new(title).font(body_small()).color(TEXT)).wrap());
                if !desc.is_empty() {
                    ui.add(egui::Label::new(RichText::new(desc).font(body_small()).color(SUBTLE)).wrap());
                }
            },
        );
        ui.allocate_ui_with_layout(
            Vec2::new(right_w, 42.0),
            Layout::right_to_left(Align::Center),
            |ui| {
                ui.set_min_width(right_w);
                control(ui);
            },
        );
    });
    ui.add_space(8.0);
}

/// Styled Spotify select dropdown (`32px`, `#242424` fill, `4px` radius, `14px` `#b3b3b3` -> `#ffffff`).
fn spotify_select<T: PartialEq + Copy + 'static>(
    ui: &mut Ui,
    id: &str,
    selected_label: &str,
    width: f32,
    items: &[(T, &str)],
    current: &mut T,
) -> bool {
    let enabled = ui.is_enabled();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 32.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        ELEVATED.linear_multiply(0.5)
    } else if resp.hovered() {
        HOVER
    } else {
        ELEVATED
    };
    let stroke_color = if resp.hovered() && enabled { Color32::from_white_alpha(64) } else { DECORATIVE_SUBDUED };
    ui.painter().rect_filled(rect, RADIUS_BASE, fill);
    ui.painter().rect_stroke(rect, RADIUS_BASE, Stroke::new(1.0, stroke_color), egui::StrokeKind::Inside);

    let fg = if !enabled {
        MUTED
    } else if resp.hovered() {
        TEXT
    } else {
        SUBTLE
    };
    text(
        ui.painter(),
        Pos2::new(rect.left() + 12.0, rect.center().y),
        selected_label,
        body_small(),
        fg,
        width - 36.0,
        Align2::LEFT_CENTER,
    );
    let chev = Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y), Vec2::splat(14.0));
    icons::paint(ui.painter(), chev, Icon::ChevronDown, fg);

    let mut changed = false;
    if enabled {
        let popup_id = ui.make_persistent_id(id);
        if resp.clicked() {
            egui::Popup::toggle_id(ui.ctx(), popup_id);
        }
        egui::Popup::from_toggle_button_response(&resp)
            .id(popup_id)
            .frame(popup_frame(ui.style()))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width((width - 16.0).max(150.0));
                for &(val, label) in items {
                    let active = *current == val;
                    let rw = ui.available_width();
                    let (ir, iresp) = ui.allocate_exact_size(Vec2::new(rw, 32.0), Sense::click());
                    if iresp.hovered() {
                        ui.painter().rect_filled(ir, RADIUS_SMALLER, Color32::from_white_alpha(25));
                    }
                    text(
                        ui.painter(),
                        Pos2::new(ir.left() + 10.0, ir.center().y),
                        label,
                        body_small(),
                        if active { ACCENT } else { TEXT },
                        rw - 34.0,
                        Align2::LEFT_CENTER,
                    );
                    if active {
                        let cr = Rect::from_center_size(Pos2::new(ir.right() - 14.0, ir.center().y), Vec2::splat(14.0));
                        icons::paint(ui.painter(), cr, Icon::Checkmark, ACCENT);
                    }
                    if iresp.clicked() {
                        if *current != val {
                            *current = val;
                            changed = true;
                        }
                        ui.close();
                    }
                }
            });
    }
    changed
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(28.0);
        let avail = ui.available_width();
        let col_w = (avail - 64.0).clamp(320.0, 900.0);
        let left_pad = ((avail - col_w) * 0.5).max(16.0);
        ui.horizontal(|ui| {
            ui.add_space(left_pad);
            ui.vertical(|ui| {
                ui.set_width(col_w);
                content(app, ui, col_w);
                ui.add_space(48.0);
            });
        });
    });
}

fn content(app: &mut App, ui: &mut Ui, col_w: f32) {
    let mut query = ui.ctx().data(|d| d.get_temp::<String>(Id::unique(SEARCH_ID)).unwrap_or_default());
    let current_lang = app.lang();
    let t = move |vi: &'static str, en: &'static str| match current_lang {
        Lang::Vi => vi,
        Lang::En => en,
    };

    // Header: "Settings" on left, live "Search in Settings" input on right.
    ui.horizontal(|ui| {
        ui.label(RichText::new(t("Cài đặt", "Settings")).font(title_large()).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let search_w = 240.0f32.min((col_w * 0.45).max(160.0));
            let (srect, _) = ui.allocate_exact_size(Vec2::new(search_w, 36.0), Sense::hover());
            ui.painter().rect_filled(srect, RADIUS_BASE, ELEVATED);
            let s_icon = Rect::from_center_size(Pos2::new(srect.left() + 18.0, srect.center().y), Vec2::splat(16.0));
            icons::paint(ui.painter(), s_icon, Icon::Search, SUBTLE);

            let right_pad = if query.is_empty() { 10.0 } else { 30.0 };
            let edit_rect = Rect::from_min_max(
                Pos2::new(srect.left() + 34.0, srect.top() + 7.0),
                Pos2::new(srect.right() - right_pad, srect.bottom() - 7.0),
            );
            let mut edit_ui = ui.new_child(UiBuilder::new().id_salt("settings-search-edit").max_rect(edit_rect));
            let resp = edit_ui.add(
                TextEdit::singleline(&mut query)
                    .frame(egui::Frame::NONE)
                    .font(body_small())
                    .hint_text(RichText::new(t("Tìm kiếm trong Cài đặt", "Search in Settings")).color(SUBTLE).font(body_small()))
                    .desired_width(edit_rect.width()),
            );
            let stroke = if resp.has_focus() {
                Stroke::new(1.5, TEXT)
            } else {
                Stroke::new(1.0, DECORATIVE_SUBDUED)
            };
            ui.painter().rect_stroke(srect, RADIUS_BASE, stroke, egui::StrokeKind::Inside);

            if !query.is_empty() {
                let clr = Rect::from_center_size(Pos2::new(srect.right() - 16.0, srect.center().y), Vec2::splat(20.0));
                if icon_button_sized(ui, clr, "settings-search-clear", Icon::Close, SUBTLE, t("Xóa tìm kiếm", "Clear search"), 14.0).clicked() {
                    query.clear();
                }
            }
        });
    });
    ui.ctx().data_mut(|d| d.insert_temp(Id::unique(SEARCH_ID), query.clone()));

    let mut changed = false;
    let mut matched_any = false;

    // 1. Language
    {
        let sec = t("Ngôn ngữ", "Language");
        let row_title = t("Chọn ngôn ngữ của ứng dụng", "Choose the app's language");
        let row_desc = t("Áp dụng ngay cho toàn bộ giao diện Nhịp.", "Applies immediately across Nhịp.");
        if matches_query(&query, sec, row_title, row_desc) {
            matched_any = true;
            section_header(ui, sec);
            let s = &mut app.data.settings;
            pref_row(ui, col_w, row_title, row_desc, |ui| {
                let cur_label = if s.lang == Lang::Vi { "Tiếng Việt" } else { "English" };
                changed |= spotify_select(
                    ui,
                    "settings-lang",
                    cur_label,
                    170.0,
                    &[(Lang::Vi, "Tiếng Việt"), (Lang::En, "English")],
                    &mut s.lang,
                );
            });
        }
    }

    // 2. Audio Quality & Normalization
    {
        let sec = t("Chất lượng âm thanh", "Audio quality");
        let q_title = t("Chất lượng phát trực tuyến", "Streaming quality");
        let q_desc = t("Chất lượng tự động theo nguồn nhạc có sẵn.", "Automatically uses the available source quality.");
        let n_title = t("Cân bằng âm lượng", "Normalize volume");
        let n_desc = t("Giữ độ to giữa các bài hát gần như nhau.", "Set the same volume level for all tracks.");
        let l_title = t("Mức âm lượng", "Volume level");
        let l_desc = t("Điều chỉnh độ lớn cho môi trường nghe của bạn.", "Adjust loudness target for your environment.");

        let show_q = matches_query(&query, sec, q_title, q_desc);
        let show_n = matches_query(&query, sec, n_title, n_desc);
        let show_l = matches_query(&query, sec, l_title, l_desc);
        if show_q || show_n || show_l {
            matched_any = true;
            section_header(ui, sec);
            if show_q {
                pref_row(ui, col_w, q_title, q_desc, |ui| {
                    ui.label(RichText::new(t("Tự động", "Automatic")).font(body_small()).color(SUBTLE));
                });
            }
            let s = &mut app.data.settings;
            if show_n {
                pref_row(ui, col_w, n_title, n_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.normalize);
                });
            }
            if show_l {
                let name = |l: NormLevel| match l {
                    NormLevel::Quiet => t("Nhỏ", "Quiet"),
                    NormLevel::Normal => t("Bình thường", "Normal"),
                    NormLevel::Loud => t("Lớn", "Loud"),
                };
                let enabled = s.normalize;
                pref_row(ui, col_w, l_title, l_desc, |ui| {
                    ui.add_enabled_ui(enabled, |ui| {
                        changed |= spotify_select(
                            ui,
                            "settings-norm-level",
                            name(s.level),
                            170.0,
                            &[
                                (NormLevel::Quiet, name(NormLevel::Quiet)),
                                (NormLevel::Normal, name(NormLevel::Normal)),
                                (NormLevel::Loud, name(NormLevel::Loud)),
                            ],
                            &mut s.level,
                        );
                    });
                });
            }
        }
    }

    // The installed settings bundle renders Crossfade and Automix as separate switches.
    {
        let sec = t("Phát nhạc", "Playback");
        let ap_title = t("Tự động phát", "Autoplay");
        let ap_desc = t("Nghe nhạc không gián đoạn với các bài tương tự khi danh sách kết thúc.", "Enjoy nonstop listening. When your audio ends, we'll play something similar.");
        let mx_title = "Automix";
        let mx_desc = t("Căn nhịp kiểu DJ, giữ cao độ và chuyển tiếp mượt giữa các bài phù hợp.", "Beat-match compatible songs like a DJ, preserving pitch through each transition.");
        let cf_title = t("Chuyển tiếp bài hát", "Crossfade songs");
        let cf_desc = t("Số giây hòa âm chuyển tiếp giữa hai bài hát (0–12 giây).", "Overlap duration between consecutive tracks (0–12 seconds).");
        let gp_title = t("Phát liền mạch", "Gapless playback");
        let gp_desc = t("Không có khoảng lặng giữa hai bài liên tiếp.", "Allow gapless playback with no silence between songs.");

        let show_ap = matches_query(&query, sec, ap_title, ap_desc);
        let show_mx = matches_query(&query, sec, mx_title, mx_desc);
        let show_cf = matches_query(&query, sec, cf_title, cf_desc);
        let show_gp = matches_query(&query, sec, gp_title, gp_desc);

        if show_ap || show_mx || show_cf || show_gp {
            matched_any = true;
            section_header(ui, sec);
            let s = &mut app.data.settings;
            if show_ap {
                pref_row(ui, col_w, ap_title, ap_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.autoplay);
                });
            }
            if show_mx {
                pref_row(ui, col_w, mx_title, mx_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.auto_mix);
                });
            }
            if show_cf {
                pref_row(ui, col_w, cf_title, cf_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.crossfade);
                });
                if s.crossfade {
                    pref_row(ui, col_w, t("Thời lượng chuyển tiếp", "Crossfade duration"), "", |ui| {
                    changed |= ui
                        .add(Slider::new(&mut s.crossfade_seconds, 0..=CROSSFADE_MAX_S).suffix(" s").step_by(1.0))
                        .changed();
                    });
                }
            }
            if show_gp {
                pref_row(ui, col_w, gp_title, gp_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.gapless);
                });
            }
        }
    }

    // 4. Equalizer (curve + presets + reset + 6 band sliders)
    {
        let sec = t("Bộ chỉnh âm", "Equalizer");
        let eq_title = t("Bật bộ chỉnh âm", "Equalizer");
        let eq_desc = t("Điều chỉnh đáp tuyến tần số 6 băng tần theo gu nghe của bạn.", "Fine-tune the 6-band frequency curve or pick a preset.");
        let pre_title = t("Thiết lập sẵn & Đặt lại", "Presets & Reset");
        let pre_desc = t("Chọn cấu hình mẫu hoặc đưa tất cả băng tần về 0 dB.", "Choose a preset curve or reset all bands back to flat.");

        if matches_query(&query, sec, eq_title, eq_desc) || matches_query(&query, sec, pre_title, pre_desc) {
            matched_any = true;
            section_header(ui, sec);
            let lang = current_lang;
            let s = &mut app.data.settings;
            pref_row(ui, col_w, eq_title, eq_desc, |ui| {
                changed |= toggle_switch(ui, &mut s.eq_enabled);
            });
            if s.eq_enabled {
                let preset_name = |i: usize| match lang {
                    Lang::Vi => EQ_PRESETS[i].vi,
                    Lang::En => EQ_PRESETS[i].en,
                };
                pref_row(ui, col_w, pre_title, pre_desc, |ui| {
                    if outline_button(ui, t("Đặt lại", "Reset")).clicked() {
                        s.eq_preset = 0;
                        s.eq_gains = [0.0; 6];
                        changed = true;
                    }
                    ui.add_space(8.0);
                    let mut selected = s.eq_preset.min(EQ_PRESETS.len() - 1);
                    let items: Vec<(usize, &str)> = (0..EQ_PRESETS.len()).map(|i| (i, preset_name(i))).collect();
                    if spotify_select(ui, "settings-eq-preset", preset_name(selected), 170.0, &items, &mut selected) {
                        s.eq_preset = selected;
                        s.eq_gains = EQ_PRESETS[selected].gains;
                        changed = true;
                    }
                });

                changed |= draw_equalizer_curve(ui, col_w, &mut s.eq_gains);
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().slider_width = 110.0;
                    let col_band_w = ((col_w - 40.0) / 6.0).clamp(54.0, 110.0);
                    for (i, band) in EQ_BANDS.iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.set_width(col_band_w);
                            if ui
                                .add(Slider::new(&mut s.eq_gains[i], -EQ_RANGE_DB..=EQ_RANGE_DB).vertical().step_by(0.1).show_value(false))
                                .changed()
                            {
                                changed = true;
                            }
                            let (r, _) = ui.allocate_exact_size(Vec2::new(col_band_w, 34.0), Sense::hover());
                            let p = ui.painter();
                            text(p, r.center_top(), &format!("{:+.1} dB", s.eq_gains[i]), marginal(), TEXT, col_band_w, Align2::CENTER_TOP);
                            text(p, r.center_top() + Vec2::new(0.0, 16.0), band.label, marginal(), SUBTLE, col_band_w, Align2::CENTER_TOP);
                        });
                    }
                });
            }
        }
    }

    // 5. Display & Lyrics
    {
        let sec = t("Hiển thị & Lời bài hát", "Display & Lyrics");
        let ct_title = t("Danh sách bài hát thu gọn", "Use compact tracklist layout");
        let ct_desc = t("Hiển thị nhiều bài hát hơn trên mỗi trang với dòng mỏng hơn.", "Fit more tracks on screen with denser rows.");
        let lm_title = t("Hiệu ứng lời bài hát", "Lyrics motion");
        let lm_desc = t("Dùng nền động và cuộn lời mượt.", "Animate the background and lyric scrolling.");
        let rm_title = t("Giảm chuyển động", "Prefer reduced motion");
        let rm_desc = t("Tắt nền động và hạn chế hiệu ứng chuyển cảnh.", "Disable animated backdrops and smooth motion effects.");
        let rt_title = t("Hiển thị thời gian còn lại", "Show remaining track time");
        let rt_desc = t("Hiển thị bộ đếm ngược âm ở góc phải thanh phát nhạc.", "Show negative countdown on the player scrubber.");

        let show_ct = matches_query(&query, sec, ct_title, ct_desc);
        let show_lm = matches_query(&query, sec, lm_title, lm_desc);
        let show_rm = matches_query(&query, sec, rm_title, rm_desc);
        let show_rt = matches_query(&query, sec, rt_title, rt_desc);
        if show_ct || show_lm || show_rm || show_rt {
            matched_any = true;
            section_header(ui, sec);
            let s = &mut app.data.settings;
            if show_ct {
                pref_row(ui, col_w, ct_title, ct_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.compact_tracklist);
                });
            }
            if show_lm {
                pref_row(ui, col_w, lm_title, lm_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.lyrics_motion);
                });
            }
            if show_rm {
                pref_row(ui, col_w, rm_title, rm_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.prefers_reduced_motion);
                });
            }
            if show_rt {
                pref_row(ui, col_w, rt_title, rt_desc, |ui| {
                    changed |= toggle_switch(ui, &mut s.show_remaining_time);
                });
            }
        }
    }

    {
        let sec = t("Gu âm nhạc", "Taste profile");
        let title = t("Bài hát đã ẩn", "Hidden songs");
        let desc = t("Không đưa các bài đã ẩn vào đề xuất và tự động phát tiếp.", "Exclude hidden songs from recommendations and autoplay.");
        if matches_query(&query, sec, title, desc) {
            matched_any = true;
            section_header(ui, sec);
            pref_row(ui, col_w, title, desc, |ui| {
                let label = format!("{} ({})", t("Khôi phục", "Restore"), app.data.hidden_tracks.len());
                if ui.add_enabled_ui(!app.data.hidden_tracks.is_empty(), |ui| outline_button(ui, &label)).inner.clicked() {
                    app.actions.push(Action::RestoreHiddenTracks);
                }
            });
        }
    }

    // 6. Social
    {
        let sec = t("Xã hội", "Social");
        let ps_title = t("Phiên riêng tư", "Private session");
        let ps_desc = t("Bắt đầu một phiên riêng tư để nghe ẩn danh.", "Start a private session to listen anonymously.");
        if matches_query(&query, sec, ps_title, ps_desc) {
            matched_any = true;
            section_header(ui, sec);
            let s = &mut app.data.settings;
            pref_row(ui, col_w, ps_title, ps_desc, |ui| {
                changed |= toggle_switch(ui, &mut s.private_session);
            });
        }
    }

    // 7. Storage (Portable Storage location)
    {
        let sec = t("Lưu trữ", "Storage");
        let st_title = t("Vị trí lưu trữ dữ liệu (Portable / AppData)", "Offline & Portable storage location");
        let path_str = data_dir().display().to_string();
        let is_portable = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("nhip.portable").is_file()))
            .unwrap_or(false);
        let mode_note = if is_portable {
            t("Đang chạy chế độ di động (nhip.portable)", "Portable mode active (nhip.portable)")
        } else {
            t("Thư mục dữ liệu chuẩn của người dùng", "Standard user data directory")
        };
        let st_desc = format!("{mode_note}: {path_str}");
        if matches_query(&query, sec, st_title, &st_desc) {
            matched_any = true;
            section_header(ui, sec);
            pref_row(ui, col_w, st_title, &st_desc, |ui| {
                if outline_button(ui, t("Mở thư mục", "Open folder")).clicked() {
                    open_folder_in_os(data_dir());
                }
            });
        }
    }

    // 8. Shortcuts & About Nhịp
    {
        let sec = t("Phím tắt & Giới thiệu", "Shortcuts & About");
        let ks_title = t("Phím tắt", "Keyboard shortcuts");
        let ks_desc = t("Xem toàn bộ phím tắt điều khiển phát nhạc, điều hướng và bố cục.", "View all playback, navigation, and layout shortcuts.");
        let ab_title = t("Giới thiệu Nhịp", "About Nhịp");
        let ab_desc = format!(
            "Nhịp v{} — {}",
            env!("CARGO_PKG_VERSION"),
            t("Trình phát nhạc desktop viết bằng Rust & egui", "Desktop music player written in Rust & egui")
        );
        let show_ks = matches_query(&query, sec, ks_title, ks_desc);
        let show_ab = matches_query(&query, sec, ab_title, &ab_desc);
        if show_ks || show_ab {
            matched_any = true;
            section_header(ui, sec);
            if show_ks {
                pref_row(ui, col_w, ks_title, ks_desc, |ui| {
                    if outline_button(ui, t("Xem phím tắt (Ctrl + /)", "Show shortcuts (Ctrl + /)")).clicked() {
                        app.help_open = true;
                    }
                });
            }
            if show_ab {
                pref_row(ui, col_w, ab_title, &ab_desc, |ui| {
                    if outline_button(ui, t("Chi tiết", "Details")).clicked() {
                        super::overlays::open_about(ui.ctx());
                    }
                });
            }
        }
    }

    if !matched_any && !query.trim().is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new(t("Không tìm thấy cài đặt phù hợp", "No matching settings found")).font(title_small()).color(TEXT));
            ui.add_space(8.0);
            if outline_button(ui, t("Xóa bộ lọc tìm kiếm", "Clear search filter")).clicked() {
                ui.ctx().data_mut(|d| d.insert_temp(Id::unique(SEARCH_ID), String::new()));
            }
        });
    }

    if changed {
        app.actions.push(Action::SettingsChanged);
    }
}

/// Interactive 6-band EQ frequency response curve painted in Spotify dark card style.
fn draw_equalizer_curve(ui: &mut Ui, col_w: f32, gains: &mut [f32; 6]) -> bool {
    let mut changed = false;
    let w = (col_w - 8.0).max(240.0);
    let h = 132.0;
    let (card_rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(card_rect, RADIUS_CARD, ELEVATED);
    p.rect_stroke(card_rect, RADIUS_CARD, Stroke::new(1.0, HAIRLINE), egui::StrokeKind::Inside);

    let plot = card_rect.shrink2(Vec2::new(36.0, 20.0));
    let db_to_y = |db: f32| -> f32 {
        let norm = (EQ_RANGE_DB - db) / (EQ_RANGE_DB * 2.0);
        plot.top() + norm.clamp(0.0, 1.0) * plot.height()
    };

    // Grid lines (+12, 0, -12 dB)
    for (db, label, alpha) in [(EQ_RANGE_DB, "+12", 24), (0.0, "0 dB", 56), (-EQ_RANGE_DB, "-12", 24)] {
        let y = db_to_y(db);
        p.hline(plot.x_range(), y, Stroke::new(1.0, Color32::from_white_alpha(alpha)));
        text(p, Pos2::new(card_rect.left() + 8.0, y), label, marginal_small(), MUTED, 26.0, Align2::LEFT_CENTER);
    }

    let pts: [Pos2; 6] = std::array::from_fn(|i| {
        let t = i as f32 / 5.0;
        Pos2::new(plot.left() + t * plot.width(), db_to_y(gains[i]))
    });

    // Sample smooth Catmull-Rom curve
    let mut curve = Vec::with_capacity(64);
    for seg in 0..5usize {
        let p0 = pts[seg.saturating_sub(1)];
        let p1 = pts[seg];
        let p2 = pts[seg + 1];
        let p3 = pts[(seg + 2).min(5)];
        for step in 0..=10 {
            if seg > 0 && step == 0 {
                continue;
            }
            let s = step as f32 / 10.0;
            let s2 = s * s;
            let s3 = s2 * s;
            let x = 0.5
                * ((2.0 * p1.x)
                    + (-p0.x + p2.x) * s
                    + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * s2
                    + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * s3);
            let y = 0.5
                * ((2.0 * p1.y)
                    + (-p0.y + p2.y) * s
                    + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * s2
                    + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * s3);
            curve.push(Pos2::new(x, y.clamp(plot.top(), plot.bottom())));
        }
    }

    let zero_y = db_to_y(0.0);
    let fill_color = Color32::from_rgba_unmultiplied(0x1e, 0xd7, 0x60, 34);
    if curve.len() >= 2 {
        let mut mesh = egui::epaint::Mesh::default();
        let uv = egui::epaint::WHITE_UV;
        for win in curve.windows(2) {
            let i = mesh.vertices.len() as u32;
            mesh.vertices.push(egui::epaint::Vertex { pos: win[0], uv, color: fill_color });
            mesh.vertices.push(egui::epaint::Vertex { pos: win[1], uv, color: fill_color });
            mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(win[1].x, zero_y), uv, color: fill_color });
            mesh.vertices.push(egui::epaint::Vertex { pos: Pos2::new(win[0].x, zero_y), uv, color: fill_color });
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        p.add(egui::Shape::mesh(mesh));
        p.add(egui::Shape::line(curve, Stroke::new(2.5, ACCENT)));
    }

    for (i, &pt) in pts.iter().enumerate() {
        p.circle_filled(pt, 5.0, TEXT);
        p.circle_stroke(pt, 5.0, Stroke::new(1.5, ACCENT));
        text(p, Pos2::new(pt.x, card_rect.bottom() - 8.0), EQ_BANDS[i].label, marginal_small(), SUBTLE, 52.0, Align2::CENTER_BOTTOM);
    }

    if (resp.dragged() || resp.clicked()) && resp.interact_pointer_pos().is_some() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let nearest = pts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (a.x - pos.x).abs().total_cmp(&(b.x - pos.x).abs()))
                .map(|(i, _)| i)
                .unwrap_or(0);
            let norm = 1.0 - ((pos.y - plot.top()) / plot.height().max(1.0)).clamp(0.0, 1.0);
            let db = ((norm * 2.0 - 1.0) * EQ_RANGE_DB * 10.0).round() / 10.0;
            if (gains[nearest] - db).abs() > 0.05 {
                gains[nearest] = db.clamp(-EQ_RANGE_DB, EQ_RANGE_DB);
                changed = true;
            }
        }
    }
    changed
}

fn open_folder_in_os(path: &std::path::Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(path).spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(path).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pref_row_flushes_control_to_column_right_edge_for_short_labels() {
        let ctx = egui::Context::default();
        apply_style(&ctx);
        for col_w in [400.0_f32, 666.0, 900.0] {
            let mut row_left = 0.0_f32;
            let mut ctrl_right = 0.0_f32;
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 400.0))),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.vertical(|ui| {
                            ui.set_width(col_w);
                            row_left = ui.cursor().left();
                            pref_row(ui, col_w, "Ngôn ngữ", "Ngắn", |ui| {
                                let (r, _) = ui.allocate_exact_size(Vec2::new(42.0, 24.0), Sense::click());
                                ctrl_right = r.right();
                            });
                        });
                    });
                },
            );
            out.textures_delta.clear();
            let expected_right = row_left + col_w;
            assert!(
                (ctrl_right - expected_right).abs() <= 1.5,
                "col_w={col_w}: expected control right near {expected_right}, got {ctrl_right}"
            );
        }
    }
}
