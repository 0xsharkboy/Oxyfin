//! Subtitle search of the player: finds subtitles online through the server,
//! downloads one and has the playback controller show it.

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use oxyfin_api::ItemId;
use oxyfin_core::{
    AudioOption, Catalog, CoreError, PlaybackCommand, RemoteSubtitleInfo, same_language,
    subtitle_language, subtitle_languages,
};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};
use tokio::runtime::Handle;

use crate::present::core_error_message;
use crate::{AppWindow, Message, SubtitleResult, SubtitleRowState, SubtitleSearch};

/// Gives the catalog of the signed-in session, if any.
pub type CatalogSource = Rc<dyn Fn() -> Option<Catalog>>;

/// Gives the ISO 639-1 code of the language the interface shows.
pub type LanguageSource = Rc<dyn Fn() -> String>;

/// What the search keeps between its callbacks.
#[derive(Default)]
struct State {
    /// Counts the searches, so that the answer of an earlier one is dropped.
    serial: u64,
    /// Results of the latest search, in the order shown.
    results: Vec<RemoteSubtitleInfo>,
}

struct Search {
    window: slint::Weak<AppWindow>,
    runtime: Handle,
    interface_language: LanguageSource,
    catalog: CatalogSource,
    send: Rc<dyn Fn(PlaybackCommand)>,
    state: RefCell<State>,
}

/// Wires the subtitle search of `window`.
///
/// `interface_language` gives the language the interface shows when the search
/// opens, offered first. Searches and downloads go through the catalog that
/// `catalog` gives at that time on `runtime`; once a download ends,
/// [`PlaybackCommand::ReloadSubtitles`] goes to `send`.
pub fn bind(
    window: &AppWindow,
    runtime: Handle,
    interface_language: LanguageSource,
    catalog: CatalogSource,
    send: Rc<dyn Fn(PlaybackCommand)>,
) {
    let search = Rc::new(Search {
        window: window.as_weak(),
        runtime,
        interface_language,
        catalog,
        send,
        state: RefCell::default(),
    });
    let global = window.global::<SubtitleSearch>();
    global.set_other_languages(strings(&subtitle_languages()));
    let opened = Rc::clone(&search);
    global.on_opened(move || opened.open());
    let chosen = Rc::clone(&search);
    global.on_language_chosen(move |code| chosen.choose(&code));
    global.on_download(move |index| {
        if let Ok(index) = usize::try_from(index) {
            search.download(index);
        }
    });
}

/// Follows the item playing, the one the search finds subtitles for.
pub fn follow_item(window: &AppWindow, item: &ItemId) {
    let global = window.global::<SubtitleSearch>();
    // The same item loads again for another quality: what the dialog shows still holds.
    if global.get_item().as_str() == item.as_str() {
        return;
    }
    global.set_item(item.as_str().into());
    global.set_results(ModelRc::default());
    global.set_has_error(false);
}

/// Remembers the language of the audio track playing, offered after the interface language.
pub fn follow_audio(window: &AppWindow, audio: &[AudioOption]) {
    let language = audio
        .iter()
        .find(|option| option.selected)
        .and_then(|option| option.language.clone())
        .unwrap_or_default();
    window
        .global::<SubtitleSearch>()
        .set_audio_language(language.into());
}

/// The languages offered first, as ISO 639-2 bibliographic codes: the
/// interface language, then the language of the audio playing when it differs.
pub fn language_chips(interface: &str, audio: Option<&str>) -> Vec<&'static str> {
    let mut chips: Vec<&'static str> = Vec::new();
    for code in [Some(interface), audio].into_iter().flatten() {
        if let Some(code) = subtitle_language(code)
            && !chips.iter().any(|chip| same_language(chip, code))
        {
            chips.push(code);
        }
    }
    if chips.is_empty() {
        chips.push("eng");
    }
    chips
}

