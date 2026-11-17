use super::*;
use std::collections::BTreeSet;
use std::path::Path;

/// Every Rust file of the launcher's sources.
fn sources(dir: &Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(std::fs::read_to_string(path).unwrap());
        }
    }
}

/// Read Rust tokens rather than quote positions: whitespace, comments, escaped and raw strings
/// must not hide a call or turn a quoted example into one. Recurse into macro bodies too.
fn strings_in(
    tokens: proc_macro2::TokenStream,
    asked: &mut BTreeSet<String>,
    all: &mut BTreeSet<String>,
) {
    use proc_macro2::{Delimiter, TokenTree};
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Ident(name) if name == "tr" || name == "trf" => {
                if let Some(TokenTree::Group(args)) = tokens.peek() {
                    if args.delimiter() == Delimiter::Parenthesis {
                        if let Some(TokenTree::Literal(value)) = args.stream().into_iter().next() {
                            if let Ok(value) = syn::parse_str::<syn::LitStr>(&value.to_string()) {
                                asked.insert(value.value());
                            }
                        }
                    }
                }
            }
            TokenTree::Group(group) => strings_in(group.stream(), asked, all),
            TokenTree::Literal(value) => {
                if let Ok(value) = syn::parse_str::<syn::LitStr>(&value.to_string()) {
                    all.insert(value.value());
                }
            }
            _ => {}
        }
    }
}

/// The English strings requested directly, plus every literal (some reach `tr` via a widget or
/// constant). The tables' shared key set below covers those indirect translations.
fn strings() -> (BTreeSet<String>, BTreeSet<String>) {
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    let (mut asked, mut all) = (BTreeSet::new(), BTreeSet::new());
    for file in &files {
        strings_in(
            file.parse().expect("valid Rust tokens"),
            &mut asked,
            &mut all,
        );
    }
    (asked, all)
}

