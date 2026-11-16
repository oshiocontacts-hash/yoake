use crate::{
    social::{SignedGrant, decode_hex},
    social_client::Endpoint,
    social_control::{SignedProvision, invalid},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

#[cfg(windows)]
mod windows_acl;

const MAX_HANDOFF: u64 = 65_536;

#[derive(Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case", deny_unknown_fields)]
pub enum Handoff {
    Host {
        session_id: String,
        host_principal: String,
        host_token: String,
        provision: SignedProvision,
        relay: String,
    },
    Guest {
        session_id: String,
        grant: crate::social::Grant,
        signature: String,
        relay: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Trust {
    portal_public_key: String,
    ca_certificates: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Pair { grant: SignedGrant },
    End {},
}

impl Trust {
    pub fn read(path: &Path) -> io::Result<Self> {
        let file = File::open(path).map_err(|_| invalid())?;
        if !file.metadata().map_err(|_| invalid())?.is_file() {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        file.take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(|_| invalid())?;
        if bytes.len() > 1_048_576 {
            return Err(invalid());
        }
        serde_json::from_slice(&bytes).map_err(|_| invalid())
    }

    pub fn endpoint(&self, relay: &str) -> io::Result<Endpoint> {
        let public_key = decode_hex::<32>(&self.portal_public_key).ok_or_else(invalid)?;
        if self.ca_certificates.is_empty() || self.ca_certificates.len() > 8 {
            return Err(invalid());
        }
        let roots =
            self.ca_certificates
                .iter()
                .map(|encoded| {
                    if encoded.is_empty() || encoded.len() % 2 != 0 {
                        return Err(invalid());
                    }
                    encoded
                        .as_bytes()
                        .chunks_exact(2)
                        .map(|pair| {
                            let pair = std::str::from_utf8(pair).map_err(|_| invalid())?;
                            if pair.bytes().any(|byte| {
                                !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte)
                            }) {
                                return Err(invalid());
                            }
                            u8::from_str_radix(pair, 16).map_err(|_| invalid())
                        })
                        .collect::<io::Result<Vec<_>>>()
                })
                .collect::<io::Result<Vec<_>>>()?;
        Endpoint::new(relay.to_owned(), roots, public_key)
    }
}

pub fn read_once(path: &Path) -> io::Result<Handoff> {
    let metadata = fs::symlink_metadata(path).map_err(|_| invalid())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000).share_mode(4);
    }
    let mut file = options.open(path).map_err(|_| invalid())?;
    if !private_regular_file(&file) {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    let read = (&mut file).take(MAX_HANDOFF + 1).read_to_end(&mut bytes);
    let deleted = fs::remove_file(path);
    drop(file);
    read.map_err(|_| invalid())?;
    deleted.map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_HANDOFF {
        return Err(invalid());
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

pub fn write_new(path: &Path, handoff: &Handoff) -> io::Result<()> {
    let bytes = serde_json::to_vec(handoff).map_err(|_| invalid())?;
    if bytes.len() as u64 > MAX_HANDOFF {
        return Err(invalid());
    }
    #[cfg(windows)]
    let mut file = windows_acl::create_private_file(path).map_err(|_| invalid())?;
    #[cfg(unix)]
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|_| invalid())?
    };
    #[cfg(not(any(unix, windows)))]
    return Err(invalid());
    #[cfg(any(unix, windows))]
    {
        if file
            .write_all(&bytes)
            .and_then(|_| file.sync_all())
            .is_err()
        {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(invalid());
        }
        Ok(())
    }
}

fn private_regular_file(file: &File) -> bool {
    let Ok(metadata) = file.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        metadata.mode() & 0o777 == 0o600
            && metadata.nlink() == 1
            && metadata.uid() == unsafe { libc::getuid() }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 == 0 && windows_acl::is_user_only(file)
    }
    #[cfg(not(any(unix, windows)))]
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn path() -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        directory.join(format!(
            "yoake-social-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn guest() -> Handoff {
        Handoff::Guest {
            session_id: "session".into(),
            relay: "localhost:25566".into(),
            signature: "secret-signature".into(),
            grant: crate::social::Grant {
                v: 1,
                session_id: "session".into(),
                host_principal: "host".into(),
                guest_principal: "guest".into(),
                mc_profile: None,
                relay_only: true,
                exp: 123,
                nonce: "ab".repeat(32),
            },
        }
    }

    #[test]
    fn social_handoff_is_private_deleted_and_not_replayable() {
        let path = path();
        write_new(&path, &guest()).unwrap();
        assert!(write_new(&path, &guest()).is_err());
        assert!(matches!(read_once(&path).unwrap(), Handoff::Guest { .. }));
        assert!(!path.exists());
        assert!(read_once(&path).is_err());
    }

    #[test]
    fn social_handoff_malformed_and_oversized_files_are_consumed_without_echo() {
        for content in [
            b"secret-signature-invalid".to_vec(),
            vec![b'x'; MAX_HANDOFF as usize + 1],
        ] {
            let path = path();
            #[cfg(windows)]
            let mut file = windows_acl::create_private_file(&path).unwrap();
            #[cfg(unix)]
            let mut file = {
                use std::os::unix::fs::OpenOptionsExt;
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&path)
                    .unwrap()
            };
            file.write_all(&content).unwrap();
            drop(file);
            let error = match read_once(&path) {
                Ok(_) => panic!("invalid handoff accepted"),
                Err(error) => error,
            };
            assert_eq!(error.to_string(), "social control request rejected");
            assert!(!path.exists());
        }
    }

    #[test]
    fn social_handoff_rejects_unknown_fields() {
        assert!(serde_json::from_str::<Control>(r#"{"type":"end","secret":"hidden"}"#).is_err());
        assert!(
            serde_json::from_str::<Trust>(
                r#"{"portal_public_key":"aa","ca_certificates":[],"other":1}"#
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn social_handoff_refuses_hard_links_without_consuming_credentials() {
        let original = path();
        let link = path();
        write_new(&original, &guest()).unwrap();
        fs::hard_link(&original, &link).unwrap();
        assert!(read_once(&link).is_err());
        assert!(read_once(&original).is_err());
        assert!(original.exists() && link.exists());
        fs::remove_file(&link).unwrap();
        assert!(read_once(&original).is_ok());
        assert!(!original.exists());
    }

    #[cfg(unix)]
    #[test]
    fn social_handoff_refuses_public_files_and_links_without_deleting_them() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let original = path();
        write_new(&original, &guest()).unwrap();
        let link = path();
        symlink(&original, &link).unwrap();
        assert!(read_once(&link).is_err());
        fs::remove_file(link).unwrap();
        fs::set_permissions(&original, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_once(&original).is_err());
        assert!(original.exists());
        fs::remove_file(original).unwrap();
    }
}
