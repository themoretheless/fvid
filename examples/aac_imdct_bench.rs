//! Compare the owned fast transform with the previous cosine-recurrence kernel.
use fvid::codec::aac_imdct::Imdct;
use std::{f64::consts::PI, hint::black_box, time::Instant};
fn main() {
    for n in [120, 128, 960, 1024] {
        let input: Vec<f32> = (0..n)
            .map(|i| ((i as f64 * 0.173).sin() * 0.4) as f32)
            .collect();
        let old: Vec<[f64; 3]> = (0..2 * n)
            .map(|i| {
                let phase = PI / n as f64 * (i as f64 + 0.5 + n as f64 / 2.0);
                [(phase * 0.5).cos(), (phase * 1.5).cos(), 2.0 * phase.cos()]
            })
            .collect();
        let plan = Imdct::new(n).unwrap();
        let mut scratch = vec![[0.0; 2]; plan.scratch_len()];
        let mut fast = vec![0.0; 2 * n];
        let mut reference = vec![0.0; 2 * n];
        let mut timings = [Vec::new(), Vec::new()];
        for round in 0..6 {
            for mode in [round % 2, 1 - round % 2] {
                let begin = Instant::now();
                for _ in 0..64 {
                    if mode == 0 {
                        plan.inverse_with_scratch(
                            black_box(&input),
                            black_box(&mut fast),
                            &mut scratch,
                        )
                        .unwrap();
                        black_box(&fast);
                    } else {
                        for (sample, &[first, second, factor]) in
                            black_box(&mut reference).iter_mut().zip(&old)
                        {
                            let (mut previous, mut current) = (first, second);
                            let mut sum = f64::from(black_box(&input)[0]) * first;
                            for &coefficient in &input[1..] {
                                sum += f64::from(coefficient) * current;
                                (previous, current) = (current, factor * current - previous);
                            }
                            *sample = sum * (2.0 / n as f64);
                        }
                        black_box(&reference);
                    }
                }
                if round > 0 {
                    timings[mode].push(begin.elapsed().as_secs_f64() / 64.0);
                }
            }
        }
        for values in &mut timings {
            values.sort_by(f64::total_cmp);
        }
        let a = timings[0][2];
        let b = timings[1][2];
        let peak = fast
            .iter()
            .zip(&reference)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f64::max);
        println!(
            "N={n}: FFT {:.2} us, recurrence {:.2} us, ratio {:.2}x, peak delta {peak:.3e}",
            a * 1e6,
            b * 1e6,
            b / a
        );
    }
}
