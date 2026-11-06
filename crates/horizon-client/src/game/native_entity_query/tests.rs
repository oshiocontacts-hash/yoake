use super::*;

mod storage;

const OVERWORLD: &str = "minecraft:overworld";
const NETHER: &str = "minecraft:the_nether";

fn tick(sequence: u64) -> (NativeWorldEpoch, NativeTickId) {
    (NativeWorldEpoch::new(1), NativeTickId::new(sequence))
}

fn point(id: u32) -> Candidate<'static> {
    Candidate::point(id, OVERWORLD, "minecraft:pig", [0.0; 3])
}

fn run<'world>(
    budgets: &mut QueryBudgets,
    sequence: u64,
    mod_index: usize,
    candidates: &[Candidate<'world>],
) -> Result<Vec<NativeEntityInfo>, String> {
    query_entities(
        budgets,
        tick(sequence),
        mod_index,
        OVERWORLD,
        Sphere::new([0.0; 3], 5.0).unwrap(),
        candidates.len(),
        |visit| {
            for candidate in candidates {
                visit(candidate.clone())?;
            }
            Ok(())
        },
    )
}

#[test]
fn sphere_includes_face_edge_corner_and_zero_radius_boundaries() {
    let sphere = Sphere::new([0.0; 3], 5.0).unwrap();
    assert!(
        sphere
            .intersects([[5.0, -1.0, -1.0], [6.0, 1.0, 1.0]])
            .unwrap()
    );
    assert!(
        sphere
            .intersects([[3.0, 4.0, 0.0], [4.0, 5.0, 1.0]])
            .unwrap()
    );
    assert!(
        sphere
            .intersects([[0.0, 3.0, 4.0], [0.0, 3.0, 4.0]])
            .unwrap()
    );
    assert!(!sphere.intersects([[5.001, 0.0, 0.0]; 2]).unwrap());
    assert!(
        Sphere::new([0.0; 3], 0.0)
            .unwrap()
            .intersects([[-1.0; 3], [0.0; 3]])
            .unwrap()
    );
}

#[test]
fn intersection_uses_the_box_instead_of_the_entity_origin() {
    let mut candidate = point(1);
    candidate.position = [8.0, 0.0, 0.0];
    candidate.bounding_box = [[5.0, -1.0, -1.0], [9.0, 1.0, 1.0]];
    assert!(
        candidate
            .matches(OVERWORLD, Sphere::new([0.0; 3], 5.0).unwrap())
            .unwrap()
    );
}

#[test]
fn invalid_geometry_is_rejected() {
    for radius in [-1.0, 64.001, f64::NAN, f64::INFINITY] {
        assert!(Sphere::new([0.0; 3], radius).is_err());
    }
    assert!(Sphere::new([f64::NAN, 0.0, 0.0], 1.0).is_err());
    assert!(Sphere::new([f64::INFINITY, 0.0, 0.0], 1.0).is_err());
    assert!(Sphere::new([0.0; 3], 64.0).is_ok());
    let sphere = Sphere::new([0.0; 3], 1.0).unwrap();
    assert!(sphere.intersects([[1.0; 3], [0.0; 3]]).is_err());
    assert!(sphere.intersects([[f64::NAN; 3], [1.0; 3]]).is_err());
    let mut candidate = point(1);
    candidate.position[0] = f64::NAN;
    assert!(candidate.matches(OVERWORLD, sphere).is_err());
}

#[test]
fn dimensions_are_isolated_and_network_ids_are_sorted() {
    let mut elsewhere = point(2);
    elsewhere.dimension = NETHER;
    let result = run(
        &mut QueryBudgets::default(),
        1,
        0,
        &[point(8), elsewhere, point(3)],
    )
    .unwrap();
    assert_eq!(
        result.iter().map(|entity| entity.id).collect::<Vec<_>>(),
        [3, 8]
    );
    assert!(result.iter().all(|entity| entity.dimension == OVERWORLD));
}

#[test]
fn dying_entities_remain_but_removed_entities_do_not() {
    let mut dying = point(1);
    dying.alive = false;
    let mut removed = point(2);
    removed.removed = true;
    let result = run(&mut QueryBudgets::default(), 1, 0, &[removed, dying]).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, 1);
    assert!(!result[0].alive);
}

