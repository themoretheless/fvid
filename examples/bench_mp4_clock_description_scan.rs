//! Opt-in startup benchmark; no timing assertions in ordinary tests.
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{hint::black_box, io::Cursor, sync::Arc, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let input: Arc<[u8]> = std::fs::read(args.next().expect("fixture")).unwrap().into();
    let count: usize = args.next().unwrap_or_else(|| "9".into()).parse().unwrap();
    assert!(count > 0);
    let mut measurements = Vec::new();
    let mut fingerprint = None;
    for n in 0..count + 2 {
        let source = Cursor::new(input.clone());
        let now = Instant::now();
        let r = Mp4AudioReader::open(source, Default::default()).unwrap();
        let ns = now.elapsed().as_nanos();
        let t = r.track();
        let points = [0, t.samples.len() / 2, t.samples.len() - 1].map(|i| {
            let s = t.samples.get(i).unwrap();
            (s.offset, s.size, s.pts, s.duration, s.description_index)
        });
        let f = serde_json::json!({"samples":t.samples.len(),"descriptions":t.sample_descriptions.len(),"rate":r.sample_rate(),"channels":r.channels(),"duration_ns":r.duration().unwrap().as_nanos(),"points":points});
        if let Some(previous) = &fingerprint {
            assert_eq!(previous, &f);
        } else {
            fingerprint = Some(f);
        }
        black_box(&r);
        if n >= 2 {
            measurements.push(ns);
        }
    }
    let mut sorted = measurements.clone();
    sorted.sort();
    println!(
        "{}",
        serde_json::json!({"open_ns":measurements,"median_ns":sorted[sorted.len()/2],"fingerprint":fingerprint})
    );
}
