use fvid::codec::avc_decoder::AvcDecoder;
use std::path::Path;

#[test]
fn separate_plane_intra_decodes_cavlc_cabac_and_per_plane_scaling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-intra");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 180);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let configuration = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        let mut decoder = AvcDecoder::new(&configuration, 1 << 20).unwrap();
        for _ in 0..2 {
            let picture = decoder
                .decode_order(&packet)
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert_eq!(picture.chroma_format().unwrap(), 3);
            assert_eq!(
                picture.component_depths(),
                [case["depth"].as_u64().unwrap() as u8; 3]
            );
            assert_eq!(picture.crop, [1; 4]);
            for (c, plane) in [&picture.y, &picture.cb, &picture.cr]
                .into_iter()
                .enumerate()
            {
                assert_eq!(plane.len(), 512);
                if let Some(index) = plane
                    .iter()
                    .zip(&golden[c * 512..(c + 1) * 512])
                    .position(|(a, b)| a != b)
                {
                    panic!(
                        "{name} plane {c} sample {index}: got {}, expected {}",
                        plane[index],
                        golden[c * 512 + index]
                    );
                }
            }
            decoder.reset();
        }
    }
}

#[test]
fn separate_plane_mp4_crops_samples_and_rewinds() {
    use fvid::{container::mp4::Limits, playback::video::mp4::Mp4VideoReader};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-intra");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        let mut expected = Vec::new();
        for c in 0..3 {
            for y in 1..15 {
                for x in 1..31 {
                    let value = golden[c * 512 + y * 32 + x];
                    if depth == 8 {
                        expected.push(value as u8);
                    } else {
                        expected.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        let data = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open(std::io::Cursor::new(data), Limits::default(), 16 << 20).unwrap();
        for _ in 0..2 {
            let frame = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert_eq!(frame.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            let packed = frame.packed.unwrap();
            assert_eq!(packed.depth, depth);
            assert_eq!(packed.frame.subsampling, Some([1, 1]));
            assert_eq!(packed.frame.data, expected, "{name}");
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn separate_plane_scaling_selects_each_component_and_resolves_eight_fallbacks() {
    use fvid::codec::{
        avc::{Pps, ScalingList, Sps},
        avc_scaling::ScalingMatrices,
        config::AvcConfig,
    };
    let bytes = include_bytes!(
        "fixtures/playback-errors/avc-separate-intra/separate-intra-8-e0-flat-r0-k0.avcc"
    );
    let config = AvcConfig::parse(bytes).unwrap();
    let mut sps = Sps::parse(config.sps[0]).unwrap();
    let mut pps = Pps::parse(config.pps[0], &sps).unwrap();
    sps.scaling_lists = Some(
        (0..12)
            .map(|i| {
                Some(ScalingList::Explicit(vec![
                    10 + i as u8;
                    if i < 6 { 16 } else { 64 }
                ]))
            })
            .collect(),
    );
    for plane in 0..3 {
        let m = ScalingMatrices::for_colour_plane(&sps, &pps, plane).unwrap();
        assert_eq!(m.four[0], [10 + plane as u8; 16]);
        assert_eq!(m.four[3], [13 + plane as u8; 16]);
        assert_eq!(m.eight[0], [16 + 2 * plane as u8; 64]);
        assert_eq!(m.eight[1], [17 + 2 * plane as u8; 64]);
    }
    // Unspecified Cb/Cr 8x8 lists inherit the previous component, separately
    // for intra/inter. The new PPS values must win over the SPS Cb/Cr lists.
    let mut lists = vec![None; 12];
    lists[6] = Some(ScalingList::Explicit(vec![35; 64]));
    lists[7] = Some(ScalingList::Explicit(vec![47; 64]));
    pps.transform_8x8 = true;
    pps.scaling_lists = Some(lists);
    for plane in 0..3 {
        let m = ScalingMatrices::for_colour_plane(&sps, &pps, plane).unwrap();
        assert_eq!(m.eight, [[35; 64], [47; 64]]);
    }
    assert!(ScalingMatrices::for_colour_plane(&sps, &pps, 3).is_err());
}

#[test]
fn separate_plane_compressed_budget_includes_retained_components() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit,
        avc_picture::decode_intra_slices,
        config::AvcConfig,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-intra");
    for entropy in [0, 1] {
        let name = format!("separate-intra-10-e{entropy}-sps-r0-k0");
        let bytes = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let config = AvcConfig::parse(&bytes).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        let slices = avc_access_unit::prepare(&packet, 4, &sps, &pps, 1 << 20).unwrap();
        let headers: Vec<_> = slices.iter().map(|s| &s.header).collect();
        let error = decode_intra_slices(&headers, &sps, &pps, 3 * 512 * 2 - 1).unwrap_err();
        assert!(error.to_string().contains("budget"), "{name}: {error}");
        assert!(decode_intra_slices(&headers, &sps, &pps, 1 << 20).is_ok());
    }
}
