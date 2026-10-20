//! Explicit index benchmark; never used by ordinary tests or fixture generation.
use fvid_media::owned_flac::{Flac, Limits};
fn main() {
    let mut args = std::env::args().skip(1);
    let iterations: usize = args.next().expect("iterations").parse().unwrap();
    let inputs: Vec<Vec<u8>> = args.map(|path| std::fs::read(path).unwrap()).collect();
    assert!(!inputs.is_empty());
    for _ in 0..10 { for data in &inputs { std::hint::black_box(Flac::parse(data, &Limits::default()).unwrap()); } }
    let start = std::time::Instant::now(); let mut frames = 0usize; let mut samples = 0u64;
    for _ in 0..iterations { for data in &inputs { let flac = Flac::parse(std::hint::black_box(data), &Limits::default()).unwrap(); frames += flac.frames.len(); samples += flac.samples(); std::hint::black_box(flac); } }
    println!("{}", serde_json::json!({"iterations":iterations,"inputs":inputs.len(),"elapsed_ns":start.elapsed().as_nanos(),"frames":frames,"samples":samples}));
}
