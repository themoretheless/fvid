#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "wav" => Box::new(
            fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "aiff" => Box::new(
            fvid::playback_aiff::AiffAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn coded_audio_packet_stamps_advance_by_packed_frames_instead_of_the_nominal_window() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/coded-window-clock");
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
        let name = case["name"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let frames = case["frames"].as_u64().unwrap();
        let grain = case["grain"].as_u64().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut source = open(kind, bytes.clone());
        let mut data = Vec::new();
        let mut packets = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            data.extend_from_slice(&p.data);
            packets.push(p);
        }
        assert!(packets.len() >= 3);
        let mut engine = source.make_decoder().unwrap();
        let grouped = engine
            .decode_packet(&data, 0, frames)
            .unwrap()
            .unwrap()
            .packet;
        assert_eq!(grouped.data.len() / 4 / channels, frames as usize);
        let mut engine = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        let mut pcm = Vec::new();
        let mut cursor = 0u64;
        for p in &packets {
            let f = engine
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let count = f.packet.data.len() / 4 / channels;
            assert_eq!(p.duration, count as i64);
            if p.pts != cursor as i64 {
                failures.push(format!(
                    "{name}: packetPTS{} instead of{cursor}, duration{count}",
                    p.pts
                ));
            }
            expected.push((
                Duration::from_nanos(cursor * 1_000_000_000 / 8000),
                f.packet.data.clone(),
            ));
            pcm.extend(f.packet.data);
            cursor += count as u64;
        }
        assert_eq!(
            pcm, grouped.data,
            "{name}: encoded grouping preserves native PCM"
        );
        assert_eq!(cursor, frames);
        assert_eq!(
            source.duration(),
            Some(Duration::from_nanos(frames * 1_000_000_000 / 8000))
        );
        let at = source.seek_to(grain as i64);
        if at != grain as i64 {
            failures.push(format!("{name}: packed-window seek{grain} landed{at}"));
        }
        if !failures.iter().any(|f| f.starts_with(&format!("{name}:"))) {
            capture::worker_check_with_seek(
                open(kind, bytes),
                &expected,
                Some(Duration::from_nanos(frames * 1_000_000_000 / 8000)),
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
