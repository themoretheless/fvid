#![cfg(feature = "player")]
use fvid::playback_native::{NativeReader, RawFrame};
use std::{io::Cursor, path::PathBuf, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/mp4-video-reconfiguration")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
fn fingerprint(raw: RawFrame) -> (usize, usize, u8, Vec<u8>) {
    match raw {
        RawFrame::Planar(p) => (p.frame.width, p.frame.height, p.depth, p.frame.data.clone()),
        _ => panic!("source precision must remain planar"),
    }
}
fn expected(record: &serde_json::Value) -> (usize, usize, u8, Vec<u8>) {
    let depth = record["depth"].as_u64().unwrap() as u8;
    let mut data = Vec::new();
    for v in record["values"].as_array().unwrap() {
        let value = v.as_u64().unwrap() as u16;
        for _ in 0..512 {
            if depth == 8 {
                data.push(value as u8)
            } else {
                data.extend(value.to_le_bytes())
            }
        }
    }
    (32, 16, depth, data)
}
fn metadata(reader: &NativeReader<Cursor<Vec<u8>>>, record: &serde_json::Value) {
    let id = record["description"].as_u64().unwrap();
    assert_eq!(reader.pixel_aspect(), (id as u32, 1));
    let colour = reader.colour();
    assert_eq!(
        (
            colour.primaries,
            colour.transfer,
            colour.matrix,
            colour.full_range
        ),
        if id == 1 {
            (1, 1, 1, false)
        } else {
            (9, 16, 9, true)
        }
    );
    assert_eq!(
        reader.hdr().light.max_cll,
        if id == 1 { 400. } else { 1000. }
    );
}
#[test]
fn independent_controls_validate_owned_changed_parameter_sets_and_exact_planes() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        for (i, record) in case["frames"].as_array().unwrap().iter().enumerate() {
            let mut reader = NativeReader::software(
                Cursor::new(bytes(&format!("{name}-control-{i}.mp4"))),
                32 << 20,
            )
            .unwrap();
            assert_eq!(
                fingerprint(reader.read_frame_raw().unwrap().unwrap()),
                expected(record),
                "{name}/{i}"
            );
            metadata(&reader, record);
            assert!(reader.read_frame_raw().unwrap().is_none());
        }
    }
}
#[test]
fn transitions_preserve_reordered_planes_metadata_navigation_and_eof() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let records = case["frames"].as_array().unwrap();
        let order = case["display_order"].as_array().unwrap();
        for suffix in [
            "",
            "-fragmented",
            "-start-second",
            "-start-second-fragmented",
        ] {
            let movie = format!("{name}{suffix}.mp4");
            let mut reader = NativeReader::software(Cursor::new(bytes(&movie)), 32 << 20).unwrap();
            for (pts, index) in order.iter().enumerate() {
                let record = &records[index.as_u64().unwrap() as usize];
                assert_eq!(
                    fingerprint(reader.read_frame_raw().unwrap().unwrap()),
                    expected(record),
                    "{movie}/{pts}"
                );
                metadata(&reader, record);
                assert_eq!(
                    reader.frame_interval(),
                    Some((pts as u128, pts as u128 + 1, 50))
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
            assert_eq!(
                fingerprint(reader.read_frame_raw().unwrap().unwrap()),
                expected(&records[0])
            );
            for ms in [20, 60, 40, 80, 0, 39, 99] {
                let at = (ms / 20) as usize;
                let record = &records[order[at].as_u64().unwrap() as usize];
                assert_eq!(
                    fingerprint(reader.seek_raw(Duration::from_millis(ms)).unwrap().unwrap()),
                    expected(record),
                    "{movie}/seek{ms}"
                );
                metadata(&reader, record);
                assert_eq!(
                    reader.frame_interval(),
                    Some((at as u128, at as u128 + 1, 50))
                );
            }
        }
    }
}
#[test]
fn retaining_first_setup_cannot_satisfy_the_same_packet_oracles() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        if name == "constant" {
            continue;
        }
        let records = case["frames"].as_array().unwrap();
        let mut reader = NativeReader::software(
            Cursor::new(bytes(&format!("{name}-wrong-first-setup.mp4"))),
            32 << 20,
        )
        .unwrap();
        let mut mismatch = false;
        for index in case["display_order"].as_array().unwrap() {
            match reader.read_frame_raw() {
                Ok(Some(raw)) => {
                    if fingerprint(raw) != expected(&records[index.as_u64().unwrap() as usize]) {
                        mismatch = true;
                        break;
                    }
                }
                Ok(None) | Err(_) => {
                    mismatch = true;
                    break;
                }
            }
        }
        assert!(
            mismatch,
            "{name}: fixture must require the changed setup rather than only contain extra descriptors"
        );
    }
}

