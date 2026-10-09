use fvid_media::owned_aac::{NativeAacDecoder, aac_main_predictor::MainPredictor};
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-main-prediction.json"
    ))
    .unwrap()
}
fn config(v: &Value) -> Vec<u8> {
    v["case"]["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn main_prediction_matches_independent_scalar_oracle_and_snapshot_replay() {
    let manifest = manifest();
    let mut bank = MainPredictor::new(64).unwrap();
    for (index, row) in manifest["oracle"].as_array().unwrap().iter().enumerate() {
        if row["short"].as_bool() == Some(true) {
            bank.short_window();
            continue;
        }
        let input: Vec<f32> = row["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap() as f32)
            .collect();
        let flags: Vec<bool> = row["used"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_bool().unwrap())
            .collect();
        let reset = row["reset"].as_u64().map(|v| v as u8);
        let mut snapshot = bank.clone();
        let mut replay = input.clone();
        let mut output = input;
        bank.process(&mut output, &[0, 4, 34, 64], &flags, reset)
            .unwrap();
        snapshot
            .process(&mut replay, &[0, 4, 34, 64], &flags, reset)
            .unwrap();
        assert_eq!(output, replay);
        assert_eq!(bank, snapshot);
        for (line, (value, expected)) in output
            .iter()
            .zip(row["output_bits"].as_array().unwrap())
            .enumerate()
        {
            assert_eq!(
                u64::from(value.to_bits()),
                expected.as_u64().unwrap(),
                "frame {index} line {line}"
            );
        }
    }
}
#[test]
fn main_prediction_video_config_and_payload_reach_prediction() {
    let m = manifest();
    let asc = config(&m);
    assert_eq!(
        fvid_media::owned_aac::config::AudioSpecificConfig::parse(&asc)
            .unwrap()
            .core
            .object_type,
        1
    );
    assert_eq!(
        fvid::codec::config::AudioSpecificConfig::parse(&asc)
            .unwrap()
            .core
            .object_type,
        1
    );
    let bytes = include_bytes!("fixtures/playback-errors/aac-main-prediction-synthetic.mp4");
    let blob = include_bytes!("fixtures/playback-errors/aac-main-prediction-packets.bin");
    let mut reader =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(bytes), Default::default())
            .unwrap();
    let audio = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let mut lc_asc = asc.clone();
    lc_asc[0] = (lc_asc[0] & 7) | (2 << 3);
    let mut lc_control = NativeAacDecoder::new(&lc_asc).unwrap();
    for (i, row) in m["case"]["frames"].as_array().unwrap().iter().enumerate() {
        let mut packet = Vec::new();
        reader.read_packet(audio, i, &mut packet).unwrap();
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(packet, &blob[start..start + len]);
        if i < 3 {
            let pcm = lc_control.decode(&packet).unwrap();
            assert_eq!(pcm.len(), 1024);
            assert!(pcm.iter().any(|v| *v != 0.0));
        } else if i == 3 {
            assert_eq!(
                lc_control.decode(&packet).unwrap_err().to_string(),
                "prediction is not allowed in AAC-LC"
            );
        }
    }
}
#[test]
fn main_prediction_video_has_native_playback_acceptance() {
    let m = manifest();
    let blob = include_bytes!("fixtures/playback-errors/aac-main-prediction-packets.bin");
    for case in m["cases"].as_array().unwrap() {
        let config_case = serde_json::json!({"case":case});
        let asc = config(&config_case);
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut root = fvid::codec::aac_native::NativeAacDecoder::new(&asc).unwrap();
        let mut pcm = Vec::new();
        for row in case["frames"].as_array().unwrap() {
            let start = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let packet = &blob[start..start + len];
            let saved = decoder.checkpoint();
            let output = decoder.decode(packet).unwrap();
            decoder.restore(&saved).unwrap();
            assert_eq!(decoder.decode(packet).unwrap(), output);
            assert_eq!(root.decode(packet).unwrap(), output);
            assert_eq!(output.len(), row["samples"].as_u64().unwrap() as usize);
            pcm.extend(output);
            let stable = decoder.checkpoint();
            assert!(decoder.decode(&packet[..packet.len() / 2]).is_err());
            let a = decoder.decode(packet).unwrap();
            decoder.restore(&stable).unwrap();
            assert_eq!(decoder.decode(packet).unwrap(), a);
            decoder.restore(&stable).unwrap();
        }
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let expected = std::fs::read(path.join(case["pcm_file"].as_str().unwrap())).unwrap();
        for (i, (a, b)) in pcm.iter().zip(expected.chunks_exact(4)).enumerate() {
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 2e-8,
                "{} sample {i}: {a} vs {b}",
                case["name"]
            );
        }
        assert_eq!(pcm.len() * 4, expected.len());
        let video = std::fs::read(path.join(case["video"]["file"].as_str().unwrap())).unwrap();
        let mut owned = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&video),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, case["samples"].as_u64().unwrap());
        let bytes: Vec<u8> = pcm.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(owned, bytes);
        let mut exported = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video, &mut exported).unwrap();
        assert_eq!(exported, bytes);
        decoder.reset();
        let row = &case["frames"][0];
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        let mut fresh = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(
            decoder.decode(&blob[start..start + len]).unwrap(),
            fresh.decode(&blob[start..start + len]).unwrap()
        );
    }
}
#[test]
fn main_prediction_invalid_reset_video_refuses_exactly_without_history_loss() {
    let m = manifest();
    let blob = include_bytes!("fixtures/playback-errors/aac-main-prediction-packets.bin");
    let valid = &m["cases"][0];
    let mut decoder = NativeAacDecoder::new(&config(&m)).unwrap();
    for row in valid["frames"].as_array().unwrap().iter().take(4) {
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        decoder.decode(&blob[start..start + len]).unwrap();
    }
    let saved = decoder.checkpoint();
    let next = &valid["frames"][4];
    let start = next["offset"].as_u64().unwrap() as usize;
    let len = next["bytes"].as_u64().unwrap() as usize;
    let expected = decoder.decode(&blob[start..start + len]).unwrap();
    decoder.restore(&saved).unwrap();
    for case in m["invalid"].as_array().unwrap() {
        let row = &case["frames"][0];
        let at = row["offset"].as_u64().unwrap() as usize;
        let size = row["bytes"].as_u64().unwrap() as usize;
        assert_eq!(
            decoder
                .decode(&blob[at..at + size])
                .unwrap_err()
                .to_string(),
            "invalid AAC Main predictor reset group"
        );
        assert_eq!(decoder.decode(&blob[start..start + len]).unwrap(), expected);
        decoder.restore(&saved).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let video = std::fs::read(path).unwrap();
        assert!(
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::io::Cursor::new(video),
                &mut Vec::new(),
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string()
            .contains("invalid AAC Main predictor reset group")
        );
    }
}

