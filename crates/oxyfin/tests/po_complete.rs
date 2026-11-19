//! Every message of the catalog has a French translation.

use std::fs;
use std::path::PathBuf;

/// One `msgid` (and `msgid_plural`) of a catalog with the translations given for it.
#[derive(Debug)]
struct Entry {
    context: String,
    id: String,
    plural: Option<String>,
    translations: Vec<String>,
    fuzzy: bool,
}

fn unquote(line: &str) -> String {
    let start = line.find('"').map_or(0, |index| index + 1);
    let end = line.rfind('"').unwrap_or(line.len());
    line.get(start..end.max(start))
        .unwrap_or_default()
        .to_owned()
}

fn parse(text: &str) -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    let mut current: Option<Entry> = None;
    let mut field = "";
    let mut fuzzy = false;
    for line in text.lines() {
        if line.starts_with("#,") {
            fuzzy = line.split([',', ' ']).any(|flag| flag == "fuzzy");
        } else if line.starts_with("msgctxt ") {
            entries.extend(current.take());
            current = Some(Entry {
                context: unquote(line),
                id: String::new(),
                plural: None,
                translations: Vec::new(),
                fuzzy,
            });
            field = "context";
            fuzzy = false;
        } else if let Some(entry) = current.as_mut() {
            if line.starts_with("msgid_plural ") {
                entry.plural = Some(unquote(line));
                field = "plural";
            } else if line.starts_with("msgid ") {
                entry.id = unquote(line);
                field = "id";
            } else if line.starts_with("msgstr") {
                entry.translations.push(unquote(line));
                field = "str";
            } else if line.starts_with('"') {
                let part = unquote(line);
                match field {
                    "id" => entry.id.push_str(&part),
                    "plural" => entry.plural.get_or_insert_with(String::new).push_str(&part),
                    "str" => {
                        if let Some(last) = entry.translations.last_mut() {
                            last.push_str(&part);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    entries.extend(current);
    entries
}

fn read(relative: &str) -> Vec<Entry> {
    let path = manifest_dir().join(relative);
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    parse(&text)
}

#[test]
fn french_catalog_is_complete() {
    let template = read("po/oxyfin.pot");
    let french = read("po/fr/LC_MESSAGES/oxyfin.po");
    assert!(!template.is_empty(), "the template has no messages");
    for message in &template {
        let found = french
            .iter()
            .find(|entry| entry.context == message.context && entry.id == message.id)
            .unwrap_or_else(|| panic!("no French entry for {:?}", message.id));
        let expected = if message.plural.is_some() { 2 } else { 1 };
        assert_eq!(
            found.translations.len(),
            expected,
            "wrong number of forms for {:?}",
            message.id
        );
        for translation in &found.translations {
            assert!(
                !translation.is_empty(),
                "{:?} is not translated",
                message.id
            );
        }
    }
}

#[test]
fn french_placeholders_match_the_source() {
    let template = read("po/oxyfin.pot");
    let french = read("po/fr/LC_MESSAGES/oxyfin.po");
    let count = |text: &str| (text.matches("{}").count(), text.matches("{n}").count());
    for message in &template {
        let Some(found) = french
            .iter()
            .find(|entry| entry.context == message.context && entry.id == message.id)
        else {
            continue;
        };
        for translation in &found.translations {
            assert_eq!(
                count(translation),
                count(&message.id),
                "placeholders differ in {:?}",
                message.id
            );
        }
    }
}

#[test]
fn french_catalog_has_no_fuzzy_entries() {
    for entry in read("po/fr/LC_MESSAGES/oxyfin.po") {
        assert!(
            !entry.fuzzy,
            "{:?} is flagged fuzzy, and Slint drops fuzzy entries",
            entry.id
        );
    }
}

/// The package directory, read at run time: cargo may reuse a test binary
/// built from another checkout of the same tree, whose path is gone.
fn manifest_dir() -> PathBuf {
    std::env::var_os("CARGO_MANIFEST_DIR")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from)
}
