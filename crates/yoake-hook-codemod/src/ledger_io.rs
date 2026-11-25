use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use yoake_hook_ledger::LedgerSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteMeta {
    pub caller: u32,
    pub callee_fn: Option<u32>,
    pub callee_path: String,
    pub anchor: String,
    pub stmt_path: Vec<String>,
    pub context: [String; 2],
    pub dup_rank: u32,
    pub shim: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig: Option<String>,
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

pub struct TempDirGuard(PathBuf);

impl TempDirGuard {
    pub fn new(label: &str) -> Result<Self, String> {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yoake_hook_codemod_{label}_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("failed to create temp dir {}: {e}", dir.display()))?;
        Ok(Self(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub struct FileUpdate {
    pub path: PathBuf,
    pub original: String,
    pub updated: String,
}

fn dir_has_entries(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .ok()
        .and_then(|mut it| it.next())
        .is_some()
}

pub fn load_ledger_set(dir: &Path) -> Result<LedgerSet, String> {
    if dir.join("meta.json").exists() || dir.is_file() || (dir.is_dir() && dir_has_entries(dir)) {
        LedgerSet::load(dir).map_err(|e| format!("invalid ledger {}: {e}", dir.display()))
    } else {
        let tmp = TempDirGuard::new("init_ledger")?;
        std::fs::write(
            tmp.path().join("meta.json"),
            "{\n  \"version\": 2,\n  \"next_id\": 1\n}\n",
        )
        .map_err(|e| format!("failed to initialize temp ledger: {e}"))?;
        LedgerSet::load(tmp.path()).map_err(|e| format!("failed to initialize ledger: {e}"))
    }
}

pub fn read_manual_exclusions(
    dir: &Path,
    crate_ident: &str,
) -> Result<BTreeMap<String, String>, String> {
    let shard_path = dir.join(format!("{crate_ident}.json"));
    if !shard_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&shard_path)
        .map_err(|e| format!("failed to read {}: {e}", shard_path.display()))?;
    let val: Value = serde_json::from_str(&text)
        .map_err(|e| format!("invalid JSON in {}: {e}", shard_path.display()))?;

    let mut out = BTreeMap::new();
    if let Some(arr) = val.get("excluded").and_then(Value::as_array) {
        for item in arr {
            if item.get("manual").and_then(Value::as_bool) == Some(true)
                && let (Some(key), Some(reason)) = (
                    item.get("key").and_then(Value::as_str),
                    item.get("reason").and_then(Value::as_str),
                )
            {
                out.insert(key.to_string(), reason.to_string());
            }
        }
    }
    Ok(out)
}

pub fn read_callsite_sites(
    dir: &Path,
    crate_ident: &str,
) -> Result<BTreeMap<u32, SiteMeta>, String> {
    let shard_path = dir.join(format!("{crate_ident}.json"));
    if !shard_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&shard_path)
        .map_err(|e| format!("failed to read {}: {e}", shard_path.display()))?;
    let val: Value = serde_json::from_str(&text)
        .map_err(|e| format!("invalid JSON in {}: {e}", shard_path.display()))?;

    let mut out = BTreeMap::new();
    if let Some(arr) = val.get("entries").and_then(Value::as_array) {
        for item in arr {
            if let (Some(id), Some(site_val)) = (
                item.get("id").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
                item.get("site").filter(|v| !v.is_null()),
            ) {
                let site: SiteMeta = serde_json::from_value(site_val.clone()).map_err(|e| {
                    format!("invalid callsite site metadata for id {id} in {}: {e}", shard_path.display())
                })?;
                out.insert(id, site);
            }
        }
    }
    Ok(out)
}

pub fn read_tombstoned_callsite_keys(
    dir: &Path,
    crate_ident: &str,
) -> Result<BTreeMap<String, SiteMeta>, String> {
    let shard_path = dir.join(format!("{crate_ident}.json"));
    if !shard_path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(&shard_path)
        .map_err(|e| format!("failed to read {}: {e}", shard_path.display()))?;
    let val: Value = serde_json::from_str(&text)
        .map_err(|e| format!("invalid JSON in {}: {e}", shard_path.display()))?;

    let mut out = BTreeMap::new();
    if let Some(arr) = val.get("entries").and_then(Value::as_array) {
        for item in arr {
            if item.get("status").and_then(Value::as_str) == Some("tombstone")
                && let (Some(key), Some(site_val)) = (
                    item.get("key").and_then(Value::as_str),
                    item.get("site").filter(|v| !v.is_null()),
                )
            {
                let site: SiteMeta = serde_json::from_value(site_val.clone()).map_err(|e| {
                    format!("invalid callsite site metadata for '{key}' in {}: {e}", shard_path.display())
                })?;
                out.insert(key.to_string(), site);
            }
        }
    }
    Ok(out)
}

pub fn read_all_fn_keys(dir: &Path) -> Result<Vec<(String, u32)>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("failed to read ledger dir {}: {e}", dir.display()))?;
    for item in entries {
        let path = item.map_err(|e| format!("failed to read ledger entry: {e}"))?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json")
            || path.file_name().and_then(|s| s.to_str()) == Some("meta.json")
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let val: Value = serde_json::from_str(&text)
            .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;
        if let Some(arr) = val.get("entries").and_then(Value::as_array) {
            for entry in arr {
                let status = entry.get("status").and_then(Value::as_str);
                let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("fn");
                if status == Some("live")
                    && kind == "fn"
                    && let (Some(key), Some(id)) = (
                        entry.get("key").and_then(Value::as_str),
                        entry.get("id").and_then(Value::as_u64).and_then(|v| u32::try_from(v).ok()),
                    )
                {
                    out.push((key.to_string(), id));
                }
            }
        }
    }
    Ok(out)
}

