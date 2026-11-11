use crate::yoake_auth::YoakeAuth;
use crate::yoake_feed::FeedTransport;
use semver::Version;
use serde::Deserialize;

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Deserialize)]
struct LatestLauncher {
    version: String,
    file: String,
    sha256: String,
    size: u64,
    published_at: String,
}

#[cfg(test)]
pub(super) fn check(
    portal: &str,
    auth: &dyn YoakeAuth,
    transport: &dyn FeedTransport,
    current: &str,
) -> Option<(String, String)> {
    check_result(portal, auth, transport, current)
        .ok()
        .flatten()
}

pub(super) fn check_result(
    portal: &str,
    auth: &dyn YoakeAuth,
    transport: &dyn FeedTransport,
    current: &str,
) -> Result<Option<(String, String)>, String> {
    let invalid = || "The portal's launcher update information is invalid.".to_string();
    let root = crate::yoake_self_update::portal_root(portal)?;
    let endpoint = crate::yoake_self_update::endpoint(&root, "yoake/launcher/latest.json")?;
    let page = crate::yoake_self_update::endpoint(&root, "launcher")?;
    let authorization = auth
        .authorization()
        .map_err(|_| "Cannot read the portal sign-in. Please sign in again.")?
        .ok_or("Please sign in to the portal before checking for updates.")?;
    let bytes = transport
        .get(endpoint.as_str(), Some(&authorization), MAX_RESPONSE_BYTES)
        .map_err(crate::yoake_self_update::transport_error)?;
    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err(invalid());
    }
    let latest: LatestLauncher = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    if latest.file.is_empty()
        || latest.sha256.len() != 64
        || !latest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        || latest.size == 0
        || latest.published_at.is_empty()
    {
        return Err(invalid());
    }
    let version = Version::parse(&latest.version).map_err(|_| invalid())?;
    let installed = Version::parse(current).map_err(|_| "This launcher's version is invalid.")?;
    if version.cmp_precedence(&installed) != std::cmp::Ordering::Greater {
        return Ok(None);
    }
    Ok(Some((latest.version, page.to_string())))
}

pub(super) fn portal_page(portal: &str) -> Option<String> {
    let root = crate::yoake_self_update::portal_root(portal).ok()?;
    crate::yoake_self_update::endpoint(&root, "launcher")
        .ok()
        .map(|url| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yoake_auth::AuthError;
    use crate::yoake_feed::FeedError;
    use std::sync::Mutex;

    struct Auth(bool);

    impl YoakeAuth for Auth {
        fn authorization(&self) -> Result<Option<String>, AuthError> {
            Ok(self.0.then(|| "Bearer saved-token".to_string()))
        }
    }

    struct Transport {
        result: Result<Vec<u8>, FeedError>,
        requests: Mutex<Vec<(String, Option<String>)>>,
    }

    impl FeedTransport for Transport {
        fn get(
            &self,
            url: &str,
            authorization: Option<&str>,
            _limit: usize,
        ) -> Result<Vec<u8>, FeedError> {
            self.requests
                .lock()
                .unwrap()
                .push((url.to_string(), authorization.map(str::to_string)));
            self.result.clone()
        }
    }

    fn response(version: &str) -> Vec<u8> {
        format!(
            r#"{{"version":"{version}","file":"launcher.zip","sha256":"{}","size":42,"published_at":"2026-11-10"}}"#,
            "a".repeat(64)
        )
        .into_bytes()
    }

    #[test]
    fn checks_with_saved_auth_and_compares_semver() {
        let transport = Transport {
            result: Ok(response("1.2.10")),
            requests: Mutex::new(Vec::new()),
        };
        assert_eq!(
            check(
                "https://portal.example/base",
                &Auth(true),
                &transport,
                "1.2.9"
            ),
            Some((
                "1.2.10".to_string(),
                "https://portal.example/base/launcher".to_string()
            ))
        );
        assert_eq!(
            transport.requests.lock().unwrap()[0],
            (
                "https://portal.example/base/yoake/launcher/latest.json".to_string(),
                Some("Bearer saved-token".to_string())
            )
        );
        assert!(check("https://portal.example", &Auth(true), &transport, "1.2.10").is_none());
        assert!(check("https://portal.example", &Auth(true), &transport, "2.0.0").is_none());

        let metadata = Transport {
            result: Ok(response("1.2.10+new-build")),
            requests: Mutex::new(Vec::new()),
        };
        assert!(
            check(
                "https://portal.example",
                &Auth(true),
                &metadata,
                "1.2.10+old-build"
            )
            .is_none()
        );
        assert!(
            check(
                "https://portal.example",
                &Auth(true),
                &metadata,
                "1.2.10-rc.1"
            )
            .is_some()
        );
    }

    #[test]
    fn ignores_signed_out_invalid_and_offline_responses() {
        let transport = Transport {
            result: Ok(b"{}".to_vec()),
            requests: Mutex::new(Vec::new()),
        };
        assert!(check("https://portal.example", &Auth(false), &transport, "1.0.0").is_none());
        assert!(transport.requests.lock().unwrap().is_empty());
        assert!(check("https://portal.example", &Auth(true), &transport, "1.0.0").is_none());
        assert!(check("http://portal.example", &Auth(true), &transport, "1.0.0").is_none());
        for error in [FeedError::Network, FeedError::HttpStatus(401)] {
            let transport = Transport {
                result: Err(error),
                requests: Mutex::new(Vec::new()),
            };
            assert!(check("https://portal.example", &Auth(true), &transport, "1.0.0").is_none());
        }
    }
}
