//! Context menu of movies, series, seasons and episodes: its rows, then the action chosen.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use oxyfin_api::ItemId;
use oxyfin_core::{CoreError, Episode, MenuTarget, PlayMode, UserState};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::App;
use crate::present::{clock, core_error_message};
use crate::screens::library::open_detail;
use crate::state::{GenerationId, Msg, Screen};
use crate::{ContextMenu, MenuAction, MenuEntry, Message};

/// The item a menu opening is about, once its rows are shown.
struct Opened {
    serial: i32,
    generation: GenerationId,
    target: MenuTarget,
}

/// An item taken out of Continue Watching, with the position that puts it back.
struct Removed {
    generation: GenerationId,
    id: ItemId,
    position: Duration,
}

/// Answers the menu's requests with the rows of the item, then performs the row chosen.
#[derive(Default)]
pub(crate) struct MenuController {
    opened: RefCell<Option<Opened>>,
    removed: RefCell<Option<Removed>>,
}

/// Rows of a menu, grouped: a line separates two groups.
fn entries(groups: Vec<Vec<(MenuAction, SharedString)>>) -> Vec<MenuEntry> {
    groups
        .into_iter()
        .filter(|group| !group.is_empty())
        .flat_map(|group| {
            group
                .into_iter()
                .enumerate()
                .map(|(index, (action, detail))| MenuEntry {
                    action,
                    detail,
                    divider_before: index == 0,
                })
        })
        .collect()
}

fn play_rows(user: &UserState) -> Vec<(MenuAction, SharedString)> {
    if user.can_resume() {
        vec![
            (MenuAction::Resume, clock(user.position)),
            (MenuAction::PlayFromStart, SharedString::default()),
        ]
    } else {
        vec![(MenuAction::Play, SharedString::default())]
    }
}

fn state_rows(user: &UserState) -> Vec<(MenuAction, SharedString)> {
    let watched = if user.played {
        MenuAction::MarkUnwatched
    } else {
        MenuAction::MarkWatched
    };
    let favorite = if user.favorite {
        MenuAction::RemoveFavorite
    } else {
        MenuAction::AddFavorite
    };
    vec![
        (watched, SharedString::default()),
        (favorite, SharedString::default()),
    ]
}

fn resume_rows(from_resume: bool) -> Vec<(MenuAction, SharedString)> {
    if from_resume {
        vec![(MenuAction::RemoveFromResume, SharedString::default())]
    } else {
        Vec::new()
    }
}

fn episode_header(episode: &Episode) -> (String, String) {
    let line = match &episode.label {
        Some(label) => format!("{label} \u{b7} {}", episode.title),
        None => episode.title.clone(),
    };
    match &episode.series_name {
        Some(series) => (series.clone(), line),
        None => (line, String::new()),
    }
}

/// Title and second line of the menu header of `target`.
pub(crate) fn header(target: &MenuTarget) -> (String, String) {
    let year = |year: Option<i32>| year.map(|year| year.to_string()).unwrap_or_default();
    match target {
        MenuTarget::Movie(movie) => (movie.title.clone(), year(movie.year)),
        MenuTarget::Episode(episode) => episode_header(episode),
        MenuTarget::Series { summary, .. } => (summary.title.clone(), year(summary.year)),
        MenuTarget::Season {
            season,
            series_name,
            ..
        } => match series_name {
            Some(series) => (series.clone(), season.name.clone()),
            None => (season.name.clone(), String::new()),
        },
    }
}

