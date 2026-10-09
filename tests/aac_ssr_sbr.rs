use std::path::Path;
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-ssr-sbr.json")).unwrap()
}
fn all_sbr(manifest: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"] != "core-control")
        .chain(manifest["downsampled"].as_array().unwrap().iter())
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
fn ssr_sbr_fixture_has_valid_silent_core_control() {
    let manifest = cases();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "core-control")
        .unwrap();
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
    assert_eq!(pcm.len(), 6144 * 4);
    assert!(pcm.iter().all(|b| *b == 0));
}
#[test]
fn ssr_sbr_transition_video_acceptance() {
    let manifest = cases();
    for case in all_sbr(&manifest) {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        check_pcm(&pcm, case);
    }
}

fn check_pcm(pcm: &[u8], case: &serde_json::Value) {
    let active = case["name"].as_str().unwrap().contains("active");
    let downsampled = case["container_rate"] == 24000;
    let reference: &[u8] = if downsampled && active {
        include_bytes!("fixtures/playback-errors/aac-ssr-sbr-downsampled-reference.f64le")
    } else if downsampled {
        include_bytes!("fixtures/playback-errors/aac-ssr-sbr-downsampled-silent-reference.f64le")
    } else if active {
        include_bytes!("fixtures/playback-errors/aac-ssr-sbr-active-reference.f64le")
    } else {
        include_bytes!("fixtures/playback-errors/aac-ssr-sbr-reference.f64le")
    };
    assert_eq!(pcm.len(), case["samples"].as_u64().unwrap() as usize * 4);
    assert_eq!(reference.len(), pcm.len() * 2);
    let mut maximum_error = 0.0f64;
    for (i, (sample, gold)) in pcm
        .chunks_exact(4)
        .zip(reference.chunks_exact(8))
        .enumerate()
    {
        let a = f32::from_le_bytes(sample.try_into().unwrap()) as f64;
        let b = f64::from_le_bytes(gold.try_into().unwrap());
        maximum_error = maximum_error.max((a - b).abs());
        assert!((a - b).abs() < 1e-9, "sample {i}: {a} vs {b}");
    }
    eprintln!(
        "{} maximum scalar difference: {maximum_error:e}",
        case["name"]
    );
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn ssr_sbr_pending_checkpoint_invalid_packet_reset_and_eof_keep_pcm() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let blob = include_bytes!("fixtures/playback-errors/aac-ssr-sbr-packets.bin");
    let manifest = cases();
    for case in all_sbr(&manifest) {
        let ticks = case["container_frame_samples"].as_u64().unwrap();
        let rate = case["container_rate"].as_u64().unwrap() as u32;
        let mut decoder =
            NativeAacDecoder::new_with_output_rate(&hex(case["asc"].as_str().unwrap()), rate)
                .unwrap();
        for replay in 0..2 {
            if replay != 0 {
                decoder.reset();
            }
            let mut pcm = Vec::new();
            for (i, p) in case["frames"].as_array().unwrap().iter().enumerate() {
                let at = p["offset"].as_u64().unwrap() as usize;
                let len = p["bytes"].as_u64().unwrap() as usize;
                let saved = decoder.checkpoint();
                assert!(
                    decoder
                        .decode_timed(&[], i as i64 * ticks as i64, ticks)
                        .is_err()
                );
                let first = decoder
                    .decode_timed(&blob[at..at + len], i as i64 * ticks as i64, ticks)
                    .unwrap();
                decoder.restore(&saved).unwrap();
                let again = decoder
                    .decode_timed(&blob[at..at + len], i as i64 * ticks as i64, ticks)
                    .unwrap();
                assert_eq!(first, again);
                if let Some(frame) = again {
                    assert_eq!(frame.pts, (pcm.len() / 4) as i64);
                    assert_eq!(frame.duration, ticks);
                    for x in frame.samples {
                        pcm.extend_from_slice(&x.to_le_bytes());
                    }
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
                assert_eq!(frame.pts, (pcm.len() / 4) as i64);
                for x in frame.samples {
                    pcm.extend_from_slice(&x.to_le_bytes());
                }
            }
            assert!(decoder.finish().unwrap().is_none());
            check_pcm(&pcm, case);
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn ssr_sbr_player_rewind_seek_eof_and_interval_match_scalar_pcm() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut pcm = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
            {
                if let Some(f) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    pcm.extend(f.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(f) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(f.data);
            }
        }
        assert!(decoder.finish_packet().unwrap().is_none());
        pcm
    }
    let manifest = cases();
    for c in all_sbr(&manifest) {
        let data = video(c);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        let rate = c["container_rate"].as_u64().unwrap() as usize;
        assert_eq!((reader.sample_rate(), reader.channels()), (rate as u32, 1));
        let expected = play(&mut reader);
        check_pcm(&expected, c);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        for target in [1100, 6500, c["samples"].as_i64().unwrap()] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), expected[landed as usize * 4..]);
        }
        let mut interval = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut interval,
            Some((Duration::from_millis(20), Duration::from_millis(200))),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(interval, expected[rate / 50 * 4..rate / 5 * 4]);
    }
}

#[test]
fn active_ssr_core_control_matches_independent_ipqf_before_sbr() {
    let c = cases();
    let mut pcm = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(&video(&c["active_core_control"]), &mut pcm).unwrap();
    let gold = include_bytes!("fixtures/playback-errors/aac-ssr-sbr-active-core-reference.f32le");
    assert_eq!(pcm.len(), gold.len());
    let mut maximum_error = 0.0f32;
    for (i, (a, b)) in pcm.chunks_exact(4).zip(gold.chunks_exact(4)).enumerate() {
        let delta = (f32::from_le_bytes(a.try_into().unwrap())
            - f32::from_le_bytes(b.try_into().unwrap()))
        .abs();
        maximum_error = maximum_error.max(delta);
        assert!(delta < 2e-7, "core sample {i}: {delta}");
    }
    eprintln!("SSR core maximum scalar difference: {maximum_error:e}");
}

#[test]
fn explicit_and_sync_ssr_sbr_downsampled_target_matches_independent_qmf() {
    for case in cases()["downsampled"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        let reference: &[u8] = if case["name"].as_str().unwrap().contains("active") {
            include_bytes!("fixtures/playback-errors/aac-ssr-sbr-downsampled-reference.f64le")
        } else {
            include_bytes!(
                "fixtures/playback-errors/aac-ssr-sbr-downsampled-silent-reference.f64le"
            )
        };
        assert_eq!(pcm.len(), 6144 * 4);
        assert_eq!(pcm.len() * 2, reference.len());
        for (i, (sample, gold)) in pcm
            .chunks_exact(4)
            .zip(reference.chunks_exact(8))
            .enumerate()
        {
            let sample = f32::from_le_bytes(sample.try_into().unwrap()) as f64;
            let gold = f64::from_le_bytes(gold.try_into().unwrap());
            assert!(
                (sample - gold).abs() < 1e-9,
                "{} sample {i}: {sample} vs {gold}",
                case["name"]
            );
        }
    }
}
