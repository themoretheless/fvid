#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod audio_capture;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-audio-padding")
}
fn check(name: &str) {
    let base = root();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["file"] == format!("{name}.mkv"))
        .unwrap();
    let bytes = std::fs::read(base.join(format!("{name}.mkv"))).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    for _ in 0..3 {
        assert!(video.read_frame_raw().unwrap().is_some());
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    let expected = std::fs::read(base.join(case["oracle"].as_str().unwrap())).unwrap();
    let mut audio =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = audio.make_decoder().unwrap();
    let mut reference = Vec::new();
    for pass in 0..3 {
        if pass != 0 {
            AudioStream::rewind(&mut audio);
            decoder.reset();
        }
        let target = if name == "overlap" {
            30_000_000
        } else {
            40_000_000
        };
        if pass == 2 {
            assert_eq!(audio.seek_to(target), target);
            decoder.reset();
        }
        let mut actual = Vec::new();
        let mut windows = Vec::new();
        while let Some(source) = audio.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&source.data, source.pts, source.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(packet) = audio
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                windows.push((packet.presentation_time(), packet.data.len() / 4));
                if pass == 0 {
                    reference.push((packet.presentation_time(), packet.data.clone()));
                }
                actual.extend(packet.data);
            }
        }
        audio.validate_eof().unwrap();
        let expected_windows: Vec<_> = case["packets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| pass != 2 || p["pts_ns"].as_u64().unwrap() >= target as u64)
            .map(|p| {
                (
                    Duration::from_nanos(p["pts_ns"].as_u64().unwrap()),
                    p["frames"].as_u64().unwrap() as usize,
                )
            })
            .collect();
        assert_eq!(windows, expected_windows, "{name}, pass{pass}");
        let retained: usize = expected_windows.iter().map(|(_, frames)| frames * 4).sum();
        assert_eq!(
            actual,
            expected[expected.len() - retained..],
            "{name}, pass{pass}"
        );
        assert_eq!(
            audio.duration(),
            Some(Duration::from_nanos(case["end_ns"].as_u64().unwrap())),
            "{name}: reported endpoint"
        );
    }
    AudioStream::rewind(&mut audio);
    audio_capture::worker_check_with_seek(
        Box::new(audio),
        &reference,
        Some(Duration::from_millis(if name == "overlap" {
            30
        } else {
            40
        })),
    );
}
#[test]
fn unpadded_control() {
    check("control");
}
#[test]
fn signed_head_padding() {
    check("start");
}
#[test]
fn signed_tail_padding() {
    check("end");
}
#[test]
fn both_packet_edges() {
    check("both");
}
#[test]
fn padding_rounds_up_to_sample_boundary() {
    check("rounding");
}
#[test]
fn whole_tail_packet_is_omitted() {
    check("whole-tail");
}
#[test]
fn negative_pts_and_head_padding_overlap() {
    check("overlap");
}
#[test]
fn excessive_padding_refuses_decoded_packet() {
    let bytes = std::fs::read(root().join("invalid-padding.mkv")).unwrap();
    let mut audio =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = audio.make_decoder().unwrap();
    let source = audio.next_packet().unwrap().unwrap();
    let frame = decoder
        .decode_packet(&source.data, source.pts, source.duration as u64)
        .unwrap()
        .unwrap();
    let error = audio
        .present_decoded(frame.packet, frame.source_pts)
        .err()
        .expect("oversized padding must refuse");
    assert!(
        error
            .to_string()
            .contains("discard padding exceeds decoded packet"),
        "{error}"
    );
    AudioStream::rewind(&mut audio);
    audio_capture::worker_refusal(Box::new(audio), "discard padding exceeds decoded packet");
}
