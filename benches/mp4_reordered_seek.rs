//! Manual comparison: reordered MP4 coverage navigation and actual software seek+RGB.
//! Generate input separately; ordinary tests never require this workload.
use std::{
    io::Cursor,
    time::{Duration, Instant},
};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let Some(path) = args.get(1).filter(|p| !p.starts_with("--")) else {
        eprintln!("manual benchmark: provide generated MP4 path and optional query count");
        return;
    };
    let data = std::fs::read(path).unwrap();
    let queries: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(1000);
    assert!(queries > 0, "query count must be positive");
    let input = Cursor::new(data.clone());
    let start = Instant::now();
    let mut r =
        fvid::playback_mp4::Mp4VideoReader::open_software(input, Default::default(), 64 << 20)
            .unwrap();
    let open_ns = start.elapsed().as_nanos();
    let count = r.track().samples.len();
    assert!(count >= 3);
    assert_eq!(count % 3, 0);
    let targets: Vec<_> = (0..queries)
        .map(|i| i.wrapping_mul(7919) % (count - 1))
        .collect();
    let start = Instant::now();
    for &i in &targets {
        let expected = i - i % 3 + if i % 3 == 2 { 2 } else { 0 };
        assert_eq!(r.seek_to_sync(i as i64), expected as i64);
    }
    let navigation_ns = start.elapsed().as_nanos();
    drop(r);
    let mut native =
        fvid::playback_native::NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
    let reference: Vec<_> = (0..3)
        .map(|_| {
            native
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(64 << 20)
                .unwrap()
        })
        .collect();
    let start = Instant::now();
    for &i in &targets {
        let got = native
            .seek_raw(Duration::from_millis(i as u64 * 20))
            .unwrap()
            .unwrap()
            .into_rgb(64 << 20)
            .unwrap();
        assert_eq!(got, reference[i % 3]);
        assert_eq!(
            native.frame_interval(),
            Some((i as u128, i as u128 + 1, 50))
        );
    }
    let seek_rgb_ns = start.elapsed().as_nanos();
    println!(
        "{}",
        serde_json::json!({"packets":count,"queries":queries,"open_ns":open_ns,"navigation_ns":navigation_ns,"seek_rgb_ns":seek_rgb_ns,"scope":"memory-resident input,software decoding; exact RGB/interval checks included; no disk/GPU/player-global speed claim"})
    );
}
