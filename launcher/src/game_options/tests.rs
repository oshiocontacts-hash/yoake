use super::*;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "horizon-launcher-game-options-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn vanilla_encodings_match_known_options_txt_strings() {
    // Master and every sound category: 0..=100% -> 0.0..=1.0
    for def in FRIENDLY
        .iter()
        .filter(|d| d.key.starts_with("soundCategory_"))
    {
        assert_eq!(encode_unit_percent(0).unwrap(), "0.0", "{}", def.key);
        assert_eq!(encode_unit_percent(50).unwrap(), "0.5", "{}", def.key);
        assert_eq!(encode_unit_percent(100).unwrap(), "1.0", "{}", def.key);
        assert_eq!(decode_unit_percent("0.0"), Some(0));
        assert_eq!(decode_unit_percent("0.5"), Some(50));
        assert_eq!(decode_unit_percent("1.0"), Some(100));
    }
    assert!(encode_unit_percent(-1).is_err());
    assert!(encode_unit_percent(101).is_err());
    assert_eq!(decode_unit_percent("1.5"), None);

    // FOV 30..=110 stored as (fov - 70) / 40
    assert_eq!(encode_fov(30).unwrap(), "-1.0");
    assert_eq!(encode_fov(70).unwrap(), "0.0");
    assert_eq!(encode_fov(90).unwrap(), "0.5");
    assert_eq!(encode_fov(110).unwrap(), "1.0");
    for deg in 30..=110 {
        let encoded = encode_fov(deg).unwrap();
        assert_eq!(decode_fov(&encoded), Some(deg), "fov {deg} -> {encoded}");
    }
    assert!(encode_fov(29).is_err());
    assert!(encode_fov(111).is_err());
    assert_eq!(decode_fov("1.5"), None);

    // Render and simulation distance
    assert_eq!(encode_render_distance(12).unwrap(), "12");
    assert_eq!(decode_render_distance("32"), Some(32));
    assert!(encode_render_distance(1).is_err());
    assert!(encode_render_distance(129).is_err());

    assert_eq!(encode_simulation_distance(12).unwrap(), "12");
    assert_eq!(decode_simulation_distance("5"), Some(5));
    assert!(encode_simulation_distance(4).is_err());
    assert!(encode_simulation_distance(33).is_err());

    // GUI scale
    assert_eq!(encode_gui_scale(0).unwrap(), "0");
    assert_eq!(encode_gui_scale(3).unwrap(), "3");
    assert!(encode_gui_scale(-1).is_err());

    // Brightness (gamma)
    assert_eq!(encode_unit_percent(0).unwrap(), "0.0");
    assert_eq!(encode_unit_percent(50).unwrap(), "0.5");
    assert_eq!(encode_unit_percent(100).unwrap(), "1.0");

    // Max FPS (10..=260 step 10)
    assert_eq!(encode_max_fps(120).unwrap(), "120");
    assert_eq!(encode_max_fps(260).unwrap(), "260");
    assert!(encode_max_fps(125).is_err());
    assert!(encode_max_fps(0).is_err());
    assert!(encode_max_fps(270).is_err());

    // Mouse sensitivity (0..=200% -> 0.0..=1.0)
    assert_eq!(encode_mouse_sensitivity(0).unwrap(), "0.0");
    assert_eq!(encode_mouse_sensitivity(100).unwrap(), "0.5");
    assert_eq!(encode_mouse_sensitivity(200).unwrap(), "1.0");
    assert_eq!(decode_mouse_sensitivity("0.5"), Some(100));
    assert!(encode_mouse_sensitivity(201).is_err());

    // View bobbing
    assert_eq!(encode_bool(true), "true");
    assert_eq!(encode_bool(false), "false");
    assert_eq!(decode_bool("true"), Some(true));
    assert_eq!(decode_bool("other"), None);

    // Language (plain string without quotes)
    assert_eq!(validate_lang("ja_jp").unwrap(), "ja_jp");
    assert_eq!(validate_lang("en_us").unwrap(), "en_us");
    assert!(validate_lang("\"ja_jp\"").is_err());
    assert!(validate_lang("ja:jp").is_err());
    assert!(validate_lang("").is_err());

    // validate_entry rejects bad friendly values and bad raw keys
    assert!(validate_entry("fov", "0.5").is_ok());
    assert!(validate_entry("fov", "not_a_num").is_err());
    assert!(validate_entry("bad:key", "1").is_err());
    assert!(validate_entry("ok_key", "line1\nline2").is_err());
}

