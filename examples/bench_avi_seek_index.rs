//! Manual public-reader navigation benchmark; parsing is outside the timer.
use fvid::{audio::AudioStream, playback_avi_audio::AviAudioReader};
use std::{hint::black_box, path::Path, time::Instant};
fn main() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avi-seek-index");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let round = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "0".into())
        .parse::<usize>()
        .unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(c["file"].as_str().unwrap())).unwrap();
        let mut reader = AviAudioReader::open(bytes.as_slice(), Default::default()).unwrap();
        let start = c["start"].as_i64().unwrap();
        let frames = c["frames"].as_i64().unwrap();
        let end = start + frames;
        let mut queries: Vec<_> = (0..512)
            .map(|i| start + (i * 104729) % (frames + 1))
            .collect();
        queries.extend([i64::MIN, -1, 0, start, start + 1, end - 1, end, i64::MAX]);
        let wanted = queries
            .iter()
            .fold(0u64, |sum, &p| sum.wrapping_add(p.clamp(start, end) as u64));
        let begun = Instant::now();
        let mut checksum = 0u64;
        for &target in &queries {
            checksum = checksum
                .wrapping_add(black_box(black_box(&mut reader).seek_to(black_box(target))) as u64);
        }
        let elapsed = begun.elapsed().as_nanos();
        assert_eq!(checksum, wanted);
        black_box(&reader);
        println!(
            "{}",
            serde_json::json!({"file":c["file"],"records":c["records"],"queries":queries.len(),"round":round,"elapsed_ns":elapsed,"checksum":checksum})
        );
    }
}
