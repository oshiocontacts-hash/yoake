//! horizon-release-sign: checks a release's signed checksums, and makes new keys
//! (docs/release-flow.md, "Signed checksums"). Release signing uses tools/ci/sign_release.sh;
//! the `sign` command is restricted to Yoake indexes, YAKs, catalogs and launcher releases.
//!
//!     horizon-release-sign verify --tag v2026.10.19 [--key <base64>] [--publishing] <folder>
//!     horizon-release-sign sign --tag <yoake-index-v1|yoake-launcher-version> --key-file <file> <folder>
//!     horizon-release-sign keygen --public-out key.pub | gh secret set RELEASE_SIGNING_KEY --env release
//!
//! `verify` checks SHA256SUMS.sig and every file SHA256SUMS lists with the keys built in from
//! release-keys.txt (or `--key`), as the launcher does. With `--publishing` (release.yml) a release
//! may go out unsigned, or signed with a key nobody can check yet, only while release-keys.txt
//! lists no key; once it lists one, anything launchers would refuse fails the release.

use horizon_release_sign::{SIGNATURE, SUMS, Sha256, SigningKey, parse_keys, trusted_keys, verify};
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[cfg(feature = "yak")]
mod yak_cli;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("verify") => check_folder(&args[1..]),
        Some("sign") => sign_folder(&args[1..]),
        Some("keygen") => keygen(&args[1..]),
        _ => Err(
            "usage: horizon-release-sign verify --tag <tag> [--key <base64>]... [--publishing] \
             <folder-or-file> | sign --tag <yoake-index-v1|yoake-index-v2|yoake-yak-v1|yoake-mods-v1|yoake-launcher-version> --key-file <file> <folder-or-file> | \
             keygen --public-out <file>"
                .into(),
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("horizon-release-sign: {e}");
            ExitCode::FAILURE
        }
    }
}

fn sign_folder(args: &[String]) -> Result<(), String> {
    let (tags, rest) = options(args, "--tag");
    if tags.len() != 1 || !allowed_signing_tag(&tags[0]) {
        return Err("sign requires a Yoake index, YAK, catalog or launcher release tag".into());
    }
    let (paths, rest) = options(&rest, "--key-file");
    if paths.len() != 1 || rest.len() != 1 || rest[0].starts_with("--") {
        return Err("sign needs one --key-file and folder or file".into());
    }
    let key = std::fs::read_to_string(&paths[0]).map_err(|e| format!("key: {e}"))?;
    let key = SigningKey::from_base64(&key)?;
    if matches!(tags[0].as_str(), "yoake-yak-v1" | "yoake-mods-v1") {
        #[cfg(feature = "yak")]
        {
            yak_cli::sign(&tags[0], Path::new(&rest[0]), &key)?;
            println!("{}", key.public_base64());
            return Ok(());
        }
        #[cfg(not(feature = "yak"))]
        return Err("YAK/catalog signing requires the yak feature".into());
    }
    let folder = Path::new(&rest[0]);
    let sums = std::fs::read(folder.join(SUMS)).map_err(|e| format!("{SUMS}: {e}"))?;
    let mut signature = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(folder.join(SIGNATURE))
        .map_err(|e| format!("{SIGNATURE}: {e}"))?;
    writeln!(signature, "{}", key.sign(&tags[0], &sums))
        .map_err(|e| format!("{SIGNATURE}: {e}"))?;
    println!("{}", key.public_base64());
    Ok(())
}

fn allowed_signing_tag(tag: &str) -> bool {
    if matches!(tag, "yoake-index-v1" | "yoake-index-v2" | "yoake-yak-v1" | "yoake-mods-v1") {
        return true;
    }
    let Some(version) = tag.strip_prefix("yoake-launcher-") else {
        return false;
    };
    if version.is_empty() {
        return false;
    }
    let mut build_parts = version.split('+');
    let without_build = build_parts.next().unwrap_or_default();
    if let Some(build) = build_parts.next() {
        if !valid_identifiers(build, false) || build_parts.next().is_some() {
            return false;
        }
    }
    let (core, prerelease) = without_build
        .split_once('-')
        .map_or((without_build, None), |(core, pre)| (core, Some(pre)));
    if prerelease.is_some_and(|pre| !valid_identifiers(pre, true)) {
        return false;
    }
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part.len() == 1 || !part.starts_with('0'))
                && part.parse::<u64>().is_ok()
        })
}

fn valid_identifiers(value: &str, prerelease: bool) -> bool {
    value.split('.').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && (!prerelease
                || part.len() == 1
                || !part.bytes().all(|byte| byte.is_ascii_digit())
                || !part.starts_with('0'))
    })
}

#[cfg(test)]
mod signing_tag_tests {
    use super::allowed_signing_tag;

