use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-number-timeline")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[test]
fn legal_numbered_streams_keep_cumulative_sample_positions() {
    for case in manifest()["controls"].as_array().unwrap() {
        let flac = fvid_media::owned_flac::Flac::parse(
            &bytes(case["file"].as_str().unwrap()),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(flac.samples(), case["frames"].as_u64().unwrap());
        let mut samples = 0;
        for (actual, packet) in flac.frames.iter().zip(case["packets"].as_array().unwrap()) {
            assert_eq!(actual.pts, samples);
            samples += packet["frames"].as_u64().unwrap();
        }
        assert_eq!(flac.frames.len(), case["packets"].as_array().unwrap().len());
        let data = bytes(case["file"].as_str().unwrap());
        let exact = fvid_media::owned_flac::Limits {
            file_bytes: data.len(),
            packets: flac.frames.len(),
        };
        assert!(fvid_media::owned_flac::Flac::parse(&data, &exact).is_ok());
        for limits in [
            fvid_media::owned_flac::Limits {
                packets: exact.packets - 1,
                ..exact
            },
            fvid_media::owned_flac::Limits {
                file_bytes: exact.file_bytes - 1,
                ..exact
            },
        ] {
            assert!(fvid_media::owned_flac::Flac::parse(&data, &limits).is_err());
        }
    }
}
#[test]
fn baseline_contradictory_header_clocks_are_silently_reindexed() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let flac = fvid_media::owned_flac::Flac::parse(&bytes(name), &Default::default()).unwrap();
        assert_eq!(
            flac.frames.iter().map(|f| f.pts).collect::<Vec<_>>(),
            [0, 64, 128]
        );
        assert_eq!(flac.samples(), 192);
        eprintln!("{name}: contradictory coded clock admitted and replaced by PTS [0,64,128]");
    }
}
#[test]
#[ignore = "enable with coded-clock validation"]
fn contradictory_coded_numbers_or_strategies_cannot_enter_the_index() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["file"].as_str().unwrap();
        let error = match fvid_media::owned_flac::Flac::parse(&bytes(name), &Default::default()) {
            Ok(_) => panic!("{name}: contradictory coded clock entered the index"),
            Err(e) => e.to_string(),
        };
        assert!(
            error.contains("coded number") || error.contains("blocking strategy"),
            "{name}: {error}"
        );
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::audio::{AudioDecode, AudioStream};
    use std::{io::Cursor, time::Duration};
    fn source(name: &str) -> fvid::playback_flac::FlacAudioReader {
        fvid::playback_flac::FlacAudioReader::open(Cursor::new(bytes(name)), Default::default())
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
    fn direct(case: &serde_json::Value) -> Vec<(Duration, Vec<u8>)> {
        let data = bytes(case["file"].as_str().unwrap());
        let mut decoder =
            fvid::codec::flac_decoder::FlacDecoder::new(&data[8..42], 8000, 1).unwrap();
        let mut samples = 0;
        let mut expected = Vec::new();
        for packet in case["packets"].as_array().unwrap() {
            let frames = packet["frames"].as_u64().unwrap();
            let coded = bytes(packet["file"].as_str().unwrap());
            assert!(fvid_media::owned_flac::frame_header(&coded).is_some());
            let output = decoder
                .decode_packet(&coded, samples as i64, frames)
                .unwrap()
                .unwrap();
            let literal = ((packet["level"].as_i64().unwrap() as f32) / 32768.0)
                .to_le_bytes()
                .repeat(frames as usize);
            assert_eq!(output.packet.data, literal);
            assert_eq!(
                output.packet.presentation_time(),
                Duration::from_nanos(samples * 1_000_000_000 / 8000)
            );
            expected.push((output.packet.presentation_time(), output.packet.data));
            samples += frames;
        }
        expected
    }
    #[test]
    fn checksum_valid_subframes_and_legal_worker_clocks_are_independent_controls() {
        video_control();
        for (key, worker) in [("cases", false), ("controls", true)] {
            for case in manifest()[key].as_array().unwrap() {
                let expected = direct(case);
                assert_eq!(
                    expected
                        .iter()
                        .flat_map(|(_, p)| p.iter().copied())
                        .collect::<Vec<_>>(),
                    bytes(case["expected"].as_str().unwrap())
                );
                if worker {
                    let name = case["file"].as_str().unwrap();
                    let input = source(name);
                    let end = input.time_of(case["frames"].as_i64().unwrap());
                    assert_eq!(input.duration(), Some(end));
                    for target in [expected[1].0, end] {
                        super::capture::worker_check_with_seek(
                            Box::new(source(name)),
                            &expected,
                            Some(target),
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn baseline_bad_clock_files_even_reach_successful_native_pcm() {
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let mut input = source(name);
            let mut decoder = input.make_decoder().unwrap();
            let mut pcm = Vec::new();
            while let Some(packet) = input.next_packet().unwrap() {
                let output = decoder
                    .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                    .unwrap()
                    .unwrap();
                pcm.extend(output.packet.data);
            }
            assert_eq!(pcm, bytes(case["expected"].as_str().unwrap()));
        }
    }
    #[test]
    #[ignore = "enable with coded-clock validation"]
    fn source_refuses_contradictory_clock_before_worker_admission() {
        video_control();
        for case in manifest()["cases"].as_array().unwrap() {
            let name = case["file"].as_str().unwrap();
            let error = match fvid::playback_flac::FlacAudioReader::open(
                Cursor::new(bytes(name)),
                Default::default(),
            ) {
                Ok(_) => panic!("{name}: contradictory clock admitted as audio"),
                Err(e) => e.to_string(),
            };
            assert!(
                error.contains("coded number") || error.contains("blocking strategy"),
                "{name}: {error}"
            );
        }
    }
}