#[test]
fn reordered_avc_vui_stays_with_displayed_frame_even_after_next_decoder_opens() {
    let document = manifest();
    let case = &document["cases"][0];
    let records = case["frames"].as_array().unwrap();
    for suffix in [
        "",
        "-fragmented",
        "-start-second",
        "-start-second-fragmented",
    ] {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes(&format!("avc{suffix}.mp4"))),
            Default::default(),
            32 << 20,
        )
        .unwrap();
        for index in case["display_order"].as_array().unwrap() {
            let record = &records[index.as_u64().unwrap() as usize];
            let second = record["description"] == 2;
            reader.read_frame().unwrap().unwrap();
            assert_eq!(reader.active_colour().unwrap().full, second);
            assert_eq!(
                reader.bitstream_colour().primaries,
                if second { 9 } else { 1 }
            );
            assert_eq!(
                reader.container_description().pixel_aspect,
                if second { (2, 1) } else { (1, 1) }
            );
        }
    }
}

#[test]
fn default_player_path_uses_owned_reconfiguration_for_changed_setups() {
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] != "constant")
    {
        let name = case["name"].as_str().unwrap();
        let records = case["frames"].as_array().unwrap();
        let mut reader =
            NativeReader::new(Cursor::new(bytes(&format!("{name}.mp4"))), 32 << 20).unwrap();
        for i in case["display_order"].as_array().unwrap() {
            let record = &records[i.as_u64().unwrap() as usize];
            assert_eq!(
                fingerprint(reader.read_frame_raw().unwrap().unwrap()),
                expected(record)
            );
            metadata(&reader, record);
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn failed_late_reconfiguration_requires_rewind_and_can_reopen_initial_setup() {
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("avc-invalid-late-configuration.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    assert_eq!(reader.read_frame().unwrap().unwrap().picture.bit_depth, 8);
    let mut failure = None;
    for _ in 0..5 {
        match reader.read_frame() {
            Err(e) => {
                failure = Some(e);
                break;
            }
            Ok(Some(_)) => {}
            Ok(None) => panic!("late invalid setup was ignored"),
        }
    }
    let error = failure.expect("late invalid configuration must fail");
    assert!(error.to_string().contains("configuration"), "{error}");
    let error = reader
        .read_frame()
        .err()
        .expect("error must latch until rewind");
    assert!(error.to_string().contains("requires rewind"), "{error}");
    reader.rewind();
    assert_eq!(reader.read_frame().unwrap().unwrap().picture.bit_depth, 8);
}

#[test]
fn rewind_metadata_returns_to_initial_configuration_before_decoding() {
    let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("avc-rewind-tail.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    while reader.read_frame().unwrap().is_some() {}
    reader.rewind();
    assert_eq!(reader.bitstream_colour().primaries, 1);
    assert!(!reader.active_colour().unwrap().full);
    assert_eq!(reader.container_description().pixel_aspect, (1, 1));
    assert_eq!(reader.read_frame().unwrap().unwrap().picture.bit_depth, 8);
}

#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn unchanged_audio_keeps_exact_pcm_and_worker_rewind_beside_video_reconfiguration() {
    use fvid::audio::AudioStream;
    let stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
        Cursor::new(bytes("avc-with-constant-audio.mp4")),
        Default::default(),
    )
    .unwrap();
    assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
    let raw = bytes("constant-stereo.s16le");
    let audio_expected: Vec<_> = raw
        .chunks_exact(4)
        .enumerate()
        .map(|(i, b)| {
            let data = b
                .chunks_exact(2)
                .flat_map(|sample| {
                    (f32::from(i16::from_le_bytes(sample.try_into().unwrap())) / 32768.)
                        .to_le_bytes()
                })
                .collect();
            (Duration::from_nanos(i as u64 * 1_000_000_000 / 48000), data)
        })
        .collect();
    capture::worker_check(Box::new(stream), &audio_expected);
    let mut video =
        NativeReader::software(Cursor::new(bytes("avc-with-constant-audio.mp4")), 32 << 20)
            .unwrap();
    let document = manifest();
    let case = &document["cases"][0];
    for index in case["display_order"].as_array().unwrap() {
        assert_eq!(
            fingerprint(video.read_frame_raw().unwrap().unwrap()),
            expected(&case["frames"][index.as_u64().unwrap() as usize])
        );
    }
}
