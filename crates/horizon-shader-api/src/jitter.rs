//! Centered Halton sampling for opt-in native pack projection jitter.

/// Returns a centered Halton (2, 3) offset in pixels for a zero-based frame.
pub fn halton_2_3(frame: u32) -> [f32; 2] {
    let index = u64::from(frame) + 1;
    [halton(index, 2) - 0.5, halton(index, 3) - 0.5]
}

fn halton(mut index: u64, base: u64) -> f32 {
    let mut factor = 1.0_f64;
    let mut value = 0.0_f64;
    while index != 0 {
        factor /= base as f64;
        value += (index % base) as f64 * factor;
        index /= base;
    }
    value as f32
}

#[cfg(test)]
mod tests {
    use super::halton_2_3;

    #[test]
    fn centered_halton_sequence_starts_at_one() {
        let expected = [
            [0.0, -1.0 / 6.0],
            [-1.0 / 4.0, 1.0 / 6.0],
            [1.0 / 4.0, -7.0 / 18.0],
            [-3.0 / 8.0, -1.0 / 18.0],
            [1.0 / 8.0, 5.0 / 18.0],
            [-1.0 / 8.0, -5.0 / 18.0],
            [3.0 / 8.0, 1.0 / 18.0],
            [-7.0 / 16.0, 7.0 / 18.0],
        ];
        for (frame, expected_offset) in expected.into_iter().enumerate() {
            let offset = halton_2_3(frame as u32);
            for axis in 0..2 {
                assert!((offset[axis] - expected_offset[axis]).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn maximum_frame_does_not_overflow() {
        let offset = halton_2_3(u32::MAX);
        assert!(
            offset
                .into_iter()
                .all(|value| (-0.5..=0.5).contains(&value))
        );
    }
}
