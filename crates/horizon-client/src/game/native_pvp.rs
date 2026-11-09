use std::cell::Cell;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Source {
    victim: u32,
    attacker: u32,
    direct: u32,
}

thread_local! {
    static SOURCE: Cell<Option<Source>> = const { Cell::new(None) };
}

pub(super) struct SourceScope(Option<Source>);

impl Drop for SourceScope {
    fn drop(&mut self) {
        SOURCE.set(self.0);
    }
}

pub(super) fn scope(victim: u32, attacker: u32, direct: u32) -> SourceScope {
    SourceScope(SOURCE.replace(Some(Source {
        victim,
        attacker,
        direct,
    })))
}

pub(super) fn source(victim: u32) -> Option<(u32, u32)> {
    SOURCE.get().and_then(|source| {
        (source.victim == victim && source.attacker != 0)
            .then_some((source.attacker, source.direct))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_scope_preserves_unknown_attacker() {
        assert_eq!(source(7), None);
    }

    #[test]
    fn pvp_scope_identifies_killer_and_restores_outer_scope() {
        let _melee = scope(7, 3, 3);
        assert_eq!(source(7), Some((3, 3)));
        assert_eq!(source(8), None);
        {
            let _projectile = scope(8, 4, 9);
            assert_eq!(source(8), Some((4, 9)));
            assert_eq!(source(7), None);
        }
        assert_eq!(source(7), Some((3, 3)));
    }
}
