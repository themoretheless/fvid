#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
fn read_bits(data: &[u8], start: usize, count: usize) -> u8 {
    (start..start + count).fold(0, |v, i| (v << 1) | ((data[i / 8] >> (7 - i % 8)) & 1))
}
#[test]
fn alac_multi_element_channels_have_exact_owned_pcm_and_seek() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel");
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in info["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let depth = c["depth"].as_u64().unwrap();
        let total = c["samples"].as_u64().unwrap_or(32) as usize;
        if c["mode"].as_str().unwrap().starts_with("adaptive-") && c["mode"] != "adaptive-1" {
            assert!(
                c["rice"]["adaptive_positions"]
                    .as_u64()
                    .unwrap()
                    .count_ones()
                    >= c["predictor_order"].as_u64().unwrap().min(2) as u32,
                "{name}: expected coefficient positions must change"
            );
            assert!(
                c["rice"]["adaptive_positive"].as_u64().unwrap_or(0) > 0,
                "{name}: positive residual branch"
            );
            if channels >= 3 {
                assert!(
                    c["rice"]["adaptive_negative"].as_u64().unwrap_or(0) > 0,
                    "{name}: negative residual branch"
                );
            }
        }
        let order = c["order"].as_array().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..2 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut targets = vec![0, 208333, 208334];
        if c["mode"].as_str().unwrap().contains("-q0") {
            let boundary = (c["frame_samples"].as_u64().unwrap() * 1_000_000_000 / 48000) as i64;
            targets.extend([boundary, boundary + 1]);
        }
        for target in targets {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                reader.speaker_mask().unwrap(),
                Some(c["mask"].as_u64().unwrap() as u32)
            );
            let router = reader.stereo_router().unwrap().unwrap();
            reader.seek_to(target);
            let mut decoder = reader.make_decoder().unwrap();
            let mut actual = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                if c["mode"] != "raw" {
                    assert_eq!(
                        read_bits(
                            &p.data,
                            c["prefix_bits"].as_u64().unwrap_or(0) as usize + 22,
                            1
                        ),
                        0,
                        "compressed element must not use raw escape flag"
                    );
                    let prefix = c["prefix_bits"].as_u64().unwrap_or(0) as usize;
                    let prediction = read_bits(&p.data, prefix + 71, 4);
                    assert_eq!(
                        read_bits(&p.data, prefix + 75, 4),
                        if c["mode"].as_str().unwrap().starts_with("zero-quant-")
                            || c["mode"].as_str().unwrap().contains("-q0")
                        {
                            0
                        } else {
                            1
                        },
                        "{name}: coded quantization"
                    );
                    assert_eq!(
                        read_bits(&p.data, prefix + 82, 5),
                        c["predictor_order"].as_u64().unwrap_or(0) as u8
                    );
                    assert_eq!(
                        prediction,
                        if c["mode"] == "first-order"
                            || c["mode"] == "zero-run"
                            || c["mode"] == "zero-run-resume"
                            || c["mode"] == "zero-quant-twice"
                            || c["mode"] == "zero-quant-double"
                            || c["mode"].as_str().unwrap().ends_with("-twice")
                        {
                            15
                        } else {
                            0
                        }
                    );
                }

                let f = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    if actual.is_empty() {
                        let first = (target as u128 * 48000).div_ceil(1_000_000_000);
                        assert_eq!(
                            out.presentation_time().as_nanos(),
                            first * 1_000_000_000 / 48000,
                            "{name}/{target}: first sample timestamp"
                        );
                    }
                    actual.extend(out.data);
                }
            }
            let first = (target as u128 * 48000).div_ceil(1_000_000_000) as usize;
            let mut expected = Vec::new();
            for n in first..total {
                for ch in order {
                    let ch = ch.as_u64().unwrap() as usize;
                    let v = ((ch + 1) * 101 + n * 7) as i32 * if ch % 2 == 0 { 1 } else { -1 };
                    let v = (v + if c["jump_sample"]
                        .as_u64()
                        .is_some_and(|start| n as u64 >= start)
                    {
                        c["jump_high"].as_i64().unwrap() as i32
                    } else {
                        0
                    }) * c["scale"].as_i64().unwrap_or(1) as i32
                        + if c["low_bias"].as_bool().unwrap_or(false) {
                            ch as i32 + 1
                        } else {
                            0
                        };
                    expected.extend((v as f32 / (1u64 << (depth - 1)) as f32).to_le_bytes());
                }
            }
            assert_eq!(actual.len(), (total - first) * channels * 4);
            assert_eq!(actual, expected, "{name}/{target}");
            if target == 0 {
                let stereo = router.route(&actual, 1 << 20).unwrap();
                assert!(stereo.iter().any(|v| *v != 0.), "{name} silent preview");
                if c["mask"] == 0 {
                    let expected: Vec<_> = actual
                        .chunks_exact(channels * 4)
                        .flat_map(|row| {
                            let mut average = 0.;
                            for v in row.chunks_exact(4) {
                                average += f32::from_le_bytes(v.try_into().unwrap())
                                    * (1. / channels as f32);
                            }
                            [average, average]
                        })
                        .collect();
                    assert_eq!(stereo, expected, "{name} discrete preview");
                }
            }
        }
    }
}

