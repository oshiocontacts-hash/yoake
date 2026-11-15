use std::collections::BTreeMap;

use horizon_client::mod_list::{ConfigField as UiField, ConfigKind as UiKind, ConfigPage};
use horizon_client::settings::Settings;
use horizon_mods::yak::{ConfigField, ConfigKind, ConfigRuntimeSide, ConfigSide};
use toml::Value;

use super::package;

pub(crate) fn populate(settings: &mut Settings, server_editable: bool) {
    settings.mods.configs.clear();
    for entry in crate::yak_mods::entries() {
        if entry.manifest.config.is_empty() {
            continue;
        }
        let mut page = ConfigPage::default();
        for (key, field) in &entry.manifest.config {
            let side = runtime_side(field, server_editable);
            let Ok(handle) = package::settings_services(&entry.manifest.id, side) else {
                continue;
            };
            let Ok(service) = handle.lock() else {
                continue;
            };
            let Some(value) = service.settings_value(key) else {
                continue;
            };
            page.fields.push(UiField {
                key: key.clone(),
                label: field
                    .label
                    .get(&settings.language)
                    .or_else(|| field.label.get("en_us"))
                    .cloned()
                    .unwrap_or_else(|| key.clone()),
                value: display_value(value),
                kind: match field.kind {
                    ConfigKind::Bool => UiKind::Bool,
                    ConfigKind::Int | ConfigKind::Float => UiKind::Number,
                    ConfigKind::String => UiKind::Text,
                    ConfigKind::Enum => UiKind::Enum,
                    ConfigKind::List => UiKind::List,
                },
                choices: field.values.clone(),
                read_only: field.side != ConfigSide::Client && !server_editable,
                restart: field.restart,
                range: match (&field.min, &field.max) {
                    (Some(min), Some(max)) => Some(format!("{min} .. {max}")),
                    (Some(min), None) => Some(format!("{min} ..")),
                    (None, Some(max)) => Some(format!(".. {max}")),
                    (None, None) => None,
                },
            });
        }
        settings
            .mods
            .configs
            .insert(entry.manifest.id.clone(), page);
    }
}

pub(crate) fn save(settings: &Settings, index: usize, server_editable: bool) -> Result<(), String> {
    let row = settings.mods.installed.get(index).ok_or("unknown mod")?;
    let entry = crate::yak_mods::entries()
        .iter()
        .find(|entry| entry.manifest.id == row.id)
        .ok_or("unknown package")?;
    let page = settings
        .mods
        .configs
        .get(&row.id)
        .ok_or("missing config page")?;
    let mut client = BTreeMap::new();
    let mut server = BTreeMap::new();
    for edited in &page.fields {
        let field = entry
            .manifest
            .config
            .get(&edited.key)
            .ok_or("unknown config key")?;
        if field.side != ConfigSide::Client && !server_editable {
            continue;
        }
        let side = runtime_side(field, server_editable);
        let handle = package::settings_services(&row.id, side)?;
        let service = handle.lock().map_err(|_| "package services unavailable")?;
        if service
            .settings_value(&edited.key)
            .map(display_value)
            .as_deref()
            == Some(edited.value.as_str())
        {
            continue;
        }
        let value = parse_value(field, &edited.value)
            .map_err(|error| format!("{}: {error}", edited.label))?;
        if side == ConfigRuntimeSide::Server {
            server.insert(edited.key.clone(), value);
        } else {
            client.insert(edited.key.clone(), value);
        }
    }
    for (side, values) in [
        (ConfigRuntimeSide::Client, client),
        (ConfigRuntimeSide::Server, server),
    ] {
        if !values.is_empty() {
            package::settings_services(&row.id, side)?
                .lock()
                .map_err(|_| "package services unavailable")?
                .save_config(&values)?;
        }
    }
    Ok(())
}

fn runtime_side(field: &ConfigField, server_editable: bool) -> ConfigRuntimeSide {
    if field.side != ConfigSide::Client && server_editable {
        ConfigRuntimeSide::Server
    } else {
        ConfigRuntimeSide::Client
    }
}

fn display_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(values) => values
            .iter()
            .map(display_value)
            .collect::<Vec<_>>()
            .join(", "),
        value => value.to_string(),
    }
}

fn parse_value(field: &ConfigField, text: &str) -> Result<Value, String> {
    let scalar = |kind, text: &str| -> Result<Value, String> {
        match kind {
            ConfigKind::Bool => text
                .trim()
                .parse::<bool>()
                .map(Value::Boolean)
                .map_err(|error| error.to_string()),
            ConfigKind::Int => text
                .trim()
                .parse::<i64>()
                .map(Value::Integer)
                .map_err(|error| error.to_string()),
            ConfigKind::Float => text
                .trim()
                .parse::<f64>()
                .map(Value::Float)
                .map_err(|error| error.to_string()),
            ConfigKind::String | ConfigKind::Enum => Ok(Value::String(text.into())),
            ConfigKind::List => Err("nested list".into()),
        }
    };
    let value = if field.kind == ConfigKind::List {
        let element = field.element.ok_or("missing list element type")?;
        Value::Array(if text.trim().is_empty() {
            Vec::new()
        } else {
            text.split(',')
                .map(|item| scalar(element, item.trim()))
                .collect::<Result<_, _>>()?
        })
    } else {
        scalar(field.kind, text)?
    };
    field.validate_value(&value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_validates_ranges_choices_and_lists() {
        let fields: BTreeMap<String, ConfigField> = toml::from_str(
            r#"
            [count]
            type = "int"
            default = 2
            side = "client"
            min = 1
            max = 3
            [mode]
            type = "enum"
            default = "one"
            values = ["one", "two"]
            side = "client"
            [list]
            type = "list"
            element = "int"
            default = []
            side = "client"
        "#,
        )
        .unwrap();
        assert_eq!(
            parse_value(&fields["count"], "3").unwrap(),
            Value::Integer(3)
        );
        assert!(parse_value(&fields["count"], "4").is_err());
        assert!(parse_value(&fields["mode"], "three").is_err());
        assert_eq!(
            display_value(&parse_value(&fields["list"], "1, 2,3").unwrap()),
            "1, 2, 3"
        );
        assert!(parse_value(&fields["list"], "one").is_err());
    }
}
