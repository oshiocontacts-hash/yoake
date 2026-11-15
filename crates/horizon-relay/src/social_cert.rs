//! Loading the social listener's DER certificate and PKCS#8 key.
//!
//! The key file is opened once and the checks run on that handle, so the file
//! cannot be swapped between the check and the read. Not done, for lack of a
//! crate for the current uid: requiring the key to be owned by the current user.
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
};

fn context(error: io::Error, action: &str, path: &Path) -> io::Error {
    io::Error::new(error.kind(), format!("can't {action} {}: {error}", path.display()))
}

pub fn read_certificate_and_key(cert: &Path, key: &Path) -> io::Result<(Vec<u8>, Vec<u8>)> {
    let key = read_private_key(key)?;
    let cert = std::fs::read(cert).map_err(|error| context(error, "read the social TLS certificate", cert))?;
    Ok((cert, key))
}

fn read_private_key(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path).map_err(|error| context(error, "open the social TLS key", path))?;
    let metadata = file.metadata().map_err(|error| context(error, "inspect the social TLS key", path))?;
    if !metadata.is_file() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("social TLS key {} is not a regular file", path.display())));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("social TLS key {} is accessible by group or others", path.display()),
            ));
        }
    }
    // Windows ACLs cannot be represented by Unix mode bits; the deployer manages them.
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| context(error, "read the social TLS key", path))?;
    Ok(bytes)
}

/// `(not_before, not_after)` of a DER certificate as Unix seconds.
pub fn certificate_validity(der: &[u8]) -> io::Result<(i64, i64)> {
    parse_validity(der).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "social TLS certificate is not valid DER X.509"))
}

pub fn check_validity(der: &[u8], now: i64) -> io::Result<()> {
    let (not_before, not_after) = certificate_validity(der)?;
    if now < not_before {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "social TLS certificate is not valid yet"));
    }
    if now >= not_after {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "social TLS certificate has expired"));
    }
    Ok(())
}

/// One DER element: (tag, content, rest).
fn tlv(input: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = input.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = if first < 0x80 {
        (first as usize, rest)
    } else {
        let count = (first & 0x7f) as usize;
        if count == 0 || count > 4 || rest.len() < count { return None; }
        (rest[..count].iter().fold(0usize, |acc, byte| acc << 8 | *byte as usize), &rest[count..])
    };
    if rest.len() < length { return None; }
    Some((tag, &rest[..length], &rest[length..]))
}

fn parse_validity(der: &[u8]) -> Option<(i64, i64)> {
    let (0x30, certificate, _) = tlv(der)? else { return None; };
    let (0x30, tbs, _) = tlv(certificate)? else { return None; };
    let (tag, _, after) = tlv(tbs)?;
    let mut rest = if tag == 0xa0 { after } else { tbs }; // optional explicit version
    for _ in 0..3 {
        rest = tlv(rest)?.2; // serial, signature algorithm, issuer
    }
    let (0x30, validity, _) = tlv(rest)? else { return None; };
    let (start_tag, start, rest) = tlv(validity)?;
    let (end_tag, end, _) = tlv(rest)?;
    Some((parse_time(start_tag, start)?, parse_time(end_tag, end)?))
}

fn parse_time(tag: u8, bytes: &[u8]) -> Option<i64> {
    // Digits then `Z` only: no signs, spaces, offsets or fractions.
    let (digits, zulu) = bytes.split_at(bytes.len().checked_sub(1)?);
    if zulu != b"Z" || !digits.iter().all(u8::is_ascii_digit) { return None; }
    let number = |range: std::ops::Range<usize>| -> Option<i64> { std::str::from_utf8(digits.get(range)?).ok()?.parse().ok() };
    let (year, rest) = match (tag, digits.len()) {
        (0x17, 12) => {
            let year = number(0..2)?;
            (if year >= 50 { 1900 + year } else { 2000 + year }, 2)
        }
        (0x18, 14) => (number(0..4)?, 4),
        _ => return None,
    };
    let (month, day, hour, minute, second) =
        (number(rest..rest + 2)?, number(rest + 2..rest + 4)?, number(rest + 4..rest + 6)?, number(rest + 6..rest + 8)?, number(rest + 8..rest + 10)?);
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let month_days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if leap { 29 } else { 28 },
        _ => return None,
    };
    if !(1..=month_days).contains(&day) || hour > 23 || minute > 59 || second > 59 { return None; }
    // Days from civil (Howard Hinnant).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair, date_time_ymd};

    fn cert(from: (i32, u8, u8), to: (i32, u8, u8)) -> Vec<u8> {
        let mut params = CertificateParams::new(vec!["relay.test".to_owned()]).unwrap();
        params.not_before = date_time_ymd(from.0, from.1, from.2);
        params.not_after = date_time_ymd(to.0, to.1, to.2);
        params.self_signed(&KeyPair::generate().unwrap()).unwrap().der().to_vec()
    }

    #[test]
    fn validity_dates_are_read_from_the_certificate() {
        let der = cert((2024, 3, 1), (2051, 1, 2)); // 2051 is written as GeneralizedTime
        assert_eq!(certificate_validity(&der).unwrap(), (1_709_251_200, 2_556_230_400));
        assert!(check_validity(&der, 1_800_000_000).is_ok());
        assert!(check_validity(&der, 1_700_000_000).unwrap_err().to_string().contains("not valid yet"));
        assert!(check_validity(&der, 2_600_000_000).unwrap_err().to_string().contains("expired"));
        let old = cert((2010, 1, 1), (2020, 1, 1));
        assert!(check_validity(&old, 1_800_000_000).unwrap_err().to_string().contains("expired"));
    }

    #[test]
    fn time_fields_must_be_plain_digits_and_real_dates() {
        let time = |tag: u8, text: &str| parse_time(tag, text.as_bytes());
        assert_eq!(time(0x17, "700101000000Z"), Some(0));
        assert_eq!(time(0x18, "19700101000000Z"), Some(0));
        assert_eq!(time(0x17, "+70101000000Z"), None); // a sign where a digit belongs
        assert_eq!(time(0x18, "-970101000000Z"), None);
        assert_eq!(time(0x17, "700230000000Z"), None); // 30 February
        assert_eq!(time(0x17, "700101006000Z"), None);
        assert_eq!(time(0x17, "700101000000+0100"), None);
        assert_eq!(time(0x17, "7001010000Z"), None);
        assert_eq!(time(0x18, "20000229000000Z"), Some(951_782_400));
        assert_eq!(time(0x18, "21000229000000Z"), None); // 2100 is not a leap year
    }

    #[test]
    fn garbage_is_not_a_certificate() {
        assert!(certificate_validity(b"nope").is_err());
        assert!(certificate_validity(&[0x30, 0x03, 0x30, 0x01]).is_err());
    }

    #[test]
    fn missing_files_name_the_path_and_keep_the_error_kind() {
        let error = read_certificate_and_key(Path::new("nope.der"), Path::new("nope.key")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("nope.key"));
    }

    #[cfg(unix)]
    #[test]
    fn group_or_other_key_permissions_are_refused() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!("yoake-social-key-{}", std::process::id()));
        std::fs::write(&path, b"test").unwrap();
        std::fs::set_permissions(&path, PermissionsExt::from_mode(0o600)).unwrap();
        assert!(read_private_key(&path).is_ok());
        std::fs::set_permissions(&path, PermissionsExt::from_mode(0o644)).unwrap();
        assert_eq!(read_private_key(&path).unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        let _ = std::fs::remove_file(path);
    }
}
