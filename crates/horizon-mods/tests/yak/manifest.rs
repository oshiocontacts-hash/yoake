use super::support::{DATA, Temp, program};
use horizon_mods::yak::{Manifest, OptionValue, Target};

#[test]
fn manifest_reads_only_bounded_regular_files() {
    let temp = Temp::new();
    temp.write("yak.toml", DATA.as_bytes());
    assert_eq!(
        Manifest::read(&temp.join("yak.toml")).unwrap().id,
        "sample-mod"
    );
    assert!(Manifest::read(&temp.0).is_err());
    let file = std::fs::File::create(temp.join("large.toml")).unwrap();
    file.set_len(1024 * 1024 + 1).unwrap();
    assert!(Manifest::read(&temp.join("large.toml")).is_err());
}

#[test]
fn mixin_manifests_require_catalog_signatures_and_injection_components() {
    let site = "world::living_fall::calculate_fall_damage#9a9a629c2a289ab6ffb883cb0cd5891af74587992a6e2da94d8090723ede5b94";
    let parse = |targets: &str, wasm: &str| {
        Manifest::parse(&format!("{DATA}\n{wasm}\n[mixin]\ntargets = [{targets}]\n"))
    };
    let wasm = "[wasm]\ninterface = '1.1'\nworld = 'inject-mod'\nserver = 'bin/wasm/a.wasm'";
    for interface in ["1.1", "1.2"] {
        let wasm = wasm.replace("1.1", interface);
        assert_eq!(
            parse(&format!("'{site}'"), &wasm)
                .unwrap()
                .mixin
                .unwrap()
                .targets,
            [site]
        );
    }
    assert!(
        parse(&format!("'{site}'"), "")
            .unwrap_err()
            .contains("requires a [wasm]")
    );
    assert!(
        parse("'game::tick'", wasm)
            .unwrap_err()
            .contains("signature hash")
    );
    assert!(
        parse("'game::tick#BAD'", wasm)
            .unwrap_err()
            .contains("invalid mixin")
    );
    assert!(
        parse(&format!("'game::tick#{}'", "a".repeat(64)), wasm)
            .unwrap_err()
            .contains("unknown mixin target")
    );
    assert!(
        parse(
            &format!("'{}#{}'", site.split_once('#').unwrap().0, "a".repeat(64)),
            wasm
        )
        .unwrap_err()
        .contains("signature mismatch")
    );
    assert!(
        parse(
            &format!("'{site}'"),
            &wasm.replace("inject-mod", "client-mod")
        )
        .unwrap_err()
        .contains("requires wasm.world")
    );
    assert!(
        parse(&format!("'{site}'"), &wasm.replace("server =", "client ="))
            .unwrap_err()
            .contains("requires a Server wasm module")
    );
    assert!(
        parse(&format!("'{site}', '{site}'"), wasm)
            .unwrap_err()
            .contains("unique")
    );
    assert!(parse("", wasm).unwrap_err().contains("1..1024"));
    assert!(Manifest::parse(DATA).unwrap().mixin.is_none());
}

#[test]
fn complete_manifest_and_typed_shader_options() {
    let text = [
        program(),
        r#"
[dependencies]
base-mod = "^0.1"
[optional]
map-mod = ">=1, <2"
[conflicts]
other-mod = "*"
[[shaders]]
id = "kage"
name = "Kage"
[shaders.options.shadow_map]
values = [1024, 2048]
default = 2048
label_ja = "影の解像度"
[shaders.options.enabled]
values = [false, true]
default = true
[shaders.options.exposure]
values = [0.5, 1.0]
default = 1.0
[shaders.options.quality]
values = ["low", "high"]
default = "high"
[[services]]
id = "mimic-brain"
name = "Mimic Brain"
required = false
default_url = "http://127.0.0.1:8790"
help_url = "https://example.test/help"
"#
        .to_owned(),
    ]
    .concat();
    let manifest = Manifest::parse(&text).unwrap();
    assert_eq!(manifest.name_ja.as_deref(), Some("サンプル"));
    assert_eq!(
        manifest.shaders[0].options["shadow_map"].default,
        OptionValue::Integer(2048)
    );
    assert_eq!(
        manifest.services[0].default_url.as_deref(),
        Some("http://127.0.0.1:8790")
    );
    assert_eq!(
        manifest.bin.unwrap().targets,
        [Target::WindowsX64, Target::LinuxX64]
    );
}