/// Rows of the menu of `target`; `from_resume` when it was opened from Continue Watching, and
/// `shown_series` the series whose page is shown, which needs no row to go to it.
pub(crate) fn rows(
    target: &MenuTarget,
    from_resume: bool,
    shown_series: Option<&str>,
) -> Vec<MenuEntry> {
    match target {
        MenuTarget::Movie(movie) => entries(vec![
            play_rows(&movie.user),
            state_rows(&movie.user),
            vec![(MenuAction::Details, SharedString::default())],
            resume_rows(from_resume),
        ]),
        MenuTarget::Episode(episode) => {
            // The series page already shows the series and the season of its rows.
            let mut places = Vec::new();
            if shown_series != Some(episode.series_id.as_str()) {
                places.push((MenuAction::GoToSeries, SharedString::default()));
                if let (Some(_), Some(number)) = (&episode.season_id, episode.season_number) {
                    places.push((MenuAction::GoToSeason, number.to_string().into()));
                }
            }
            entries(vec![
                play_rows(&episode.user),
                state_rows(&episode.user),
                places,
                resume_rows(from_resume),
            ])
        }
        MenuTarget::Series {
            summary,
            next_episode,
        } => {
            let play = match next_episode {
                Some(episode) => {
                    let label: SharedString = episode.label.clone().unwrap_or_default().into();
                    if episode.can_resume() {
                        (MenuAction::ResumeNext, label)
                    } else {
                        (MenuAction::PlayNext, label)
                    }
                }
                None => (MenuAction::PlayNext, SharedString::default()),
            };
            let details = if shown_series == Some(summary.id.as_str()) {
                Vec::new()
            } else {
                vec![(MenuAction::Details, SharedString::default())]
            };
            entries(vec![vec![play], state_rows(&summary.user), details])
        }
        MenuTarget::Season { season, .. } => {
            let watched = if season.user.played {
                MenuAction::MarkSeasonUnwatched
            } else {
                MenuAction::MarkSeasonWatched
            };
            entries(vec![
                vec![(MenuAction::PlaySeason, SharedString::default())],
                vec![(watched, SharedString::default())],
            ])
        }
    }
}

/// A change of the user state of an item, asked from the menu.
#[derive(Clone, Copy)]
enum Change {
    Played(bool),
    Favorite(bool),
    /// Takes the item out of Continue Watching; it was resumed at that position.
    HideFromResume(Duration),
}

fn shown_series(app: &App) -> Option<String> {
    match app.screen() {
        Screen::Series(id) => Some(id),
        _ => None,
    }
}

fn close(window: &crate::AppWindow) {
    window.global::<ContextMenu>().invoke_close();
}

impl MenuController {
    /// Fetches the item `id` the menu opening `serial` is about, then shows its rows.
    pub(crate) fn request(&self, app: &Rc<App>, serial: i32, id: &str, from_resume: bool) {
        self.opened.borrow_mut().take();
        let Some((generation, catalog, _)) = app.services() else {
            if let Some(window) = app.window() {
                close(&window);
            }
            return;
        };
        let item = ItemId::from(id);
        app.spawn(
            async move { catalog.menu_target(&item).await },
            move |app, result| {
                app.menu
                    .answered(app, serial, generation, from_resume, result)
            },
        );
    }

    fn answered(
        &self,
        app: &Rc<App>,
        serial: i32,
        generation: GenerationId,
        from_resume: bool,
        result: Result<Option<MenuTarget>, CoreError>,
    ) {
        let Some(window) = app.window() else { return };
        let menu = window.global::<ContextMenu>();
        if !menu.get_open()
            || menu.get_serial() != serial
            || app
                .services()
                .is_none_or(|(current, ..)| current != generation)
        {
            return;
        }
        let target = match result {
            Ok(Some(target)) => target,
            Ok(None) => return close(&window),
            Err(CoreError::SessionExpired) => {
                close(&window);
                return app.dispatch(Msg::Expired(generation));
            }
            Err(error) => {
                log::warn!("cannot open the menu of the item: {error}");
                close(&window);
                return app.notify(core_error_message(&error).0);
            }
        };
        let (title, subtitle) = header(&target);
        let shown = shown_series(app);
        menu.set_title(title.into());
        menu.set_subtitle(subtitle.into());
        menu.set_entries(ModelRc::new(VecModel::from(rows(
            &target,
            from_resume,
            shown.as_deref(),
        ))));
        menu.set_ready_serial(serial);
        *self.opened.borrow_mut() = Some(Opened {
            serial,
            generation,
            target,
        });
    }

