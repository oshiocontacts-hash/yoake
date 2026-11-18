use horizon_client_lib::entities::{ArmorStand, StandMedium, StandStep, move_applies};
use horizon_core::geometry::Vec3;
use std::hint::black_box;
use std::time::Instant;

#[cfg(not(feature = "yoake-hook-baseline"))]
type SettleCallback = fn(
    &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep),
    &mut ArmorStand,
    StandMedium,
    StandStep,
);

#[cfg(not(feature = "yoake-hook-baseline"))]
fn noop_settle(
    original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep),
    stand: &mut ArmorStand,
    medium: StandMedium,
    step: StandStep,
) {
    original(stand, medium, step);
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn medium(index: usize) -> StandMedium {
    match index % 4 {
        0 => StandMedium::Air,
        1 => StandMedium::Water,
        2 => StandMedium::Lava { shallow: true },
        _ => StandMedium::Lava { shallow: false },
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "null".into());
    if mode == "noop" {
        #[cfg(not(feature = "yoake-hook-baseline"))]
        yoake_hook_rt::register::<SettleCallback>(
            ArmorStand::__YOAKE_HOOK_ID_tick_settle,
            noop_settle,
            yoake_hook_rt::Order::default(),
        )
        .unwrap();
        #[cfg(feature = "yoake-hook-baseline")]
        panic!("baseline has no registration sites");
    } else {
        assert_eq!(mode, "null");
    }
    let step = StandStep::for_hook_bench(true, 0.6);
    let mut stand = ArmorStand::new(1, Vec3::ZERO, 0.0);
    let mut stands: Vec<_> = (0..1024)
        .map(|index| ArmorStand::new(index, Vec3::new(f64::from(index), 64.0, 0.0), 30.0))
        .collect();
    let mut leaf_samples = Vec::new();
    let mut settle_samples = Vec::new();
    let mut slice_samples = Vec::new();
    let mut checksum = 0.0;
    for sample in 0..12 {
        let start = Instant::now();
        for iteration in 0..5_000_000 {
            let wanted = black_box(Vec3::new(0.2, -0.08, 0.1));
            let moved = black_box(Vec3::new(0.2, 0.0, (iteration % 2) as f64));
            black_box(move_applies(wanted, moved));
        }
        let leaf_ns = start.elapsed().as_secs_f64() * 1e9 / 5_000_000.0;
        let start = Instant::now();
        for iteration in 0..3_000_000 {
            stand.velocity = black_box(Vec3::new(0.1, -0.15, 0.04));
            black_box(&mut stand).tick_settle(black_box(medium(iteration)), black_box(step));
            black_box(stand.velocity);
        }
        let settle_ns = start.elapsed().as_secs_f64() * 1e9 / 3_000_000.0;
        let start = Instant::now();
        for _ in 0..2048 {
            for (index, entity) in stands.iter_mut().enumerate() {
                entity.tick_settle(black_box(medium(index)), black_box(step));
                let position = entity.position.add_vector(entity.velocity);
                entity.interpolate(black_box(Some((position, entity.yaw))));
                checksum += black_box(entity.render_position(0.5)).y;
            }
        }
        let slice_ns = start.elapsed().as_secs_f64() * 1e9 / (2048.0 * 1024.0);
        if sample != 0 {
            leaf_samples.push(leaf_ns);
            settle_samples.push(settle_ns);
            slice_samples.push(slice_ns);
        }
    }
    black_box(checksum);
    println!(
        "{}",
        serde_json::json!({
            "baseline": cfg!(feature = "yoake-hook-baseline"),
            "mode": mode,
            "samples": 11,
            "move_applies_ns": median(&mut leaf_samples),
            "tick_settle_ns": median(&mut settle_samples),
            "settle_interpolation_ns_per_entity": median(&mut slice_samples),
            "settle_interpolation_ns_per_1024_entities": slice_samples[5] * 1024.0,
            "leaf_samples_ns": leaf_samples,
            "settle_samples_ns": settle_samples,
            "slice_samples_ns": slice_samples,
            "checksum": checksum,
        })
    );
}
