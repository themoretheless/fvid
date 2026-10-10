use serde_json::Value;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn er_active_960_ltp_sbr_explicit_sync_and_implicit_mp4_clocks_match_scalar_pcm() {
    let m: Value = serde_json::from_slice(&bytes("aac-er-active-sbr-960.json")).unwrap();
    for case in m["cases"].as_array().unwrap() {
        let rate = case["rate"].as_u64().unwrap();
        let gold = bytes(&format!("aac-er-active-sbr-960-{rate}-reference.f64le"));
        let control = bytes(&format!(
            "aac-er-active-sbr-960-{rate}-inactive-control.f64le"
        ));
        let source = bytes(case["video"]["file"].as_str().unwrap());
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&source, &mut pcm).unwrap();
        assert_eq!(pcm.len() * 2, gold.len());
        let mut prediction_peak = 0f64;
        for (i, ((actual, reference), inactive)) in pcm
            .chunks_exact(4)
            .zip(gold.chunks_exact(8))
            .zip(control.chunks_exact(8))
            .enumerate()
        {
            let actual = f32::from_le_bytes(actual.try_into().unwrap()) as f64;
            let reference = f64::from_le_bytes(reference.try_into().unwrap());
            let inactive = f64::from_le_bytes(inactive.try_into().unwrap());
            assert!(
                (actual - reference).abs() < 1e-9,
                "rate={rate} signal={} sample={i}: {actual} vs {reference}",
                case["signal"]
            );
            prediction_peak = prediction_peak.max((actual - inactive).abs());
        }
        assert!(
            prediction_peak > 1e-5,
            "fixture must exercise LTP through SBR: {prediction_peak}"
        );
        let mut owned = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&source),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_rate, rate as u32);
        assert_eq!(owned, pcm);
        for (from, to) in [(130, 200), (10, 60), (130, 200)] {
            let mut ranged = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&source),
                &mut ranged,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                ranged,
                &pcm[(from * rate / 1000 * 4) as usize..(to * rate / 1000 * 4) as usize]
            );
        }
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
fn er_active_960_sbr_player_ranges_rewind_seek_preserve_output_clock() {
    use fvid::audio::AudioStream;
    for c in serde_json::from_slice::<serde_json::Value>(&bytes("aac-er-active-sbr-960.json"))
        .unwrap()["cases"]
        .as_array()
        .unwrap()
    {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = 4;
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

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn er_active_960_sbr_native_discovery_clock_checkpoint_and_crc_failure_are_atomic() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m: Value = serde_json::from_slice(&bytes("aac-er-active-sbr-960.json")).unwrap();
    let blob = bytes("aac-er-active-sbr-960-packets.bin");
    let packet = |r: &Value| {
        let at = r["offset"].as_u64().unwrap() as usize;
        &blob[at..at + r["bytes"].as_u64().unwrap() as usize]
    };
    for c in m["cases"].as_array().unwrap() {
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut d =
            NativeAacDecoder::new_with_output_rate(&hex(c["asc"].as_str().unwrap()), rate).unwrap();
        let mut pcm = vec![];
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let saved = d.checkpoint();
            let error = d.decode(packet(&c["bad_frames"][i])).unwrap_err();
            assert!(
                error.to_string().contains("ER AAC SBR CRC is forbidden"),
                "{error}"
            );
            assert_eq!(d.sample_rate(), rate);
            let output = d.decode(packet(row)).unwrap();
            assert_eq!(d.sample_rate(), rate);
            d.restore(&saved).unwrap();
            assert_eq!(output, d.decode(packet(row)).unwrap());
            pcm.extend(output);
        }
        let gold = bytes(&format!("aac-er-active-sbr-960-{rate}-reference.f64le"));
        assert_eq!(pcm.len() * 8, gold.len());
        for (v, b) in pcm.iter().zip(gold.chunks_exact(8)) {
            let x = f64::from_le_bytes(b.try_into().unwrap());
            assert!(
                (f64::from(*v) - x).abs() < 1e-9,
                "{}: {v} vs {x}",
                c["name"]
            );
        }
        if c["signal"] == "implicit" && rate == 48000 {
            let mut discovered =
                NativeAacDecoder::new_with_sbr_detection(&hex(c["asc"].as_str().unwrap())).unwrap();
            assert_eq!(discovered.sample_rate(), 24000);
            let saved = discovered.checkpoint();
            assert!(discovered.decode(packet(&c["bad_frames"][0])).is_err());
            assert_eq!(discovered.sample_rate(), 24000);
            let first = discovered.decode(packet(&c["frames"][0])).unwrap();
            assert_eq!(discovered.sample_rate(), 48000);
            discovered.restore(&saved).unwrap();
            assert_eq!(discovered.sample_rate(), 24000);
            assert_eq!(discovered.decode(packet(&c["frames"][0])).unwrap(), first);
            let mut output = first;
            for row in &c["frames"].as_array().unwrap()[1..] {
                output.extend(discovered.decode(packet(row)).unwrap());
            }
            assert_eq!(output, pcm);
        }
        let mut output = vec![];
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["bad_video"]["file"].as_str().unwrap()),
            &mut output,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("ER AAC SBR CRC is forbidden"),
            "{error}"
        );
    }
}
