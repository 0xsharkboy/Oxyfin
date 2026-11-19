//! Image loading: bytes from the cache decoded off the Slint thread, then shown as a `slint::Image`.

use std::collections::{HashSet, VecDeque};
use std::rc::Rc;
use std::sync::Arc;

use crate::app::App;
use crate::state::{GenerationId, Msg};
use oxyfin_core::{CoreError, ImageCache, ImageRef, ImageSize, TrickplaySheet};
use slint::{Rgba8Pixel, SharedPixelBuffer};

/// Decodes JPEG, PNG or WebP bytes into RGBA pixels; `None` for anything else.
pub fn decode(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let image = image::load_from_memory(bytes).ok()?.into_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(
        image.as_raw(),
        image.width(),
        image.height(),
    ))
}

/// Fetches `image` at `size` and decodes it on the runtime, then calls `apply`
/// with the result on the Slint thread. A fetch rejected with 401 ends the
/// session of generation `id`; any other failure shows nothing.
pub fn load_into(
    app: &App,
    id: GenerationId,
    images: &Arc<ImageCache>,
    image: ImageRef,
    size: ImageSize,
    apply: impl FnOnce(slint::Image) + 'static,
) {
    let images = Arc::clone(images);
    app.spawn(
        async move {
            let bytes = match images.get(&image, size).await {
                Ok(bytes) => bytes,
                Err(CoreError::SessionExpired) => return Loaded::Expired,
                Err(error) => {
                    log::debug!(
                        "cannot fetch image {:?} of {}: {error}",
                        image.image_type,
                        image.item_id
                    );
                    return Loaded::Failed;
                }
            };
            match tokio::task::spawn_blocking(move || decode(&bytes)).await {
                Ok(Some(pixels)) => Loaded::Pixels(pixels),
                _ => {
                    log::debug!(
                        "cannot decode image {:?} of {}",
                        image.image_type,
                        image.item_id
                    );
                    Loaded::Failed
                }
            }
        },
        move |app, loaded| match loaded {
            Loaded::Pixels(pixels) => apply(slint::Image::from_rgba8(pixels)),
            Loaded::Expired => app.dispatch(Msg::Expired(id)),
            Loaded::Failed => {}
        },
    );
}

/// Fetches a trickplay sheet and decodes it on the runtime, then calls
/// `apply` with the result on the Slint thread, `None` on a failure. A fetch
/// rejected with 401 ends the session of generation `id` instead.
pub fn load_sheet(
    app: &App,
    id: GenerationId,
    images: &Arc<ImageCache>,
    sheet: TrickplaySheet,
    apply: impl FnOnce(&Rc<App>, Option<slint::Image>) + 'static,
) {
    let images = Arc::clone(images);
    app.spawn(
        async move {
            let bytes = match images.trickplay_sheet(&sheet).await {
                Ok(bytes) => bytes,
                Err(CoreError::SessionExpired) => return Loaded::Expired,
                Err(error) => {
                    log::debug!("cannot fetch trickplay sheet {sheet:?}: {error}");
                    return Loaded::Failed;
                }
            };
            match tokio::task::spawn_blocking(move || decode(&bytes)).await {
                Ok(Some(pixels)) => Loaded::Pixels(pixels),
                _ => {
                    log::debug!("cannot decode trickplay sheet {sheet:?}");
                    Loaded::Failed
                }
            }
        },
        move |app, loaded| match loaded {
            Loaded::Pixels(pixels) => apply(app, Some(slint::Image::from_rgba8(pixels))),
            Loaded::Expired => {
                apply(app, None);
                app.dispatch(Msg::Expired(id));
            }
            Loaded::Failed => apply(app, None),
        },
    );
}

/// Decoded trickplay sheets of the seek bar preview: the last
/// [`DECODED_SHEETS`] shown, and the ones being loaded.
///
/// A decoded sheet of 10 by 10 thumbnails 320 pixels wide takes 23 MB, so
/// few are kept; the encoded ones stay in the [`ImageCache`].
#[derive(Default)]
pub struct SheetCache {
    decoded: VecDeque<(TrickplaySheet, slint::Image)>,
    loading: HashSet<TrickplaySheet>,
}

/// Number of decoded trickplay sheets kept.
pub const DECODED_SHEETS: usize = 4;

