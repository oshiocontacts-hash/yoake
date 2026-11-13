use super::{Installed, entries};
use horizon_client::shader_pack_list::{self, Entry, Kind};
use horizon_mods::yak::{OptionValue, Shader};
use horizon_shader_api::{OptionKind, Pack, PackOption};
use std::{collections::BTreeMap, path::{Path, PathBuf}};

fn key(entry: &Installed, shader: &Shader) -> String {
    format!("yak-{}-{}-{}", entry.manifest.id.len(), entry.manifest.id, shader.id)
}

fn find(name: &str) -> Option<(&'static Installed, &'static Shader)> {
    entries().iter().find_map(|entry| {
        entry.manifest.shaders.iter().find(|shader| key(entry, shader) == name)
            .map(|shader| (entry, shader))
    })
}

fn find_path(path: &Path) -> Option<(&'static Installed, &'static Shader)> {
    entries().iter().find_map(|entry| {
        entry.manifest.shaders.iter()
            .find(|shader| entry.root.join("shaders").join(&shader.id) == path)
            .map(|shader| (entry, shader))
    })
}

pub(crate) fn path(name: &str, dir: &Path) -> PathBuf {
    find(name).map_or_else(|| dir.join(name), |(entry, shader)| {
        entry.root.join("shaders").join(&shader.id)
    })
}

pub(crate) fn values_path(path: &Path) -> PathBuf {
    let preference = find_path(path).and_then(|(entry, shader)| {
        Some(entry.root.ancestors().nth(3)?.join(shader_pack_list::DIR).join(key(entry, shader)))
    });
    shader_pack_list::values_path(preference.as_deref().unwrap_or(path))
}

fn value_text(value: &OptionValue) -> String {
    match value {
        OptionValue::Boolean(value) => value.to_string(),
        OptionValue::Integer(value) => value.to_string(),
        OptionValue::Float(value) => value.to_string(),
        OptionValue::Text(value) => value.clone(),
    }
}

fn options(original: &[PackOption], shader: &Shader) -> Vec<PackOption> {
    let mut options = original.to_vec();
    for (id, declared) in &shader.options {
        let option = PackOption {
            id: id.clone(),
            name: declared.label_ja.as_ref().map_or_else(|| id.clone(), |label| format!("{label} / {id}")),
            description: String::new(),
            kind: OptionKind::Choice {
                values: declared.values.iter().map(value_text).collect(),
                default: declared.values.iter().position(|value| value == &declared.default).unwrap_or(0) as u32,
            },
        };
        if let Some(index) = options.iter().position(|option| option.id == *id) {
            options[index] = option;
        } else {
            options.push(option);
        }
    }
    options
}

pub(crate) fn scan(dir: &Path) -> Vec<Entry> {
    scan_installed(dir, entries())
}

pub(super) fn scan_installed(dir: &Path, installed: &[Installed]) -> Vec<Entry> {
    let mut packs = shader_pack_list::scan(dir);
    for entry in installed {
        for shader in &entry.manifest.shaders {
            let path = entry.root.join("shaders").join(&shader.id);
            if !path.is_dir() { continue; }
            let file = key(entry, shader);
            packs.retain(|pack| pack.file != file);
            let native = horizon_shader_api::package::is_pack(&path);
            let manifest = native.then(|| Pack::read(&path)).and_then(Result::ok).map(|pack| pack.manifest);
            let mut pages = manifest.as_ref().map(|manifest| manifest.pages.clone()).unwrap_or_default();
            let options = if native {
                options(manifest.as_ref().map(|manifest| manifest.options.as_slice()).unwrap_or_default(), shader)
            } else { Vec::new() };
            if !shader.options.is_empty() { pages.clear(); }
            let values = shader_pack_list::load_values(&dir.join(&file), &options);
            packs.push(Entry {
                file,
                title: format!("{} (.yak)", shader.name),
                kind: if native { Kind::Horizon } else { Kind::Classic },
                options,
                values,
                pages,
            });
        }
    }
    packs
}

pub(crate) fn save_values(entry: &Entry, dir: &Path) {
    if find(&entry.file).is_some() {
        if let Err(error) = std::fs::create_dir_all(dir) {
            log::warn!("YAK shader options: {error}");
            return;
        }
    }
    entry.save_values(dir);
}

