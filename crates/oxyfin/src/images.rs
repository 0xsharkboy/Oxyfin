//! Image loading: bytes from the cache decoded off the Slint thread, then shown as a `slint::Image`.

use std::sync::Arc;

use oxyfin_core::{ImageCache, ImageRef, ImageSize};
use slint::{Rgba8Pixel, SharedPixelBuffer};
use tokio::runtime::Handle;

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
/// with the result on the Slint thread. Nothing is called when either step fails.
pub fn load_into(
    images: &Arc<ImageCache>,
    runtime: &Handle,
    image: ImageRef,
    size: ImageSize,
    apply: impl FnOnce(slint::Image) + Send + 'static,
) {
    let images = Arc::clone(images);
    runtime.spawn(async move {
        let bytes = match images.get(&image, size).await {
            Ok(bytes) => bytes,
            Err(error) => {
                log::debug!(
                    "cannot fetch image {:?} of {}: {error}",
                    image.image_type,
                    image.item_id
                );
                return;
            }
        };
        let Ok(Some(pixels)) = tokio::task::spawn_blocking(move || decode(&bytes)).await else {
            log::debug!(
                "cannot decode image {:?} of {}",
                image.image_type,
                image.item_id
            );
            return;
        };
        let shown = slint::invoke_from_event_loop(move || apply(slint::Image::from_rgba8(pixels)));
        if let Err(error) = shown {
            log::debug!("cannot hand an image to the event loop: {error}");
        }
    });
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

    #[test]
    fn svg_does_not_decode() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><rect width="4" height="4"/></svg>"#;
        assert!(decode(svg).is_none());
    }
}
