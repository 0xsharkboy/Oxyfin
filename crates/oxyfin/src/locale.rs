//! Language selection: maps the saved choice or the system locale to one of the bundled
//! translations.

use std::sync::Mutex;

/// Language code of a locale such as `fr_FR.UTF-8`, or `None` for `C`, `POSIX` and empty values.
pub fn language_of(locale: &str) -> Option<String> {
    let language = locale.split(['.', '@', '_', '-']).next()?.trim();
    let valid = !language.is_empty() && language.chars().all(|c| c.is_ascii_alphabetic());
    if !valid || language.eq_ignore_ascii_case("c") || language.eq_ignore_ascii_case("posix") {
        return None;
    }
    Some(language.to_ascii_lowercase())
}

fn is_c_locale(locale: &str) -> bool {
    let name = locale.split(['.', '@']).next().unwrap_or("").trim();
    name.eq_ignore_ascii_case("c") || name.eq_ignore_ascii_case("posix")
}

/// Language wanted by the environment; `en` for a C locale, `None` when nothing is set.
fn detect(
    var: impl Fn(&str) -> Option<String>,
    system: impl FnOnce() -> Option<String>,
) -> Option<String> {
    // POSIX: LC_ALL overrides LC_MESSAGES. Like gettext, a C locale there ignores LANGUAGE.
    let forced = ["LC_ALL", "LC_MESSAGES"]
        .iter()
        .find_map(|name| var(name).filter(|value| !value.trim().is_empty()));
    if forced.is_some_and(|value| is_c_locale(&value)) {
        return Some("en".to_owned());
    }
    var("LANGUAGE")
        .and_then(|list| list.split(':').next().and_then(language_of))
        .or_else(|| {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .find_map(|name| var(name).and_then(|value| language_of(&value)))
        })
        .or_else(|| system().and_then(|locale| language_of(&locale)))
}

/// Language of the interface, once [`select_language`] chose it.
static INTERFACE_LANGUAGE: Mutex<Option<String>> = Mutex::new(None);

/// The language the interface shows, as an ISO 639-1 code such as `fr`; `en`
/// before [`select_language`] runs.
pub fn interface_language() -> String {
    INTERFACE_LANGUAGE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_else(|| "en".to_owned())
}

/// Names of the languages the interface can show, each in its own language.
const NAMES: &[(&str, &str)] = &[("en", "English"), ("fr", "Français")];

/// Name of language `code` in that language; the code itself when unknown.
pub fn language_name(code: &str) -> String {
    NAMES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or_else(|| code.to_owned(), |(_, name)| (*name).to_owned())
}

/// Codes of the languages the interface can show: English, the source language, then each
/// bundled translation.
///
/// Must be called after the first window is created.
pub fn bundled_languages() -> Vec<String> {
    let mut codes = vec!["en".to_owned()];
    // No translation has this code: Slint lists the bundled ones, the source as an empty code,
    // and selects nothing.
    if let Err(slint::SelectBundledTranslationError::LanguageNotFound {
        available_languages,
    }) = slint::select_bundled_translation("*")
    {
        for code in available_languages.iter() {
            if !code.is_empty() && !codes.iter().any(|known| known == code.as_str()) {
                codes.push(code.to_string());
            }
        }
    }
    codes
}

/// The language the interface shows: `choice` when bundled, else `system` when bundled, else
/// English.
pub fn resolve(choice: Option<&str>, system: Option<&str>, bundled: &[String]) -> String {
    [choice, system]
        .into_iter()
        .flatten()
        .find(|code| bundled.iter().any(|known| known == code))
        .unwrap_or("en")
        .to_owned()
}

/// Language the environment asks for, as an ISO 639-1 code; `None` when nothing is set.
pub fn system_language() -> Option<String> {
    detect(|name| std::env::var(name).ok(), sys_locale::get_locale)
}