#[cfg(feature = "player")]
#[test]
fn main_prediction_playback_checkpoint_rewind_seek_and_eof_replay_exactly() {
    use fvid::audio::AudioStream;
    fn play(
        reader: &mut fvid::playback_mp4_audio::Mp4AudioReader<std::io::Cursor<&Vec<u8>>>,
    ) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            let saved = decoder.checkpoint().unwrap();
            let actual = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap();
            decoder.restore(&saved).unwrap();
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
        assert!(decoder.finish_packet().unwrap().is_none());
        output
    }
    for case in manifest()["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(path).unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut expected).unwrap();
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&bytes),
            Default::default(),
        )
        .unwrap();
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for position in [1100, 3000, 6100, case["samples"].as_u64().unwrap()] {
            let landed = reader.seek_to(position as i64);
            assert_eq!(
                play(&mut reader),
                expected[landed as usize * 4..],
                "{} seek {position}",
                case["name"]
            );
        }
    }
}

#[test]
fn main_ics_predictor_side_information_respects_limits_and_cursor_transactions() {
    use fvid_media::owned_aac::{aac_ics::IcsInfo, bits::BitReader};
    fn packed(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut data = Vec::new();
        let mut count = 0;
        for &(value, width) in fields {
            for shift in (0..width).rev() {
                if count % 8 == 0 {
                    data.push(0);
                }
                *data.last_mut().unwrap() |= (((value >> shift) & 1) as u8) << (7 - count % 8);
                count += 1;
            }
        }
        (data, count)
    }
    for group in 1..=30 {
        let (data, count) = packed(&[
            (0, 1),
            (0, 2),
            (0, 1),
            (5, 6),
            (1, 1),
            (1, 1),
            (group, 5),
            (5, 3),
        ]);
        let mut bits = BitReader::new(&data);
        let info = IcsInfo::read_profile(&mut bits, (49, 14), Some(3)).unwrap();
        assert_eq!(bits.position(), count);
        let prediction = info.prediction.unwrap();
        assert_eq!(prediction.reset_group, Some(group as u8));
        assert_eq!(prediction.used, vec![true, false, true]);
    }
    for group in [0, 31] {
        let (data, _) = packed(&[
            (0, 1),
            (0, 2),
            (0, 1),
            (5, 6),
            (1, 1),
            (1, 1),
            (group, 5),
            (5, 3),
        ]);
        let mut bits = BitReader::new(&data);
        assert_eq!(
            IcsInfo::read_profile(&mut bits, (49, 14), Some(3))
                .unwrap_err()
                .to_string(),
            "invalid AAC Main predictor reset group"
        );
        assert_eq!(bits.position(), 0);
    }
    let (data, _) = packed(&[(0, 1), (0, 2), (0, 1), (5, 6), (1, 1), (1, 1)]);
    let mut bits = BitReader::new(&data);
    assert!(IcsInfo::read_profile(&mut bits, (49, 14), Some(40)).is_err());
    assert_eq!(bits.position(), 0);
}
