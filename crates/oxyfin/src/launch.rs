//! Opt-in launch settings read from the environment, for headless agent runs.

/// Logical window size, as `WIDTHxHEIGHT`.
pub const WINDOW_SIZE_VAR: &str = "OXYFIN_WINDOW_SIZE";

/// Keeps tokens in memory instead of the system keyring when enabled.
pub const MEMORY_TOKENS_VAR: &str = "OXYFIN_MEMORY_TOKENS";

/// Scale factor of the window, for backends that read none from the system.
pub const SCALE_FACTOR_VAR: &str = "OXYFIN_SCALE_FACTOR";

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
