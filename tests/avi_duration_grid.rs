#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn avi_duration_and_actual_eof_seek_share_the_sample_clock() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avi-duration-grid");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let mut failures = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let bytes = std::fs::read(base.join(name)).unwrap();
        let expected = std::fs::read(base.join(case["expected"].as_str().unwrap())).unwrap();
        let samples = case["samples"].as_i64().unwrap();
        let end = Duration::from_nanos(case["end_ns"].as_u64().unwrap());
        let mut source = fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let published = source.duration().unwrap();
        if published != end {
            failures.push(format!("{name}: duration {published:?}, exact {end:?}"));
        }
        assert_eq!(source.time_of(samples), end);
        let mut decoder = source.make_decoder().unwrap();
        let mut packets = Vec::new();
        let mut flat = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            flat.extend_from_slice(&f.packet.data);
            packets.push((f.packet.presentation_time(), f.packet.data));
        }
        assert_eq!(flat, expected, "{name}: independent PCM");
        assert_eq!(flat.len() / 4, samples as usize);
        assert_eq!(source.seek_to(samples), samples);
        assert!(source.next_packet().unwrap().is_none());
        let worker =
            fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap();
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            capture::worker_check_with_seek(Box::new(worker), &packets, Some(published))
        }))
        .is_err()
        {
            failures.push(format!("{name}: actual public EOF seek clock"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
