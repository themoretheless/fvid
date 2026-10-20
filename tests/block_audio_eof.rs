#![cfg(feature = "player")]
use fvid::audio::AudioStream;
#[path = "support/audio_capture.rs"]
mod capture;
use std::{io::Cursor, path::Path};
fn open(kind: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match kind {
        "flac" => Box::new(
            fvid::playback_flac::FlacAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "ac3" => Box::new(
            fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "mp2" => Box::new(
            fvid::playback_mp3::Mp3AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
#[test]
fn elementary_block_audio_seeks_to_eof_without_replaying_the_last_frame() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let base = root.join("playback-errors/block-audio-eof");
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
        let mut decoder = source.make_decoder().unwrap();
        let mut decoded = Vec::new();
        let end;
        let mut count = 0;
        let mut encoded = Vec::new();
        while let Some(p) = source.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            let packet = source
                .present_decoded(f.packet, f.source_pts)
                .unwrap()
                .unwrap();
            decoded.push((packet.presentation_time(), packet.data));

            count += 1;
            encoded.push(p);
        }
        end = decoded
            .iter()
            .map(|(_, data)| data.len() / 4 / usize::from(source.channels()))
            .sum::<usize>() as i64;
        assert!(count > 0);
        source.validate_eof().unwrap();
        for target in [0, 1, end - 1] {
            if matches!(kind, "mp2" | "ac3") {
                assert_eq!(source.seek_to(target), target);
                let mut engine = source.make_decoder().unwrap();
                let mut suffix = Vec::new();
                while let Some(p) = source.next_packet().unwrap() {
                    let f = engine
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(packet) = source.present_decoded(f.packet, f.source_pts).unwrap() {
                        suffix.extend(packet.data);
                    }
                }
                let full = decoded
                    .iter()
                    .flat_map(|(_, data)| data.iter().copied())
                    .collect::<Vec<_>>();
                assert!(
                    suffix == full[target as usize * usize::from(source.channels()) * 4..],
                    "{kind} preroll suffix at{target}"
                );
                continue;
            }
            let wanted = encoded.iter().rev().find(|p| p.pts <= target).unwrap();
            assert_eq!(
                source.seek_to(target),
                wanted.pts,
                "{kind} in-range landing"
            );
            let actual = source.next_packet().unwrap().unwrap();
            assert_eq!(
                (actual.pts, actual.duration, actual.data.as_slice()),
                (wanted.pts, wanted.duration, wanted.data.as_slice()),
                "{kind} in-range packet"
            );
        }
        let before_failures = failures.len();
        for target in [end, end + 1, i64::MAX] {
            let at = source.seek_to(target);
            let replayed = if matches!(kind, "mp2" | "ac3") {
                let mut engine = source.make_decoder().unwrap();
                let mut replayed = false;
                while let Some(p) = source.next_packet().unwrap() {
                    let f = engine
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    replayed |= source
                        .present_decoded(f.packet, f.source_pts)
                        .unwrap()
                        .is_some();
                }
                replayed
            } else {
                source.next_packet().unwrap().is_some()
            };
            if at != end || replayed {
                failures.push(format!(
                    "{kind} target{target}: landed{at} instead of{end}, replay={}",
                    replayed
                ));
            }
        }
        let duration = source.duration().unwrap();
        if failures.len() == before_failures {
            capture::worker_check_with_seek(open(kind, bytes), &decoded, Some(duration));
        }
        source.rewind();
        assert_eq!(source.seek_to(i64::MIN), 0);
        assert!(source.next_packet().unwrap().is_some());
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}