#[test]
fn apply_always_and_once_override_and_preserve_unknown_lines_and_order() {
    let root = scratch("apply");
    let home = root.join("instance");

    let global = GlobalProfile {
        entries: vec![
            OptionEntry {
                key: "soundCategory_music".into(),
                value: "0.0".into(),
                mode: ApplyMode::Always,
            },
            OptionEntry {
                key: "fov".into(),
                value: "0.5".into(), // 90 deg
                mode: ApplyMode::Once,
            },
            OptionEntry {
                key: "lang".into(),
                value: "ja_jp".into(),
                mode: ApplyMode::Once,
            },
        ],
    };
    let mut cfg = InstanceConfig {
        use_usual: true,
        entries: vec![OptionEntry {
            key: "fov".into(),
            value: "1.0".into(), // 110 deg overrides global 90 deg
            mode: ApplyMode::Once,
        }],
        applied: BTreeMap::new(),
    };

    // Existing options.txt with unknown keys and specific order
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(OPTIONS_TXT),
        "version:4325\n# comment\nfov:0.0\ncustomModOption:keep_me\nsoundCategory_music:1.0\nkey_key.jump:key.keyboard.space\n",
    )
    .unwrap();

    // First launch: writes Always (soundCategory_music) and Once (fov overridden to 1.0, lang=ja_jp)
    let report = plan_and_apply(&home, &global, &mut cfg).unwrap();
    assert_eq!(report.written_count, 3);
    assert!(report.applied_changed);
    assert_eq!(cfg.applied.get("fov").map(String::as_str), Some("1.0"));
    assert_eq!(cfg.applied.get("lang").map(String::as_str), Some("ja_jp"));
    assert!(!cfg.applied.contains_key("soundCategory_music"));

    let after_first = fs::read_to_string(home.join(OPTIONS_TXT)).unwrap();
    assert_eq!(
        after_first,
        "version:4325\n# comment\nfov:1.0\ncustomModOption:keep_me\nsoundCategory_music:0.0\nkey_key.jump:key.keyboard.space\nlang:ja_jp\n"
    );

    // Player changes both in-game
    fs::write(
        home.join(OPTIONS_TXT),
        "version:4325\n# comment\nfov:0.25\ncustomModOption:keep_me\nsoundCategory_music:0.8\nkey_key.jump:key.keyboard.space\nlang:en_us\n",
    )
    .unwrap();

    // Second launch: Always overwrites soundCategory_music back to 0.0; Once leaves fov:0.25 and lang:en_us
    let report = plan_and_apply(&home, &global, &mut cfg).unwrap();
    assert_eq!(report.written_count, 1);
    assert!(!report.applied_changed);
    let after_second = fs::read_to_string(home.join(OPTIONS_TXT)).unwrap();
    assert_eq!(
        after_second,
        "version:4325\n# comment\nfov:0.25\ncustomModOption:keep_me\nsoundCategory_music:0.0\nkey_key.jump:key.keyboard.space\nlang:en_us\n"
    );

    // Change the Once entry value in launcher (fov -> 0.75 / 100 deg): applies once more!
    upsert_entry(
        &mut cfg.entries,
        "fov".into(),
        "0.75".into(),
        ApplyMode::Once,
    );
    let report = plan_and_apply(&home, &global, &mut cfg).unwrap();
    assert_eq!(report.written_count, 2); // soundCategory_music (Always) + fov (changed Once)
    assert!(report.applied_changed);
    assert_eq!(cfg.applied.get("fov").map(String::as_str), Some("0.75"));
    let after_third = fs::read_to_string(home.join(OPTIONS_TXT)).unwrap();
    assert!(after_third.contains("fov:0.75\n"));
    assert!(after_third.contains("lang:en_us\n"));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn missing_options_txt_is_created_and_use_usual_switch_controls_inheritance() {
    let root = scratch("missing");
    let home = root.join("fresh-instance");

    let global = GlobalProfile {
        entries: vec![OptionEntry {
            key: "soundCategory_music".into(),
            value: "0.0".into(),
            mode: ApplyMode::Always,
        }],
    };
    let mut cfg = InstanceConfig {
        use_usual: false,
        entries: vec![OptionEntry {
            key: "bobView".into(),
            value: "false".into(),
            mode: ApplyMode::Always,
        }],
        applied: BTreeMap::new(),
    };

    plan_and_apply(&home, &global, &mut cfg).unwrap();
    let written = fs::read_to_string(home.join(OPTIONS_TXT)).unwrap();
    assert_eq!(written, "bobView:false\n");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn storage_round_trips_global_and_instance_config() {
    let root = scratch("storage");
    let data = root.join("data");
    let profile = GlobalProfile {
        entries: vec![
            OptionEntry {
                key: "soundCategory_music".into(),
                value: "0.0".into(),
                mode: ApplyMode::Always,
            },
            OptionEntry {
                key: "fov".into(),
                value: "0.5".into(),
                mode: ApplyMode::Once,
            },
        ],
    };
    save_global_in(&data, &profile).unwrap();
    assert_eq!(load_global_in(&data), profile);

    let mut cfg = InstanceConfig {
        use_usual: false,
        entries: vec![OptionEntry {
            key: "renderDistance".into(),
            value: "16".into(),
            mode: ApplyMode::Once,
        }],
        applied: BTreeMap::from([("renderDistance".into(), "16".into())]),
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let decoded: InstanceConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, cfg);
    assert!(!cfg.is_default());

    cfg.use_usual = true;
    cfg.entries.clear();
    cfg.applied.clear();
    assert!(cfg.is_default());
    // Default config deserializes with use_usual = true when absent
    let empty: InstanceConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(empty, InstanceConfig::default());
    assert!(empty.use_usual);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn loads_friendly_and_existing_custom_options_from_options_txt() {
    let root = scratch("load-from-file");
    let home = root.join("inst");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(OPTIONS_TXT),
        "fov:0.5\nsoundCategory_music:0.0\nbobView:false\nlang:ja_jp\nkey_key.jump:key.keyboard.space\nrenderClouds:\"fast\"\n",
    )
    .unwrap();

    let existing = vec![
        OptionEntry {
            key: "fov".into(),
            value: "0.0".into(),
            mode: ApplyMode::Once,
        },
        OptionEntry {
            key: "renderClouds".into(),
            value: "\"true\"".into(),
            mode: ApplyMode::Once,
        },
    ];
    let loaded = load_profile_from_options_txt(&home, &existing)
        .unwrap()
        .unwrap();
    assert_eq!(
        get_entry(&loaded, "fov"),
        Some(&OptionEntry {
            key: "fov".into(),
            value: "0.5".into(),
            mode: ApplyMode::Once,
        })
    );
    assert_eq!(
        get_entry(&loaded, "soundCategory_music"),
        Some(&OptionEntry {
            key: "soundCategory_music".into(),
            value: "0.0".into(),
            mode: ApplyMode::Always,
        })
    );
    assert_eq!(
        get_entry(&loaded, "renderClouds"),
        Some(&OptionEntry {
            key: "renderClouds".into(),
            value: "\"fast\"".into(),
            mode: ApplyMode::Once,
        })
    );
    // Unrequested keybind lines are not dumped into the profile automatically
    assert!(get_entry(&loaded, "key_key.jump").is_none());

    let _ = fs::remove_dir_all(&root);
}
