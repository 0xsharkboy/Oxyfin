//! Item images: URLs and downloads.

use crate::{ImageType, ItemId, Result, Session};
use url::Url;

/// Which image of an item to address, and how to size it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRequest {
    /// Item owning the image.
    pub item_id: ItemId,
    /// Kind of image.
    pub image_type: ImageType,
    /// Image tag, for cache busting.
    pub tag: Option<String>,
    /// Maximum width in pixels.
    pub max_width: Option<u32>,
    /// Maximum height in pixels.
    pub max_height: Option<u32>,
    /// JPEG quality from 0 to 100, passed as is.
    pub quality: Option<u8>,
}

impl ImageRequest {
    /// A request for the image of `image_type` of `item_id`, with no tag or sizing.
    pub fn new(item_id: ItemId, image_type: ImageType) -> Self {
        Self {
            item_id,
            image_type,
            tag: None,
            max_width: None,
            max_height: None,
            quality: None,
        }
    }

    fn route(&self) -> String {
        format!("Items/{}/Images/{}", self.item_id, self.image_type.as_str())
    }

    fn params(&self) -> Vec<(&'static str, String)> {
        let mut params = Vec::new();
        if let Some(tag) = &self.tag {
            params.push(("tag", tag.clone()));
        }
        if let Some(width) = self.max_width {
            params.push(("maxWidth", width.to_string()));
        }
        if let Some(height) = self.max_height {
            params.push(("maxHeight", height.to_string()));
        }
        if let Some(quality) = self.quality {
            params.push(("quality", quality.to_string()));
        }
        params
    }
}

impl Session {
    /// URL of an image, without credentials; authenticate with [`Session::fetch_image`].
    pub fn image_url(&self, request: &ImageRequest) -> Url {
        self.server()
            .http()
            .url(&request.route(), &request.params())
    }

    /// Downloads an image (`GET Items/{id}/Images/{type}`).
    pub async fn fetch_image(&self, request: &ImageRequest) -> Result<Vec<u8>> {
        self.server()
            .http()
            .get_bytes(Some(self.token()), &request.route(), &request.params())
            .await
    }
}
