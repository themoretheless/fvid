use std::path::Path;
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ssr-cce-absence.json"
    ))
    .unwrap()
}
fn video(case: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap()),
    )
    .unwrap()
}
#[test]
fn absent_cce_core_preserves_queued_pcm_without_repeating_or_dropping_samples() {
    let m = cases();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let mut count = 0;
    for case in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["core"] == true)
    {
        let reference = std::fs::read(root.join(case["reference"].as_str().unwrap())).unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e:?}", case["name"]));
        assert_eq!(pcm.len(), 6144 * 4);
        assert_eq!(pcm, reference, "{}", case["name"]);
        assert!(
            pcm.chunks_exact(4)
                .any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() > 1e-8)
        );
        count += 1;
    }
    assert_eq!(count, 4);
}
#[test]
fn absent_cce_sbr_preserves_scalar_qmf_and_original_chunk_gain_boundaries() {
    let m = cases();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let mut count = 0;
    for case in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["core"] == false)
    {
        let gold = std::fs::read(root.join(case["reference"].as_str().unwrap())).unwrap();
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm)
            .unwrap_or_else(|e| panic!("{}: {e:?}", case["name"]));
        assert_eq!(pcm.len(), case["samples"].as_u64().unwrap() as usize * 4);
        assert_eq!(gold.len(), pcm.len() * 2);
        for (i, (a, b)) in pcm.chunks_exact(4).zip(gold.chunks_exact(8)).enumerate() {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 1e-9,
                "{} sample {i}: {a} vs {b}",
                case["name"]
            );
        }
        count += 1;
    }
    assert_eq!(count, 8);
}
#[test]
fn absent_cce_checkpoint_invalid_packet_reset_and_eof_keep_sources() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let manifest = cases();
    for case in manifest["cases"].as_array().unwrap().iter() {
        let ticks = case["container_frame_samples"].as_u64().unwrap();
        let asc: Vec<_> = case["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
            .collect();
        let blob = include_bytes!("fixtures/playback-errors/aac-ssr-cce-absence-packets.bin");
        let mut decoder = NativeAacDecoder::new_with_output_rate(
            &asc,
            case["container_rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let mut reference = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut reference).unwrap();
        for replay in 0..2 {
            if replay != 0 {
                decoder.reset();
            }
            let mut pcm = Vec::new();
            for (i, row) in case["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let end = at + row["bytes"].as_u64().unwrap() as usize;
                let saved = decoder.checkpoint();
                assert!(
                    decoder
                        .decode_timed(&[], i as i64 * ticks as i64, ticks)
                        .is_err()
                );
                let first = decoder
                    .decode_timed(&blob[at..end], i as i64 * ticks as i64, ticks)
                    .unwrap();
                decoder.restore(&saved).unwrap();
                let again = decoder
                    .decode_timed(&blob[at..end], i as i64 * ticks as i64, ticks)
                    .unwrap();
                assert_eq!(first, again);
                if let Some(frame) = again {
                    assert_eq!(
                        frame.pts,
                        (pcm.len() / (4 * case["channels"].as_u64().unwrap() as usize)) as i64
                    );
                    assert_eq!(frame.duration, ticks);
                    pcm.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
                }
                assert!(
                    decoder
                        .retained_payload_bytes_with_checkpoint(Some(&saved))
                        .unwrap()
                        > 0
                );
            }
            let saved = decoder.checkpoint();
            let last = decoder.finish().unwrap();
            decoder.restore(&saved).unwrap();
            assert_eq!(decoder.finish().unwrap(), last);
            if let Some(frame) = last {
                assert_eq!(
                    frame.pts,
                    (pcm.len() / (4 * case["channels"].as_u64().unwrap() as usize)) as i64
                );
                pcm.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
            }
            assert!(decoder.finish().unwrap().is_none());
            assert_eq!(pcm, reference);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn absent_cce_player_rewind_seek_and_eof_keep_pcm() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(frame) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    pcm.extend(frame.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(frame) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(frame.data);
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        pcm
    }
    let manifest = cases();
    for case in manifest["cases"].as_array().unwrap().iter() {
        let data = video(case);
        let stride = 4 * case["channels"].as_u64().unwrap() as usize;
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut expected).unwrap();
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&data),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            reader.sample_rate(),
            case["container_rate"].as_u64().unwrap() as u32
        );
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 4300, case["samples"].as_i64().unwrap()] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * stride..]);
        }
    }
}
