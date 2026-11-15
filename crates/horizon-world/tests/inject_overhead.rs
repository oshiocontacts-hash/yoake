use std::convert::Infallible;
use std::hint::black_box;
use std::time::{Duration, Instant};

use horizon_core::geometry::Vec3;
use horizon_world::living_fall::{LivingFallWorld, calculate_fall_damage};

struct FallWorld {
    immune: bool,
    safe_distance: f64,
    multiplier: f64,
}

impl LivingFallWorld for FallWorld {
    type Error = Infallible;
    type Sound = ();

    fn impact_position(&self) -> Option<Vec3> {
        None
    }

    fn y(&self) -> f64 {
        64.0
    }

    fn reset_impulse(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn try_reset_impulse(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn immune_to_fall_damage(&mut self) -> Result<bool, Self::Error> {
        Ok(self.immune)
    }

    fn propagate_fall_to_passengers(
        &mut self,
        _distance: f64,
        _multiplier: f32,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn safe_fall_distance(&mut self) -> Result<f64, Self::Error> {
        Ok(self.safe_distance)
    }

    fn fall_damage_multiplier(&mut self) -> Result<f64, Self::Error> {
        Ok(self.multiplier)
    }

    fn fall_sound(&mut self, _big: bool) -> Result<Self::Sound, Self::Error> {
        Ok(())
    }

    fn play_fall_sound(&mut self, _sound: Self::Sound) -> Result<(), Self::Error> {
        Ok(())
    }

    fn play_block_fall_sound(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn hurt(&mut self, _damage: f32) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[inline(never)]
fn plain(world: &mut FallWorld, distance: f64, multiplier: f32) -> i32 {
    if world.immune_to_fall_damage().unwrap() {
        return 0;
    }
    let power = distance + 1.0e-6 - world.safe_fall_distance().unwrap();
    (power * f64::from(multiplier) * world.fall_damage_multiplier().unwrap()).floor() as i32
}

#[inline(never)]
fn annotated(world: &mut FallWorld, distance: f64, multiplier: f32) -> i32 {
    calculate_fall_damage(world, distance, multiplier).unwrap()
}

fn measure(function: fn(&mut FallWorld, f64, f32) -> i32, iterations: u64) -> Duration {
    let mut world = FallWorld {
        immune: black_box(false),
        safe_distance: black_box(3.0),
        multiplier: black_box(1.0),
    };
    let started = Instant::now();
    let mut total = 0_i64;
    for iteration in 0..iterations {
        total += i64::from(function(
            black_box(&mut world),
            black_box(4.0 + (iteration % 32) as f64),
            black_box(1.0),
        ));
    }
    black_box(total);
    started.elapsed()
}

#[test]
fn native_and_zero_mask_fall_damage_match() {
    let mut world = FallWorld {
        immune: false,
        safe_distance: 3.0,
        multiplier: 1.0,
    };
    for distance in [-1.0, 0.0, 3.0, 4.0, 20.0, 100.0] {
        assert_eq!(
            annotated(&mut world, distance, 1.0),
            plain(&mut world, distance, 1.0)
        );
    }
    world.immune = true;
    assert_eq!(annotated(&mut world, 20.0, 1.0), 0);
}

#[test]
#[ignore = "timing benchmark: run with --release --ignored --nocapture"]
fn realistic_zero_mask_microbenchmark() {
    let iterations = 1_000_000;
    let mut plain_times = Vec::new();
    let mut annotated_times = Vec::new();
    black_box(measure(plain, iterations));
    black_box(measure(annotated, iterations));
    for trial in 0..11 {
        if trial % 2 == 0 {
            plain_times.push(measure(plain, iterations));
            annotated_times.push(measure(annotated, iterations));
        } else {
            annotated_times.push(measure(annotated, iterations));
            plain_times.push(measure(plain, iterations));
        }
    }
    plain_times.sort_unstable();
    annotated_times.sort_unstable();
    let plain_ns = plain_times[5].as_secs_f64() * 1e9 / iterations as f64;
    let annotated_ns = annotated_times[5].as_secs_f64() * 1e9 / iterations as f64;
    println!(
        "fall damage: median of 11 alternating trials, {iterations} calls/trial; plain={plain_ns:.4} ns/call; annotated={annotated_ns:.4} ns/call; delta={:.4} ns/call ({:.2}%)",
        annotated_ns - plain_ns,
        (annotated_ns / plain_ns - 1.0) * 100.0,
    );
}
