use fvid::container::mp4::Mp4Reader;
use fvid_media::owned_aac::aac_native::NativeAacDecoder;
use serde_json::Value;
use std::io::Cursor;
fn cases() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ssr-coupling.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn original_ssr_coupling_videos_decode_nonzero_pcm_with_active_gain_and_window_transitions() {
    let packets = include_bytes!("fixtures/playback-errors/aac-ssr-coupling-packets.bin");
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-coupling-pcm.f32le");
    for case in cases()["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let bytes = std::fs::read(path).unwrap();
        let mut reader = Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let asc = hex(case["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut actual = Vec::new();
        let mut encoded = vec![];
        let off = case["pcm_offset"].as_u64().unwrap() as usize;
        for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            reader.read_packet(track, i, &mut encoded).unwrap();
            let start = frame["offset"].as_u64().unwrap() as usize;
            assert_eq!(
                encoded,
                &packets[start..start + frame["bytes"].as_u64().unwrap() as usize]
            );
            let checkpoint = decoder.checkpoint();
            let pcm = decoder.decode(&encoded).unwrap();
            assert_eq!(
                pcm.len(),
                frame["samples"].as_u64().unwrap() as usize
                    * case["channels"].as_u64().unwrap() as usize
            );
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(decoder.decode(&encoded).unwrap(), pcm);
            let base = off + frame["pcm_offset"].as_u64().unwrap() as usize;
            for (j, (sample, expected)) in pcm
                .iter()
                .zip(gold[base..base + pcm.len() * 4].chunks_exact(4))
                .enumerate()
            {
                let expected = f32::from_le_bytes(expected.try_into().unwrap());
                assert!(
                    (sample - expected).abs() < 2e-7,
                    "{} packet {i} sample {j}: {sample} vs {expected}",
                    case["video"]["file"]
                );
            }
            actual.extend(pcm);
        }
        assert!(actual.iter().any(|v| v.abs() > 1e-4));
        decoder.reset();
        let mut replay = vec![];
        for i in 0..case["frames"].as_array().unwrap().len() {
            reader.read_packet(track, i, &mut encoded).unwrap();
            replay.extend(decoder.decode(&encoded).unwrap());
        }
        assert_eq!(actual, replay);
        let mut exported = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut exported,
            None,
            &Default::default(),
        )
        .unwrap();
        let native: Vec<u8> = actual.iter().flat_map(|v| v.to_le_bytes()).collect();
        assert_eq!(exported, native);
        assert_eq!(stats.sample_frames, 6144);
        let mut root = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut root).unwrap();
        assert_eq!(root, native);
    }
}

#[test]
fn invalid_ssr_coupling_videos_refuse_exact_failure_and_preserves_all_history() {
    for case in cases()["invalid"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let mut reader = Mp4Reader::open(
            Cursor::new(std::fs::read(path).unwrap()),
            Default::default(),
        )
        .unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut decoder = NativeAacDecoder::new(&hex(case["asc"].as_str().unwrap())).unwrap();
        let mut first = vec![];
        reader.read_packet(track, 0, &mut first).unwrap();
        decoder.decode(&first).unwrap();
        let saved = decoder.checkpoint();
        let mut bad = vec![];
        reader.read_packet(track, 1, &mut bad).unwrap();
        assert_eq!(
            decoder.decode(&bad).unwrap_err().to_string(),
            case["error"].as_str().unwrap()
        );
        let after = decoder.decode(&first).unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(decoder.decode(&first).unwrap(), after);
    }
}

#[cfg(feature = "player")]
#[test]
fn ssr_coupling_video_playback_rewind_seek_and_ranges_keep_variable_window_timing() {
    use fvid::audio::AudioStream;
    fn play(s: &mut fvid::playback_mp4_audio::Mp4AudioReader<Cursor<&Vec<u8>>>) -> Vec<u8> {
        let mut decoder = s.make_decoder().unwrap();
        let mut out = vec![];
        while let Some(packet) = s.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(pcm) = s.present_decoded(frame.packet, frame.source_pts).unwrap() {
                    out.extend(pcm.data);
                }
            }
        }
        out
    }
    for case in cases()["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let data = std::fs::read(path).unwrap();
        let mut expected = vec![];
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
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        let landed = reader.seek_to(2600);
        let size = case["channels"].as_u64().unwrap() as usize * 4;
        assert_eq!(play(&mut reader), expected[landed as usize * size..]);
        for _ in 0..2 {
            let mut range = vec![];
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut range,
                Some((
                    std::time::Duration::from_millis(10),
                    std::time::Duration::from_millis(150),
                )),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(range, expected[240 * size..3600 * size]);
        }
    }
}