/// A count of downloads in short, such as `12.4k`.
pub fn short_count(count: u64) -> String {
    let short = |value: f64, unit: &str| {
        let text = format!("{value:.1}");
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match count {
        0..1_000 => count.to_string(),
        1_000..1_000_000 => short(count as f64 / 1_000.0, "k"),
        _ => short(count as f64 / 1_000_000.0, "M"),
    }
}

/// The row of a result, as the dialog shows it.
pub fn result_row(result: &RemoteSubtitleInfo) -> SubtitleResult {
    SubtitleResult {
        name: result.name.as_str().into(),
        provider: result.provider_name.clone().unwrap_or_default().into(),
        format: result
            .format
            .as_deref()
            .unwrap_or_default()
            .to_uppercase()
            .into(),
        downloads: result
            .download_count
            .map_or(-1, |count| i32::try_from(count).unwrap_or(i32::MAX)),
        downloads_text: result
            .download_count
            .map(short_count)
            .unwrap_or_default()
            .into(),
        perfect_match: result.is_perfect_match.unwrap_or(false),
        forced: result.is_forced.unwrap_or(false),
        hearing_impaired: result.is_hearing_impaired.unwrap_or(false),
        state: SubtitleRowState::Idle,
        awaiting: false,
    }
}

fn strings(codes: &[&str]) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(
        codes
            .iter()
            .map(|code| SharedString::from(*code))
            .collect::<Vec<_>>(),
    ))
}

impl Search {
    /// Runs `task` on the runtime, then `then` on the Slint thread with the window.
    fn spawn<T, F>(&self, task: F, then: impl FnOnce(&AppWindow, T) + 'static)
    where
        T: Send + 'static,
        F: Future<Output = T> + Send + 'static,
    {
        let task = self.runtime.spawn(task);
        let window = self.window.clone();
        let result = slint::spawn_local(async move {
            let Ok(value) = task.await else { return };
            if let Some(window) = window.upgrade() {
                then(&window, value);
            }
        });
        if let Err(error) = result {
            log::error!("cannot schedule work on the event loop: {error}");
        }
    }

    fn open(self: &Rc<Self>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let global = window.global::<SubtitleSearch>();
        let audio = global.get_audio_language();
        let interface_language = (self.interface_language)();
        let chips = language_chips(
            &interface_language,
            Some(audio.as_str()).filter(|audio| !audio.is_empty()),
        );
        global.set_languages(strings(&chips));
        self.search(&window, chips[0]);
    }

    fn choose(self: &Rc<Self>, code: &str) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let global = window.global::<SubtitleSearch>();
        let mut chips: Vec<SharedString> = global.get_languages().iter().collect();
        if !chips.iter().any(|chip| chip.as_str() == code) {
            chips.push(code.into());
            global.set_languages(ModelRc::new(VecModel::from(chips)));
        }
        self.search(&window, code);
    }

    fn search(self: &Rc<Self>, window: &AppWindow, code: &str) {
        let global = window.global::<SubtitleSearch>();
        let serial = {
            let mut state = self.state.borrow_mut();
            state.serial += 1;
            state.results.clear();
            state.serial
        };
        global.set_language(code.into());
        global.set_results(ModelRc::default());
        global.set_has_error(false);
        let Some(catalog) = (self.catalog)() else {
            global.set_searching(false);
            return;
        };
        global.set_searching(true);
        let item = ItemId::from(global.get_item().as_str());
        let code = code.to_owned();
        let this = Rc::clone(self);
        self.spawn(
            async move { catalog.search_subtitles(&item, &code).await },
            move |window, result| {
                if this.state.borrow().serial != serial {
                    return;
                }
                let global = window.global::<SubtitleSearch>();
                global.set_searching(false);
                match result {
                    Ok(found) => {
                        let rows: Vec<_> = found.iter().map(result_row).collect();
                        this.state.borrow_mut().results = found;
                        global.set_results(ModelRc::new(VecModel::from(rows)));
                    }
                    Err(error) => {
                        log::warn!("cannot search subtitles: {error}");
                        show_error(window, &error);
                    }
                }
            },
        );
    }

