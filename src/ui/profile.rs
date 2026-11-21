//! Local profile and taste, drawn from saved likes and distinct recent songs.

use std::collections::{BTreeMap, HashSet};

use eframe::egui::{self, Align2, Pos2, Rect, RichText, ScrollArea, Sense, Ui, UiBuilder, Vec2};

use super::pages::{self, CardData};
use super::widgets::*;
use crate::app::{Action, App, Route};
use crate::library::{split_artists, Library, RecentItem, UserData};
use crate::theme::*;

#[derive(Debug, PartialEq, Eq)]
struct ArtistTaste {
    name: String,
    liked: usize,
    recent: usize,
}

fn taste_artists(lib: &Library, data: &UserData) -> Vec<ArtistTaste> {
    let mut counts = BTreeMap::<String, (usize, usize)>::new();
    let mut seen = HashSet::new();
    for (recent, id) in data
        .liked
        .iter()
        .map(|entry| (false, entry.id))
        .chain(data.listening_history.iter().take(50).map(|&id| (true, id)))
    {
        if data.hidden_tracks.contains(&id) || !seen.insert((recent, id)) {
            continue;
        }
        let Some(track) = lib.tracks.get(&id) else {
            continue;
        };
        let mut artists = split_artists(&track.artist);
        artists.sort();
        artists.dedup();
        for name in artists.into_iter().filter(|name| !name.trim().is_empty()) {
            let count = counts.entry(name).or_default();
            if recent {
                count.1 += 1;
            } else {
                count.0 += 1;
            }
        }
    }
    let mut artists: Vec<_> = counts
        .into_iter()
        .map(|(name, (liked, recent))| ArtistTaste {
            name,
            liked,
            recent,
        })
        .collect();
    artists.sort_by(|a, b| {
        b.liked
            .cmp(&a.liked)
            .then_with(|| b.recent.cmp(&a.recent))
            .then_with(|| a.name.cmp(&b.name))
    });
    artists
}

