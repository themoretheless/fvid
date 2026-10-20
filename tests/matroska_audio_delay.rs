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
        .join("tests/fixtures/playback-errors/matroska-audio-delay")
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
    let delay = case["delay_ns"].as_i64().unwrap();
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
    let last_source = 40_000_000 - delay;
    let target = last_source.max(0);
    for pass in 0..3 {
        if pass != 0 {
            AudioStream::rewind(&mut audio);
            decoder.reset();
        }
        if pass == 2 {
            assert_eq!(
                audio.seek_to(target),
                last_source,
                "{name}: seek source clock"
            );
        }
        let mut index = if pass == 2 { 2 } else { 0 };
        let mut actual = Vec::new();
        let mut windows = Vec::new();
        while let Some(source) = audio.next_packet().unwrap() {
            assert_eq!(
                source.pts,
                index * 20_000_000 - delay,
                "{name}: CodecDelay source packet{index}"
            );
            index += 1;
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
        assert_eq!(index, 3);
        audio.validate_eof().unwrap();
        let wanted: Vec<_> = case["packets"]
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
        assert_eq!(windows, wanted, "{name}, pass{pass}");
        let retained: usize = wanted.iter().map(|(_, frames)| frames * 4).sum();
        assert_eq!(actual, expected[expected.len() - retained..]);
        assert_eq!(
            audio.duration(),
            Some(Duration::from_nanos(case["end_ns"].as_u64().unwrap())),
            "{name}: endpoint"
        );
    }
    AudioStream::rewind(&mut audio);
    audio_capture::worker_check_with_seek(
        Box::new(audio),
        &reference,
        Some(Duration::from_nanos(target as u64)),
    );
}
#[test]
fn no_delay_control() {
    check("control");
}
#[test]
fn one_nanosecond_delay() {
    check("one-ns");
}
#[test]
fn fractional_packet_delay() {
    check("ten-ms");
}
#[test]
fn whole_packet_delay() {
    check("whole-head");
}
#[test]
fn entirely_negative_audio() {
    check("all-preroll");
}
#[test]
fn delay_overlaps_signed_padding() {
    check("padding-overlap");
}
#[test]
fn unrepresentable_delay_refuses_source_timestamp() {
    let bytes = std::fs::read(root().join("overflow-delay.mkv")).unwrap();
    let mut audio =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let error = audio
        .next_packet()
        .err()
        .expect("unrepresentable delayed timestamp must refuse");
    assert!(
        error.to_string().contains("codec delay timestamp overflow"),
        "{error}"
    );
    AudioStream::rewind(&mut audio);
    audio_capture::worker_refusal(Box::new(audio), "codec delay timestamp overflow");
}
