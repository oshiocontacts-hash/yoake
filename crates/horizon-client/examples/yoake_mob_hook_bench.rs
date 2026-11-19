// cargo build --locked --release -p horizon-client-lib --example yoake_mob_hook_bench --target-dir target/yoake-mob-hook-baseline --features yoake-hook-bench,yoake-hook-baseline
// cargo build --locked --release -p horizon-client-lib --example yoake_mob_hook_bench --target-dir target/yoake-mob-hook-instrumented --features yoake-hook-bench
// python3 tools/yoake-hook-bench-gate.py target/yoake-mob-hook-baseline/release/examples/yoake_mob_hook_bench target/yoake-mob-hook-instrumented/release/examples/yoake_mob_hook_bench --metrics mob_tick_ns_per_mob

use horizon_client_lib::mobs::mob_hook_bench::{BenchWorld, spawn};
use horizon_core::random::LegacyRandom;
use std::hint::black_box;
use std::time::Instant;

const MOBS: usize = 1024;
const TICKS: usize = 200;
const SAMPLES: usize = 12;

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "null".into());
    assert_eq!(mode, "null");
    let fixture = BenchWorld::new();
    let world = fixture.world();
    let mut samples = Vec::new();
    let mut checksum = 0.0;
    for sample in 0..SAMPLES {
        let mut mobs = spawn(MOBS);
        let mut random = LegacyRandom::new(0x4d59_5df4_d0f3_3173);
        let mut events = Vec::new();
        let start = Instant::now();
        for _ in 0..TICKS {
            for mob in &mut mobs {
                black_box(mob).tick(
                    black_box(&world),
                    black_box(&mut random),
                    black_box(&mut events),
                );
            }
            events.clear();
        }
        let elapsed = start.elapsed().as_secs_f64() * 1e9 / (MOBS * TICKS) as f64;
        if sample != 0 {
            samples.push(elapsed);
        }
        for mob in &mobs {
            checksum += mob.position.x + mob.position.y + mob.position.z;
            checksum += f64::from(mob.yaw + mob.body_yaw + mob.head_yaw);
            checksum += f64::from(mob.health);
            checksum += f64::from(mob.age);
        }
        checksum += events.len() as f64 + random.state() as f64;
    }
    black_box(checksum);
    println!(
        "{}",
        serde_json::json!({
            "baseline": cfg!(feature = "yoake-hook-baseline"),
            "mode": mode,
            "mobs": MOBS,
            "ticks_per_sample": TICKS,
            "samples": SAMPLES - 1,
            "mob_tick_ns_per_mob": median(&mut samples),
            "mob_tick_samples_ns_per_mob": samples,
            "checksum": checksum,
        })
    );
}
