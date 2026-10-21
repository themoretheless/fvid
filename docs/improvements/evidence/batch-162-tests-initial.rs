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
fn baseline_crc_valid_counter_gaps_and_repeats_are_admitted() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let ogg = fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default()).unwrap();
        assert_eq!(ogg.pages(), case["packets"].as_u64().unwrap() as usize);
        assert!(ogg.chains()[0].ended);
        eprintln!("{name}: all page CRCs valid,but contradictory sequence accepted");
    }
}
#[test]
#[ignore = "enable with per-stream page sequence validation"]
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
    fn render(name: &str) -> Vec<(Duration, Vec<u8>)> {
        let mut input = source(name);
        let mut decoder = input.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(packet) = input.next_packet().unwrap() {
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
    fn healthy_opus_clocks_keep_literal_silence_and_actual_worker_navigation() {
        video_control();
        for case in manifest()["controls"].as_array().unwrap() {
            if !case["native"].as_bool().unwrap() {
                continue;
            }
            let name = case["file"].as_str().unwrap();
            let expected = render(name);
            let frames = case["frames"].as_u64().unwrap();
            let all: Vec<_> = expected
                .iter()
                .flat_map(|(_, p)| p.iter().copied())
                .collect();
            assert_eq!(all, vec![0; frames as usize * 4], "{name}");
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
    fn baseline_lost_audio_page_is_hidden_as_a_legitimate_granule_origin() {
        video_control();
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let expected_frames = case["before_frames"].as_u64().unwrap();
            let pcm: Vec<_> = render(name).into_iter().flat_map(|(_, p)| p).collect();
            assert_eq!(pcm, vec![0; expected_frames as usize * 4]);
            assert_eq!(
                source(name).duration(),
                Some(Duration::from_nanos(
                    expected_frames * 1_000_000_000 / 48000
                ))
            );
            eprintln!("{name}: admitted as successful audio with{expected_frames}frames");
        }
        assert_eq!(
            source("complete-control.ogg").duration(),
            Some(Duration::from_millis(60))
        );
        assert_eq!(
            source("lost-first-audio.ogg").duration(),
            Some(Duration::from_millis(40))
        );
    }
    #[test]
    #[ignore = "enable with per-stream page sequence validation"]
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
