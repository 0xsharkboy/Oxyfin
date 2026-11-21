//! Subtitles found online by the server's subtitle providers.

use crate::{ItemId, Result, Session};
use serde::Deserialize;
use serde::de::IgnoredAny;

/// A subtitle file a provider of the server offers for an item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
pub struct RemoteSubtitleInfo {
    /// Identifier to download it with.
    pub id: String,
    /// Name of the file, as the provider gives it.
    pub name: String,
    /// Provider offering it, such as `Open Subtitles`.
    pub provider_name: Option<String>,
    /// File format, such as `srt`.
    pub format: Option<String>,
    /// How many times it was downloaded from the provider.
    pub download_count: Option<u64>,
    /// Whether it only covers foreign or written parts.
    #[serde(rename = "Forced")]
    pub is_forced: Option<bool>,
    /// Whether it describes sounds for the deaf and hard of hearing.
    #[serde(rename = "HearingImpaired")]
    pub is_hearing_impaired: Option<bool>,
    /// Whether the provider matched it on the hash of the file itself.
    #[serde(rename = "IsHashMatch")]
    pub is_perfect_match: Option<bool>,
}

impl Session {
    /// Asks the server's subtitle providers for subtitles of an item in `language`, an
    /// ISO 639-2 code such as `fre` (`GET Items/{id}/RemoteSearch/Subtitles/{language}`).
    pub async fn search_subtitles(
        &self,
        item: &ItemId,
        language: &str,
    ) -> Result<Vec<RemoteSubtitleInfo>> {
        self.server()
            .http()
            .get(
                Some(self.token()),
                &format!("Items/{item}/RemoteSearch/Subtitles/{language}"),
                &[],
            )
            .await
    }

    /// Makes the server download a subtitle found by [`Session::search_subtitles`] and keep
    /// it with the item (`POST Items/{id}/RemoteSearch/Subtitles/{subtitleId}`).
    ///
    /// The server lists the new subtitle stream once it has refreshed the item.
    pub async fn download_subtitle(&self, item: &ItemId, id: &str) -> Result<()> {
        let _: IgnoredAny = self
            .server()
            .http()
            .post_empty(
                Some(self.token()),
                &format!("Items/{item}/RemoteSearch/Subtitles/{id}"),
                &[],
            )
            .await?;
        Ok(())
    }
}
