//! How the user wants subtitles to look, and how that reaches the player.

use oxyfin_player::SubtitleAppearance;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

/// Size of the subtitle text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleSize {
    /// Smaller than the player's default.
    Small,
    /// The player's default size.
    #[default]
    Normal,
    /// Larger than the default.
    Large,
    /// The largest size.
    ExtraLarge,
}

impl SubtitleSize {
    /// Every size, smallest first.
    pub const ALL: [Self; 4] = [Self::Small, Self::Normal, Self::Large, Self::ExtraLarge];

    fn scale(self) -> f64 {
        match self {
            Self::Small => 0.8,
            Self::Normal => 1.0,
            Self::Large => 1.25,
            Self::ExtraLarge => 1.5,
        }
    }
}

/// Color of the subtitle text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleColor {
    /// White text.
    #[default]
    White,
    /// Yellow text.
    Yellow,
}

impl SubtitleColor {
    /// Every color, in menu order.
    pub const ALL: [Self; 2] = [Self::White, Self::Yellow];

    /// Red, green and blue of the color.
    pub fn rgb(self) -> [u8; 3] {
        match self {
            Self::White => [0xFF, 0xFF, 0xFF],
            Self::Yellow => [0xFF, 0xE0, 0x00],
        }
    }
}

/// What keeps the subtitle text readable over the video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleBackground {
    /// A dark outline around the letters.
    #[default]
    Outline,
    /// A dark translucent box behind the text.
    Box,
}

impl SubtitleBackground {
    /// Every background, in menu order.
    pub const ALL: [Self; 2] = [Self::Outline, Self::Box];
}

/// Vertical position of the subtitles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubtitlePosition {
    /// At the bottom of the video, the player's default.
    #[default]
    Bottom,
    /// A tenth of the video height above the bottom.
    Raised,
}

impl SubtitlePosition {
    /// Every position, lowest first.
    pub const ALL: [Self; 2] = [Self::Bottom, Self::Raised];

    fn percent(self) -> f64 {
        match self {
            Self::Bottom => 100.0,
            Self::Raised => 90.0,
        }
    }
}

/// How subtitles look, saved in the configuration.
///
/// A value the configuration cannot read, such as one written by a newer
/// version, falls back to its default instead of failing the whole file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SubtitleStyle {
    /// Size of the text.
    #[serde(default, deserialize_with = "lenient")]
    pub size: SubtitleSize,
    /// Color of the text.
    #[serde(default, deserialize_with = "lenient")]
    pub color: SubtitleColor,
    /// Outline or box.
    #[serde(default, deserialize_with = "lenient")]
    pub background: SubtitleBackground,
    /// Vertical position.
    #[serde(default, deserialize_with = "lenient")]
    pub position: SubtitlePosition,
}

impl SubtitleStyle {
    /// What the player draws for this style.
    pub fn appearance(&self) -> SubtitleAppearance {
        SubtitleAppearance {
            scale: self.size.scale(),
            color: self.color.rgb(),
            background_box: self.background == SubtitleBackground::Box,
            position: self.position.percent(),
        }
    }
}

/// Deserializes `T`, or gives its default when the value does not fit.
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: DeserializeOwned + Default,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_style_is_the_players_default() {
        assert_eq!(
            SubtitleStyle::default().appearance(),
            SubtitleAppearance::default()
        );
    }

    #[test]
    fn each_choice_maps_to_the_player() {
        let style = SubtitleStyle {
            size: SubtitleSize::ExtraLarge,
            color: SubtitleColor::Yellow,
            background: SubtitleBackground::Box,
            position: SubtitlePosition::Raised,
        };
        assert_eq!(
            style.appearance(),
            SubtitleAppearance {
                scale: 1.5,
                color: [0xFF, 0xE0, 0x00],
                background_box: true,
                position: 90.0,
            }
        );
        let scales: Vec<f64> = SubtitleSize::ALL.iter().map(|s| s.scale()).collect();
        assert_eq!(scales, [0.8, 1.0, 1.25, 1.5]);
    }

    #[test]
    fn style_round_trips_as_snake_case() {
        let style = SubtitleStyle {
            size: SubtitleSize::ExtraLarge,
            color: SubtitleColor::Yellow,
            background: SubtitleBackground::Box,
            position: SubtitlePosition::Raised,
        };
        let json = serde_json::to_value(style).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "size": "extra_large",
                "color": "yellow",
                "background": "box",
                "position": "raised"
            })
        );
        assert_eq!(
            serde_json::from_value::<SubtitleStyle>(json).unwrap(),
            style
        );
    }

    #[test]
    fn unknown_or_missing_values_fall_back_to_defaults() {
        let style: SubtitleStyle =
            serde_json::from_str(r#"{"size":"huge","color":7,"position":"raised"}"#).unwrap();
        assert_eq!(
            style,
            SubtitleStyle {
                position: SubtitlePosition::Raised,
                ..SubtitleStyle::default()
            }
        );
    }
}
