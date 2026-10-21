use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-page-sequence")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[test]
fn complete_streams_keep_per_serial_packets_and_counter_compatibility() {
    for case in manifest()["controls"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let data = bytes(name);
        let ogg = fvid_media::owned_ogg::Ogg::parse(&data, &Default::default()).unwrap();
        assert_eq!(
            ogg.chains().len(),
            case["chains"].as_u64().unwrap() as usize
        );
        assert_eq!(
            ogg.chains().iter().map(|c| c.packets.len()).sum::<usize>(),
            case["packets"].as_u64().unwrap() as usize
        );
        for chain in ogg.chains() {
            assert!(chain.ended);
            assert_eq!(&chain.packets[0].data[..8], b"OpusHead");
            for packet in &chain.packets[2..] {
                assert_eq!(packet.data, [0xf8, 0xff, 0xfe]);
            }
        }
        let limits = fvid_media::owned_ogg::Limits {
            file_bytes: data.len(),
            packets: case["packets"].as_u64().unwrap() as usize,
        };
        assert!(fvid_media::owned_ogg::Ogg::parse(&data, &limits).is_ok());
        assert!(
            fvid_media::owned_ogg::Ogg::parse(
                &data,
                &fvid_media::owned_ogg::Limits {
                    packets: limits.packets - 1,
                    ..limits
                }
            )
            .is_err()
        );
    }
}
#[test]
fn missing_or_repeated_pages_cannot_be_treated_as_complete_streams() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let error = match fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default()) {
            Ok(_) => panic!("{name}: page sequence contradiction ignored"),
            Err(e) => e.to_string(),
        };
        assert!(error.contains("sequence"), "{name}: {error}");
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn source(name: &str) -> fvid::playback_ogg_opus::OggOpusAudioReader {
        fvid::playback_ogg_opus::OggOpusAudioReader::open(
            Cursor::new(bytes(name)),
            Default::default(),
        )
        .unwrap()
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
    fn reference(frames: u64, chains: u64) -> Vec<(Duration, Vec<u8>)> {
        assert!(frames.is_multiple_of(960 * chains));
        let per_chain = frames / 960 / chains;
        let mut expected = Vec::new();
        let mut decoder =
            fvid::codec::make_audio_decoder("A_OPUS", &bytes("mono.head"), 48000, 1, 0).unwrap();
        for index in 0..frames / 960 {
            if index > 0 && index.is_multiple_of(per_chain) {
                decoder =
                    fvid::codec::make_audio_decoder("A_OPUS", &bytes("mono.head"), 48000, 1, 0)
                        .unwrap();
            }
            let output = decoder
                .decode_packet(&bytes("20ms.packet"), (index * 960) as i64, 960)
                .unwrap()
                .unwrap();
            assert_eq!(output.packet.data.len(), 960 * 4);
            assert!(
                output.packet.data.chunks_exact(4).all(|b| {
                    let v = f32::from_le_bytes(b.try_into().unwrap());
                    v.is_finite() && v.abs() < 1e-6
                }),
                "authored DTX packet must be finite near-silence"
            );
            expected.push((Duration::from_millis(index * 20), output.packet.data));
        }
        expected
    }
    fn matches(name: &str, actual: &[(Duration, Vec<u8>)], expected: &[(Duration, Vec<u8>)]) {
        assert_eq!(actual.len(), expected.len(), "{name}: packet count");
        for (index, (a, e)) in actual.iter().zip(expected).enumerate() {
            assert_eq!(a.0, e.0, "{name}: packet{index} PTS");
            assert!(a.1 == e.1, "{name}: packet{index} PCM differs");
        }
    }
    fn render(name: &str) -> Vec<(Duration, Vec<u8>)> {
        let mut input = source(name);
        let mut decoder = input.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(step) = input.next_step().unwrap() {
            let packet = match step {
                fvid::audio::AudioStep::ResetDecoder => {
                    input.reset_decoder(&mut decoder).unwrap();
                    continue;
                }
                fvid::audio::AudioStep::Pcm(packet) => {
                    out.push((packet.presentation_time(), packet.data));
                    continue;
                }
                fvid::audio::AudioStep::Encoded(packet) => packet,
            };
            let output = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(pcm) = input.present_decoded(output.packet, packet.pts).unwrap() {
                out.push((pcm.presentation_time(), pcm.data));
            }
        }
        out
    }
    #[test]
    fn healthy_opus_clocks_match_independent_packets_and_worker_navigation() {
        video_control();
        for case in manifest()["controls"].as_array().unwrap() {
            if !case["native"].as_bool().unwrap() {
                continue;
            }
            let name = case["file"].as_str().unwrap();
            let frames = case["frames"].as_u64().unwrap();
            let expected = reference(frames, case["chains"].as_u64().unwrap());
            matches(name, &render(name), &expected);
            let end = Duration::from_nanos(frames * 1_000_000_000 / 48000);
            assert_eq!(source(name).duration(), Some(end));
            for target in [expected[1].0, end] {
                super::capture::worker_check_with_seek(
                    Box::new(source(name)),
                    &expected,
                    Some(target),
                );
            }
        }
    }
    #[test]
    fn incomplete_sequence_is_refused_before_audio_worker_admission() {
        video_control();
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let error = match fvid::playback_ogg_opus::OggOpusAudioReader::open(
                Cursor::new(bytes(name)),
                Default::default(),
            ) {
                Ok(_) => panic!("{name}: page loss admitted as complete audio"),
                Err(e) => e.to_string(),
            };
            assert!(error.contains("sequence"), "{name}: {error}");
        }
    }
}
