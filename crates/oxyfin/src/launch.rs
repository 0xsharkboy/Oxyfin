//! Launch settings: the command-line flags, and opt-in settings read from the
//! environment for headless agent runs.

use std::ffi::OsStr;

/// Command-line flag that opens the window fullscreen for this launch, whatever the setting.
pub const FULLSCREEN_FLAG: &str = "--fullscreen";

/// Command-line flags that print the version and exit.
pub const VERSION_FLAGS: [&str; 2] = ["--version", "-V"];

/// Logical window size, as `WIDTHxHEIGHT`.
pub const WINDOW_SIZE_VAR: &str = "OXYFIN_WINDOW_SIZE";

/// Keeps tokens in memory instead of the system keyring when enabled.
pub const MEMORY_TOKENS_VAR: &str = "OXYFIN_MEMORY_TOKENS";

/// Scale factor of the window, for backends that read none from the system.
pub const SCALE_FACTOR_VAR: &str = "OXYFIN_SCALE_FACTOR";

/// Whether the command-line arguments, without the program name, ask for fullscreen.
pub fn fullscreen_requested<A: AsRef<OsStr>>(args: impl IntoIterator<Item = A>) -> bool {
    args.into_iter()
        .any(|arg| arg.as_ref() == OsStr::new(FULLSCREEN_FLAG))
}

/// Whether the command-line arguments, without the program name, ask for the version.
pub fn version_requested<A: AsRef<OsStr>>(args: impl IntoIterator<Item = A>) -> bool {
    args.into_iter().any(|arg| {
        VERSION_FLAGS
            .iter()
            .any(|flag| arg.as_ref() == OsStr::new(flag))
    })
}

/// The line printed for the version flags, such as `oxyfin 0.1.0`.
pub fn version_line() -> String {
    format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
}

/// Parses a positive, finite scale factor.
pub fn scale_factor(value: &str) -> Option<f32> {
    value
        .trim()
        .parse()
        .ok()
        .filter(|&factor: &f32| factor.is_finite() && factor > 0.0)
}

/// Parses `WIDTHxHEIGHT` into a non-zero size.
pub fn window_size(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.trim().split_once('x')?;
    let width = width.parse().ok().filter(|&w: &u32| w > 0)?;
    let height = height.parse().ok().filter(|&h: &u32| h > 0)?;
    Some((width, height))
}

/// Whether a flag variable is set to a true value.
pub fn enabled(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let value = value.trim();
        ["1", "true", "yes"]
            .iter()
            .any(|on| value.eq_ignore_ascii_case(on))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fullscreen_flag_is_found_among_the_arguments() {
        assert!(fullscreen_requested(["--fullscreen"]));
        assert!(fullscreen_requested(["--verbose", "--fullscreen"]));
        assert!(!fullscreen_requested(Vec::<String>::new()));
        assert!(!fullscreen_requested([
            "--fullscreen=no",
            "fullscreen",
            "-f"
        ]));
    }

    #[test]
    fn the_version_flags_are_found_among_the_arguments() {
        assert!(version_requested(["--version"]));
        assert!(version_requested(["--fullscreen", "-V"]));
        assert!(!version_requested(Vec::<String>::new()));
        assert!(!version_requested(["--version=1", "version", "-v"]));
    }

    #[test]
    fn the_version_line_names_the_binary_and_its_version() {
        assert_eq!(
            version_line(),
            format!("oxyfin {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn window_sizes_parse() {
        assert_eq!(window_size("900x700"), Some((900, 700)));
        assert_eq!(window_size(" 900x700 "), Some((900, 700)));
        for bad in ["900", "0x700", "900x0", "axb", ""] {
            assert_eq!(window_size(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn scale_factors_parse() {
        assert_eq!(scale_factor("2"), Some(2.0));
        assert_eq!(scale_factor(" 1.5 "), Some(1.5));
        for bad in ["0", "-1", "abc", "", "NaN", "inf"] {
            assert_eq!(scale_factor(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn flags_parse() {
        for on in ["1", "true", "yes", "TRUE", " Yes "] {
            assert!(enabled(Some(on)), "{on:?}");
        }
        for off in [None, Some(""), Some("0"), Some("false")] {
            assert!(!enabled(off), "{off:?}");
        }
    }
}
