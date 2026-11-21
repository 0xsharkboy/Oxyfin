//! Language codes of subtitle searches.
//!
//! Jellyfin searches subtitles with ISO 639-2 bibliographic codes such as `fre`,
//! while media streams may carry the terminology code `fra` and the interface
//! language is a two-letter code such as `fr`.

/// Languages offered for a subtitle search, as (ISO 639-1, ISO 639-2/B, ISO 639-2/T) codes.
const LANGUAGES: [(&str, &str, &str); 40] = [
    ("en", "eng", "eng"),
    ("fr", "fre", "fra"),
    ("es", "spa", "spa"),
    ("de", "ger", "deu"),
    ("it", "ita", "ita"),
    ("pt", "por", "por"),
    ("nl", "dut", "nld"),
    ("sv", "swe", "swe"),
    ("no", "nor", "nor"),
    ("da", "dan", "dan"),
    ("fi", "fin", "fin"),
    ("is", "ice", "isl"),
    ("pl", "pol", "pol"),
    ("cs", "cze", "ces"),
    ("sk", "slo", "slk"),
    ("hu", "hun", "hun"),
    ("ro", "rum", "ron"),
    ("bg", "bul", "bul"),
    ("el", "gre", "ell"),
    ("ru", "rus", "rus"),
    ("uk", "ukr", "ukr"),
    ("hr", "hrv", "hrv"),
    ("sr", "srp", "srp"),
    ("sl", "slv", "slv"),
    ("et", "est", "est"),
    ("lv", "lav", "lav"),
    ("lt", "lit", "lit"),
    ("tr", "tur", "tur"),
    ("ar", "ara", "ara"),
    ("he", "heb", "heb"),
    ("fa", "per", "fas"),
    ("hi", "hin", "hin"),
    ("th", "tha", "tha"),
    ("vi", "vie", "vie"),
    ("id", "ind", "ind"),
    ("ms", "may", "msa"),
    ("zh", "chi", "zho"),
    ("ja", "jpn", "jpn"),
    ("ko", "kor", "kor"),
    ("ca", "cat", "cat"),
];

/// The ISO 639-2 bibliographic codes of the languages offered for a subtitle
/// search, English first, then by how widely subtitles exist in them.
pub fn subtitle_languages() -> Vec<&'static str> {
    LANGUAGES.iter().map(|(_, code, _)| *code).collect()
}

/// The ISO 639-2 bibliographic code to search subtitles in for `code`, given
/// as an ISO 639-1 code (`fr`) or an ISO 639-2 code of either form (`fre`,
/// `fra`), in any case. `None` for a language not offered.
pub fn subtitle_language(code: &str) -> Option<&'static str> {
    let code = code.trim().to_ascii_lowercase();
    LANGUAGES
        .iter()
        .find(|(short, bibliographic, terminology)| {
            [*short, *bibliographic, *terminology].contains(&code.as_str())
        })
        .map(|(_, bibliographic, _)| *bibliographic)
}

/// Whether two language codes name the same language, whatever their form.
pub fn same_language(first: &str, second: &str) -> bool {
    match (subtitle_language(first), subtitle_language(second)) {
        (Some(first), Some(second)) => first == second,
        _ => first.trim().eq_ignore_ascii_case(second.trim()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_gives_the_bibliographic_code() {
        for code in ["fr", "fre", "fra", "FRA", " fr "] {
            assert_eq!(subtitle_language(code), Some("fre"), "{code}");
        }
        assert_eq!(subtitle_language("deu"), Some("ger"));
        assert_eq!(subtitle_language("zh"), Some("chi"));
        assert_eq!(subtitle_language("en"), Some("eng"));
        assert_eq!(subtitle_language("und"), None);
        assert_eq!(subtitle_language(""), None);
    }

    #[test]
    fn both_forms_name_the_same_language() {
        assert!(same_language("fre", "fra"));
        assert!(same_language("fr", "FRE"));
        assert!(!same_language("fre", "eng"));
        assert!(same_language("xyz", "XYZ"));
        assert!(!same_language("xyz", "eng"));
    }

    #[test]
    fn the_list_starts_with_english_and_has_no_duplicate() {
        let codes = subtitle_languages();
        assert_eq!(codes[0], "eng");
        let mut unique = codes.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len());
    }
}
