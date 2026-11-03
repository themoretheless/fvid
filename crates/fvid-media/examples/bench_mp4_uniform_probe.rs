//! Opt-in comparison harness; ordinary tests have no timing thresholds.
use std::{hint::black_box, path::PathBuf, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().expect("source"));
    let iterations: usize = args.next().unwrap_or_else(|| "31".into()).parse().unwrap();
    assert!(iterations > 0);
    let bounded = args.next().is_some_and(|s| s == "bounded");
    let options = fvid_media::CopyOptions {
        max_controlled_bytes: bounded.then_some(64 << 20),
        ..Default::default()
    };
    let probe =
        || black_box(fvid_media::owned_probe::probe_with_options(&source, &options).unwrap());
    for _ in 0..3 {
        probe();
    }
    let mut ns = Vec::new();
    for _ in 0..iterations {
        let begin = Instant::now();
        let info = probe();
        ns.push(begin.elapsed().as_nanos());
        drop(info);
    }
    println!(
        "{}",
        serde_json::json!({"bounded":bounded,"elapsed_ns":ns,"info":probe()})
    );
}