pub fn stage_ledger_set(
    ledger_set: &LedgerSet,
    crate_ident: &str,
    manual_keys: &BTreeSet<String>,
) -> Result<TempDirGuard, String> {
    let empty_sites = BTreeMap::new();
    stage_ledger_set_with_sites(ledger_set, crate_ident, manual_keys, &empty_sites)
}

pub fn stage_ledger_set_with_sites(
    ledger_set: &LedgerSet,
    crate_ident: &str,
    manual_keys: &BTreeSet<String>,
    callsite_sites: &BTreeMap<u32, SiteMeta>,
) -> Result<TempDirGuard, String> {
    let staging = TempDirGuard::new("stage")?;
    ledger_set
        .save(staging.path())
        .map_err(|e| format!("failed to stage ledger: {e}"))?;

    let shard_path = staging.path().join(format!("{crate_ident}.json"));
    if shard_path.is_file() {
        if !manual_keys.is_empty() {
            patch_manual_in_shard_json(&shard_path, manual_keys)?;
        }
        if !callsite_sites.is_empty() {
            patch_callsite_sites_in_shard_json(&shard_path, callsite_sites)?;
        }
    }
    Ok(staging)
}

fn patch_manual_in_shard_json(path: &Path, manual_keys: &BTreeSet<String>) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read staged shard {}: {e}", path.display()))?;
    let Some(arr_bounds) = find_top_array_bounds(&text, "\"excluded\"") else {
        return Ok(());
    };
    let object_spans = find_array_object_spans(&text, arr_bounds.0, arr_bounds.1);

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (start, end) in object_spans {
        let obj_slice = &text[start..=end];
        let Ok(val) = serde_json::from_str::<Value>(obj_slice) else {
            continue;
        };
        let Some(key) = val.get("key").and_then(Value::as_str) else {
            continue;
        };
        if !manual_keys.contains(key) {
            continue;
        }
        if val.get("manual").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if val.get("manual").is_some() {
            if let Some(pos) = obj_slice.rfind("false") {
                edits.push((start + pos, start + pos + 5, "true".to_string()));
            }
        } else if let Some(rel_nl) = obj_slice.rfind('\n') {
            let insert_at = start + rel_nl;
            edits.push((insert_at, insert_at, ",\n      \"manual\": true".to_string()));
        }
    }

    if edits.is_empty() {
        return Ok(());
    }
    let mut out = text;
    for (start, end, rep) in edits.into_iter().rev() {
        out.replace_range(start..end, &rep);
    }
    std::fs::write(path, out)
        .map_err(|e| format!("failed to update staged shard {}: {e}", path.display()))
}

