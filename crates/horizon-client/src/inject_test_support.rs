use std::sync::{Mutex, MutexGuard};

static SITE_CLAIMS: Mutex<()> = Mutex::new(());

pub(crate) fn site_claim_guard() -> MutexGuard<'static, ()> {
    SITE_CLAIMS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::site_claim_guard;
    use yoake_inject_rt::{RETURN, Registry, Side, Site, ValueType};

    static SITE: Site = Site::new(
        "test::guarded_site",
        "0000000000000000000000000000000000000000000000000000000000000000",
        Side::Server,
        &[],
        ValueType::S32,
        RETURN,
    );

    #[test]
    fn releases_claim_and_recovers_lock_after_panic() {
        let result = std::panic::catch_unwind(|| {
            let _guard = site_claim_guard();
            let _registry = Registry::new([&SITE]).unwrap();
            panic!("intentional panic while holding a claim");
        });
        assert!(result.is_err());

        let _guard = site_claim_guard();
        assert!(Registry::new([&SITE]).is_ok());
    }
}
