//! Opt-in constant-description reader opening benchmark; no timing assertions in tests.
use std::{hint::black_box, io::Cursor, sync::Arc, time::Instant};
fn main() {
    let mut args = std::env::args().skip(1);
    let input: Arc<[u8]> = std::fs::read(args.next().expect("fixture")).unwrap().into();
    let rounds: usize = args.next().unwrap_or_else(|| "9".into()).parse().unwrap();
    let iterations: usize = args.next().unwrap_or_else(|| "256".into()).parse().unwrap();
    assert!(rounds > 0 && iterations > 0);
    let mut times = Vec::new();
    let mut fingerprint = None;
    for i in 0..rounds + 2 {
        let now = Instant::now();
        let mut last = None;
        for _ in 0..iterations {
            last = Some(black_box(
                fvid_media::owned_mp4::Mp4Reader::open(
                    Cursor::new(black_box(input.clone())),
                    Default::default(),
                )
                .unwrap(),
            ));
        }
        let ns = now.elapsed().as_nanos();
        let reader = last.unwrap();
        let tracks:Vec<_>=reader.tracks().iter().map(|t|{let points:Vec<_>=[0,t.samples.len()/2,t.samples.len()-1].into_iter().map(|i|{let s=t.samples.get(i).unwrap();(s.description_index,s.offset,s.size,s.dts,s.pts,s.duration,s.sync)}).collect();serde_json::json!({"codec":t.codec,"rate":t.sample_rate,"channels":t.channels,"bits":t.bit_depth,"scale":t.timescale,"duration":t.duration,"descriptions":t.sample_descriptions.len(),"count":t.samples.len(),"cost":t.samples.cost(),"points":points})}).collect();
        let value = serde_json::json!({"tracks":tracks,"retained_payload":reader.estimated_index_payload_bytes().unwrap()});
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