fn format_indented_json<T: Serialize>(value: &T, base_indent: &str) -> Result<String, String> {
    let raw = serde_json::to_string_pretty(value)
        .map_err(|e| format!("failed to serialize site metadata: {e}"))?;
    let mut out = String::with_capacity(raw.len() + 64);
    for (idx, line) in raw.lines().enumerate() {
        if idx > 0 {
            out.push('\n');
            out.push_str(base_indent);
        }
        out.push_str(line);
    }
    Ok(out)
}

fn find_field_value_span(obj_slice: &str, field_key_quoted: &str) -> Option<(usize, usize)> {
    let bytes = obj_slice.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                let end = skip_json_string(bytes, i);
                if depth == 1 && &obj_slice[start..end] == field_key_quoted {
                    let mut j = end;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b':' {
                        j += 1;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        let val_start = j;
                        if j >= bytes.len() {
                            return None;
                        }
                        if bytes[j] == b'"' {
                            return Some((val_start, skip_json_string(bytes, j)));
                        }
                        if bytes[j] == b'{' || bytes[j] == b'[' {
                            let mut val_depth = 0usize;
                            while j < bytes.len() {
                                match bytes[j] {
                                    b'"' => j = skip_json_string(bytes, j),
                                    b'{' | b'[' => {
                                        val_depth += 1;
                                        j += 1;
                                    }
                                    b'}' | b']' => {
                                        val_depth -= 1;
                                        j += 1;
                                        if val_depth == 0 {
                                            return Some((val_start, j));
                                        }
                                    }
                                    _ => j += 1,
                                }
                            }
                            return None;
                        }
                        while j < bytes.len()
                            && !bytes[j].is_ascii_whitespace()
                            && bytes[j] != b','
                            && bytes[j] != b'}'
                        {
                            j += 1;
                        }
                        return Some((val_start, j));
                    }
                }
                i = end;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

fn patch_callsite_sites_in_shard_json(
    path: &Path,
    callsite_sites: &BTreeMap<u32, SiteMeta>,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read staged shard {}: {e}", path.display()))?;
    let Some(arr_bounds) = find_top_array_bounds(&text, "\"entries\"") else {
        return Ok(());
    };
    let object_spans = find_array_object_spans(&text, arr_bounds.0, arr_bounds.1);

    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for (start, end) in object_spans {
        let obj_slice = &text[start..=end];
        let Ok(val) = serde_json::from_str::<Value>(obj_slice) else {
            continue;
        };
        let Some(id) = val
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|v| u32::try_from(v).ok())
        else {
            continue;
        };
        let Some(site_meta) = callsite_sites.get(&id) else {
            continue;
        };
        let expected_val = serde_json::to_value(site_meta)
            .map_err(|e| format!("failed to serialize site metadata: {e}"))?;
        if val.get("site") == Some(&expected_val) {
            continue;
        }
        let formatted_site = format_indented_json(site_meta, "      ")?;
        if let Some((vs, ve)) = find_field_value_span(obj_slice, "\"site\"") {
            edits.push((start + vs, start + ve, formatted_site));
        } else if let Some(rel_nl) = obj_slice.rfind('\n') {
            let insert_at = start + rel_nl;
            edits.push((
                insert_at,
                insert_at,
                format!(",\n      \"site\": {formatted_site}"),
            ));
        }
    }

    if edits.is_empty() {
        return Ok(());
    }
    let mut out = text;
    for (start, end, rep) in edits.into_iter().rev() {
        out.replace_range(start..end, &rep);
    }
    std::fs::write(path, out)
        .map_err(|e| format!("failed to update staged shard {}: {e}", path.display()))
}

fn skip_json_string(bytes: &[u8], mut i: usize) -> usize {
    i += 1; // skip opening '"'
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn find_top_array_bounds(text: &str, field_key_quoted: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let start = i;
                let end = skip_json_string(bytes, i);
                if depth == 1 && &text[start..end] == field_key_quoted {
                    let mut j = end;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < bytes.len() && bytes[j] == b':' {
                        j += 1;
                        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                            j += 1;
                        }
                        if j < bytes.len() && bytes[j] == b'[' {
                            let arr_start = j;
                            let mut arr_depth = 0usize;
                            while j < bytes.len() {
                                match bytes[j] {
                                    b'"' => j = skip_json_string(bytes, j),
                                    b'[' => {
                                        arr_depth += 1;
                                        j += 1;
                                    }
                                    b']' => {
                                        arr_depth -= 1;
                                        if arr_depth == 0 {
                                            return Some((arr_start, j));
                                        }
                                        j += 1;
                                    }
                                    _ => j += 1,
                                }
                            }
                        }
                    }
                }
                i = end;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    None
}

