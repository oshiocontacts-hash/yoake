use yoake_slice::runtime::{ABI_VERSION, AbiCheck, AbiVersion, check_abi, module_abi_version};

const fn version(major: u16, minor: u16) -> AbiVersion {
    AbiVersion { major, minor }
}

fn custom_section(version: AbiVersion) -> String {
    let major = version.major.to_le_bytes();
    let minor = version.minor.to_le_bytes();
    format!(
        r#"(@custom "yoake_abi" "\{:02x}\{:02x}\{:02x}\{:02x}")"#,
        major[0], major[1], minor[0], minor[1]
    )
}

fn assert_binary_and_wat(wat: &str, expected: Option<AbiVersion>) {
    assert_eq!(module_abi_version(wat.as_bytes()).unwrap(), expected, "WAT input");
    let binary = wat::parse_str(wat).unwrap();
    assert_eq!(module_abi_version(&binary).unwrap(), expected, "binary input");
}

fn assert_binary_and_wat_error(wat: &str, message: &str) {
    assert!(module_abi_version(wat.as_bytes()).unwrap_err().to_string().contains(message), "WAT input");
    let binary = wat::parse_str(wat).unwrap();
    assert!(module_abi_version(&binary).unwrap_err().to_string().contains(message), "binary input");
}

#[test]
fn reads_versions_from_binary_and_wat_modules() {
    assert_eq!(ABI_VERSION, version(1, 1));
    for guest in [version(1, 0), version(1, 5), version(2, 0), version(0, 9)] {
        let wat = format!("(module {})", custom_section(guest));
        assert_binary_and_wat(&wat, Some(guest));
    }
}

#[test]
fn a_missing_section_returns_none() {
    assert_binary_and_wat("(module)", None);
}

#[test]
fn a_wrong_payload_length_is_rejected() {
    assert_binary_and_wat_error(r#"(module (@custom "yoake_abi" "\01\00\00"))"#, "expected 4");
}

#[test]
fn duplicate_abi_sections_are_rejected() {
    let section = custom_section(ABI_VERSION);
    let wat = format!("(module {section} {section})");
    assert_binary_and_wat_error(&wat, "more than one");
}

#[test]
fn other_custom_sections_may_precede_and_follow_the_abi_section() {
    let wat = format!(
        r#"(module
            (@custom "name" "\00\01module")
            (@custom "other" "before")
            {}
            (@custom "last" "after"))"#,
        custom_section(ABI_VERSION)
    );
    assert_binary_and_wat(&wat, Some(ABI_VERSION));
}

#[test]
fn compatibility_checks_follow_major_and_minor_rules() {
    assert_eq!(check_abi(ABI_VERSION), AbiCheck::Ok);
    assert_eq!(check_abi(version(1, 0)), AbiCheck::OkOlderMinor);
    assert_eq!(check_abi(version(1, 5)), AbiCheck::OkNewerMinor);
    for guest in [version(2, 0), version(0, 9)] {
        let AbiCheck::Refuse(reason) = check_abi(guest) else {
            panic!("guest {}.{} was not refused", guest.major, guest.minor);
        };
        assert!(reason.contains("major version"));
    }
}