    /// Performs `action` on the item of the menu opening `serial`.
    pub(crate) fn chosen(&self, app: &Rc<App>, serial: i32, action: MenuAction) {
        let Some(opened) = self.opened.borrow_mut().take() else {
            return;
        };
        if opened.serial != serial
            || app
                .services()
                .is_none_or(|(current, ..)| current != opened.generation)
        {
            return;
        }
        let target = opened.target;
        let id = item_id(&target);
        match action {
            MenuAction::Play | MenuAction::Resume => app.play(id, PlayMode::Auto),
            MenuAction::PlayFromStart => app.play(id, PlayMode::FromStart),
            MenuAction::PlayNext | MenuAction::ResumeNext => app.play(id, PlayMode::Auto),
            MenuAction::PlaySeason => self.play_season(app, &target),
            MenuAction::MarkWatched | MenuAction::MarkSeasonWatched => {
                self.change(app, id, Change::Played(true));
            }
            MenuAction::MarkUnwatched | MenuAction::MarkSeasonUnwatched => {
                self.change(app, id, Change::Played(false));
            }
            MenuAction::AddFavorite => self.change(app, id, Change::Favorite(true)),
            MenuAction::RemoveFavorite => self.change(app, id, Change::Favorite(false)),
            MenuAction::RemoveFromResume => {
                let position = user_of(&target).position;
                self.change(app, id, Change::HideFromResume(position));
            }
            MenuAction::GoToSeries => {
                if let MenuTarget::Episode(episode) = &target {
                    open_detail(app, Screen::Series(episode.series_id.as_str().to_owned()));
                }
            }
            MenuAction::GoToSeason => {
                if let MenuTarget::Episode(Episode {
                    series_id,
                    season_id: Some(season),
                    ..
                }) = &target
                {
                    app.series
                        .show_season(app, series_id.as_str(), season.as_str());
                    if shown_series(app).as_deref() != Some(series_id.as_str()) {
                        open_detail(app, Screen::Series(series_id.as_str().to_owned()));
                    }
                }
            }
            MenuAction::Details => match &target {
                MenuTarget::Movie(movie) => {
                    open_detail(app, Screen::Movie(movie.id.as_str().to_owned()));
                }
                MenuTarget::Series { summary, .. } => {
                    open_detail(app, Screen::Series(summary.id.as_str().to_owned()));
                }
                MenuTarget::Episode(_) | MenuTarget::Season { .. } => {}
            },
        }
    }