fn find_array_object_spans(text: &str, arr_start: usize, arr_end: usize) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = arr_start + 1;
    let mut depth = 0usize;
    let mut obj_start = 0usize;
    while i < arr_end {
        match bytes[i] {
            b'"' => i = skip_json_string(bytes, i),
            b'{' => {
                if depth == 0 {
                    obj_start = i;
                }
                depth += 1;
                i += 1;
            }
            b'}' => {
                if depth == 1 {
                    spans.push((obj_start, i));
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            _ => i += 1,
        }
    }
    spans
}

pub fn diff_ledger_dirs(staging: &Path, target: &Path) -> Result<Vec<PathBuf>, String> {
    let mut entries: Vec<String> = std::fs::read_dir(staging)
        .map_err(|e| format!("failed to read staged dir {}: {e}", staging.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();

    // Put shards before meta.json
    entries.sort_by(|a, b| {
        let a_meta = a == "meta.json";
        let b_meta = b == "meta.json";
        a_meta.cmp(&b_meta).then_with(|| a.cmp(b))
    });

    let mut changed = Vec::new();
    for name in entries {
        let staged_file = staging.join(&name);
        let target_file = target.join(&name);
        let staged_bytes = std::fs::read(&staged_file)
            .map_err(|e| format!("failed to read {}: {e}", staged_file.display()))?;
        let matches = std::fs::read(&target_file).is_ok_and(|existing| existing == staged_bytes);
        if !matches {
            changed.push(target_file);
        }
    }
    Ok(changed)
}

fn tmp_path_for(target: &Path) -> PathBuf {
    PathBuf::from(format!("{}.yoake-tmp", target.display()))
}

fn remove_tmp_files(tmps: &[PathBuf]) {
    for tmp in tmps {
        let _ = std::fs::remove_file(tmp);
    }
}

fn run_rustfmt_on_files(edition: &str, paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    // Each file goes through stdin: rustfmt then formats only that text and does not try to
    // resolve `mod x;` children next to the `.yoake-tmp` name, which it would get wrong.
    for path in paths {
        let source = std::fs::read(path)
            .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
        let child = Command::new("rustfmt")
            .arg("--edition")
            .arg(edition)
            .arg("--emit")
            .arg("stdout")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("warning: rustfmt not found on PATH; skipping formatting");
                return Ok(());
            }
            Err(err) => return Err(format!("failed to execute rustfmt: {err}")),
        };
        {
            use std::io::Write as _;
            let mut stdin = child.stdin.take().expect("piped stdin");
            stdin
                .write_all(&source)
                .map_err(|err| format!("failed to feed rustfmt: {err}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|err| format!("failed to wait for rustfmt: {err}"))?;
        if !output.status.success() {
            return Err(format!(
                "rustfmt failed on {} ({}): {}",
                path.display(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        std::fs::write(path, &output.stdout)
            .map_err(|err| format!("failed to write {}: {err}", path.display()))?;
    }
    Ok(())
}

pub fn atomic_apply(
    file_updates: &[FileUpdate],
    staging: &Path,
    ledger_changed: &[PathBuf],
    ledger_dir: &Path,
    rustfmt_edition: Option<&str>,
) -> Result<(), String> {
    // 1. Write all changed source files to <file>.yoake-tmp
    let mut rs_tmps = Vec::with_capacity(file_updates.len());
    for update in file_updates {
        let tmp = tmp_path_for(&update.path);
        if let Err(err) = std::fs::write(&tmp, &update.updated) {
            remove_tmp_files(&rs_tmps);
            return Err(format!("failed to write temp file {}: {err}", tmp.display()));
        }
        rs_tmps.push(tmp);
    }

    if let Some(edition) = rustfmt_edition
        && let Err(err) = run_rustfmt_on_files(edition, &rs_tmps)
    {
        remove_tmp_files(&rs_tmps);
        return Err(err);
    }

    // 2. Rename all source .yoake-tmp files into place, rolling back on error
    let mut renamed_rs: Vec<(&Path, &str)> = Vec::new();
    for (idx, update) in file_updates.iter().enumerate() {
        let tmp = &rs_tmps[idx];
        let rename_res: Result<(), String> = (|| {
            if let Ok(meta) = std::fs::metadata(&update.path)
                && meta.permissions().readonly()
            {
                return Err(format!("target file {} is read-only", update.path.display()));
            }
            std::fs::rename(tmp, &update.path).map_err(|e| {
                format!(
                    "failed to rename {} -> {}: {e}",
                    tmp.display(),
                    update.path.display()
                )
            })
        })();

        if let Err(err) = rename_res {
            remove_tmp_files(&rs_tmps[idx..]);
            for (orig_path, orig_content) in renamed_rs.iter().rev() {
                let _ = std::fs::write(orig_path, orig_content);
            }
            return Err(err);
        }
        renamed_rs.push((&update.path, &update.original));
    }

    // 3. Save ledger files last (shards first, meta.json last), atomically
    if !ledger_changed.is_empty() {
        if let Err(err) = apply_ledger_files(staging, ledger_changed, ledger_dir) {
            for (orig_path, orig_content) in renamed_rs.iter().rev() {
                let _ = std::fs::write(orig_path, orig_content);
            }
            return Err(err);
        }
    }

    Ok(())
}

fn apply_ledger_files(
    staging: &Path,
    ledger_changed: &[PathBuf],
    ledger_dir: &Path,
) -> Result<(), String> {
    std::fs::create_dir_all(ledger_dir)
        .map_err(|e| format!("failed to create ledger dir {}: {e}", ledger_dir.display()))?;

    let mut ledger_tmps = Vec::with_capacity(ledger_changed.len());
    let mut prev_contents: Vec<(&Path, Option<Vec<u8>>)> = Vec::new();

    for target_file in ledger_changed {
        let Some(name) = target_file.file_name() else {
            continue;
        };
        let staged_file = staging.join(name);
        let bytes = std::fs::read(&staged_file)
            .map_err(|e| format!("failed to read staged file {}: {e}", staged_file.display()))?;
        let tmp = tmp_path_for(target_file);
        if let Err(err) = std::fs::write(&tmp, bytes) {
            for (rem_tmp, _) in &ledger_tmps {
                let _ = std::fs::remove_file(rem_tmp);
            }
            return Err(format!("failed to write temp file {}: {err}", tmp.display()));
        }
        ledger_tmps.push((tmp, target_file.as_path()));
    }

    for (idx, (tmp, target_file)) in ledger_tmps.iter().enumerate() {
        let prev = std::fs::read(target_file).ok();
        let rename_res: Result<(), String> = (|| {
            if let Ok(meta) = std::fs::metadata(target_file)
                && meta.permissions().readonly()
            {
                return Err(format!("target file {} is read-only", target_file.display()));
            }
            std::fs::rename(tmp, target_file).map_err(|e| {
                format!(
                    "failed to rename {} -> {}: {e}",
                    tmp.display(),
                    target_file.display()
                )
            })
        })();

        if let Err(err) = rename_res {
            for (rem_tmp, _) in &ledger_tmps[idx..] {
                let _ = std::fs::remove_file(rem_tmp);
            }
            for (old_path, old_bytes) in prev_contents.iter().rev() {
                match old_bytes {
                    Some(bytes) => {
                        let _ = std::fs::write(old_path, bytes);
                    }
                    None => {
                        let _ = std::fs::remove_file(old_path);
                    }
                }
            }
            return Err(err);
        }
        prev_contents.push((target_file, prev));
    }

    Ok(())
}

pub fn run_check_history(base_dir: &Path, current_dir: &Path) -> Result<bool, String> {
    let base = LedgerSet::load(base_dir)
        .map_err(|e| format!("failed to load base ledger {}: {e}", base_dir.display()))?;
    let current = LedgerSet::load(current_dir)
        .map_err(|e| format!("failed to load current ledger {}: {e}", current_dir.display()))?;
    match yoake_hook_ledger::check_append_only(&base, &current) {
        Ok(()) => {
            println!("check-history: ok");
            Ok(true)
        }
        Err(errors) => {
            for err in &errors {
                eprintln!("history error: {err}");
            }
            Ok(false)
        }
    }
}