#[test]
fn the_scan_finds_multiline_calls_without_reading_comments_or_quoted_code() {
    let code = r###"
        tr
        (
            "multiline"
        );
        trf(/* before the string */ "hello {name}", &[("name", "player")]);
        some_macro!(crate::i18n::tr(r#"raw "quotes""#));
        tr("escaped \"quotes\" and \u{4e2d}\nnext");
        tr("continued \
            line");
        // tr("line comment");
        /* tr("block comment"); */
        let example = r#"tr("quoted code")"#;
        other_tr("not a translation");
        trf_other("not a translation either");
        let indirect = "widget label";
        tr(indirect);
    "###;
    let (mut asked, mut all) = (BTreeSet::new(), BTreeSet::new());
    strings_in(code.parse().unwrap(), &mut asked, &mut all);
    assert_eq!(
        asked,
        [
            "multiline",
            "hello {name}",
            "raw \"quotes\"",
            "escaped \"quotes\" and 中\nnext",
            "continued line",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    assert!(all.contains("widget label"));
    assert!(asked.is_subset(&all));
    assert!(!all.contains("line comment"));
    assert!(!all.contains("block comment"));
}

fn holes(s: &str) -> BTreeSet<&str> {
    s.split('{')
        .skip(1)
        .filter_map(|part| part.split_once('}').map(|(name, _)| name))
        .collect()
}

#[test]
fn japanese_has_every_string_the_code_asks_for() {
    let (mut asked, all) = strings();
    assert!(asked.contains("Language"), "the scan finds tr(\"…\")");
    let table: HashMap<String, String> = serde_json::from_str(Language::Japanese.source())
        .unwrap_or_else(|e| panic!("lang/ja.json: {e}"));
    // A label passed through a widget or a constant is not a literal argument to `tr`. Once
    // translated it must stay translated, not silently fall back to English.
    asked.extend(table.keys().cloned());
    let missing: Vec<_> = asked.iter().filter(|s| !table.contains_key(*s)).collect();
    assert!(missing.is_empty(), "lang/ja.json lacks {missing:#?}");
    let unused: Vec<_> = table.keys().filter(|s| !all.contains(*s)).collect();
    assert!(
        unused.is_empty(),
        "lang/ja.json has strings the code no longer has: {unused:#?}"
    );
    for (en, said) in &table {
        assert_eq!(holes(en), holes(said), "ja: {en:?}");
        assert!(!said.trim().is_empty(), "ja: {en:?} is empty");
    }
}

#[test]
fn japanese_has_yoake_launcher_update_strings() {
    let keys = [
        "Update launcher",
        "View on portal",
        "Check for launcher updates",
        "Downloading launcher update...",
        "The game keeps running while the launcher restarts.",
    ];
    let table: HashMap<String, String> = serde_json::from_str(Language::Japanese.source())
        .unwrap_or_else(|error| panic!("lang/ja.json: {error}"));
    for key in keys {
        let translation = table
            .get(key)
            .unwrap_or_else(|| panic!("lang/ja.json lacks {key}"));
        assert_ne!(translation, key, "lang/ja.json");
    }
}

#[test]
fn the_first_language_is_english_on_an_english_system_and_japanese_otherwise() {
    assert_eq!(Language::pick(["en-US", "ja"]), Language::English);
    assert_eq!(Language::pick(["en_GB.UTF-8"]), Language::English);
    assert_eq!(Language::pick(["ja-JP", "en-US"]), Language::Japanese);
    assert_eq!(Language::pick(["de-DE", "en-US"]), Language::Japanese);
    assert_eq!(Language::pick(["de-DE"]), Language::Japanese);
    assert_eq!(Language::pick(["fr-CH", "ja"]), Language::Japanese);
    assert_eq!(Language::pick([]), Language::Japanese);
    assert_eq!(Language::pick(["???"]), Language::Japanese);
    assert_eq!(Language::parse("ja_JP.UTF-8"), Some(Language::Japanese));
    assert_eq!(Language::parse("EN"), Some(Language::English));
    assert_eq!(Language::parse("de"), None);
    assert_eq!(Language::ALL, [Language::Japanese, Language::English]);
    assert_eq!(serde_json::to_string(&Language::Japanese).unwrap(), "\"ja\"");
    assert_eq!(serde_json::to_string(&Language::English).unwrap(), "\"en\"");
}

#[test]
fn a_saved_language_other_than_japanese_or_english_shows_in_japanese() {
    let saved = |code: &str| serde_json::from_str::<Language>(&format!("\"{code}\"")).unwrap();
    assert_eq!(saved("en"), Language::English);
    assert_eq!(saved("ja"), Language::Japanese);
    assert_eq!(saved("de"), Language::Japanese);
    assert_eq!(saved("pt-br"), Language::Japanese);
}

#[test]
fn a_missing_string_shows_in_english() {
    assert_eq!(tr_in(Language::Japanese, "Settings"), "設定");
    assert_eq!(tr_in(Language::English, "Settings"), "Settings");
    assert_eq!(
        tr_in(Language::Japanese, "No such string"),
        "No such string"
    );
}

#[test]
fn yoake_translations_keep_attribution_and_brand_visible_text() {
    let attribution = "Yoake Launcher is built on Horizon (licence below)";
    for language in Language::ALL {
        assert_eq!(
            tr_in_brand(language, attribution, true),
            tr_in_brand(language, attribution, false)
        );
        assert!(tr_in_brand(language, attribution, true).contains("Horizon"));
        assert_eq!(
            tr_in_brand(language, "Horizon Launcher", true),
            "Yoake Launcher"
        );
    }
    assert_eq!(
        tr_in_brand(Language::Japanese, "Horizon crashed", true),
        "Yoakeがクラッシュしました"
    );
    assert_eq!(
        tr_in_brand(Language::English, "Horizon crashed", true),
        "Yoake crashed"
    );
    assert_eq!(
        tr_in_brand(Language::English, "Horizon Launcher", false),
        "Horizon Launcher"
    );
}