    /// Plays the first unplayed episode of a season, else its first one.
    fn play_season(&self, app: &Rc<App>, target: &MenuTarget) {
        let MenuTarget::Season {
            season, series_id, ..
        } = target
        else {
            return;
        };
        let Some((generation, catalog, _)) = app.services() else {
            return;
        };
        let (series, season) = (series_id.clone(), season.id.clone());
        app.spawn(
            async move { catalog.season_episode_to_play(&series, &season).await },
            move |app, result| {
                if app
                    .services()
                    .is_none_or(|(current, ..)| current != generation)
                {
                    return;
                }
                match result {
                    Ok(Some(episode)) => app.play(episode.id, PlayMode::Auto),
                    Ok(None) => app.notify(Message::NothingToPlay),
                    Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(generation)),
                    Err(error) => {
                        log::warn!("cannot list the episodes of the season: {error}");
                        app.notify(core_error_message(&error).0);
                    }
                }
            },
        );
    }

    /// Sends `change` to the server, then reloads what shows the item; nothing changes on screen
    /// when the server refuses it.
    fn change(&self, app: &Rc<App>, id: ItemId, change: Change) {
        let Some((generation, catalog, _)) = app.services() else {
            return;
        };
        let item = id.clone();
        app.spawn(
            async move {
                match change {
                    Change::Played(played) => catalog.set_played(&id, played).await,
                    Change::Favorite(favorite) => catalog.set_favorite(&id, favorite).await,
                    Change::HideFromResume(_) => catalog.hide_from_resume(&id).await,
                }
            },
            move |app, result| {
                if app
                    .services()
                    .is_none_or(|(current, ..)| current != generation)
                {
                    return;
                }
                match result {
                    Ok(user) => {
                        if let Change::HideFromResume(position) = change {
                            *app.menu.removed.borrow_mut() = Some(Removed {
                                generation,
                                id: item.clone(),
                                position,
                            });
                            app.notify_undo(Message::RemovedFromResume);
                        }
                        app.item_changed(&item, &user);
                    }
                    Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(generation)),
                    Err(error) => {
                        log::warn!("cannot change the item: {error}");
                        app.notify(core_error_message(&error).0);
                    }
                }
            },
        );
    }

    /// Puts the item last taken out of Continue Watching back at its position.
    pub(crate) fn undo(&self, app: &Rc<App>) {
        let Some(removed) = self.removed.borrow_mut().take() else {
            return;
        };
        let Some((generation, catalog, _)) = app.services() else {
            return;
        };
        if generation != removed.generation {
            return;
        }
        let Removed { id, position, .. } = removed;
        let item = id.clone();
        app.spawn(
            async move { catalog.restore_resume(&id, position).await },
            move |app, result| {
                if app
                    .services()
                    .is_none_or(|(current, ..)| current != generation)
                {
                    return;
                }
                match result {
                    Ok(user) => app.item_changed(&item, &user),
                    Err(CoreError::SessionExpired) => app.dispatch(Msg::Expired(generation)),
                    Err(error) => {
                        log::warn!("cannot put the item back in Continue Watching: {error}");
                        app.notify(core_error_message(&error).0);
                    }
                }
            },
        );
    }
}

/// Presses and releases the right button at `(x, y)`, in window coordinates.
pub(crate) fn press_right(window: &crate::AppWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    let button = slint::platform::PointerEventButton::Right;
    for event in [
        WindowEvent::PointerMoved { position },
        WindowEvent::PointerPressed { position, button },
        WindowEvent::PointerReleased { position, button },
    ] {
        window.window().dispatch_event(event);
    }
}

fn user_of(target: &MenuTarget) -> &UserState {
    match target {
        MenuTarget::Movie(movie) => &movie.user,
        MenuTarget::Episode(episode) => &episode.user,
        MenuTarget::Series { summary, .. } => &summary.user,
        MenuTarget::Season { season, .. } => &season.user,
    }
}

