#![cfg(feature = "player")]
use fvid::audio::AudioStream;
#[path = "support/audio_capture.rs"]
mod capture;
use std::{io::Cursor, path::Path};
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "smf" => Box::new(
            fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "xm" => Box::new(
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn timeline_pcm_uses_millisecond_pts_without_overlapping_audio_windows() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/timeline-audio-clock");
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
        let kind = case["kind"].as_str().unwrap();
        let bytes = std::fs::read(root.join(case["source"].as_str().unwrap())).unwrap();
        let mut source = open(kind, bytes.clone());
        for rate in [8_000, 48_000] {
            let mut probe = open(kind, bytes.clone());
            let mut engine = fvid::codec::make_audio_decoder(
                probe.codec(),
                probe.extra_data(),
                rate,
                probe.channels(),
                probe.bits_per_sample(),
            )
            .unwrap();
            while let Some(p) = probe.next_packet().unwrap() {
                let f = engine
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    f.packet.presentation_time(),
                    probe.time_of(p.pts),
                    "{kind} output rate{rate}"
                );
                assert_eq!(
                    f.packet.data.len() / 4 / usize::from(probe.channels()),
                    p.duration as usize * rate as usize / 1000
                );
            }
        }
        let stated = source.duration().unwrap();
        let mut decoder = source.make_decoder().unwrap();
        let mut expected = Vec::new();
        let mut end = std::time::Duration::ZERO;
        while let Some(p) = source.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let start = source.time_of(p.pts);
            if f.packet.presentation_time() != start {
                failures.push(format!(
                    "{kind} sourcePTS{}: output{:?} instead of{start:?}",
                    p.pts,
                    f.packet.presentation_time()
                ));
            }

            let frames = f.packet.data.len() / 4 / usize::from(source.channels());
            end = start
                + std::time::Duration::from_nanos(
                    frames as u64 * 1_000_000_000 / u64::from(source.sample_rate()),
                );
            expected.push((start, f.packet.data));
        }
        assert_eq!(
            end,
            std::time::Duration::from_millis(if kind == "smf" { 2200 } else { 480 })
        );
        assert_eq!(source.duration(), Some(stated));
        source.rewind();
        assert_eq!(source.duration(), Some(stated));
        if failures.is_empty() {
            capture::worker_check_with_seek(
                open(kind, bytes.clone()),
                &expected,
                Some(std::time::Duration::ZERO),
            );
            capture::worker_check_with_seek(open(kind, bytes), &expected, Some(end));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
