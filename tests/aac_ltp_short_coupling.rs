use fvid_media::owned_aac::NativeAacDecoder;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-ltp-short-coupling.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn native(c: &serde_json::Value, key: &str, blob: &[u8]) -> Vec<f32> {
    let mut decoder = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
    let mut out = vec![];
    for row in c[key].as_array().unwrap() {
        let at = row["offset"].as_u64().unwrap() as usize;
        let size = row["bytes"].as_u64().unwrap() as usize;
        let saved = decoder.checkpoint();
        let mut bad = blob[at..at + size].to_vec();
        bad.push(0);
        assert!(decoder.decode(&bad).is_err());
        let pcm = decoder.decode(&blob[at..at + size]).unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(pcm, decoder.decode(&blob[at..at + size]).unwrap());
        out.extend(pcm);
    }
    out
}
fn reference(c: &serde_json::Value, raw: &[u8]) -> Vec<f32> {
    let start = c["reference_offset"].as_u64().unwrap() as usize;
    let size = c["reference_bytes"].as_u64().unwrap() as usize;
    raw[start..start + size]
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
        .collect()
}
#[test]
fn short_coupling_preserves_source_ltp_noise_tns_and_pcm_histories() {
    let blob = bytes("aac-ltp-short-coupling-packets.bin");
    let gold = bytes("aac-ltp-short-coupling-reference.f32le");
    let wrong = bytes("aac-ltp-short-coupling-stale-short.f32le");
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 4);
    let roster: std::collections::BTreeSet<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["n"].as_u64().unwrap(), c["point"].as_u64().unwrap()))
        .collect();
    assert_eq!(
        roster,
        [(960, 0), (960, 1), (1024, 0), (1024, 1)]
            .into_iter()
            .collect()
    );
    for c in m["cases"].as_array().unwrap() {
        let sequences: std::collections::BTreeSet<_> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["sequence"].as_u64().unwrap())
            .collect();
        assert_eq!(sequences, (0..=3).collect());
    }
    for c in m["cases"].as_array().unwrap() {
        let actual = native(c, "frames", &blob);
        let control = native(c, "control_frames", &blob);
        let correct = reference(c, &gold);
        assert_eq!(actual.len(), correct.len(), "{} PCM geometry", c["name"]);
        if let Some(selection @ (1 | 2)) = c["selection"].as_u64() {
            let silent = if selection == 1 { 0 } else { 1 };
            assert!(
                actual.chunks_exact(2).all(|pair| pair[silent] == 0.),
                "{} unselected channel is audible",
                c["name"]
            );
        }
        let mutant = reference(c, &wrong);
        assert!(
            correct
                .iter()
                .zip(&mutant)
                .any(|(a, b)| (*a - *b).abs() > 1e-6),
            "{} not a short-history reproducer",
            c["name"]
        );
        for (a, b) in control.iter().zip(&correct) {
            assert!(
                (*a - *b).abs() < 1e-8,
                "{} invalid control: {a} vs {b}",
                c["name"]
            );
        }
        assert_eq!(
            actual, control,
            "{} overlapping flags changed PNS PCM",
            c["name"]
        );
        for (a, b) in actual.iter().zip(&correct) {
            assert!(
                (*a - *b).abs() < 1e-8,
                "{} short-history PCM failure: {a} vs {b}",
                c["name"]
            );
        }
    }
}

#[test]
fn short_coupling_ltp_public_videos_match_controls_and_owned_export() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let control = bytes(c["control_video"]["file"].as_str().unwrap());
        let mut out = vec![];
        let mut expected = vec![];
        let mut owned = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut expected).unwrap();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, expected, "{}", c["name"]);
        assert_eq!(out, owned);
    }
}
#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = vec![];
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
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
#[cfg(feature = "player")]
#[test]
fn short_coupling_ltp_player_ranges_rewind_seek_restore_noise_and_predictor_state() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(10, 80), (1, 20), (10, 80)] {
            let mut range = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &data,
                &mut range,
                Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                )),
            )
            .unwrap();
            assert_eq!(
                range,
                full[from as usize * 24 * stride..to as usize * 24 * stride]
            );
        }
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            std::io::Cursor::new(&data),
            Default::default(),
        )
        .unwrap();
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [13, 1400, 3500, (full.len() / stride) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(
                play(&mut reader),
                full[landed as usize * stride..],
                "{}",
                c["name"]
            );
        }
    }
}