pub(crate) fn native_parameters(path: &Path, pack: &mut Pack) -> Result<BTreeMap<String, f64>, String> {
    if let Some((_, shader)) = find_path(path) {
        let preference = values_path(path);
        let preference = preference.to_str().and_then(|path| path.strip_suffix(".txt"))
            .ok_or("non-UTF-8 shader preference path")?;
        apply_parameters(pack, shader, Path::new(preference))
    } else {
        let values = shader_pack_list::load_values(path, &pack.manifest.options);
        Ok(pack.manifest.options.iter().map(|option| option.id.clone()).zip(values).collect())
    }
}

pub(super) fn apply_parameters(pack: &mut Pack, shader: &Shader, preference: &Path) -> Result<BTreeMap<String, f64>, String> {
    let options = options(&pack.manifest.options, shader);
    let values = shader_pack_list::load_values(preference, &options);
    let mut constants = BTreeMap::new();
    for (option, chosen) in options.iter().zip(values) {
        let declared = shader.options.get(&option.id).map(|declared| {
            let index = if chosen.is_finite() { chosen.round().max(0.0) as usize } else { usize::MAX };
            declared.values.get(index).unwrap_or(&declared.default)
        });
        if option.id == "shadow_map" {
            if let Some(value) = declared {
                let OptionValue::Integer(size) = value else {
                    return Err("shadow_map choices must be integer texel sizes".into());
                };
                let size = u32::try_from(*size).map_err(|_| "invalid shadow_map size")?;
                if !size.is_power_of_two() || !horizon_shader_api::manifest::SHADOW_RESOLUTIONS.contains(&size) {
                    return Err("shadow_map size must be a power of two from 256 to 8192".into());
                }
                pack.manifest.shadow.as_mut().ok_or("shadow_map requires a [shadow] pass")?.resolution = size;
            }
        }
        if let Some(original) = pack.manifest.options.iter().find(|original| original.id == option.id) {
            let constant = match declared {
                Some(OptionValue::Boolean(value)) => if *value { 1.0 } else { 0.0 },
                Some(OptionValue::Integer(value)) => *value as f64,
                Some(OptionValue::Float(value)) => *value,
                Some(OptionValue::Text(value)) => match &original.kind {
                    OptionKind::Choice { values, .. } => values.iter().position(|candidate| candidate == value)
                        .map(|index| index as f64).ok_or("unknown native shader choice")?,
                    _ => value.parse().map_err(|_| "native shader override must be numeric")?,
                },
                None => chosen,
            };
            if !constant.is_finite() { return Err("native shader override must be finite".into()); }
            constants.insert(option.id.clone(), constant);
        }
    }
    Ok(constants)
}

pub(crate) fn classic_chosen(name: &str, chosen: &mut BTreeMap<String, String>) {
    let Some((_, shader)) = find(name) else { return; };
    for (id, declared) in &shader.options {
        let valid = chosen.get(id).is_some_and(|chosen| declared.values.iter().any(|value| value_text(value) == *chosen));
        if !valid { chosen.insert(id.clone(), value_text(&declared.default)); }
    }
}

pub(crate) fn classic_options(name: &str, options: &mut BTreeMap<String, horizon_shaders::PackOption>) -> bool {
    let Some((_, shader)) = find(name) else { return false; };
    for (id, declared) in &shader.options {
        options.insert(id.clone(), horizon_shaders::PackOption {
            name: id.clone(), default: value_text(&declared.default),
            values: declared.values.iter().map(value_text).collect(), switch: false, tested: true,
        });
    }
    !shader.options.is_empty()
}

pub(crate) fn classic_lang(name: &str, pack: &horizon_shaders::Pack, locale: &str) -> horizon_shaders::Lang {
    let Some((_, shader)) = find(name) else { return horizon_shaders::Lang::load(pack, locale); };
    let files = pack.list("lang");
    let mut text = String::new();
    for wanted in ["en_us", locale] {
        if let Some(file) = files.iter().find(|file| file.strip_suffix(".lang").is_some_and(|stem| stem.eq_ignore_ascii_case(wanted))) {
            if let Some(source) = pack.read_text(&format!("lang/{file}")) { text.push_str(&source); text.push('\n'); }
        }
    }
    for (id, declared) in &shader.options {
        let label = declared.label_ja.as_deref().unwrap_or(id).replace(['\r', '\n'], " ");
        text.push_str(&format!("option.{id}={label} / {id}\n"));
    }
    horizon_shaders::Lang::parse(&text)
}

pub(crate) fn prepare_classic_save(name: &str, dir: &Path) -> std::io::Result<()> {
    if find(name).is_some() { std::fs::create_dir_all(dir)?; }
    Ok(())
}
