#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn decode_control(bytes: Vec<u8>) -> Vec<u8> {
    let mut source =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(encoded) = source.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&encoded.data, encoded.pts, encoded.duration as u64)
            .unwrap()
            .unwrap();
        assert!(
            frame
                .packet
                .data
                .chunks_exact(4)
                .all(|b| f32::from_le_bytes(b.try_into().unwrap()).is_finite())
        );
        if let Some(p) = source
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            pcm.extend(p.data);
        }
    }
    pcm
}
#[test]
fn opus_audible_endpoint_does_not_count_discarded_tail_packets_or_depend_on_decode_prefix() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-opus-endpoint");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut wrong = Vec::new();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let bytes = std::fs::read(base.join(case["file"].as_str().unwrap())).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..case["video_frames"].as_u64().unwrap() {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let control =
            decode_control(std::fs::read(base.join(case["control"].as_str().unwrap())).unwrap());
        assert_eq!(
            control.len() / 4,
            case["total_frames"].as_u64().unwrap() as usize
        );
        let first = case["first"].as_u64().unwrap() as usize * 4;
        let last = case["last"].as_u64().unwrap() as usize * 4;
        let expected = &control[first..last];
        let end = Some(Duration::from_nanos(case["end_ns"].as_u64().unwrap()));
        let mut source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        let initial = source.duration();
        let prefix_end = if case["indexed_known"].as_bool().unwrap() {
            end
        } else {
            initial
        };
        if initial != prefix_end {
            wrong.push(format!(
                "{name} before decode: {initial:?} versus{prefix_end:?}"
            ));
        }
        let mut decoder = source.make_decoder().unwrap();
        let mut pcm = Vec::new();
        let mut intervals = Vec::new();
        let mut decoded = 0;
        while let Some(encoded) = source.next_packet().unwrap() {
            let frame = decoder
                .decode_packet(&encoded.data, encoded.pts, encoded.duration as u64)
                .unwrap()
                .unwrap();
            decoded += frame.packet.data.len() / 4;
            if let Some(packet) = source
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                intervals.push((packet.presentation_time(), packet.data.clone()));
                pcm.extend(packet.data);
            }
            if source.duration() != prefix_end {
                wrong.push(format!(
                    "{name} decoded{decoded}: {:?} versus{prefix_end:?}",
                    source.duration()
                ));
            }
        }
        assert_eq!(decoded, case["total_frames"].as_u64().unwrap() as usize);
        assert_eq!(pcm, expected, "{name} correct native audio control");
        source.validate_eof().unwrap();
        if source.duration() != end {
            wrong.push(format!("{name} EOF: {:?} versus{end:?}", source.duration()));
        }
        AudioStream::rewind(&mut source);
        if source.duration() != end {
            wrong.push(format!(
                "{name} rewind: {:?} versus{end:?}",
                source.duration()
            ));
        }
        let source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        capture::worker_check_with_seek(Box::new(source), &intervals, Some(Duration::ZERO));
        let source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        let playback = fvid::audio_thread::AudioPlayback::start(Box::new(source), || {
            Box::new(fvid::audio::NullBackend::default())
        });
        if playback.duration() != prefix_end {
            wrong.push(format!(
                "{name} handle: {:?} versus{prefix_end:?}",
                playback.duration()
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "Opus endpoint used untrimmed/raw tail or partial decode history: {wrong:?}"
    );
}
