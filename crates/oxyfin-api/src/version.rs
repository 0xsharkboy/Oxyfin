//! Server version.

use crate::Error;
use std::fmt;
use std::str::FromStr;

/// Oldest Jellyfin server version supported by this client.
pub const MINIMUM_SERVER_VERSION: ServerVersion = ServerVersion {
    major: 10,
    minor: 10,
    patch: 0,
};

/// A Jellyfin server version, ordered by major, minor then patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ServerVersion {
    /// Major version.
    pub major: u32,
    /// Minor version.
    pub minor: u32,
    /// Patch version.
    pub patch: u32,
}

impl FromStr for ServerVersion {
    type Err = Error;

    /// Parses `major.minor[.patch[.build]]`; a missing patch is 0 and a fourth part is ignored.
    fn from_str(s: &str) -> Result<Self, Error> {
        let invalid = || Error::InvalidVersion(s.to_owned());
        let mut parts = s.split('.');
        let mut next = |required: bool| match parts.next() {
            Some(part) => part.parse::<u32>().map_err(|_| invalid()),
            None if required => Err(invalid()),
            None => Ok(0),
        };
        let major = next(true)?;
        let minor = next(true)?;
        let patch = next(false)?;
        Ok(Self {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for ServerVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    fn v(major: u32, minor: u32, patch: u32) -> ServerVersion {
        ServerVersion {
            major,
            minor,
            patch,
        }
    }

    #[test]
    fn version_parses_three_and_two_parts() {
        assert_eq!("10.11.11".parse::<ServerVersion>().unwrap(), v(10, 11, 11));
        assert_eq!("12.4.0".parse::<ServerVersion>().unwrap(), v(12, 4, 0));
        assert_eq!("12.4".parse::<ServerVersion>().unwrap(), v(12, 4, 0));
        assert_eq!("10.11.0.0".parse::<ServerVersion>().unwrap(), v(10, 11, 0));
    }

    #[test]
    fn version_rejects_garbage() {
        for s in ["", "abc", "10.x"] {
            assert!(
                matches!(s.parse::<ServerVersion>(), Err(Error::InvalidVersion(_))),
                "{s}"
            );
        }
    }

    #[test]
    fn version_ordering() {
        assert!(v(10, 9, 11) < v(10, 10, 0));
        assert!(v(10, 10, 0) < v(10, 11, 11));
        assert!(v(10, 11, 11) < v(12, 4, 0));
    }
}
