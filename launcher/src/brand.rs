use std::borrow::Cow;

pub fn yoake() -> bool {
    crate::account::owner_required()
}

pub fn name() -> &'static str {
    if yoake() {
        "Yoake"
    } else {
        "Horizon"
    }
}

pub fn launcher_name() -> &'static str {
    if yoake() {
        "Yoake Launcher"
    } else {
        "Horizon Launcher"
    }
}

pub fn text(value: &str) -> Cow<'_, str> {
    translate(value, value, yoake())
}

pub fn show_url(url: &str) -> bool {
    show_url_in_brand(url, yoake())
}

fn show_url_in_brand(url: &str, yoake: bool) -> bool {
    !yoake || !url.to_ascii_lowercase().contains("playhorizon.dev")
}

pub(crate) fn translate<'a>(source: &str, value: &'a str, active: bool) -> Cow<'a, str> {
    const KEEP_HORIZON: &[&str] = &["Yoake Launcher is built on Horizon (licence below)"];
    if !active || KEEP_HORIZON.contains(&source) {
        return Cow::Borrowed(value);
    }

    let mut output = String::new();
    let mut copied = 0;
    for (index, _) in value.match_indices("Horizon") {
        let before = value[..index].chars().next_back();
        let after = value[index + "Horizon".len()..].chars().next();
        let identifier_char =
            |character: char| character.is_ascii_alphanumeric() || character == '_';
        if before.is_some_and(identifier_char) || after.is_some_and(identifier_char) {
            continue;
        }
        output.push_str(&value[copied..index]);
        output.push_str("Yoake");
        copied = index + "Horizon".len();
    }
    if copied == 0 && !value.contains("ホライズン") && !value.contains("ホライゾン") {
        return Cow::Borrowed(value);
    }
    output.push_str(&value[copied..]);
    Cow::Owned(
        output
            .replace("ホライズン", "Yoake")
            .replace("ホライゾン", "Yoake"),
    )
}

#[cfg(test)]
mod tests {
    use super::{launcher_name, name, show_url_in_brand, translate, yoake};

    #[test]
    fn upstream_urls_are_hidden_only_in_yoake() {
        assert!(!show_url_in_brand("PLAYHORIZON.dev/mods", true));
        assert!(!show_url_in_brand("mcp.playhorizon.dev", true));
        assert!(show_url_in_brand("example.org", true));
        assert!(show_url_in_brand("playhorizon.dev", false));
    }

    #[test]
    fn switch_matches_owner_requirement() {
        assert_eq!(yoake(), crate::account::owner_required());
        assert_eq!(name(), if yoake() { "Yoake" } else { "Horizon" });
        assert_eq!(
            launcher_name(),
            if yoake() {
                "Yoake Launcher"
            } else {
                "Horizon Launcher"
            }
        );
    }

    #[test]
    fn substitutes_whole_words_only() {
        assert_eq!(
            translate(
                "",
                "Horizon Launcher, Horizon's Horizon_id and SuperHorizon",
                true
            ),
            "Yoake Launcher, Yoake's Horizon_id and SuperHorizon"
        );
        assert_eq!(
            translate("", "ホライズンとホライゾン", true),
            "YoakeとYoake"
        );
        assert_eq!(translate("", "このHorizonを開く", true), "このYoakeを開く");
    }

    #[test]
    fn preserves_attribution_in_every_language() {
        let attribution = "Yoake Launcher is built on Horizon (licence below)";
        assert_eq!(translate(attribution, attribution, true), attribution);
        assert_eq!(
            translate(attribution, "Horizon のライセンス", true),
            "Horizon のライセンス"
        );
    }

    #[test]
    fn upstream_text_is_unchanged() {
        assert_eq!(
            translate("", "Horizon Launcher ホライズン", false),
            "Horizon Launcher ホライズン"
        );
    }
}
