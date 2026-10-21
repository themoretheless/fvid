use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-continuation-boundary")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    manifest()[key].as_array().unwrap().clone()
}
fn check_packets(case: &serde_json::Value, key: &str) {
    let name = case["file"].as_str().unwrap();
    let parsed = fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default()).unwrap();
    assert_eq!(parsed.chains().len(), 1);
    let chain = &parsed.chains()[0];
    assert_eq!(chain.ended, case["ended"].as_bool().unwrap());
    let expected = case[key].as_array().unwrap();
    assert_eq!(
        chain.packets.len(),
        expected.len() + 2,
        "{name}: complete audio packet count"
    );
    for (actual, wanted) in chain.packets[2..].iter().zip(expected) {
        assert!(
            actual.data == bytes(wanted["file"].as_str().unwrap()),
            "{name}: recovered packet bytes differ"
        );
    }
}
#[test]
fn complete_and_prefix_packet_boundaries_match_authored_payloads() {
    for case in cases("controls") {
        check_packets(&case, "parts");
        let data = bytes(case["file"].as_str().unwrap());
        let exact = fvid_media::owned_ogg::Limits {
            file_bytes: data.len(),
            packets: case["parts"].as_array().unwrap().len() + 2,
        };
        assert!(fvid_media::owned_ogg::Ogg::parse(&data, &exact).is_ok());
        assert!(
            fvid_media::owned_ogg::Ogg::parse(
                &data,
                &fvid_media::owned_ogg::Limits {
                    packets: exact.packets - 1,
                    ..exact
                }
            )
            .is_err()
        );
    }
}
#[test]
fn baseline_fragment_tails_are_published_as_complete_codec_packets() {
    for key in ["malformed", "joined"] {
        for case in cases(key) {
            check_packets(&case, "before_parts");
            eprintln!(
                "{}: fragment boundary ignored;published{}audio packets",
                case["file"].as_str().unwrap(),
                case["before_parts"].as_array().unwrap().len()
            );
        }
    }
}
#[test]
#[ignore = "enable with continuation-aware assembly"]
fn joined_initial_fragments_are_discarded_until_a_whole_packet_begins() {
    for case in cases("joined") {
        check_packets(&case, "parts");
        let data = bytes(case["file"].as_str().unwrap());
        assert!(
            fvid_media::owned_ogg::Ogg::parse(
                &data,
                &fvid_media::owned_ogg::Limits {
                    file_bytes: data.len(),
                    packets: 3
                }
            )
            .is_ok()
        );
    }
}
#[test]
#[ignore = "enable with continuation-aware assembly"]
fn a_fresh_page_cannot_complete_or_bypass_an_unfinished_packet() {
    for case in cases("malformed") {
        let name = case["file"].as_str().unwrap();
        let error = match fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default()) {
            Ok(_) => panic!("{name}: unfinished packet bypassed"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains("continuation"), "{name}: {error}");
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::audio::{AudioStep, AudioStream};
    use std::{io::Cursor, time::Duration};
    fn source(name: &str) -> fvid::playback_ogg_opus::OggOpusAudioReader {
        fvid::playback_ogg_opus::OggOpusAudioReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap()
    }
    fn time(samples: u64) -> Duration {
        Duration::from_nanos(samples * 1_000_000_000 / 48000)
    }
    fn reference(parts: &[serde_json::Value]) -> Vec<(Duration, Vec<u8>)> {
        let mut decoder =
            fvid::codec::make_audio_decoder("A_OPUS", &bytes("mono.head"), 48000, 1, 0).unwrap();
        let mut at = 0u64;
        let mut expected = Vec::new();
        for part in parts {
            let frames = part["frames"].as_u64().unwrap();
            assert!(frames > 0);
            let output = decoder
                // The generic Opus decoder's packet clock is nanoseconds;
                // the independent oracle's frame counter is in 48 kHz samples.
                .decode_packet(
                    &bytes(part["file"].as_str().unwrap()),
                    i64::try_from(time(at).as_nanos()).unwrap(),
                    u64::try_from(time(frames).as_nanos()).unwrap(),
                )
                .unwrap()
                .unwrap();
            assert_eq!(output.packet.presentation_time(), time(at));
            assert_eq!(output.packet.data.len(), frames as usize * 4);
            assert!(
                output.packet.data.chunks_exact(4).all(|b| {
                    let v = f32::from_le_bytes(b.try_into().unwrap());
                    v.is_finite() && v.abs() < 1e-6
                }),
                "authored DTX must be finite near-silence"
            );
            expected.push((time(at), output.packet.data));
            at += frames;
        }
        expected
    }
    fn render(name: &str) -> Vec<(Duration, Vec<u8>)> {
        let mut input = source(name);
        let mut decoder = input.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(step) = input.next_step().unwrap() {
            match step {
                AudioStep::ResetDecoder => input.reset_decoder(&mut decoder).unwrap(),
                AudioStep::Pcm(p) => out.push((p.presentation_time(), p.data)),
                AudioStep::Encoded(p) => {
                    let output = decoder
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    if let Some(pcm) = input.present_decoded(output.packet, p.pts).unwrap() {
                        out.push((pcm.presentation_time(), pcm.data));
                    }
                }
            }
        }
        out
    }
    fn matches(name: &str, actual: &[(Duration, Vec<u8>)], expected: &[(Duration, Vec<u8>)]) {
        assert_eq!(actual.len(), expected.len(), "{name}: packet count");
        for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(a.0, e.0, "{name}:packet{index}PTS");
            assert!(a.1 == e.1, "{name}:packet{index}PCM differs");
        }
    }
    fn end(expected: &[(Duration, Vec<u8>)]) -> Duration {
        let (at, pcm) = expected.last().unwrap();
        *at + time(pcm.len() as u64 / 4)
    }
    fn worker(name: &str, expected: &[(Duration, Vec<u8>)]) {
        let target = time(expected[0].1.len() as u64 / 4 / 2);
        let mut clipped = Vec::new();
        for (at, pcm) in expected {
            if *at + time(pcm.len() as u64 / 4) <= target {
                continue;
            }
            let skip = if target > *at {
                ((target - *at).as_nanos() * 48000 / 1_000_000_000) as usize
            } else {
                0
            };
            clipped.push((*at + time(skip as u64), pcm[skip * 4..].to_vec()));
        }
        capture::worker_check_with_seek_expected(
            Box::new(source(name)),
            expected,
            target,
            &clipped,
        );
        capture::worker_check_with_seek(Box::new(source(name)), expected, Some(end(expected)));
    }
    fn video_control() {
        let mut video = fvid::playback_native::NativeReader::software(
            Cursor::new(bytes("video.mkv")),
            32 << 20,
        )
        .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
    }
    #[test]
    fn complete_packet_controls_match_independent_codec_pcm_and_worker_seeks() {
        video_control();
        for case in cases("controls") {
            let name = case["file"].as_str().unwrap();
            let expected = reference(case["parts"].as_array().unwrap());
            matches(name, &render(name), &expected);
            assert_eq!(source(name).duration(), Some(end(&expected)));
            worker(name, &expected);
        }
    }
    #[test]
    fn baseline_fragment_boundaries_change_audio_clocks_or_reach_a_codec_refusal() {
        video_control();
        for key in ["malformed", "joined"] {
            for case in cases(key) {
                let name = case["file"].as_str().unwrap();
                if case["before_source_refusal"].as_bool().unwrap_or(false) {
                    let error = match fvid::playback_ogg_opus::OggOpusAudioReader::open(
                        Cursor::new(bytes(name)),
                        Default::default(),
                    ) {
                        Ok(_) => panic!("{name}: expected existing empty-packet codec refusal"),
                        Err(e) => e.to_string(),
                    };
                    assert!(error.contains("Opus packet"), "{name}:{error}");
                    eprintln!("{name}: existing codec refusal {error}");
                    continue;
                }
                let expected = reference(case["before_parts"].as_array().unwrap());
                matches(name, &render(name), &expected);
                assert_eq!(source(name).duration(), Some(end(&expected)));
                eprintln!(
                    "{name}: wrong/ambiguous framing reaches native{}ms audio",
                    end(&expected).as_millis()
                );
                if name == "missing-final-prefix.ogg" {
                    capture::worker_check_with_seek(
                        Box::new(source(name)),
                        &expected,
                        Some(Duration::from_millis(10)),
                    );
                }
            }
        }
        assert_eq!(
            source("split-padded.ogg").duration(),
            Some(Duration::from_millis(20))
        );
    }
    #[test]
    #[ignore = "enable with continuation-aware assembly"]
    fn joined_captures_play_only_whole_packets_and_seek_on_their_clock() {
        video_control();
        for case in cases("joined") {
            let name = case["file"].as_str().unwrap();
            let expected = reference(case["parts"].as_array().unwrap());
            matches(name, &render(name), &expected);
            assert_eq!(source(name).duration(), Some(end(&expected)));
            worker(name, &expected);
        }
    }
    #[test]
    #[ignore = "enable with continuation-aware assembly"]
    fn malformed_continuation_is_refused_before_native_audio_admission() {
        video_control();
        for case in cases("malformed") {
            let name = case["file"].as_str().unwrap();
            let error = match fvid::playback_ogg_opus::OggOpusAudioReader::open(
                Cursor::new(bytes(name)),
                Default::default(),
            ) {
                Ok(_) => panic!("{name}: malformed continuation reached playback"),
                Err(e) => e.to_string(),
            };
            assert!(error.contains("continuation"), "{name}:{error}");
        }
    }
}