    fn download(self: &Rc<Self>, index: usize) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let (serial, id) = {
            let state = self.state.borrow();
            let Some(result) = state.results.get(index) else {
                return;
            };
            (state.serial, result.id.clone())
        };
        let Some(catalog) = (self.catalog)() else {
            return;
        };
        let global = window.global::<SubtitleSearch>();
        global.set_has_error(false);
        set_state(&window, index, SubtitleRowState::Downloading);
        let item = ItemId::from(global.get_item().as_str());
        let language = global.get_language().to_string();
        let this = Rc::clone(self);
        self.spawn(
            async move { catalog.download_subtitle(&item, &id).await },
            move |window, result| {
                let current = this.state.borrow().serial == serial;
                match result {
                    Ok(()) => {
                        if current {
                            update_row(window, index, |row| row.awaiting = true);
                        }
                        (this.send)(PlaybackCommand::ReloadSubtitles {
                            select_language: language,
                        });
                    }
                    Err(error) => {
                        log::warn!("cannot download the subtitle: {error}");
                        if current {
                            set_state(window, index, SubtitleRowState::Idle);
                            show_error(window, &error);
                        }
                    }
                }
            },
        );
    }
}

/// Ends the downloads waiting for the player: their file was `added` to the
/// item playing, or the server did not add it in time.
pub fn reloaded(window: &AppWindow, added: bool) {
    let results = window.global::<SubtitleSearch>().get_results();
    let mut failed = false;
    for index in 0..results.row_count() {
        if let Some(mut row) = results.row_data(index).filter(|row| row.awaiting) {
            row.awaiting = false;
            row.state = if added {
                SubtitleRowState::Added
            } else {
                failed = true;
                SubtitleRowState::Idle
            };
            results.set_row_data(index, row);
        }
    }
    if failed {
        show_message(window, Message::SubtitleNotAdded);
    }
}

fn update_row(window: &AppWindow, index: usize, change: impl FnOnce(&mut SubtitleResult)) {
    let results = window.global::<SubtitleSearch>().get_results();
    if let Some(mut row) = results.row_data(index) {
        change(&mut row);
        results.set_row_data(index, row);
    }
}

fn set_state(window: &AppWindow, index: usize, state: SubtitleRowState) {
    update_row(window, index, |row| row.state = state);
}

fn show_error(window: &AppWindow, error: &CoreError) {
    show_message(window, core_error_message(error).0);
}

fn show_message(window: &AppWindow, message: Message) {
    let global = window.global::<SubtitleSearch>();
    global.set_error(message);
    global.set_has_error(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interface_language_comes_first_then_the_audio_one() {
        assert_eq!(language_chips("fr", Some("eng")), ["fre", "eng"]);
        assert_eq!(language_chips("en", Some("fra")), ["eng", "fre"]);
        assert_eq!(language_chips("fr", Some("fra")), ["fre"]);
        assert_eq!(language_chips("en", None), ["eng"]);
        assert_eq!(language_chips("en", Some("und")), ["eng"]);
        assert_eq!(language_chips("xx", None), ["eng"]);
    }

    #[test]
    fn download_counts_are_short() {
        assert_eq!(short_count(412), "412");
        assert_eq!(short_count(3005), "3k");
        assert_eq!(short_count(12_400), "12.4k");
        assert_eq!(short_count(3_100), "3.1k");
        assert_eq!(short_count(1_250_000), "1.2M");
    }

    #[test]
    fn a_row_shows_the_format_in_capitals_and_the_tags() {
        let row = result_row(&RemoteSubtitleInfo {
            id: "a".into(),
            name: "Film.FR".into(),
            provider_name: Some("Open Subtitles".into()),
            format: Some("srt".into()),
            download_count: Some(12_400),
            is_forced: Some(true),
            is_hearing_impaired: None,
            is_perfect_match: Some(true),
        });
        assert_eq!(row.format, "SRT");
        assert_eq!(row.downloads, 12_400);
        assert_eq!(row.downloads_text, "12.4k");
        assert!(row.forced && row.perfect_match && !row.hearing_impaired);
        assert_eq!(row.state, SubtitleRowState::Idle);
        let unknown = result_row(&RemoteSubtitleInfo::default());
        assert_eq!(unknown.downloads, -1);
        assert_eq!(unknown.provider, "");
    }
}