    #[test]
    fn permits_only_yoake_domains_and_launcher_semver_tags() {
        for tag in [
            "yoake-index-v1",
            "yoake-index-v2",
            "yoake-yak-v1",
            "yoake-mods-v1",
            "yoake-launcher-0.2.0",
            "yoake-launcher-1.2.3-rc.1+build.42",
            "yoake-launcher-1.2.3-this-is-a-long-but-valid-prerelease",
        ] {
            assert!(allowed_signing_tag(tag), "{tag}");
        }
        for tag in [
            "v1.2.3",
            "yoake-index-v3",
            "yoake-yak-v2",
            "yoake-mods-v2",
            "yoake-launcher-",
            "yoake-launcher-1.2",
            "yoake-launcher-01.2.3",
            "yoake-launcher-1.2.3-01",
            "yoake-launcher-1.2.3+",
            "yoake-launcher-1.2.3+one+two",
            "yoake-launcher-1.2.3/other",
        ] {
            assert!(!allowed_signing_tag(tag), "{tag}");
        }
    }
}

/// The values of every `--name value` in `args`, and the other arguments.
fn options(args: &[String], name: &str) -> (Vec<String>, Vec<String>) {
    let mut values = Vec::new();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            values.extend(it.next().cloned());
        } else {
            rest.push(arg.clone());
        }
    }
    (values, rest)
}

fn tag_of(args: &[String]) -> Result<(String, Vec<String>), String> {
    let (tags, rest) = options(args, "--tag");
    let tag = tags.into_iter().next().ok_or("--tag <tag> is needed")?;
    Ok((tag, rest))
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Sha256::default();
    let mut buffer = vec![0; 1 << 16];
    loop {
        let n = file
            .read(&mut buffer)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if n == 0 {
            return Ok(hash.hex());
        }
        hash.update(&buffer[..n]);
    }
}

/// Checks a folder holding a release's assets, as the launcher does.
fn check_folder(args: &[String]) -> Result<(), String> {
    let (tag, rest) = tag_of(args)?;
    let (keys, rest) = options(&rest, "--key");
    let publishing = rest.iter().any(|a| a == "--publishing");
    let folder = PathBuf::from(
        rest.iter()
            .find(|a| *a != "--publishing")
            .ok_or("verify needs the release's folder")?,
    );
    let keys = if keys.is_empty() {
        trusted_keys()
    } else {
        parse_keys(&keys.join("\n"))?
    };
    if matches!(tag.as_str(), "yoake-yak-v1" | "yoake-mods-v1") {
        if publishing || rest.len() != 1 {
            return Err("YAK/catalog verify needs one file and does not allow --publishing".into());
        }
        #[cfg(feature = "yak")]
        return yak_cli::verify(&tag, &folder, &keys);
        #[cfg(not(feature = "yak"))]
        return Err("YAK/catalog verification requires the yak feature".into());
    }
    let sums = std::fs::read(folder.join(SUMS)).map_err(|e| format!("{SUMS}: {e}"))?;
    let listed = match std::fs::read_to_string(folder.join(SIGNATURE)) {
        Ok(signature) if !keys.is_empty() => verify(&tag, &sums, &signature, &keys)?,
        Ok(_) if publishing => {
            println!(
                "::warning::{tag}: signed, but release-keys.txt lists no key yet, so launchers can't check it"
            );
            horizon_release_sign::parse_sums(&String::from_utf8_lossy(&sums))?
        }
        Err(_) if publishing && keys.is_empty() => {
            println!(
                "::warning::{tag}: published with {SUMS} but unsigned: release-keys.txt lists no key yet"
            );
            horizon_release_sign::parse_sums(&String::from_utf8_lossy(&sums))?
        }
        Ok(_) => return Err("no key to check the signature with".into()),
        Err(_) => {
            return Err(format!(
                "{tag} has no {SIGNATURE}: launchers would refuse it (is the release environment's \
                 RELEASE_SIGNING_KEY set?)"
            ));
        }
    };
    for (name, hex) in &listed {
        horizon_release_sign::check(name, hex, &file_hash(&folder.join(name))?)?;
    }
    println!("{tag}: {} files match {SUMS}", listed.len());
    Ok(())
}

/// Prints a new private key to stdout, which must be a pipe (into `gh secret set`), never a
/// terminal, and writes its public key to `--public-out`.
fn keygen(args: &[String]) -> Result<(), String> {
    let (public_out, _) = options(args, "--public-out");
    let public_out = public_out
        .into_iter()
        .next()
        .ok_or("keygen needs --public-out <file>")?;
    if std::io::stdout().is_terminal() {
        return Err(
            "keygen writes the private key to its output: pipe it into `gh secret set`".into(),
        );
    }
    let (key, secret) = SigningKey::generate()?;
    std::fs::write(&public_out, key.public_base64() + "\n").map_err(|e| e.to_string())?;
    let mut out = std::io::stdout().lock();
    out.write_all(secret.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|e| e.to_string())
}