impl SheetCache {
    /// The decoded `sheet`, if kept; it becomes the most recent one.
    pub fn get(&mut self, sheet: &TrickplaySheet) -> Option<slint::Image> {
        let index = self.decoded.iter().position(|(kept, _)| kept == sheet)?;
        let entry = self.decoded.remove(index)?;
        let image = entry.1.clone();
        self.decoded.push_back(entry);
        Some(image)
    }

    /// Marks `sheet` as loading; false when it already is.
    pub fn start(&mut self, sheet: &TrickplaySheet) -> bool {
        self.loading.insert(sheet.clone())
    }

    /// Ends the load of `sheet`, keeping its image when it has one.
    pub fn finish(&mut self, sheet: &TrickplaySheet, image: Option<slint::Image>) {
        self.loading.remove(sheet);
        if let Some(image) = image {
            self.decoded.retain(|(kept, _)| kept != sheet);
            self.decoded.push_back((sheet.clone(), image));
            while self.decoded.len() > DECODED_SHEETS {
                self.decoded.pop_front();
            }
        }
    }

    /// Drops every decoded sheet, to free their memory after playback.
    pub fn clear(&mut self) {
        self.decoded.clear();
    }
}

enum Loaded {
    Pixels(SharedPixelBuffer<Rgba8Pixel>),
    Expired,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, Rgba([10, 20, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    #[test]
    fn decodes_a_png_with_its_size() {
        let buffer = decode(&png(3, 2)).unwrap();
        assert_eq!((buffer.width(), buffer.height()), (3, 2));
        assert_eq!(buffer.as_slice()[0], Rgba8Pixel::new(10, 20, 30, 255));
    }

    #[test]
    fn truncated_bytes_do_not_decode() {
        let bytes = png(16, 16);
        assert!(decode(&bytes[..bytes.len() / 2]).is_none());
    }

    fn sheet(index: u32) -> TrickplaySheet {
        TrickplaySheet {
            item: "item".into(),
            media_source: "source".into(),
            resolution: 320,
            index,
            layout: (10_000, 10, 10),
        }
    }

    fn image(width: u32) -> slint::Image {
        slint::Image::from_rgba8(SharedPixelBuffer::new(width, 1))
    }

    #[test]
    fn sheet_cache_keeps_four_sheets() {
        let mut cache = SheetCache::default();
        assert!(cache.start(&sheet(0)));
        assert!(!cache.start(&sheet(0)), "already loading");
        for index in 0..5 {
            cache.finish(&sheet(index), Some(image(index + 1)));
        }
        assert!(cache.get(&sheet(0)).is_none(), "the least recent goes");
        for index in 1..5 {
            assert_eq!(
                cache.get(&sheet(index)).map(|i| i.size().width),
                Some(index + 1)
            );
        }
        assert!(cache.get(&sheet(1)).is_some());
        cache.finish(&sheet(5), Some(image(6)));
        assert!(cache.get(&sheet(2)).is_none(), "a sheet shown again stays");
        assert!(cache.get(&sheet(1)).is_some());
        cache.clear();
        assert!(cache.get(&sheet(1)).is_none());
    }

    #[test]
    #[ignore = "timing, run by hand"]
    fn decodes_a_full_sheet_quickly() {
        let pixels: Vec<u8> = (0..3200u32 * 1800 * 3).map(|i| (i % 251) as u8).collect();
        let mut bytes = Cursor::new(Vec::new());
        image::RgbImage::from_raw(3200, 1800, pixels)
            .unwrap()
            .write_to(&mut bytes, ImageFormat::Jpeg)
            .unwrap();
        let bytes = bytes.into_inner();
        let start = std::time::Instant::now();
        let buffer = decode(&bytes).unwrap();
        let took = start.elapsed();
        eprintln!("decoded a 3200x1800 sheet in {took:?}");
        assert_eq!((buffer.width(), buffer.height()), (3200, 1800));
        assert!(took < std::time::Duration::from_millis(400), "{took:?}");
    }

    #[test]
    fn a_failed_sheet_can_be_loaded_again() {
        let mut cache = SheetCache::default();
        assert!(cache.start(&sheet(4)));
        cache.finish(&sheet(4), None);
        assert!(cache.get(&sheet(4)).is_none());
        assert!(cache.start(&sheet(4)));
    }

    #[test]
    fn svg_does_not_decode() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#;
        assert!(decode(svg).is_none());
    }
}