/// Shows the interface in `language`, one of [`bundled_languages`]; English when it has no
/// translation.
///
/// Must be called after the first window is created.
pub fn select_language(language: &str) {
    let selected = match slint::select_bundled_translation(language) {
        Ok(()) => language,
        Err(error) => {
            log::debug!("no translation for {language}, using English: {error}");
            if let Err(error) = slint::select_bundled_translation("en") {
                log::debug!("cannot select English: {error}");
            }
            "en"
        }
    };
    *INTERFACE_LANGUAGE.lock().unwrap_or_else(|e| e.into_inner()) = Some(selected.to_owned());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locales_map_to_languages() {
        assert_eq!(language_of("fr_FR.UTF-8").as_deref(), Some("fr"));
        assert_eq!(language_of("fr").as_deref(), Some("fr"));
        assert_eq!(language_of("en_US").as_deref(), Some("en"));
        assert_eq!(language_of("fr-CA").as_deref(), Some("fr"));
        assert_eq!(language_of("de_DE@euro").as_deref(), Some("de"));
        for none in ["", "C", "POSIX", "C.UTF-8", "  "] {
            assert_eq!(language_of(none), None, "{none:?}");
        }
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn environment_variables_come_in_order() {
        let all = [
            ("LANGUAGE", "fr:en"),
            ("LC_ALL", "de_DE"),
            ("LANG", "es_ES"),
        ];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("fr"));
        let all = [("LC_ALL", "de_DE"), ("LANG", "es_ES")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("de"));
        let all = [("LC_MESSAGES", "it_IT"), ("LANG", "es_ES")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("it"));
        let all = [("LANGUAGE", ""), ("LANG", "C")];
        assert_eq!(
            detect(env(&all), || Some("pt-BR".into())).as_deref(),
            Some("pt")
        );
        assert_eq!(detect(env(&[]), || None), None);
    }

    #[test]
    fn a_c_locale_in_lc_all_or_lc_messages_selects_english() {
        for value in ["C", "POSIX", "C.UTF-8"] {
            let all = [("LC_ALL", value), ("LANG", "fr_FR.UTF-8")];
            assert_eq!(
                detect(env(&all), || Some("pt-BR".into())).as_deref(),
                Some("en"),
                "LC_ALL={value}"
            );
        }
        let all = [("LC_MESSAGES", "C"), ("LANG", "fr_FR.UTF-8")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("en"));
    }

    #[test]
    fn language_is_ignored_in_a_c_locale() {
        let all = [("LANGUAGE", "fr"), ("LC_ALL", "C")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("en"));
        let all = [("LANGUAGE", "fr"), ("LC_MESSAGES", "POSIX")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("en"));
    }

    #[test]
    fn lc_all_overrides_lc_messages() {
        let all = [("LC_ALL", "de_DE"), ("LC_MESSAGES", "C")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("de"));
        let all = [("LC_ALL", "C"), ("LC_MESSAGES", "de_DE")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("en"));
        let all = [("LC_ALL", ""), ("LC_MESSAGES", "C"), ("LANG", "fr_FR")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("en"));
    }

    #[test]
    fn language_still_wins_over_a_real_locale() {
        let all = [("LANGUAGE", "it:en"), ("LC_ALL", "de_DE")];
        assert_eq!(detect(env(&all), || None).as_deref(), Some("it"));
    }

    fn bundled() -> Vec<String> {
        vec!["en".to_owned(), "fr".to_owned()]
    }

    #[test]
    fn the_system_language_shows_when_it_is_bundled() {
        assert_eq!(resolve(None, Some("fr"), &bundled()), "fr");
        assert_eq!(resolve(None, Some("en"), &bundled()), "en");
    }

    #[test]
    fn english_shows_when_the_system_language_is_not_bundled() {
        assert_eq!(resolve(None, Some("de"), &bundled()), "en");
        assert_eq!(resolve(None, None, &bundled()), "en");
    }

    #[test]
    fn a_chosen_language_wins_over_the_system_one() {
        assert_eq!(resolve(Some("en"), Some("fr"), &bundled()), "en");
        assert_eq!(resolve(Some("fr"), Some("de"), &bundled()), "fr");
    }

    #[test]
    fn a_chosen_language_no_longer_bundled_follows_the_system() {
        assert_eq!(resolve(Some("de"), Some("fr"), &bundled()), "fr");
        assert_eq!(resolve(Some("de"), None, &bundled()), "en");
    }

    #[test]
    fn every_bundled_translation_has_a_name() {
        let po = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("po");
        let entries = std::fs::read_dir(&po).unwrap();
        let mut codes: Vec<String> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().is_dir())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        codes.push("en".to_owned());
        assert!(codes.len() > 1);
        for code in codes {
            assert!(
                NAMES.iter().any(|(known, _)| *known == code),
                "{code} has no name"
            );
        }
        assert_eq!(language_name("en"), "English");
        assert_eq!(language_name("fr"), "Français");
        assert_eq!(language_name("xx"), "xx");
    }
}
