use fvid_media::owned_aac::{config::AudioSpecificConfig, NativeAacDecoder};
use serde_json::Value;
use std::{io::Cursor, path::PathBuf, time::Duration};
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-pce-profile.json"
    ))
    .unwrap()
}
fn path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}
fn hex(value: &Value) -> Vec<u8> {
    value
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn bytes(v: &Value) -> Vec<u8> {
    std::fs::read(path(v["file"].as_str().unwrap())).unwrap()
}
fn compare(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}");
    assert!(expected
        .chunks_exact(4)
        .any(|v| f32::from_le_bytes(v.try_into().unwrap()) != 0.0));
    for (i, (a, b)) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-7, "{name} sample {i}: {a} vs {b}");
    }
}
#[test]
fn explicit_pce_serialization_preserves_main_lc_and_ssr_object_and_program() {
    for case in manifest()["cases"].as_array().unwrap() {
        let config = hex(&case["asc"]);
        let owned = AudioSpecificConfig::parse(&config).unwrap();
        let program = owned.program.as_ref().unwrap();
        let output = program.audio_specific_config().unwrap();
        let parsed = AudioSpecificConfig::parse(&output).unwrap();
        assert_eq!(parsed, owned, "{} owned serialization", case["name"]);
        assert_eq!(
            parsed.core.object_type,
            case["object_type"].as_u64().unwrap() as u32
        );
        let root = fvid::codec::config::AudioSpecificConfig::parse(&config).unwrap();
        let output = root
            .program
            .as_ref()
            .unwrap()
            .audio_specific_config()
            .unwrap();
        assert_eq!(
            fvid::codec::config::AudioSpecificConfig::parse(&output).unwrap(),
            root,
            "{} root serialization",
            case["name"]
        );
    }
}
#[test]
fn profile_pce_adts_and_video_decode_match_independent_pcm_and_bootstrap_metadata() {
    for case in manifest()["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let config = hex(&case["asc"]);
        let expected_config = AudioSpecificConfig::parse(&config).unwrap();
        let video = bytes(&case["video"]);
        let mut mp4 = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&video),
            &mut mp4,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, case["samples"].as_u64().unwrap());
        compare(
            &mp4,
            &std::fs::read(path(case["pcm_file"].as_str().unwrap())).unwrap(),
            name,
        );
        let mut root = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video, &mut root).unwrap();
        assert_eq!(root, mp4);
        for file in case["adts"].as_array().unwrap() {
            let data = bytes(file);
            let indexed =
                fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default()).unwrap();
            assert_eq!(
                AudioSpecificConfig::parse(&indexed.configuration).unwrap(),
                expected_config
            );
            assert_eq!(indexed.samples(), case["samples"].as_u64().unwrap());
            let indexed_root =
                fvid::container::adts::Aac::parse(&data, &Default::default()).unwrap();
            assert_eq!(indexed_root.configuration, indexed.configuration);
            let mut reader =
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            assert_eq!(
                AudioSpecificConfig::parse(reader.audio_specific_config()).unwrap(),
                expected_config
            );
            assert_eq!(
                reader.configuration().channels,
                case["channels"].as_u64().unwrap() as u16
            );
            let mut decoder = NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
            let mut pcm = Vec::new();
            let mut pts = 0;
            while let Some(packet) = reader.next_packet().unwrap() {
                let saved = decoder.checkpoint();
                let actual = decoder.decode_timed(&packet, pts, 1024).unwrap();
                decoder.restore(&saved).unwrap();
                assert_eq!(decoder.decode_timed(&packet, pts, 1024).unwrap(), actual);
                if let Some(frame) = actual {
                    pcm.extend(frame.samples.iter().flat_map(|v| v.to_le_bytes()));
                }
                pts += 1024;
            }
            if let Some(frame) = decoder.finish().unwrap() {
                pcm.extend(frame.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
            assert!(decoder.finish().unwrap().is_none());
            assert_eq!(pcm, mp4, "{name} raw ADTS");
            let mut stream = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut stream,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stream, mp4, "{name} owned ADTS");
            let mut root = Vec::new();
            fvid::native_media::decode_aac_pcm_interval(
                &data,
                &mut root,
                &Default::default(),
                None,
            )
            .unwrap();
            assert_eq!(root, mp4, "{name} root ADTS");
            let interval = (Duration::from_millis(55), Duration::from_millis(145));
            let mut selection = Vec::new();
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut selection,
                Some(interval),
                &Default::default(),
            )
            .unwrap();
            let size = case["channels"].as_u64().unwrap() as usize * 4;
            assert_eq!(selection, mp4[1320 * size..3480 * size], "{name} interval");
        }
    }
}
#[test]
fn mismatched_pce_profile_and_rate_have_precise_adts_and_mp4_refusals() {
    for case in manifest()["invalid"].as_array().unwrap() {
        assert_eq!(
            AudioSpecificConfig::parse(&hex(&case["asc"]))
                .unwrap_err()
                .to_string(),
            "AAC PCE disagrees with AudioSpecificConfig"
        );
        let data = bytes(&case["adts"]);
        assert_eq!(
            fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default())
                .unwrap_err()
                .to_string(),
            "ADTS PCE disagrees with frame coding or rate"
        );
        assert_eq!(
            fvid::container::adts::Aac::parse(&data, &Default::default())
                .unwrap_err()
                .to_string(),
            "ADTS PCE disagrees with frame coding or rate"
        );
        assert_eq!(
            fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data))
                .err()
                .unwrap()
                .to_string(),
            "ADTS PCE disagrees with frame coding or rate"
        );
        assert!(fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(&case["video"])),
            &mut Vec::new(),
            None,
            &Default::default()
        )
        .unwrap_err()
        .to_string()
        .contains("AAC PCE disagrees with AudioSpecificConfig"));
    }
}