#[test]
fn invalid_fields_versions_and_targets_are_rejected() {
    for (old, new) in [
        ("format = 1", "format = 2"),
        ("sample-mod", "../escape"),
        ("sample-mod", "ABc"),
        ("0.3.1", "not-semver"),
        (">=0.3, <0.4", "banana"),
        ("authors = [\"Test author\"]", "authors = []"),
        (
            "client = true\nserver = true",
            "client = false\nserver = false",
        ),
        ("format = 1", "format = 1\nunknown = true"),
    ] {
        assert!(Manifest::parse(&DATA.replace(old, new)).is_err(), "{new}");
    }
    for (old, new) in [
        ("library = \"sample\"", "library = \"../sample\""),
        ("windows-x64", "windows-arm64"),
        ("linux-x64", "windows-x64"),
        ("abi = 2", "abi = 0"),
        ("0.3.1+build.test", "unversioned"),
    ] {
        assert!(
            Manifest::parse(&program().replace(old, new)).is_err(),
            "{new}"
        );
    }
    assert!(Manifest::parse(&format!("{DATA}\n[dependencies]\nsample-mod = '*'\n")).is_err());
    assert!(
        Manifest::parse(&format!(
            "{DATA}\n[dependencies]\nbase-mod = '*'\n[optional]\nbase-mod = '*'\n"
        ))
        .is_err()
    );
}

#[test]
fn shader_options_require_homogeneous_unique_finite_values_and_a_member_default() {
    for options in [
        "values = []\ndefault = 1",
        "values = [1, 1]\ndefault = 1",
        "values = [1, 'high']\ndefault = 1",
        "values = [1, 2]\ndefault = 3",
        "values = [1, 2]\ndefault = 1.0",
        "values = [nan]\ndefault = nan",
        "values = [inf]\ndefault = inf",
    ] {
        let text = format!(
            "{DATA}\n[[shaders]]\nid = 'kage'\nname = 'Kage'\n[shaders.options.test]\n{options}\n"
        );
        assert!(Manifest::parse(&text).is_err(), "{options}");
    }
}

#[test]
fn secret_like_service_keys_are_rejected_without_echoing_values() {
    for field in [
        "api_key",
        "apiKey",
        "token",
        "access-token",
        "password",
        "secret",
        "private_key",
        "credentials",
        "authorization",
        "key",
    ] {
        let text = format!(
            "{DATA}\n[[services]]\nid = 'brain'\nname = 'Brain'\nrequired = false\ndefault_url = 'http://127.0.0.1:8790'\n{field} = 'sensitive-test-value'\n"
        );
        let error = Manifest::parse(&text).unwrap_err();
        assert!(error.to_lowercase().contains("secret"), "{field}: {error}");
        assert!(!error.contains("sensitive-test-value"));
    }
    for nested in [
        "settings = { auth = { bearer_token = 'hidden' } }",
        "settings = [{ password = 'hidden' }]",
    ] {
        let text = format!(
            "{DATA}\n[[services]]\nid = 'brain'\nname = 'Brain'\nrequired = false\ndefault_url = 'http://localhost:8790'\n{nested}\n"
        );
        assert!(
            Manifest::parse(&text)
                .unwrap_err()
                .to_lowercase()
                .contains("secret")
        );
    }
}

#[test]
fn service_urls_cannot_embed_credentials_or_secrets() {
    for url in [
        "http://user:pass@localhost/",
        "https://example.test/?token=hidden",
        "http://localhost/#secret",
        "file:///private",
        "http://",
    ] {
        let text = format!(
            "{DATA}\n[[services]]\nid = 'brain'\nname = 'Brain'\nrequired = false\ndefault_url = '{url}'\n"
        );
        assert!(Manifest::parse(&text).is_err(), "{url}");
    }
}

fn with_wasm(section: &str) -> String {
    [DATA, section].concat()
}

#[test]
fn wasm_section_is_optional_and_parses() {
    assert!(Manifest::parse(DATA).unwrap().wasm.is_none());
    let manifest = Manifest::parse(&with_wasm("\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/sample.wasm\"\nserver = \"bin/wasm/server.wasm\"\n")).unwrap();
    let wasm = manifest.wasm.unwrap();
    assert_eq!(wasm.interface_version().unwrap(), (1, 0));
    assert_eq!(
        wasm.modules(),
        [
            ("client", "bin/wasm/sample.wasm"),
            ("server", "bin/wasm/server.wasm")
        ]
    );
    assert!(
        Manifest::parse(&with_wasm(
            "\n[wasm]\ninterface = \"1.0\"\nserver = \"bin/wasm/s.wasm\"\n"
        ))
        .is_ok()
    );
}

#[test]
fn wasm_interface_versions_are_checked() {
    let parse = |interface: &str| {
        Manifest::parse(&with_wasm(&format!(
            "\n[wasm]\ninterface = \"{interface}\"\nclient = \"bin/wasm/a.wasm\"\n"
        )))
    };
    assert!(parse("1.0").is_ok());
    assert!(parse("2.0").unwrap_err().contains("major"));
    assert!(parse("0.9").unwrap_err().contains("major"));
    assert!(parse("1.1").unwrap_err().contains("require wasm.world"));
    assert!(parse("1.2").unwrap_err().contains("require wasm.world"));
    assert!(parse("1.3").unwrap_err().contains("newer"));
    for bad in ["1", "1.", ".0", "1.x", "1.0.0", "", "-1.0", "1. 0"] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
    assert!(Manifest::parse(&with_wasm("\n[wasm]\nclient = \"bin/wasm/a.wasm\"\n")).is_err());
}

