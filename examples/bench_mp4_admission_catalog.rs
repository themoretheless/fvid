//! Opt-in planning benchmark; ordinary tests contain no timing assertions.
use std::{hint::black_box, path::PathBuf, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().expect("fixture"));
    let rounds: usize = args.next().unwrap_or_else(|| "9".into()).parse().unwrap();
    let iterations: usize = args.next().unwrap_or_else(|| "16".into()).parse().unwrap();
    assert!(rounds > 0 && iterations > 0);
    let opts = fvid_media::CopyOptions {
        max_controlled_bytes: Some(64 << 20),
        ..Default::default()
    };
    let mut times = Vec::new();
    let mut fingerprint = None;
    for i in 0..rounds + 2 {
        let now = Instant::now();
        let mut last = None;
        for _ in 0..iterations {
            last = Some(black_box(
                fvid_media::plan_decode_audio(
                    black_box(&source),
                    &Default::default(),
                    black_box(&opts),
                )
                .unwrap(),
            ));
        }
        let ns = now.elapsed().as_nanos();
        let value = serde_json::to_value(last.unwrap()).unwrap();
        if let Some(previous) = &fingerprint {
            assert_eq!(previous, &value);
        } else {
            fingerprint = Some(value);
        }
        if i >= 2 {
            times.push(ns);
        }
    }
    let mut sorted = times.clone();
    sorted.sort();
    println!(
        "{}",
        serde_json::json!({"iterations":iterations,"round_ns":times,"median_ns":sorted[sorted.len()/2],"fingerprint":fingerprint})
    );
}
