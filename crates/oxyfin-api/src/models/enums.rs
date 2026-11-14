//! Enumerations used by item models and queries.

use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
            /// A value this client does not know, kept as received.
            Other(String),
        }

        impl $name {
            /// The name used on the wire.
            pub fn as_str(&self) -> &str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                    Self::Other(value) => value,
                }
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $(stringify!($variant) => Self::$variant,)+
                    _ => Self::Other(value),
                }
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                match value {
                    $($name::$variant => stringify!($variant).to_owned(),)+
                    $name::Other(value) => value,
                }
            }
        }
    };
}

string_enum!(
    /// Kind of a library item.
    BaseItemKind {
        /// A movie.
        Movie,
        /// A standalone video, such as a home video.
        Video,
        /// A TV series.
        Series,
        /// A season of a series.
        Season,
        /// An episode of a season.
        Episode,
        /// A collection of items.
        BoxSet,
        /// A generic folder.
        Folder,
        /// The root folder of a library.
        CollectionFolder,
        /// A library view of the user.
        UserView,
    }
);

string_enum!(
    /// Kind of an item image.
    ImageType {
        /// The main poster or cover.
        Primary,
        /// A background image.
        Backdrop,
        /// A landscape thumbnail.
        Thumb,
        /// A title logo.
        Logo,
        /// A banner.
        Banner,
    }
);

string_enum!(
    /// Kind of a media stream inside a media source.
    MediaStreamType {
        /// A video track.
        Video,
        /// An audio track.
        Audio,
        /// A subtitle track.
        Subtitle,
    }
);

macro_rules! query_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum $name {
            $($(#[$vmeta])* $variant,)+
        }

        impl $name {
            /// The name used on the wire.
            pub fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                }
            }
        }
    };
}

query_enum!(
    /// Field an item list is sorted by.
    ItemSortBy {
        /// Sort name.
        SortName,
        /// Date the item was added.
        DateCreated,
        /// Production year.
        ProductionYear,
        /// Premiere date.
        PremiereDate,
        /// Community rating.
        CommunityRating,
        /// Date the item was last played.
        DatePlayed,
        /// Random order.
        Random,
    }
);

query_enum!(
    /// Direction of a sort.
    SortOrder {
        /// Smallest first.
        Ascending,
        /// Largest first.
        Descending,
    }
);

query_enum!(
    /// Optional item fields to request.
    ItemFields {
        /// Synopsis.
        Overview,
        /// Genres.
        Genres,
        /// Cast and crew.
        People,
        /// Date the item was added.
        DateCreated,
        /// Aspect ratio of the primary image.
        PrimaryImageAspectRatio,
        /// Number of children.
        ChildCount,
        /// Media streams.
        MediaStreams,
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_and_unknown_kinds_round_trip() {
        assert_eq!(BaseItemKind::from("Movie".to_owned()), BaseItemKind::Movie);
        let other = BaseItemKind::from("Trailer".to_owned());
        assert_eq!(other, BaseItemKind::Other("Trailer".into()));
        assert_eq!(other.as_str(), "Trailer");
        assert_eq!(String::from(BaseItemKind::BoxSet), "BoxSet");
    }
}