fn item_id(target: &MenuTarget) -> ItemId {
    match target {
        MenuTarget::Movie(movie) => movie.id.clone(),
        MenuTarget::Episode(episode) => episode.id.clone(),
        MenuTarget::Series { summary, .. } => summary.id.clone(),
        MenuTarget::Season { season, .. } => season.id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxyfin_core::{ItemKind, ItemSummary, Season};

    use super::*;

    fn user(played: bool, favorite: bool, position: u64) -> UserState {
        UserState {
            played,
            favorite,
            position: Duration::from_secs(position),
            ..UserState::default()
        }
    }

    fn summary(kind: ItemKind, user: UserState) -> ItemSummary {
        ItemSummary {
            id: ItemId::from("item"),
            kind,
            title: "Title".into(),
            year: Some(2020),
            series_id: None,
            series_name: None,
            episode_label: None,
            poster: None,
            backdrop: None,
            thumb: None,
            duration: None,
            overview: None,
            user,
        }
    }

    fn episode(user: UserState) -> Episode {
        Episode {
            id: ItemId::from("episode"),
            series_id: ItemId::from("series"),
            series_name: Some("Les Veilleurs".into()),
            season_id: Some(ItemId::from("season")),
            title: "The Pact".into(),
            season_number: Some(2),
            number: Some(5),
            label: Some("S02E05".into()),
            overview: None,
            thumb: None,
            duration: None,
            user,
        }
    }

    fn listed(rows: &[MenuEntry]) -> Vec<(MenuAction, String, bool)> {
        rows.iter()
            .map(|row| (row.action, row.detail.to_string(), row.divider_before))
            .collect()
    }

    #[test]
    fn a_started_episode_in_continue_watching_has_every_row() {
        let target = MenuTarget::Episode(episode(user(false, false, 18 * 60 + 42)));
        assert_eq!(
            header(&target),
            ("Les Veilleurs".into(), "S02E05 \u{b7} The Pact".into())
        );
        assert_eq!(
            listed(&rows(&target, true, None)),
            [
                (MenuAction::Resume, "18:42".into(), true),
                (MenuAction::PlayFromStart, String::new(), false),
                (MenuAction::MarkWatched, String::new(), true),
                (MenuAction::AddFavorite, String::new(), false),
                (MenuAction::GoToSeries, String::new(), true),
                (MenuAction::GoToSeason, "2".into(), false),
                (MenuAction::RemoveFromResume, String::new(), true),
            ]
        );
    }

    #[test]
    fn an_episode_on_its_series_page_does_not_go_to_it() {
        let target = MenuTarget::Episode(episode(user(true, true, 0)));
        let actions: Vec<MenuAction> = rows(&target, false, Some("series"))
            .iter()
            .map(|row| row.action)
            .collect();
        assert_eq!(
            actions,
            [
                MenuAction::Play,
                MenuAction::MarkUnwatched,
                MenuAction::RemoveFavorite,
            ]
        );
    }

    #[test]
    fn a_new_movie_plays_and_opens_its_details() {
        let target = MenuTarget::Movie(summary(ItemKind::Movie, user(false, true, 0)));
        assert_eq!(header(&target), ("Title".into(), "2020".into()));
        assert_eq!(
            listed(&rows(&target, false, None)),
            [
                (MenuAction::Play, String::new(), true),
                (MenuAction::MarkWatched, String::new(), true),
                (MenuAction::RemoveFavorite, String::new(), false),
                (MenuAction::Details, String::new(), true),
            ]
        );
    }

    #[test]
    fn a_series_names_the_episode_it_resumes() {
        let target = MenuTarget::Series {
            summary: summary(ItemKind::Series, user(false, false, 0)),
            next_episode: Some(Box::new(episode(user(false, false, 60)))),
        };
        assert_eq!(
            listed(&rows(&target, false, None)),
            [
                (MenuAction::ResumeNext, "S02E05".into(), true),
                (MenuAction::MarkWatched, String::new(), true),
                (MenuAction::AddFavorite, String::new(), false),
                (MenuAction::Details, String::new(), true),
            ]
        );
        let target = MenuTarget::Series {
            summary: summary(ItemKind::Series, user(true, false, 0)),
            next_episode: None,
        };
        assert_eq!(
            listed(&rows(&target, false, Some("item")))[..2],
            [
                (MenuAction::PlayNext, String::new(), true),
                (MenuAction::MarkUnwatched, String::new(), true),
            ]
        );
    }

    #[test]
    fn a_watched_season_plays_and_can_be_unwatched() {
        let target = MenuTarget::Season {
            season: Season {
                id: ItemId::from("season"),
                name: "Season 2".into(),
                number: Some(2),
                poster: None,
                user: user(true, false, 0),
            },
            series_id: ItemId::from("series"),
            series_name: Some("Les Veilleurs".into()),
        };
        assert_eq!(header(&target), ("Les Veilleurs".into(), "Season 2".into()));
        assert_eq!(
            listed(&rows(&target, false, Some("series"))),
            [
                (MenuAction::PlaySeason, String::new(), true),
                (MenuAction::MarkSeasonUnwatched, String::new(), true),
            ]
        );
    }
}
