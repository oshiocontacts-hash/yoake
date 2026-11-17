//! Which Windows icon a Horizon build embeds. `launcher/build.rs` and
//! `crates/horizon-client/build.rs` both include this file with `#[path]`, so the launcher and the
//! game always agree; the launcher's tests include it too (`launcher/src/lib.rs`).

/// `yoake.ico` for a Yoake build, `horizon.ico` for an upstream one. A Yoake build is one with
/// `YOAKE_FEED_URL` or `YOAKE_PORTAL_URL` set (set, even if empty).
#[allow(dead_code)]
pub fn icon_file(feed_url_set: bool, portal_url_set: bool) -> &'static str {
    if feed_url_set || portal_url_set {
        "yoake.ico"
    } else {
        "horizon.ico"
    }
}

/// The same, read from this build's environment.
#[allow(dead_code)]
pub fn icon_file_from_env() -> &'static str {
    icon_file(
        std::env::var_os("YOAKE_FEED_URL").is_some(),
        std::env::var_os("YOAKE_PORTAL_URL").is_some(),
    )
}

#[cfg(test)]
mod tests {
    use super::icon_file;

    #[test]
    fn an_upstream_build_keeps_the_horizon_icon() {
        assert_eq!(icon_file(false, false), "horizon.ico");
    }

    #[test]
    fn a_portal_url_picks_the_yoake_icon() {
        assert_eq!(icon_file(false, true), "yoake.ico");
    }

    #[test]
    fn a_feed_url_picks_the_yoake_icon() {
        assert_eq!(icon_file(true, false), "yoake.ico");
        assert_eq!(icon_file(true, true), "yoake.ico");
    }

    #[test]
    fn both_icons_exist() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packaging/icons");
        for file in [icon_file(false, false), icon_file(true, true)] {
            assert!(dir.join(file).exists(), "{file} is missing");
        }
    }
}
