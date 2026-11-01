//! Opt-in release benchmark; ordinary regressions never execute this harness.
use std::{hint::black_box, path::PathBuf, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let source = PathBuf::from(args.next().expect("source"));
    let iterations: usize = args.next().unwrap_or_else(|| "15".into()).parse().unwrap();
    assert!(iterations > 0);
    let reader = fvid_media::owned_mp4::Mp4Reader::open_indexed(
        std::io::BufReader::new(std::fs::File::open(&source).unwrap()),
        Default::default(),
    )
    .unwrap();
    let track = reader
        .tracks()
        .iter()
        .find(|t| t.handler == *b"soun")
        .unwrap();
    let fvid_media::owned_mp4::SampleIndex::Uniform(index) = &track.samples else {
        panic!("witness must use compact index")
    };
    assert_eq!(index.runs.len(), 1);
    let count = index.count;
    drop(reader);
    let memory = args.next().is_some_and(|arg| arg == "bounded");
    let options = fvid_media::CopyOptions {
        max_controlled_bytes: memory.then_some(64 << 20),
        ..Default::default()
    };
    let transform = fvid_media::AudioDecodeTransform::default();
    let plan = || {
        let p = fvid_media::plan_decode_audio(&source, &transform, &options).unwrap();
        assert!(!p.steps.is_empty());
        black_box(p)
    };
    for _ in 0..3 {
        plan();
    }
    let mut ns = Vec::new();
    for _ in 0..iterations {
        let now = Instant::now();
        let p = plan();
        ns.push(now.elapsed().as_nanos());
        drop(p);
    }
    println!(
        "{}",
        serde_json::json!({"frames":count,"controlled_bytes":options.max_controlled_bytes,"iterations":iterations,"elapsed_ns":ns,"plan":plan()})
    );
}
