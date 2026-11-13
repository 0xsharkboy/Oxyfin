//! Typed identifiers for Jellyfin entities.

use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Returns the identifier as a string slice.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

id_type!(
    /// Identifier of a library item.
    ItemId
);
id_type!(
    /// Identifier of a user.
    UserId
);
id_type!(
    /// Identifier of a media source.
    MediaSourceId
);
id_type!(
    /// Identifier of a playback session.
    PlaySessionId
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_serialize_as_plain_strings() {
        let id = ItemId::from("abc");
        assert_eq!(serde_json::to_string(&id).unwrap(), r#""abc""#);
        assert_eq!(serde_json::from_str::<ItemId>(r#""abc""#).unwrap(), id);
        assert_eq!(id.as_str(), "abc");
        assert_eq!(id.to_string(), "abc");
    }
}
