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
fn primary_sp_video_refusal_remains_until_picture_integration() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        avc_slice::{SliceHeader, SliceType},
        config::AvcConfig,
    };
    let m = manifest();
    let config = hex(m["video"]["configuration"].as_str().unwrap());
    let avc = AvcConfig::parse(&config).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
    decoder
        .decode(&hex(m["video"]["packets"][0].as_str().unwrap()))
        .unwrap()
        .unwrap();
    let packet = hex(m["video"]["packets"][1].as_str().unwrap());
    let h = SliceHeader::parse(&packet[4..], &sps, &pps).unwrap();
    assert_eq!(h.slice_type, SliceType::Sp);
    assert!(!h.sp_for_switch);
    assert_eq!(h.slice_qs, Some(0));
    let error = decoder.decode(&packet).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("AVC picture type is not implemented"),
        "{error}"
    );
}
