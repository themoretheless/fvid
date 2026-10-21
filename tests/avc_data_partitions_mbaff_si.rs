use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-mbaff-si")
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
fn partitioned_mbaff_si_matches_availability_matrix_and_combined_controls() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 672);
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
fn partitioned_mbaff_si_mp4_preserves_switching_pixels_and_clocks() {
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
                assert_eq!(r.seek_to_sync(3), 0);
            }
        }
    }
}

#[test]
fn mbaff_si_control_trace_and_partition_presence_match_switching_and_pcm_blocks() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        avc_slice::SliceType,
        bits::BitReader,
        config::AvcConfig,
    };
    let mut checked = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let configuration = read(case["configuration"].as_str().unwrap());
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert!(!sps.frame_mbs_only);
        assert!(sps.mb_adaptive_frame_field);
        assert_eq!(
            pps.constrained_intra_pred,
            case["constrained"].as_bool().unwrap()
        );
        for record in case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["presentation"] != 0)
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
                assert_eq!(h.slice_type, SliceType::Si);
                assert!(!h.field_pic);
                assert_eq!(h.slice_qp, 26);
                assert_eq!(h.slice_qs, Some(record["qs"].as_i64().unwrap() as i32));
                let p = h.data_partitions.as_ref().unwrap();
                assert_eq!(p.slice_id, h.first_mb);
                assert!(p.inter.is_none());
                let group = case["groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|g| g[0].as_u64().unwrap() == u64::from(h.first_mb) * 2)
                    .unwrap()
                    .as_array()
                    .unwrap();
                let mut bits = BitReader::new(&h.rbsp);
                bits.skip(h.entropy_bit_offset).unwrap();
                let mut expected_b = false;
                for address in group.iter().map(|v| v.as_u64().unwrap() as usize) {
                    if address % 2 == 0 {
                        assert_eq!(
                            bits.bit().unwrap(),
                            case["fields"][address / 2].as_bool().unwrap()
                        );
                    }
                    let pcm = case["pcm"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|v| v.as_u64().unwrap() as usize == address);
                    let switching = case["switching"][address].as_bool().unwrap();
                    let active = case["coded"].as_bool().unwrap() && switching;
                    assert_eq!(
                        bits.unsigned_golomb().unwrap(),
                        if pcm {
                            26
                        } else if switching {
                            0
                        } else {
                            1
                        }
                    );
                    if pcm {
                        expected_b = true;
                        continue;
                    }
                    for _ in 0..16 {
                        assert!(bits.bit().unwrap());
                    }
                    assert_eq!(bits.unsigned_golomb().unwrap(), 0);
                    assert_eq!(
                        bits.unsigned_golomb().unwrap(),
                        if active {
                            if case["chroma"].is_null() { 2 } else { 0 }
                        } else {
                            3
                        }
                    );
                    if active {
                        assert_eq!(bits.signed_golomb().unwrap(), 0);
                        expected_b = true;
                    }
                }
                bits.finish_rbsp().unwrap();
                assert_eq!(p.intra.is_some(), expected_b);
                if let Some(b) = &p.intra {
                    assert_eq!(b.entropy_bit_offset, if h.first_mb == 0 { 1 } else { 3 });
                }
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
    assert_eq!(checked, 3360);
    let unconstrained = read("00-top-constrained0-signed1-chroma1-split0-aso0.yuv");
    let constrained = read("00-top-constrained1-signed1-chroma1-split0-aso0.yuv");
    assert_ne!(unconstrained[4608..6144], constrained[4608..6144]);
}
