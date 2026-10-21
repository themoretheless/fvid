use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-interlaced-switching")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn pixels(p: &IntraPicture) -> Vec<u8> {
    let mut v = Vec::new();
    p.write_planar(&mut v).unwrap();
    v
}
#[test]
fn partitioned_interlaced_switching_matches_matrix_pixels_and_combined_controls() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 184);
    for case in m["cases"].as_array().unwrap() {
        let configuration = read(case["configuration"].as_str().unwrap());
        let mut decoder = AvcDecoder::new(&configuration, 4 << 20).unwrap();
        let mut control = AvcDecoder::new(&configuration, 4 << 20).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for _ in 0..2 {
            for record in case["frames"].as_array().unwrap() {
                let actual = decoder
                    .decode_order(&read(record["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} {}: {e}", case["name"], record["packet"]));
                let expected = control
                    .decode_order(&read(record["control"].as_str().unwrap()))
                    .unwrap_or_else(|e| {
                        panic!("{} combined {}: {e}", case["name"], record["packet"])
                    });
                assert_eq!(
                    actual.is_some(),
                    record["emits"].as_bool().unwrap(),
                    "{} {}",
                    case["name"],
                    record["packet"]
                );
                assert_eq!(actual.is_some(), expected.is_some());
                if let Some(actual) = actual {
                    let index = record["presentation"].as_u64().unwrap() as usize;
                    assert_eq!(
                        pixels(&actual),
                        pixels(&expected.unwrap()),
                        "{} combined",
                        case["name"]
                    );
                    assert_eq!(
                        pixels(&actual),
                        oracle[index * 1536..(index + 1) * 1536],
                        "{} scalar frame{index}",
                        case["name"]
                    );
                }
            }
            decoder.reset();
            control.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn partitioned_interlaced_switching_mp4_preserves_pixels_pairing_and_clocks() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for case in manifest()["cases"].as_array().unwrap() {
        let mut r = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(case["file"].as_str().unwrap())),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for pass in 0..3 {
            for index in 0..4 {
                let f = r
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"]))
                    .unwrap();
                assert_eq!(
                    pixels(&f.picture),
                    oracle[index * 1536..(index + 1) * 1536],
                    "{} frame{index}",
                    case["name"]
                );
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    index as i128 * 40_000_000
                );
                assert_eq!(f.duration.nanoseconds().unwrap(), 40_000_000);
            }
            assert!(r.read_frame().unwrap().is_none());
            if pass == 0 {
                r.rewind();
            } else {
                assert_eq!(r.seek_to_sync(6), 0);
            }
        }
    }
}

#[test]
fn switching_headers_bind_qs_flags_field_order_and_residual_category() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        avc_slice::{RefModification, SliceType},
        config::AvcConfig,
    };
    let mut checked = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let configuration = read(case["configuration"].as_str().unwrap());
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let mbaff = case["mbaff"].as_bool().unwrap();
        let si = case["kind"].as_str().unwrap().starts_with("si");
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only);
        assert_eq!(sps.mb_adaptive_frame_field, mbaff);
        for (index, record) in case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, r)| r["presentation"] != 0)
        {
            let packet = read(record["packet"].as_str().unwrap());
            let selected = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            assert_eq!(
                selected.len(),
                if case["split"].as_bool().unwrap() {
                    2
                } else {
                    1
                }
            );
            for slice in selected {
                let h = &slice.header;
                assert_eq!(h.slice_type, if si { SliceType::Si } else { SliceType::Sp });
                assert_eq!(h.slice_qp, 26);
                assert_eq!(h.slice_qs, Some(record["qs"].as_i64().unwrap() as i32));
                assert_eq!(h.sp_for_switch, case["kind"] == "secondary");
                assert_eq!(h.field_pic, !mbaff);
                if !mbaff {
                    assert_eq!(h.bottom_field, record["bottom"].as_bool().unwrap());
                    if !si {
                        assert_eq!(h.modifications_l0, vec![RefModification::Subtract(1)]);
                    }
                }
                assert_eq!(
                    h.poc_lsb,
                    Some(
                        record["presentation"].as_u64().unwrap() as u32 * 2
                            + if mbaff { 0 } else { index as u32 % 2 }
                    )
                );
                let p = h.data_partitions.as_ref().unwrap();
                assert_eq!(p.slice_id, h.first_mb);
                assert_eq!(p.intra.is_some(), si && case["coded"].as_bool().unwrap());
                assert_eq!(
                    p.inter.is_some(),
                    !si && case["coded"].as_bool().unwrap() && case["skip"] != "both"
                );
                checked += 1;
            }
            assert!(
                prepare(&packet, 4, &sps, &pps, packet.len() - 1)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("byte limit")
            );
        }
    }
    assert_eq!(checked, 1272);
    let ordinary = read("paff-si-ordinary-signed0-bottom0-split0-aso0.yuv");
    assert_eq!(ordinary[1536..3072], ordinary[4608..6144]);
    let switching = read("paff-si-signed0-bottom0-split0-aso0.yuv");
    assert_ne!(switching[1536..3072], ordinary[1536..3072]);
}
