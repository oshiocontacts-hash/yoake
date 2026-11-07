pub use crate::crash::mod_guard::*;

use serde_json::{Value, json};

pub fn notice_text(id: &str, hook: &str) -> crate::text::Text {
    crate::text::Text::translate_with_fallback(
        "horizon.mod_guard.stopped",
        "Mod %s stopped after an error (%s). The game goes on without it.",
        &[
            crate::text::Text::literal(id),
            crate::text::Text::literal(hook),
        ],
    )
}

fn cost_window(window: CostWindow) -> Value {
    json!({
        "calls": window.calls,
        "total_ns": window.total_ns,
        "max_ns": window.max_ns,
    })
}

fn profile_json() -> Value {
    let costs: Vec<_> = profile_snapshot()
        .into_iter()
        .map(|cost| {
            json!({
                "id": cost.id,
                "index": cost.index,
                "hook": cost.hook,
                "thread": cost.thread.as_str(),
                "last_second": cost_window(cost.last_second),
                "since_enabled": cost_window(cost.since_enabled),
            })
        })
        .collect();
    json!({ "enabled": profile_enabled(), "costs": costs })
}

pub fn control_command(line: &str) -> Option<Value> {
    let mut words = line.split_whitespace();
    match words.next()? {
        "mods" => {
            if words.next().is_some() {
                return Some(json!({ "error": "usage: mods" }));
            }
            let mods: Vec<_> = mod_statuses()
                .into_iter()
                .map(|status| {
                    let first_error = status
                        .first_error
                        .map(|error| json!({ "hook": error.hook, "message": error.message }));
                    json!({
                        "id": status.id,
                        "index": status.index,
                        "enabled": status.enabled,
                        "first_error": first_error,
                    })
                })
                .collect();
            Some(json!({ "mods": mods }))
        }
        "mod-profile" => {
            let action = words.next();
            if words.next().is_some() || !matches!(action, Some("on" | "off" | "get")) {
                return Some(json!({ "error": "usage: mod-profile on|off|get" }));
            }
            if action != Some("get") {
                if let Err(error) = set_profile_enabled(action == Some("on")) {
                    return Some(json!({ "error": error }));
                }
            }
            Some(profile_json())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_handles_its_commands_and_checks_arguments() {
        assert!(control_command("").is_none());
        assert!(control_command("status").is_none());
        for command in [
            "mods extra",
            "mod-profile",
            "mod-profile yes",
            "mod-profile get extra",
        ] {
            assert!(control_command(command).unwrap()["error"].is_string());
        }
        assert!(control_command("mods").unwrap()["mods"].is_array());
        let profile = control_command("mod-profile get").unwrap();
        assert!(profile["enabled"].is_boolean());
        assert!(profile["costs"].is_array());
    }

    #[test]
    fn player_notice_preserves_the_translation_and_fallback() {
        let text = notice_text("broken", "level_tick_pre");
        let component = text.component();
        assert_eq!(component["translate"], "horizon.mod_guard.stopped");
        assert_eq!(
            component["fallback"],
            "Mod %s stopped after an error (%s). The game goes on without it.",
        );
        assert_eq!(component["with"].as_array().unwrap().len(), 2);
    }
}
