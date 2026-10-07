//! Reads Unity version strings such as `2019.4.41f2` and orders them the
//! way Unity released them.

use std::cmp::Ordering;

/// Splits a Unity version into its numbers: major, minor, patch and the
/// number after the release letter. `2019.4.41f2` gives `[2019, 4, 41, 2]`.
pub(crate) fn parts(version: &str) -> [u32; 4] {
    let mut out = [0; 4];
    for (slot, part) in out.iter_mut().zip(version.split(['.', 'a', 'b', 'f', 'p'])) {
        *slot = part.parse().unwrap_or(0);
    }
    out
}

/// Orders two versions by their numbers.
pub(crate) fn order(a: &str, b: &str) -> Ordering {
    parts(a).cmp(&parts(b))
}

/// Tells a full release, `f`, from an alpha, a beta or a patch release.
pub(crate) fn is_release(version: &str) -> bool {
    version.contains('f')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_read_every_number() {
        assert_eq!(parts("2019.4.41f2"), [2019, 4, 41, 2]);
        assert_eq!(parts("6000.5.0f1"), [6000, 5, 0, 1]);
        assert_eq!(parts("5.6.7f1"), [5, 6, 7, 1]);
    }

    #[test]
    fn versions_order_by_number_not_by_text() {
        assert_eq!(order("2019.4.9f1", "2019.4.41f2"), Ordering::Less);
        assert_eq!(order("6000.0.0f1", "2023.2.22f1"), Ordering::Greater);
        assert_eq!(order("2018.2.0f1", "2018.2.0f2"), Ordering::Less);
    }

    #[test]
    fn only_f_versions_are_releases() {
        assert!(is_release("2019.4.41f2"));
        assert!(!is_release("2019.4.0b3"));
        assert!(!is_release("2017.1.0p4"));
    }
}