#[test]
fn wasm_needs_a_module_and_safe_paths_under_bin_wasm() {
    assert!(
        Manifest::parse(&with_wasm("\n[wasm]\ninterface = \"1.0\"\n"))
            .unwrap_err()
            .contains("at least one")
    );
    for path in [
        "bin/wasm/../a.wasm",
        "../bin/wasm/a.wasm",
        "/bin/wasm/a.wasm",
        "bin/wasm/a.dll",
        "bin/wasm/.wasm",
        "bin/wasm/",
        "bin/a.wasm",
        "content/a.wasm",
        "bin/wasm/sub/a.wasm",
        "bin\\wasm\\a.wasm",
        "a.wasm",
        "",
    ] {
        let text = with_wasm(&format!(
            "\n[wasm]\ninterface = \"1.0\"\nclient = \"{}\"\n",
            path.replace(char::from(92), "\\\\")
        ));
        assert!(Manifest::parse(&text).is_err(), "client {path:?}");
        let text = with_wasm(&format!(
            "\n[wasm]\ninterface = \"1.0\"\nserver = \"{}\"\n",
            path.replace(char::from(92), "\\\\")
        ));
        assert!(Manifest::parse(&text).is_err(), "server {path:?}");
    }
    assert!(
        Manifest::parse(&with_wasm(
            "\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/a.wasm\"\nextra = 1\n"
        ))
        .is_err()
    );
}

#[test]
fn native_and_wasm_code_cannot_share_a_package() {
    let text = [
        program(),
        "\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/a.wasm\"\n".to_owned(),
    ]
    .concat();
    assert!(Manifest::parse(&text).unwrap_err().contains("not both"));
}

#[test]
fn wasm_permissions_are_known_and_need_a_server_module() {
    let parse = |extra: &str| {
        Manifest::parse(&with_wasm(&format!(
            "
[wasm]
interface = \"1.0\"
client = \"bin/wasm/a.wasm\"
server = \"bin/wasm/s.wasm\"
{extra}"
        )))
    };
    let manifest = parse(
        "permissions = [\"service.commands\"]
",
    )
    .unwrap();
    assert!(
        manifest
            .wasm
            .as_ref()
            .unwrap()
            .has_permission("service.commands")
    );
    let manifest = parse(
        "permissions = [\"service.chat\", \"service.config\", \"service.permissions\", \"service.storage\"]",
    )
    .unwrap();
    for permission in [
        "service.chat",
        "service.config",
        "service.permissions",
        "service.storage",
    ] {
        assert!(manifest.wasm.as_ref().unwrap().has_permission(permission));
    }
    assert!(
        !parse("")
            .unwrap()
            .wasm
            .unwrap()
            .has_permission("service.commands")
    );
    assert!(
        parse(
            "permissions = [\"service.everything\"]
"
        )
        .unwrap_err()
        .contains("unknown wasm permission")
    );
    let client_only = with_wasm(
        "
[wasm]
interface = \"1.0\"
client = \"bin/wasm/a.wasm\"
permissions = [\"service.commands\"]
",
    );
    assert!(
        Manifest::parse(&client_only)
            .unwrap_err()
            .contains("server module")
    );
    for permission in ["service.chat", "service.config", "service.storage"] {
        let client_only = with_wasm(&format!(
            "\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/a.wasm\"\npermissions = [\"{permission}\"]\n"
        ));
        assert!(Manifest::parse(&client_only).is_ok(), "{permission}");
    }
    let client_only = with_wasm(
        "\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/a.wasm\"\npermissions = [\"service.permissions\"]\n",
    );
    assert!(
        Manifest::parse(&client_only)
            .unwrap_err()
            .contains("server module")
    );
}

#[test]
fn http_hosts_and_permission_must_be_declared_together() {
    let parse = |permissions: &str, http: &str| {
        Manifest::parse(&with_wasm(&format!(
            r#"
[wasm]
interface = "1.0"
client = "bin/wasm/a.wasm"
{permissions}
{http}
"#
        )))
    };
    let permission = r#"permissions = ["service.http"]"#;
    let hosts = "[http]\nhosts = ['api.example.com']";
    assert!(parse(permission, hosts).is_ok());
    assert!(parse(permission, "").is_err());
    assert!(parse("", hosts).is_err());
    for host in [
        "127.0.0.1",
        "*.example.com",
        "https://example.com",
        "example.com.",
    ] {
        let http = format!("[http]\nhosts = ['{host}']");
        assert!(parse(permission, &http).is_err(), "{host}");
    }
}