pub fn draw(app: &mut App, ui: &mut Ui) {
    let taste = app.route == Route::TasteProfile;
    let artists = taste_artists(&app.lib, &app.data);
    let artist_cards: Vec<_> = artists
        .iter()
        .filter_map(|artist| {
            let item = pages::item_of(app, &RecentItem::Artist(artist.name.clone()))?;
            let mut card = CardData::from_local(&item);
            card.sub = format!(
                "{} {} • {} {}",
                artist.liked,
                app.t("bài đã thích", "liked songs"),
                artist.recent,
                app.t("bài gần đây", "recent songs")
            );
            Some(card)
        })
        .collect();

    let collections: Vec<_> = std::iter::once(RecentItem::Liked)
        .chain(
            app.data
                .playlists
                .iter()
                .map(|list| RecentItem::Playlist(list.id)),
        )
        .chain(
            app.data
                .followed_ytm
                .iter()
                .map(|list| RecentItem::FollowedYouTube(list.id.clone())),
        )
        .filter_map(|kind| pages::item_of(app, &kind))
        .map(|item| CardData::from_local(&item))
        .collect();

    let album_keys: Vec<_> = if taste {
        artists
            .iter()
            .filter_map(|artist| app.lib.artist(&artist.name))
            .flat_map(|artist| artist.albums.iter().cloned())
            .collect()
    } else {
        app.data
            .listening_history
            .iter()
            .take(50)
            .filter(|id| !app.data.hidden_tracks.contains(id))
            .filter_map(|id| app.lib.tracks.get(id))
            .map(|track| track.album_key())
            .collect()
    };
    let mut seen_albums = HashSet::new();
    let albums: Vec<_> = album_keys
        .into_iter()
        .filter(|key| seen_albums.insert(key.clone()))
        .filter(|key| {
            app.lib.album(key).is_some_and(|album| {
                album
                    .tracks
                    .iter()
                    .any(|id| !app.data.hidden_tracks.contains(id))
            })
        })
        .filter_map(|key| pages::item_of(app, &RecentItem::Album(key)))
        .map(|item| CardData::from_local(&item))
        .collect();

    ScrollArea::vertical()
        .id_salt(("local-profile", taste))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let w = ui.available_width();
            let side = if w < 640.0 { 16.0 } else { 24.0 };
            let hero_h = pages::hero_height_for_width(w);
            let (hero, _) = ui.allocate_exact_size(Vec2::new(w, hero_h), Sense::hover());
            let tint = if taste {
                pages::adaptive_hero_color(app, "liked", true)
            } else {
                ELEVATED.lerp_to_gamma(ACCENT, 0.18)
            };
            pages::draw_full_bleed_gradient(
                ui.painter(),
                Rect::from_min_size(hero.min, Vec2::new(w, hero_h + 100.0)),
                tint,
                PANEL,
            );
            let avatar_side = hero_h - 56.0;
            let avatar = Rect::from_min_size(
                Pos2::new(hero.left() + side, hero.bottom() - avatar_side - 24.0),
                Vec2::splat(avatar_side),
            );
            cover_shadow(ui.painter(), avatar, avatar_side * 0.5);
            ui.painter()
                .circle_filled(avatar.center(), avatar_side * 0.5, ELEVATED);
            text(
                ui.painter(),
                avatar.center(),
                "N",
                bold(avatar_side * 0.5),
                TEXT,
                avatar_side,
                Align2::CENTER_CENTER,
            );
            let details = Rect::from_min_max(
                Pos2::new(avatar.right() + 24.0, avatar.top()),
                Pos2::new(hero.right() - side, avatar.bottom()),
            );
            {
                let mut details_ui = ui.new_child(UiBuilder::new().max_rect(details));
                let ui = &mut details_ui;
                ui.label(
                    RichText::new(app.t("Hồ sơ cục bộ", "Local profile"))
                        .font(body_small())
                        .color(TEXT),
                );
                ui.label(
                    RichText::new(if taste {
                        app.t("Gu âm nhạc", "Music taste")
                    } else {
                        "Nhịp"
                    })
                    .font(bold(if taste { 32.0 } else { 64.0 }))
                    .color(TEXT),
                );
                let liked = app
                    .data
                    .liked
                    .iter()
                    .filter(|entry| {
                        app.lib.tracks.contains_key(&entry.id)
                            && !app.data.hidden_tracks.contains(&entry.id)
                    })
                    .count();
                ui.label(
                    RichText::new(format!(
                        "{liked} {} • {} {}",
                        app.t("bài đã thích", "liked songs"),
                        app.data.playlists.len() + app.data.followed_ytm.len(),
                        app.t("danh sách phát", "playlists")
                    ))
                    .font(body_small())
                    .color(SUBTLE),
                );
            }

            let (tabs, _) = ui.allocate_exact_size(Vec2::new(w, 56.0), Sense::hover());
            let mut x = tabs.left() + side;
            for (label, route) in [
                (app.t("Hồ sơ", "Profile"), Route::Profile),
                (app.t("Gu âm nhạc", "Music taste"), Route::TasteProfile),
            ] {
                let width = pill_width(ui, label);
                let rect =
                    Rect::from_min_size(Pos2::new(x, tabs.top() + 12.0), Vec2::new(width, 32.0));
                if pill(ui, rect, ("profile-tab", label), label, app.route == route).clicked() {
                    app.actions.push(Action::Navigate(route));
                }
                x += width + 8.0;
            }

            if taste || artists.is_empty() {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(side as i8, 12))
                    .show(ui, |ui| {
                        let message = if artists.is_empty() {
                            app.t(
                                "Thích bài hát hoặc nghe nhạc để khám phá gu âm nhạc của bạn.",
                                "Like songs or listen to music to start building your music taste.",
                            )
                        } else {
                            app.t(
                                "Dựa trên những bài bạn đã thích và nghe gần đây.",
                                "Based on songs you've liked and listened to recently.",
                            )
                        };
                        ui.label(RichText::new(message).font(body_small()).color(SUBTLE));
                        if artists.is_empty()
                            && outline_button(ui, app.t("Khám phá nhạc", "Browse music")).clicked()
                        {
                            app.actions.push(Action::Navigate(Route::Search));
                        }
                    });
            }
            if !taste {
                let title = app.t("Bộ sưu tập của bạn", "Your collection");
                pages::draw_carousel_shelf(
                    app,
                    ui,
                    "profile-collections",
                    title,
                    &collections,
                    side,
                );
            }
            let artist_title = app.t("Nghệ sĩ trong gu của bạn", "Your artists");
            pages::draw_carousel_shelf(
                app,
                ui,
                "profile-artists",
                artist_title,
                &artist_cards,
                side,
            );
            let album_title = if taste {
                app.t("Album từ nghệ sĩ bạn yêu thích", "Albums from your artists")
            } else {
                app.t("Album nghe gần đây", "Recently heard albums")
            };
            pages::draw_carousel_shelf(app, ui, "profile-albums", album_title, &albums, side);
            ui.add_space(32.0);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{demo_tracks, PlaylistEntry};

    #[test]
    fn taste_counts_distinct_visible_songs_and_collaborators() {
        let mut tracks = demo_tracks();
        tracks.truncate(6);
        for (track, (id, artist)) in tracks.iter_mut().zip([
            (1, "A & B & A"),
            (2, "B"),
            (3, "C"),
            (4, "Hidden"),
            (5, "A"),
            (6, "D"),
        ]) {
            track.id = id;
            track.artist = artist.into();
        }
        let lib = Library::new(tracks);
        let mut data = UserData::default();
        data.liked = [1, 1, 2, 4, 999]
            .into_iter()
            .map(|id| PlaylistEntry { id, added: 0 })
            .collect();
        data.listening_history = vec![6, 1, 1, 3, 4, 5, 999];
        data.hidden_tracks.insert(4);
        assert_eq!(
            taste_artists(&lib, &data),
            vec![
                ArtistTaste {
                    name: "B".into(),
                    liked: 2,
                    recent: 1
                },
                ArtistTaste {
                    name: "A".into(),
                    liked: 1,
                    recent: 2
                },
                ArtistTaste {
                    name: "C".into(),
                    liked: 0,
                    recent: 1
                },
                ArtistTaste {
                    name: "D".into(),
                    liked: 0,
                    recent: 1
                },
            ]
        );
        data.hidden_tracks.clear();
        assert!(taste_artists(&lib, &data)
            .iter()
            .any(|artist| artist.name == "Hidden"));
        data.liked.clear();
        data.listening_history = vec![999; 50];
        data.listening_history.push(5);
        assert!(taste_artists(&lib, &data).is_empty());
    }
}
