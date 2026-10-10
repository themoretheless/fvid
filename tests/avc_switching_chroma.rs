use fvid::codec::avc_transform::primary_sp_chroma_420;
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-switching-chroma.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn primary_sp_chroma_matches_matrix_and_hadamard_oracle() {
    for c in manifest()["cases"].as_array().unwrap() {
        let p = std::array::from_fn(|i| c["prediction"][i].as_u64().unwrap() as u16);
        let dc = std::array::from_fn(|i| c["dc"][i].as_i64().unwrap() as i32);
        let ac = std::array::from_fn(|b| {
            std::array::from_fn(|i| c["ac"][b][i].as_i64().unwrap() as i32)
        });
        let expected: [u16; 64] =
            std::array::from_fn(|i| c["expected"][i].as_u64().unwrap() as u16);
        assert_eq!(
            primary_sp_chroma_420(
                &p,
                &dc,
                &ac,
                c["qp"].as_u64().unwrap() as u8,
                c["qs"].as_u64().unwrap() as u8
            )
            .unwrap(),
            expected,
            "{c}"
        );
    }
}
#[test]
fn primary_sp_chroma_rejects_invalid_component_inputs() {
    let p = [128; 64];
    let dc = [0; 4];
    let mut ac = [[0; 16]; 4];
    assert!(primary_sp_chroma_420(&p, &dc, &ac, 40, 0).is_err());
    assert!(primary_sp_chroma_420(&p, &dc, &ac, 0, 40).is_err());
    assert!(primary_sp_chroma_420(&[256; 64], &dc, &ac, 0, 0).is_err());
    assert!(primary_sp_chroma_420(&p, &[i32::MAX; 4], &ac, 0, 0).is_err());
    ac[3][0] = 1;
    assert!(primary_sp_chroma_420(&p, &dc, &ac, 0, 0).is_err());
}
#[test]
fn complete_primary_sp_kernel_sequence_matches_saved_scalar_yuv() {
    use fvid::codec::avc_transform::switching_luma_4x4;
    for coded in [false, true] {
        let mut pixels: Vec<u16> = (0..384).map(|i| ((i * 13) % 256) as u16).collect();
        let mut output: Vec<u8> = pixels.iter().map(|v| *v as u8).collect();
        for qs in [0, 26, 51] {
            let mut next = vec![0; 384];
            for block in 0..16 {
                let x = block % 4 * 4;
                let y = block / 4 * 4;
                let p = std::array::from_fn(|i| pixels[(y + i / 4) * 16 + x + i % 4]);
                let mut levels = [0; 16];
                if coded {
                    levels[0] = if block % 2 == 0 { 1 } else { -1 };
                }
                let samples = switching_luma_4x4(&p, &levels, 26, qs, false).unwrap();
                for i in 0..16 {
                    next[(y + i / 4) * 16 + x + i % 4] = samples[i];
                }
            }
            for (component, offset) in [256, 320].into_iter().enumerate() {
                let p = pixels[offset..offset + 64].try_into().unwrap();
                let mut dc = [0; 4];
                let mut ac = [[0; 16]; 4];
                if coded {
                    // H.264 chroma DC scan index 1 is raster row 1, column 0.
                    dc[2] = if component == 0 { 1 } else { -1 };
                    for b in 0..4 {
                        ac[b][1] = if (b + component) % 2 == 0 { 1 } else { -1 };
                    }
                }
                let samples =
                    primary_sp_chroma_420(p, &dc, &ac, 26, if qs == 51 { 39 } else { qs }).unwrap();
                next[offset..offset + 64].copy_from_slice(&samples);
            }
            output.extend(next.iter().map(|v| *v as u8));
            pixels = next;
        }
        let expected: &[u8] = if coded {
            include_bytes!("fixtures/playback-errors/avc-primary-sp-signed-residual-reference.yuv")
        } else {
            include_bytes!("fixtures/playback-errors/avc-primary-sp-skip-reference.yuv")
        };
        assert_eq!(output, expected, "coded residual={coded}");
    }
}
#[test]
fn primary_sp_video_decodes_exact_signed_residual_and_skip_frames_after_reset() {
    use fvid::codec::avc_decoder::AvcDecoder;
    let m = manifest();
    for key in ["video", "coded_video"] {
        let c = &m[key];
        let config = hex(c["configuration"].as_str().unwrap());
        let expected: &[u8] = if key == "video" {
            include_bytes!("fixtures/playback-errors/avc-primary-sp-skip-reference.yuv")
        } else {
            include_bytes!("fixtures/playback-errors/avc-primary-sp-signed-residual-reference.yuv")
        };
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            let mut actual = vec![];
            for packet in c["packets"].as_array().unwrap() {
                let frame = decoder
                    .decode(&hex(packet.as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                frame.write_planar(&mut actual).unwrap();
            }
            assert_eq!(actual, expected, "{key}");
            decoder.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn primary_sp_mp4_software_playback_and_rewind_match_oracle() {
    let m = manifest();
    for key in ["video", "coded_video"] {
        let c = &m[key];
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(c["file"].as_str().unwrap()),
        )
        .unwrap();
        let expected: &[u8] = if key == "video" {
            include_bytes!("fixtures/playback-errors/avc-primary-sp-skip-reference.yuv")
        } else {
            include_bytes!("fixtures/playback-errors/avc-primary-sp-signed-residual-reference.yuv")
        };
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(data),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        for target in [None, Some(2), Some(3), None] {
            if let Some(target) = target {
                assert_eq!(reader.seek_to_sync(target), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            while let Some(frame) = reader.read_frame().unwrap() {
                actual.extend(
                    frame
                        .picture
                        .y
                        .iter()
                        .chain(&frame.picture.cb)
                        .chain(&frame.picture.cr)
                        .map(|v| *v as u8),
                );
            }
            assert_eq!(actual, expected, "{key}");
            reader.rewind();
        }
    }
}

#[test]
fn primary_sp_deblocking_matches_jm_and_secondary_sp_decodes() {
    use fvid::codec::{
        avc_decoder::AvcDecoder,
        avc_transform::{primary_sp_chroma_420, switching_luma_4x4},
    };
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-primary-sp-filter.json"
    ))
    .unwrap();
    for c in m["cases"].as_array().unwrap() {
        let config = hex(c["configuration"].as_str().unwrap());
        let expected = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(c["reference"].as_str().unwrap()),
        )
        .unwrap();
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        let mut actual = vec![];
        for packet in c["packets"].as_array().unwrap() {
            decoder
                .decode(&hex(packet.as_str().unwrap()))
                .unwrap()
                .unwrap()
                .write_planar(&mut actual)
                .unwrap();
        }
        assert_eq!(actual, expected, "filter mode {}", c["mode"]);
        // Prove that this stream exercises filtering rather than matching by omission.
        let source: Vec<u16> = c["source"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u16)
            .collect();
        let mut unfiltered = vec![0u16; 384];
        for b in 0..16 {
            let x = b % 4 * 4;
            let y = b / 4 * 4;
            let p = std::array::from_fn(|i| source[(y + i / 4) * 16 + x + i % 4]);
            let samples = switching_luma_4x4(&p, &[0; 16], 50, 0, false).unwrap();
            for i in 0..16 {
                unfiltered[(y + i / 4) * 16 + x + i % 4] = samples[i];
            }
        }
        for offset in [256, 320] {
            let p = source[offset..offset + 64].try_into().unwrap();
            unfiltered[offset..offset + 64]
                .copy_from_slice(&primary_sp_chroma_420(p, &[0; 4], &[[0; 16]; 4], 39, 0).unwrap());
        }
        assert_ne!(
            unfiltered.iter().map(|v| *v as u8).collect::<Vec<_>>(),
            expected[384..768]
        );
        decoder.reset();
        decoder
            .decode(&hex(c["packets"][0].as_str().unwrap()))
            .unwrap();
        assert!(decoder
            .decode(&hex(c["secondary_packets"][0].as_str().unwrap()))
            .unwrap()
            .is_some());
    }
}

#[cfg(feature = "player")]
#[test]
fn formerly_refused_secondary_sp_mp4_now_decodes_all_frames() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-primary-sp-filter.json"
    ))
    .unwrap();
    for c in m["cases"].as_array().unwrap() {
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(c["secondary_file"].as_str().unwrap()),
        )
        .unwrap();
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(data),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        let mut frames = 0;
        while reader.read_frame().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 4);
    }
}