#[test]
fn mp4_alac_cookies_preserve_multichannel_order_and_speaker_identity() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel");
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in info["cases"].as_array().unwrap() {
        let source = root.join(c["file"].as_str().unwrap()).with_extension("m4a");
        let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(std::fs::read(source).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(
            reader.speaker_mask().unwrap(),
            Some(c["mask"].as_u64().unwrap() as u32)
        );
        let mut decoder = reader.make_decoder().unwrap();
        let mut actual = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            let f = decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                actual.extend(out.data);
            }
        }
        let depth = c["depth"].as_u64().unwrap();
        let total = c["samples"].as_u64().unwrap_or(32) as usize;
        let mut expected = Vec::new();
        for n in 0..total {
            for ch in c["order"].as_array().unwrap() {
                let ch = ch.as_u64().unwrap() as usize;
                let v = ((ch + 1) * 101 + n * 7) as i32 * if ch % 2 == 0 { 1 } else { -1 };
                let v = (v + if c["jump_sample"]
                    .as_u64()
                    .is_some_and(|start| n as u64 >= start)
                {
                    c["jump_high"].as_i64().unwrap() as i32
                } else {
                    0
                }) * c["scale"].as_i64().unwrap_or(1) as i32
                    + if c["low_bias"].as_bool().unwrap_or(false) {
                        ch as i32 + 1
                    } else {
                        0
                    };
                expected.extend((v as f32 / (1u64 << (depth - 1)) as f32).to_le_bytes());
            }
        }
        assert_eq!(actual, expected, "{}", c["file"]);
    }
}

#[test]
fn malformed_alac_metadata_refuses_the_specific_error() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel");
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for c in info["invalid"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(c["file"].as_str().unwrap())).unwrap();
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        for _ in 0..2 {
            assert!(video.read_frame_raw().unwrap().is_some());
        }
        let error = match fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        ) {
            Ok(_) => panic!("{}: invalid metadata accepted", c["file"]),
            Err(e) => e.to_string(),
        };
        assert!(error.contains(c["error"].as_str().unwrap()), "{error}");
    }
}

#[test]
fn discrete_alac_preview_must_not_silence_all_channels() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel/3-16-discrete.mkv");
    let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(std::fs::read(root).unwrap()),
        Default::default(),
    )
    .unwrap();
    let router = reader.stereo_router().unwrap().unwrap();
    let mut decoder = reader.make_decoder().unwrap();
    let p = reader.next_packet().unwrap().unwrap();
    let f = decoder
        .decode_packet(&p.data, p.pts, p.duration as u64)
        .unwrap()
        .unwrap();
    let stereo = router.route(&f.packet.data, 1 << 20).unwrap();
    let expected: Vec<f32> = f
        .packet
        .data
        .chunks_exact(12)
        .flat_map(|row| {
            let mut average = 0.;
            for v in row.chunks_exact(4) {
                average += f32::from_le_bytes(v.try_into().unwrap()) * (1. / 3.);
            }
            [average, average]
        })
        .collect();
    assert_eq!(stereo, expected);
    assert!(
        stereo.iter().any(|v| *v != 0.),
        "nonzero discrete ALAC channels became complete silence"
    );
}

#[test]
fn truncated_ancillary_elements_refuse_in_full_decode_and_metadata_scan() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/alac-multichannel");
    let cookie = std::fs::read(root.join("stereo32.cookie")).unwrap();
    for byte in [0xc6, 0xde, 0x81] {
        let data = [byte];
        let error =
            fvid::codec::alac_decoder::AlacDecoder::packet_samples(&cookie, 48000, 2, &data)
                .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("ALAC ancillary element exceeds packet")
        );
        let mut decoder = fvid::codec::alac_decoder::AlacDecoder::new(&cookie, 48000, 2).unwrap();
        let error = decoder.decode_pcm(&data).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("ALAC ancillary element exceeds packet")
        );
    }
}
