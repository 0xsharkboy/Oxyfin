//! Fallback fonts: families with a face that draws nothing go after the others.
//!
//! A family such as Unifont can install a face made of bitmaps only. Its characters are mapped,
//! so the text layout picks it, but the renderer draws outlines: the text shows blank.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::thread::JoinHandle;

use slint::fontique_011::fontique::{
    Collection, FamilyId, FamilyInfo, FontInfo, GenericFamily, SourceKind,
};

/// Families the text layout falls back to when the font of the interface lacks a character.
const FALLBACKS: [GenericFamily; 2] = [GenericFamily::SansSerif, GenericFamily::SystemUi];

/// Tables that hold glyphs the renderer draws: outlines, or color bitmaps and layers.
const DRAWN_TABLES: [&[u8; 4]; 7] = [
    b"CFF ", b"CFF2", b"CBDT", b"COLR", b"SVG ", b"glyf", b"sbix",
];

/// Puts the fallback families that draw every text first, on a thread of its own: looking at
/// every font installed takes a while. Join it before the first text is laid out.
pub fn prefer_drawn_in_background() -> Option<JoinHandle<()>> {
    // Windows and macOS pick their fallback fonts themselves.
    if !cfg!(target_os = "linux") {
        return None;
    }
    let mut collection = slint::fontique_011::shared_collection();
    let spawned = std::thread::Builder::new()
        .name("fonts".into())
        .spawn(move || prefer_drawn(&mut collection));
    match spawned {
        Ok(thread) => Some(thread),
        Err(error) => {
            log::warn!("cannot look at the fallback fonts: {error}");
            None
        }
    }
}

/// Puts the families of the fallback lists of `collection` that draw every text before the
/// others, which stay as the last resort.
fn prefer_drawn(collection: &mut Collection) {
    for generic in FALLBACKS {
        let families: Vec<FamilyId> = collection.generic_families(generic).collect();
        let drawn: Vec<FamilyId> = families
            .iter()
            .copied()
            .filter(|id| {
                collection
                    .family(*id)
                    .is_none_or(|family| draws_all(&family))
            })
            .collect();
        if drawn.len() < families.len() {
            log::debug!(
                "{} fallback families of {generic:?} have a face without outlines",
                families.len() - drawn.len()
            );
            collection.set_generic_families(generic, drawn.into_iter());
        }
    }
}

fn draws_all(family: &FamilyInfo) -> bool {
    family.fonts().iter().all(draws)
}

/// Whether the renderer draws the glyphs of `font`; a font that cannot be read is trusted.
fn draws(font: &FontInfo) -> bool {
    match font.source().kind() {
        SourceKind::Path(path) => File::open(path)
            .and_then(|file| has_drawn_table(file, font.index()))
            .unwrap_or(true),
        SourceKind::Memory(data) => {
            has_drawn_table(io::Cursor::new(data.as_ref()), font.index()).unwrap_or(true)
        }
    }
}