#[cfg(feature = "player")]
#[test]
fn explicit_pce_profile_and_main_coupling_playback_rewinds_seeks_and_replays_checkpoints() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            let before = decoder.checkpoint().unwrap();
            let actual = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            decoder.restore(&before).unwrap();
            let replay = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            assert_eq!(
                actual
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration)),
                replay
                    .as_ref()
                    .map(|f| (&f.packet.data, f.source_pts, f.source_duration))
            );
            if let Some(frame) = actual {
                if let Some(pcm) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    output.extend(pcm.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        output
    }
    for case in manifest()["cases"].as_array().unwrap() {
        let data = bytes(&case["video"]);
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(play(&mut reader), expected, "{} playback", case["name"]);
        reader.rewind();
        assert_eq!(play(&mut reader), expected, "{} rewind", case["name"]);
        for at in [1100, 3000, case["samples"].as_u64().unwrap()] {
            let landed = reader.seek_to(at as i64);
            assert_eq!(
                play(&mut reader),
                expected[landed as usize * case["channels"].as_u64().unwrap() as usize * 4..],
                "{} seek {at}",
                case["name"]
            );
        }
    }
}

#[test]
fn truncated_main_cce_end_video_has_exact_failure_and_preserves_predictor_and_synthesis_history() {
    let m = manifest();
    let blob = include_bytes!("fixtures/playback-errors/aac-pce-profile-packets.bin");
    let bad = &m["invalid_packets"][0];
    let valid = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == bad["valid_name"])
        .unwrap();
    let mut owned = NativeAacDecoder::new(&hex(&valid["asc"])).unwrap();
    let mut root = fvid::codec::aac_native::NativeAacDecoder::new(&hex(&valid["asc"])).unwrap();
    let first = &valid["frames"][0];
    let at = first["offset"].as_u64().unwrap() as usize;
    let len = first["bytes"].as_u64().unwrap() as usize;
    owned.decode_timed(&blob[at..at + len], 0, 1024).unwrap();
    root.decode_timed(&blob[at..at + len], 0, 1024).unwrap();
    let saved = owned.checkpoint();
    let row = &valid["frames"][1];
    let start = row["offset"].as_u64().unwrap() as usize;
    let size = row["bytes"].as_u64().unwrap() as usize;
    let expected = owned
        .decode_timed(&blob[start..start + size], 1024, 1024)
        .unwrap();
    owned.restore(&saved).unwrap();
    let row = &bad["frames"][1];
    let at = row["offset"].as_u64().unwrap() as usize;
    let len = row["bytes"].as_u64().unwrap() as usize;
    assert_eq!(
        owned
            .decode_timed(&blob[at..at + len], 1024, 1024)
            .unwrap_err()
            .to_string(),
        bad["error"].as_str().unwrap()
    );
    assert_eq!(
        root.decode_timed(&blob[at..at + len], 1024, 1024)
            .unwrap_err()
            .to_string(),
        bad["error"].as_str().unwrap()
    );
    assert_eq!(
        owned
            .decode_timed(&blob[start..start + size], 1024, 1024)
            .unwrap(),
        expected
    );
    let replay = root
        .decode_timed(&blob[start..start + size], 1024, 1024)
        .unwrap();
    let replay = replay.unwrap();
    let expected = expected.unwrap();
    assert_eq!(replay.samples, expected.samples);
    assert_eq!(replay.pts, expected.pts);
    assert_eq!(replay.duration, expected.duration);
    let adts = bytes(&bad["adts"]);
    assert_eq!(
        fvid_media::owned_aac::adts::Aac::parse(&adts, &Default::default())
            .unwrap()
            .packets(),
        2
    );
    assert_eq!(
        fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(adts),
            &mut Vec::new(),
            None,
            &Default::default()
        )
        .unwrap_err()
        .to_string(),
        bad["error"].as_str().unwrap()
    );
    assert!(fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(&bad["video"])),
        &mut Vec::new(),
        None,
        &Default::default()
    )
    .unwrap_err()
    .to_string()
    .contains(bad["error"].as_str().unwrap()));
}
