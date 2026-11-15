use std::collections::BTreeSet;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::{Host, Url};

const MAX_HOSTS: usize = 256;
const MAX_HOST_LENGTH: usize = 253;
const MAX_URL_LENGTH: usize = 8192;
const MAX_DNS_ADDRESSES: usize = 64;

#[derive(Clone, Debug)]
pub struct HttpsPolicy {
    hosts: BTreeSet<String>,
}

impl HttpsPolicy {
    pub fn new(hosts: &[String]) -> Result<Self, String> {
        if hosts.is_empty() || hosts.len() > MAX_HOSTS {
            return Err("HTTPS host declarations must contain between 1 and 256 hosts".into());
        }
        let mut declared = BTreeSet::new();
        for host in hosts {
            if host.is_empty()
                || host.len() > MAX_HOST_LENGTH
                || host.chars().any(|character| matches!(character, '/' | '\\' | '?' | '#' | '@' | ':' | '%' | '*'))
            {
                return Err("invalid HTTPS host declaration".into());
            }
            let url = Url::parse(&format!("https://{host}/")).map_err(|_| "invalid HTTPS host declaration")?;
            let Some(Host::Domain(domain)) = url.host() else {
                return Err("HTTPS host declarations must be domain names".into());
            };
            if domain.len() > MAX_HOST_LENGTH || domain.ends_with('.') || !declared.insert(domain.to_owned()) {
                return Err("duplicate or non-canonical HTTPS host declaration".into());
            }
        }
        Ok(Self { hosts: declared })
    }

    pub fn authorize_url(
        &self,
        raw_url: &str,
        permission_declared: bool,
        user_allowed: bool,
    ) -> Result<Url, String> {
        if !permission_declared || !user_allowed {
            return Err("HTTPS permission was not granted".into());
        }
        if raw_url.len() > MAX_URL_LENGTH {
            return Err("HTTPS URL is too long".into());
        }
        let url = Url::parse(raw_url).map_err(|_| "invalid HTTPS URL")?;
        if url.scheme() != "https" {
            return Err("URL must use HTTPS".into());
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err("HTTPS URL must not contain credentials".into());
        }
        let Some(Host::Domain(domain)) = url.host() else {
            return Err("HTTPS URL must use a declared domain".into());
        };
        if !self.hosts.contains(domain) {
            return Err("HTTPS host was not declared".into());
        }
        Ok(url)
    }
}

fn public_ipv4(address: Ipv4Addr) -> bool {
    let value = u32::from(address);
    const BLOCKED: [(u32, u32); 15] = [
        (0x0000_0000, 8),
        (0x0a00_0000, 8),
        (0x6440_0000, 10),
        (0x7f00_0000, 8),
        (0xa9fe_0000, 16),
        (0xac10_0000, 12),
        (0xc000_0000, 24),
        (0xc000_0200, 24),
        (0xc058_6300, 24),
        (0xc0a8_0000, 16),
        (0xc612_0000, 15),
        (0xc633_6400, 24),
        (0xcb00_7100, 24),
        (0xe000_0000, 4),
        (0xf000_0000, 4),
    ];
    !BLOCKED.iter().any(|(network, prefix)| value & (u32::MAX << (32 - *prefix)) == *network)
}

fn public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return public_ipv4(mapped);
    }
    let segments = address.segments();
    segments[0] & 0xe000 == 0x2000
        && !(segments[0] == 0x2001 && segments[1] & 0xfe00 == 0)
        && !(segments[0] == 0x2001 && segments[1] == 0x0db8)
        && segments[0] != 0x2002
        && !(segments[0] == 0x3fff && segments[1] & 0xf000 == 0)
}

pub fn validate_https_addresses(addresses: &[IpAddr]) -> bool {
    !addresses.is_empty()
        && addresses.len() <= MAX_DNS_ADDRESSES
        && addresses.iter().all(|address| match address {
            IpAddr::V4(address) => public_ipv4(*address),
            IpAddr::V6(address) => public_ipv6(*address),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_requires_declared_host_permission_and_user_consent() {
        let policy = HttpsPolicy::new(&["Example.COM".into()]).unwrap();
        assert!(policy.authorize_url("https://example.com/path", true, true).is_ok());
        assert!(policy.authorize_url("https://example.com", false, true).is_err());
        assert!(policy.authorize_url("https://example.com", true, false).is_err());
        assert!(policy.authorize_url("http://example.com", true, true).is_err());
        assert!(policy.authorize_url("https://other.example.com", true, true).is_err());
        assert!(policy.authorize_url("https://example.com@evil.example", true, true).is_err());
        assert!(policy.authorize_url("https://127.0.0.1", true, true).is_err());
        assert!(HttpsPolicy::new(&["example.com/path".into()]).is_err());
        assert!(HttpsPolicy::new(&["*.example.com".into()]).is_err());
        assert!(HttpsPolicy::new(&["EXAMPLE.com".into(), "example.com".into()]).is_err());
    }

    #[test]
    fn every_resolved_address_must_be_public() {
        let parse = |address: &str| address.parse::<IpAddr>().unwrap();
        assert!(validate_https_addresses(&[parse("8.8.8.8"), parse("2606:4700:4700::1111")]));
        assert!(!validate_https_addresses(&[]));
        for address in [
            "127.0.0.1", "10.1.2.3", "100.64.0.1", "169.254.169.254", "172.16.0.1",
            "192.168.1.1", "198.18.0.1", "224.0.0.1", "::1", "fe80::1", "fc00::1",
            "::ffff:127.0.0.1", "2001:db8::1", "2002:0a00:0001::",
        ] {
            assert!(!validate_https_addresses(&[parse("8.8.8.8"), parse(address)]), "{address}");
        }
    }
}
