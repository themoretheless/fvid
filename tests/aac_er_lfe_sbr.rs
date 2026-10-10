use fvid_media::owned_aac::NativeAacDecoder;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn nonzero_er_lfe_sbr_matches_direct_qmf_delay_mapping_and_atomic_replay() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("aac-er-lfe-sbr.json")).unwrap();
    let blob = bytes("aac-er-lfe-sbr-packets.bin");
    let gold = bytes("aac-er-lfe-sbr-reference.f64le");
    assert_eq!(m["cases"].as_array().unwrap().len(), 24);
    for peak in m["delay_mutant_peak"].as_object().unwrap().values() {
        assert!(peak.as_f64().unwrap() > 1e-5);
    }
    let packet = |r: &serde_json::Value| {
        let at = r["offset"].as_u64().unwrap() as usize;
        &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
    };
    for c in m["cases"].as_array().unwrap() {
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut d =
            NativeAacDecoder::new_with_output_rate(&hex(c["asc"].as_str().unwrap()), rate).unwrap();
        let mut control = NativeAacDecoder::new(&hex(c["control_asc"].as_str().unwrap())).unwrap();
        assert_eq!(d.channel_mask(), c["channel_mask"].as_u64().unwrap() as u32);
        let mut actual = vec![];
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let saved = d.checkpoint();
            let error = d.decode(packet(&c["bad_frames"][i])).unwrap_err();
            assert!(
                error.to_string().contains("excess ER AAC SBR element"),
                "{}: {error}",
                c["name"]
            );
            let output = d.decode(packet(row)).unwrap();
            assert_eq!(
                output,
                control.decode(packet(&c["control_frames"][i])).unwrap(),
                "{}",
                c["name"]
            );
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(packet(row)).unwrap());
            actual.extend(output);
        }
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        let size = c["reference_bytes"].as_u64().unwrap() as usize;
        assert_eq!(actual.len() * 8, size);
        for (i, (v, raw)) in actual
            .iter()
            .zip(gold[at..at + size].chunks_exact(8))
            .enumerate()
        {
            let expected = f64::from_le_bytes(raw.try_into().unwrap());
            assert!(
                (f64::from(*v) - expected).abs() < 1e-9,
                "{} sample {i}: {v} vs {expected}",
                c["name"]
            );
        }
        let channels = c["channels"].as_u64().unwrap() as usize;
        let target = c["lfe_target"].as_u64().unwrap() as usize;
        assert!(
            actual
                .chunks_exact(channels)
                .map(|r| r[target].abs())
                .fold(0.0f32, f32::max)
                > 1e-5
        );
        d.reset();
        let replay: Vec<_> = c["frames"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|r| d.decode(packet(r)).unwrap())
            .collect();
        assert_eq!(actual, replay);
    }
}
#[test]
fn nonzero_er_lfe_sbr_public_owned_export_and_excess_extension_refusal() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("aac-er-lfe-sbr.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let source = bytes(c["video"]["file"].as_str().unwrap());
        let mut public = vec![];
        let mut owned = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&source, &mut public).unwrap();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&source),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(public, owned);
        assert_eq!(stats.sample_rate, c["rate"].as_u64().unwrap() as u32);
        assert_eq!(
            public.len(),
            c["samples"].as_u64().unwrap() as usize * c["channels"].as_u64().unwrap() as usize * 4
        );
        let mut rejected = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["bad_video"]["file"].as_str().unwrap()),
            &mut rejected,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("excess ER AAC SBR element"),
            "{}: {error}",
            c["name"]
        );
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
fn er_lfe_sbr_player_ranges_rewind_seek_preserve_output_clock() {
    use fvid::audio::AudioStream;
    for c in
        serde_json::from_slice::<serde_json::Value>(&bytes("aac-er-lfe-sbr.json")).unwrap()["cases"]
            .as_array()
            .unwrap()
    {
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
                full[from as usize
                    * (c["container_rate"].as_u64().unwrap() as usize / 1000)
                    * stride
                    ..to as usize
                        * (c["container_rate"].as_u64().unwrap() as usize / 1000)
                        * stride]
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
