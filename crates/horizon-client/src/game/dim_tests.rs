use super::Dim;

#[test]
fn vanilla_ids_have_fixed_indices() {
    assert_eq!(Dim::Overworld.0, 0);
    assert_eq!(Dim::Nether.0, 1);
    assert_eq!(Dim::End.0, 2);
    assert_eq!(Dim::Overworld.id(), "minecraft:overworld");
    assert_eq!(Dim::Nether.id(), "minecraft:the_nether");
    assert_eq!(Dim::End.id(), "minecraft:the_end");
    assert_eq!(Dim::lookup("minecraft:the_end"), Some(Dim::End));
}

#[test]
fn unknown_lookup_is_none_and_old_fallback_is_overworld() {
    assert_eq!(Dim::lookup("f30test:never_interned"), None);
    assert_eq!(Dim::lookup_or_overworld("f30test:never_interned"), Dim::Overworld);
    assert_eq!(Dim::lookup_or_overworld(""), Dim::Overworld);
}

#[test]
fn intern_round_trips_and_is_idempotent() {
    let a = Dim::intern("f30test:arena_1");
    assert!(a.0 >= 3);
    assert_eq!(a.id(), "f30test:arena_1");
    assert_eq!(Dim::intern("f30test:arena_1"), a);
    assert_eq!(Dim::lookup("f30test:arena_1"), Some(a));
    assert_eq!(Dim::lookup_or_overworld("f30test:arena_1"), a);
    assert_eq!(Dim::intern("minecraft:the_nether"), Dim::Nether);
    let b = Dim::intern("f30test:arena_2");
    assert_ne!(a, b);
}

#[test]
fn intern_is_thread_safe() {
    let handles: Vec<_> = (0..8)
        .map(|_| {
            std::thread::spawn(|| {
                (0..50)
                    .map(|i| Dim::intern(&format!("f30test:threaded_{}", i % 10)))
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    for r in &results {
        assert_eq!(r, &results[0]);
    }
    let mut distinct: Vec<_> = results[0].clone();
    distinct.sort_by_key(|d| d.0);
    distinct.dedup();
    assert_eq!(distinct.len(), 10);
    for d in distinct {
        assert!(d.id().starts_with("f30test:threaded_"));
    }
}
