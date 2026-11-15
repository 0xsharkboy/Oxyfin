//! Language selection: maps the system locale to one of the bundled translations.

/// Language code of a locale such as `fr_FR.UTF-8`, or `None` for `C`, `POSIX` and empty values.
pub fn language_of(locale: &str) -> Option<String> {
    let language = locale.split(['.', '@', '_', '-']).next()?.trim();
    let valid = !language.is_empty() && language.chars().all(|c| c.is_ascii_alphabetic());
    if !valid || language.eq_ignore_ascii_case("c") || language.eq_ignore_ascii_case("posix") {
        return None;
    }
    Some(language.to_ascii_lowercase())
}

fn detect(
    var: impl Fn(&str) -> Option<String>,
    system: impl FnOnce() -> Option<String>,
) -> Option<String> {
    var("LANGUAGE")
        .and_then(|list| list.split(':').next().and_then(language_of))
        .or_else(|| {
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .find_map(|name| var(name).and_then(|value| language_of(&value)))
        })
        .or_else(|| system().and_then(|locale| language_of(&locale)))
}

/// Selects the bundled translation for the system language; English when there is none.
///
/// Must be called after the first window is created.
pub fn select_language() {
    let language = detect(|name| std::env::var(name).ok(), sys_locale::get_locale);
    let wanted = language.as_deref().unwrap_or("en");
    if let Err(error) = slint::select_bundled_translation(wanted) {
        log::debug!("no translation for {wanted}, using English: {error}");
        if let Err(error) = slint::select_bundled_translation("en") {
            log::debug!("cannot select English: {error}");
        }
    }
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
        let all = [("LANGUAGE", ""), ("LANG", "C"), ("LC_ALL", "POSIX")];
        assert_eq!(
            detect(env(&all), || Some("pt-BR".into())).as_deref(),
            Some("pt")
        );
        assert_eq!(detect(env(&[]), || None), None);
    }
}
