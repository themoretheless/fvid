use std::path::Path;
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ssr-sbr-late.json"
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
fn late_sbr_cce_has_valid_silent_ssr_core_control() {
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(&manifest()["cases"][0]), &mut pcm).unwrap();
    assert_eq!(pcm.len(), 6144 * 4);
    assert!(pcm.iter().all(|b| *b == 0));
}
#[test]
fn late_sbr_cce_pcm_acceptance() {
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(&manifest()["cases"][1]), &mut pcm).unwrap();
    assert_eq!(pcm.len(), 6144 * 4);
    assert!(pcm[..3072 * 4].iter().all(|b| *b == 0));
    let gold =
        include_bytes!("fixtures/playback-errors/aac-ssr-sbr-downsampled-silent-reference.f64le");
    for (i, (sample, gold)) in pcm[3072 * 4..]
        .chunks_exact(4)
        .zip(gold.chunks_exact(8))
        .enumerate()
    {
        let sample = f32::from_le_bytes(sample.try_into().unwrap()) as f64;
        let gold = f64::from_le_bytes(gold.try_into().unwrap());
        assert!(
            (sample - gold).abs() < 1e-9,
            "sample {i}: {sample} vs {gold}"
        );
    }
}

#[test]
fn late_sbr_transition_checkpoint_replay_and_reset_keep_pending_core_and_sbr() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    let case = &m["cases"][1];
    let asc: Vec<u8> = case["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect();
    let blob = include_bytes!("fixtures/playback-errors/aac-ssr-sbr-late-packets.bin");
    let mut decoder = NativeAacDecoder::new_with_output_rate(&asc, 24000).unwrap();
    let mut expected = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut expected).unwrap();
    for replay in 0..2 {
        if replay > 0 {
            decoder.reset();
        }
        let mut pcm = Vec::new();
        for (i, row) in case["frames"].as_array().unwrap().iter().enumerate() {
            let at = row["offset"].as_u64().unwrap() as usize;
            let end = at + row["bytes"].as_u64().unwrap() as usize;
            let saved = decoder.checkpoint();
            assert!(decoder.decode_timed(&[], i as i64 * 1024, 1024).is_err());
            let first = decoder
                .decode_timed(&blob[at..end], i as i64 * 1024, 1024)
                .unwrap();
            decoder.restore(&saved).unwrap();
            let again = decoder
                .decode_timed(&blob[at..end], i as i64 * 1024, 1024)
                .unwrap();
            assert_eq!(first, again);
            if let Some(frame) = again {
                assert_eq!(frame.pts, (pcm.len() / 4) as i64);
                pcm.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
            }
        }
        let saved = decoder.checkpoint();
        let tail = decoder.finish().unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(decoder.finish().unwrap(), tail);
        if let Some(frame) = tail {
            pcm.extend(frame.samples.iter().flat_map(|s| s.to_le_bytes()));
        }
        assert!(decoder.finish().unwrap().is_none());
        assert_eq!(pcm, expected);
    }
}

#[cfg(feature = "player")]
#[test]
fn late_sbr_player_seek_on_both_sides_of_transition_preserves_pcm() {
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
        pcm
    }
    let data = video(&manifest()["cases"][1]);
    let mut expected = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&data, &mut expected).unwrap();
    let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
        std::io::Cursor::new(&data),
        Default::default(),
    )
    .unwrap();
    assert_eq!(reader.sample_rate(), 24000);
    assert_eq!(play(&mut reader), expected);
    reader.rewind();
    assert_eq!(play(&mut reader), expected);
    for target in [1100, 3072, 4300, 6144] {
        let landed = reader.seek_to(target);
        assert_eq!(play(&mut reader), expected[landed as usize * 4..]);
    }
}