#[test]
fn duplicate_ids_fail_without_returning_results() {
    let mut budgets = QueryBudgets::default();
    assert!(run(&mut budgets, 1, 0, &[point(1), point(1)]).is_err());
    let usage = budgets.usage(tick(1), 0);
    assert_eq!(usage.queries, 1);
    assert_eq!(usage.candidates, 2);
    assert_eq!(usage.results, 0);
}

#[test]
fn query_limit_rejects_before_visiting_entities() {
    let mut budgets = QueryBudgets::default();
    for _ in 0..MAX_QUERIES {
        run(&mut budgets, 1, 0, &[]).unwrap();
    }
    let before = budgets.usage(tick(1), 0);
    let mut visited = false;
    let result = query_entities(
        &mut budgets,
        tick(1),
        0,
        OVERWORLD,
        Sphere::new([0.0; 3], 1.0).unwrap(),
        0,
        |_| {
            visited = true;
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(!visited);
    assert_eq!(budgets.usage(tick(1), 0), before);
}

#[test]
fn candidate_limit_is_cumulative_and_inclusive() {
    let outside = Candidate::point(1, OVERWORLD, "minecraft:item", [65.0; 3]);
    let mut budgets = QueryBudgets::default();
    run(&mut budgets, 1, 0, &vec![outside.clone(); 2048]).unwrap();
    run(&mut budgets, 1, 0, &vec![outside.clone(); 2048]).unwrap();
    let before = budgets.usage(tick(1), 0);
    assert_eq!(before.candidates, MAX_CANDIDATES);
    assert!(run(&mut budgets, 1, 0, &[outside]).is_err());
    assert_eq!(budgets.usage(tick(1), 0), before);
}

#[test]
fn oversized_candidate_count_is_rejected_before_enumeration() {
    let mut visited = false;
    let mut budgets = QueryBudgets::default();
    let result = query_entities(
        &mut budgets,
        tick(1),
        0,
        OVERWORLD,
        Sphere::new([0.0; 3], 1.0).unwrap(),
        MAX_CANDIDATES + 1,
        |_| {
            visited = true;
            Ok(())
        },
    );
    assert!(result.is_err());
    assert!(!visited);
    assert!(budgets.tick.is_none());
}

#[test]
fn result_limit_is_cumulative_and_inclusive() {
    let candidates = (0..128).map(point).collect::<Vec<_>>();
    let mut budgets = QueryBudgets::default();
    assert_eq!(run(&mut budgets, 1, 0, &candidates).unwrap().len(), 128);
    assert_eq!(run(&mut budgets, 1, 0, &candidates).unwrap().len(), 128);
    let before = budgets.usage(tick(1), 0);
    assert_eq!(before.results, MAX_RESULTS);
    assert!(run(&mut budgets, 1, 0, &[point(1)]).is_err());
    assert_eq!(
        budgets.usage(tick(1), 0),
        QueryUsage {
            queries: before.queries + 1,
            candidates: before.candidates + 1,
            ..before
        }
    );
    assert!(run(&mut budgets, 1, 0, &[]).is_ok());
}

#[test]
fn an_oversized_result_returns_no_partial_list_but_consumes_work_budget() {
    let candidates = (0..257).map(point).collect::<Vec<_>>();
    let mut budgets = QueryBudgets::default();
    assert!(run(&mut budgets, 1, 0, &candidates).is_err());
    let usage = budgets.usage(tick(1), 0);
    assert_eq!(usage.queries, 1);
    assert_eq!(usage.candidates, candidates.len());
    assert_eq!(usage.results, 0);
}

#[test]
fn failed_result_scans_cannot_bypass_work_budgets() {
    for (candidate_count, attempts) in [(257, MAX_QUERIES), (2048, 2)] {
        let candidates = (0..candidate_count).map(point).collect::<Vec<_>>();
        let mut budgets = QueryBudgets::default();
        for _ in 0..attempts {
            assert!(run(&mut budgets, 1, 0, &candidates).is_err());
        }
        let before = budgets.usage(tick(1), 0);
        assert_eq!(before.queries, attempts);
        assert_eq!(before.candidates, candidates.len() * attempts);
        assert_eq!(before.results, 0);
        let mut visited = false;
        assert!(
            query_entities(
                &mut budgets,
                tick(1),
                0,
                OVERWORLD,
                Sphere::new([0.0; 3], 1.0).unwrap(),
                1,
                |_| {
                    visited = true;
                    Ok(())
                },
            )
            .is_err()
        );
        assert!(!visited);
        assert_eq!(budgets.usage(tick(1), 0), before);
    }
}

#[test]
fn budgets_are_per_mod_and_reset_lazily_by_epoch_and_tick() {
    let mut budgets = QueryBudgets::default();
    assert_eq!(budgets.mods.capacity(), 0);
    for _ in 0..MAX_QUERIES {
        run(&mut budgets, 1, 0, &[]).unwrap();
    }
    assert!(run(&mut budgets, 1, 0, &[]).is_err());
    run(&mut budgets, 1, 1, &[]).unwrap();
    assert_eq!(budgets.usage(tick(1), 1).queries, 1);
    run(&mut budgets, 2, 0, &[]).unwrap();
    assert_eq!(budgets.mods.len(), 1);
    assert_eq!(budgets.usage(tick(2), 0).queries, 1);
    let next_world = (NativeWorldEpoch::new(2), NativeTickId::new(2));
    assert_eq!(budgets.usage(next_world, 0).queries, 0);
    assert_eq!(budgets.tick, Some(tick(2)));
    query_entities(
        &mut budgets,
        next_world,
        0,
        OVERWORLD,
        Sphere::new([0.0; 3], 0.0).unwrap(),
        0,
        |_| Ok(()),
    )
    .unwrap();
    assert_eq!(budgets.tick, Some(next_world));
    assert_eq!(budgets.usage(next_world, 0).queries, 1);
}

#[test]
fn enumeration_errors_and_unnumbered_entities_fail_closed() {
    for reported in [0, 2] {
        let mut budgets = QueryBudgets::default();
        let result = query_entities(
            &mut budgets,
            tick(1),
            0,
            OVERWORLD,
            Sphere::new([0.0; 3], 1.0).unwrap(),
            reported,
            |visit| visit(point(1)),
        );
        assert!(result.is_err());
        let usage = budgets.usage(tick(1), 0);
        assert_eq!(usage.queries, 1);
        assert_eq!(usage.candidates, reported);
        assert_eq!(usage.results, 0);
    }
    let mut budgets = QueryBudgets::default();
    let result = query_entities(
        &mut budgets,
        tick(1),
        0,
        OVERWORLD,
        Sphere::new([0.0; 3], 1.0).unwrap(),
        1,
        |visit| {
            visit(point(1))?;
            Err("adapter unavailable".into())
        },
    );
    assert!(result.is_err());
    let usage = budgets.usage(tick(1), 0);
    assert_eq!(usage.queries, 1);
    assert_eq!(usage.candidates, 1);
    assert_eq!(usage.results, 0);
}

#[test]
fn only_authoritative_server_contexts_with_the_host_in_place_are_allowed() {
    for role in [
        ExecutionRole::IntegratedServer,
        ExecutionRole::DedicatedServer,
    ] {
        assert!(require_server_context(role, false, false, true).is_ok());
        assert!(require_server_context(role, true, false, true).is_err());
        assert!(require_server_context(role, false, true, true).is_err());
        assert!(require_server_context(role, false, false, false).is_err());
    }
    assert!(require_server_context(ExecutionRole::Client, false, false, true).is_err());
}

#[test]
fn unscoped_transients_are_not_assigned_to_arbitrary_dimensions() {
    assert!(require_scoped_transients(1, 0, true).is_err());
    assert!(require_scoped_transients(0, 1, true).is_err());
    assert!(require_scoped_transients(1, 1, false).is_ok());
    assert!(require_scoped_transients(0, 0, true).is_ok());
}

#[test]
fn snapshots_and_player_ids_preserve_values_without_uuid_generation() {
    let snapshot = NativeEntityInfo::new(
        7,
        OVERWORLD,
        "minecraft:player",
        [1.0, 2.0, 3.0],
        [[0.5, 2.0, 2.5], [1.5, 3.8, 3.5]],
        false,
        Some(0x1234),
    );
    assert_eq!(snapshot.id, 7);
    assert_eq!(snapshot.player_uuid, Some(0x1234));
    assert!(!snapshot.alive);
    assert_eq!(player_id(7).unwrap(), 7);
    assert!(player_id(-1).is_err());
}