fn read<const N: usize>(file: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Whether the font at `index` of an OpenType file holds a table of [`DRAWN_TABLES`] with
/// something in it: a font of bitmaps only can carry an empty `glyf`.
fn has_drawn_table(mut file: impl Read + Seek, index: u32) -> io::Result<bool> {
    if &read::<4>(&mut file)? == b"ttcf" {
        file.seek(SeekFrom::Start(12 + 4 * u64::from(index)))?;
        let offset = u32::from_be_bytes(read(&mut file)?);
        file.seek(SeekFrom::Start(u64::from(offset) + 4))?;
    }
    let tables = u16::from_be_bytes(read(&mut file)?);
    file.seek(SeekFrom::Current(6))?;
    for _ in 0..tables {
        let record = read::<16>(&mut file)?;
        let (tag, length) = (&record[..4], &record[12..]);
        if DRAWN_TABLES.iter().any(|drawn| *drawn == tag) && length != [0; 4] {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::fontique_011::fontique::{Blob, CollectionOptions, SourceCache};
    use std::sync::Arc;

    /// An OpenType header listing `tables`, as (tag, length).
    fn header(tables: &[(&[u8; 4], u32)]) -> Vec<u8> {
        let mut bytes = vec![0, 1, 0, 0];
        bytes.extend((tables.len() as u16).to_be_bytes());
        bytes.extend([0; 6]);
        for (tag, length) in tables {
            bytes.extend(*tag);
            bytes.extend([0; 8]);
            bytes.extend(length.to_be_bytes());
        }
        bytes
    }

    fn drawn(bytes: Vec<u8>, index: u32) -> Option<bool> {
        has_drawn_table(io::Cursor::new(bytes), index).ok()
    }

    #[test]
    fn a_font_of_bitmaps_only_draws_nothing() {
        let bitmaps = header(&[(b"EBDT", 900), (b"cmap", 40), (b"glyf", 0)]);
        assert_eq!(drawn(bitmaps, 0), Some(false));
        let without_glyf = header(&[(b"EBDT", 900), (b"cmap", 40)]);
        assert_eq!(drawn(without_glyf, 0), Some(false));
    }

    #[test]
    fn outlines_and_color_glyphs_are_drawn() {
        for tag in [b"glyf", b"CFF ", b"CFF2", b"CBDT"] {
            let font = header(&[(b"EBDT", 900), (tag, 12)]);
            assert_eq!(drawn(font, 0), Some(true), "{tag:?}");
        }
    }

    #[test]
    fn the_font_at_the_index_of_a_collection_is_read() {
        let bitmaps = header(&[(b"EBDT", 900)]);
        let outlines = header(&[(b"CFF2", 900)]);
        let mut bytes = b"ttcf\0\x01\0\0\0\0\0\x02".to_vec();
        let first = 20u32;
        bytes.extend(first.to_be_bytes());
        bytes.extend((first + bitmaps.len() as u32).to_be_bytes());
        bytes.extend(bitmaps);
        bytes.extend(outlines);
        assert_eq!(drawn(bytes.clone(), 0), Some(false));
        assert_eq!(drawn(bytes.clone(), 1), Some(true));
        assert_eq!(drawn(bytes, 1000), None);
    }

    /// The font of the interface with the tables in `without` renamed, so they look missing.
    fn inter(without: &[&[u8; 4]]) -> Vec<u8> {
        let mut bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/fonts/Inter-Regular.ttf"),
        )
        .unwrap();
        let tables = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
        for record in (0..tables).map(|index| 12 + 16 * index) {
            if without.iter().any(|tag| **tag == bytes[record..record + 4]) {
                bytes[record..record + 4].copy_from_slice(b"EBDT");
            }
        }
        bytes
    }

    fn register(collection: &mut Collection, data: Vec<u8>, name: &str) -> FamilyId {
        let info = slint::fontique_011::fontique::FontInfoOverride {
            family_name: Some(name),
            ..Default::default()
        };
        collection.register_fonts(Blob::new(Arc::new(data)), Some(info))[0].0
    }

    #[test]
    fn the_first_fallback_for_a_character_is_one_that_draws_it() {
        use slint::fontique_011::fontique::{QueryFamily, QueryStatus};

        let mut collection = Collection::new(CollectionOptions {
            shared: true,
            system_fonts: false,
        });
        let bitmaps = register(&mut collection, inter(&[b"glyf"]), "Bitmaps");
        let outlines = register(&mut collection, inter(&[]), "Outlines");
        for generic in FALLBACKS {
            collection.set_generic_families(generic, [bitmaps, outlines].into_iter());
        }
        // A clone shares the lists, as the collection of the renderer does.
        let mut shared = collection.clone();
        prefer_drawn(&mut collection);
        let mut source_cache = SourceCache::default();
        let mut first = None;
        let mut query = shared.query(&mut source_cache);
        query.set_families([QueryFamily::Generic(GenericFamily::SansSerif)]);
        query.matches_with(|font| {
            first = Some(font.family.0);
            QueryStatus::Stop
        });
        drop(query);
        assert_eq!(first, Some(outlines));
    }
}
