use std::hint::black_box;
use std::time::Instant;

use c1_bindgen::rt::codec::{Decode, Decoder, Encode};
use c1_bindgen::rt::{CallError, PathHandle, ResolvePath, Root};

#[derive(Default)]
struct Game {
    value: u64,
}

impl Game {
    fn value(&self) -> u64 {
        self.value
    }

    fn mix(&mut self, integer: u32, float: f64, enabled: bool) -> f64 {
        self.value = self.value.wrapping_add(u64::from(integer));
        if enabled {
            float + integer as f64
        } else {
            float - integer as f64
        }
    }

    fn string_len(&self, text: &str) -> usize {
        text.len()
    }
}

impl ResolvePath<Game> for Game {
    fn resolve_path<'a>(&'a mut self, path: &PathHandle) -> Result<&'a mut Game, CallError> {
        if path.root == Root::Game && path.accessors.is_empty() {
            Ok(self)
        } else {
            Err(CallError::InvalidPath("benchmark only has a root Game".into()))
        }
    }
}

fn call_1(root: &mut Game, path: &PathHandle, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    Decoder::new(args).finish()?;
    let target: &mut Game = root.resolve_path(path)?;
    target.value().encode(out);
    Ok(())
}

fn call_2(root: &mut Game, path: &PathHandle, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    let mut decoder = Decoder::new(args);
    let integer = u32::decode(&mut decoder)?;
    let float = f64::decode(&mut decoder)?;
    let enabled = bool::decode(&mut decoder)?;
    decoder.finish()?;
    let target: &mut Game = root.resolve_path(path)?;
    target.mix(integer, float, enabled).encode(out);
    Ok(())
}

fn call_3(root: &mut Game, path: &PathHandle, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    let mut decoder = Decoder::new(args);
    let text = String::decode(&mut decoder)?;
    decoder.finish()?;
    let target: &mut Game = root.resolve_path(path)?;
    target.string_len(text.as_str()).encode(out);
    Ok(())
}

const ITERATIONS: usize = 100_000;

fn measure(mut call: impl FnMut()) -> f64 {
    for _ in 0..ITERATIONS {
        call();
    }
    let mut samples = Vec::with_capacity(11);
    for _ in 0..11 {
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            call();
        }
        samples.push(start.elapsed().as_nanos() as f64 / ITERATIONS as f64);
    }
    samples.sort_by(f64::total_cmp);
    samples[5]
}

fn main() {
    let path = PathHandle::game();
    let mut three_args = Vec::new();
    (17u32, 2.5f64, true).encode(&mut three_args);
    let text = "x".repeat(64);
    let mut string_arg = Vec::new();
    text.encode(&mut string_arg);

    let mut game = Game::default();
    let mut out = Vec::new();
    let getter = measure(|| {
        out.clear();
        call_1(&mut game, &path, &[], &mut out).unwrap();
        black_box(out.as_slice());
    });
    let getter_direct = measure(|| {
        black_box(game.value());
    });
    let plain = measure(|| {
        out.clear();
        call_2(&mut game, &path, black_box(&three_args), &mut out).unwrap();
        black_box(out.as_slice());
    });
    let plain_direct = measure(|| {
        black_box(game.mix(black_box(17), black_box(2.5), black_box(true)));
    });
    let string = measure(|| {
        out.clear();
        call_3(&mut game, &path, black_box(&string_arg), &mut out).unwrap();
        black_box(out.as_slice());
    });
    let string_direct = measure(|| {
        black_box(game.string_len(black_box(text.as_str())));
    });

    println!("median of 11 samples after one warmup; {ITERATIONS} calls/sample");
    println!("0-arg getter: glue {getter:.1} ns, direct {getter_direct:.1} ns");
    println!("3-arg plain data: glue {plain:.1} ns, direct {plain_direct:.1} ns");
    println!("64-byte string: glue {string:.1} ns, direct {string_direct:.1} ns");
}
