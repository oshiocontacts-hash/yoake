//! Embeds Horizon's icon (`packaging/icons/horizon.ico`) in the Windows executables, as icon
//! resource 1: Explorer and the task bar show it, and the window takes it as its icon.
#[path = "../packaging/yoake_icon.rs"]
mod yoake_icon;

fn main() {
    println!("cargo:rerun-if-env-changed=YOAKE_FEED_URL");
    println!("cargo:rerun-if-env-changed=YOAKE_PUBLIC_KEYS_FILE");
    println!("cargo:rerun-if-env-changed=YOAKE_FEED_SOURCE_ID");
    println!("cargo:rerun-if-env-changed=YOAKE_PORTAL_URL");
    println!("cargo:rerun-if-env-changed=YOAKE_DISTRIBUTION");
    println!("cargo:rerun-if-env-changed=YOAKE_VERSION");
    let feed_url = std::env::var("YOAKE_FEED_URL").ok();
    let keys_path = std::env::var("YOAKE_PUBLIC_KEYS_FILE").ok();
    match (feed_url, keys_path) {
        (None, None) => {}
        (Some(feed_url), Some(keys_path)) if !feed_url.is_empty() && !keys_path.is_empty() => {
            let keys = std::fs::read_to_string(&keys_path).expect("reading Yoake public keys");
            println!("cargo:rerun-if-changed={keys_path}");
            let keys = keys
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .collect::<Vec<_>>()
                .join(";");
            assert!(!keys.is_empty(), "Yoake public key file must not be empty");
            assert!(
                !feed_url.contains(['\r', '\n']) && !keys.contains(['\r', '\n']),
                "Yoake configuration must not contain newlines"
            );
            println!("cargo:rustc-env=YOAKE_COMPILED_FEED_URL={feed_url}");
            println!("cargo:rustc-env=YOAKE_COMPILED_PUBLIC_KEYS={keys}");
            if let Ok(source_id) = std::env::var("YOAKE_FEED_SOURCE_ID") {
                assert!(
                    !source_id.is_empty() && !source_id.contains(['\r', '\n']),
                    "Yoake source ID must not be empty or contain newlines"
                );
                println!("cargo:rustc-env=YOAKE_COMPILED_SOURCE_ID={source_id}");
            }
        }
        _ => panic!("set both YOAKE_FEED_URL and YOAKE_PUBLIC_KEYS_FILE, or neither"),
    }
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    println!("cargo:rerun-if-changed={dir}/../packaging/icons/horizon.ico");
    println!("cargo:rerun-if-changed={dir}/../packaging/icons/yoake.ico");
    let icon = yoake_icon::icon_file_from_env();
    let ico = format!("{dir}/../packaging/icons/{icon}");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource.set_icon(&ico);
        if let Err(e) = resource.compile() {
            panic!("embedding {ico}: {e}");
        }
    }
}
