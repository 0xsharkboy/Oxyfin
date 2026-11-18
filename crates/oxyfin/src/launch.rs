//! Opt-in launch settings read from the environment, for headless agent runs.

/// Logical window size, as `WIDTHxHEIGHT`.
pub const WINDOW_SIZE_VAR: &str = "OXYFIN_WINDOW_SIZE";

/// Keeps tokens in memory instead of the system keyring when enabled.
pub const MEMORY_TOKENS_VAR: &str = "OXYFIN_MEMORY_TOKENS";

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
    fn window_sizes_parse() {
        assert_eq!(window_size("900x700"), Some((900, 700)));
        assert_eq!(window_size(" 900x700 "), Some((900, 700)));
        for bad in ["900", "0x700", "900x0", "axb", ""] {
            assert_eq!(window_size(bad), None, "{bad:?}");
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
