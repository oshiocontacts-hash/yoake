use yoake_wasm::Subscriptions;

#[test]
fn known_hooks_are_selected_and_duplicates_are_ignored() {
    let subscriptions = Subscriptions::from_hook_names([
        "tick",
        "net_frames_listened",
        "settings_click",
        "tick",
    ])
    .unwrap();
    let unique =
        Subscriptions::from_hook_names(["tick", "net_frames_listened", "settings_click"]).unwrap();

    assert_eq!(subscriptions, unique);
    assert!(subscriptions.subscribed("tick"));
    assert!(subscriptions.subscribed("net_frames_listened"));
    assert!(subscriptions.subscribed("settings_click"));
    assert!(!subscriptions.subscribed("hud"));
}

#[test]
fn unknown_hook_names_are_rejected() {
    let error = Subscriptions::from_hook_names(["tick", "unknown_hook"]).unwrap_err();
    assert!(error.contains("unknown_hook"));
    assert!(Subscriptions::from_hook_names(["Tick"]).is_err());
}

#[test]
fn empty_subscriptions_are_allowed() {
    let subscriptions = Subscriptions::from_hook_names(Vec::<String>::new()).unwrap();
    assert!(!subscriptions.subscribed("tick"));
}
