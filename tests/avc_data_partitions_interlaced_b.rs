use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-interlaced-b")
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
fn partitioned_interlaced_b_matches_two_reference_pixels_and_combined_controls() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 128);
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
fn partitioned_interlaced_b_reorders_frames_and_fields_with_exact_clocks() {
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
            for index in 0..5 {
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
                assert_eq!(r.seek_to_sync(8), 0);
            }
        }
    }
}

#[test]
fn b_headers_prove_two_references_nonreference_outputs_and_actual_partition_modes() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        avc_slice::{RefModification, SliceType},
        config::{AvcConfig, NalUnits},
    };
    let mut checked = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let configuration = read(case["configuration"].as_str().unwrap());
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let mbaff = case["mbaff"].as_bool().unwrap();
        let split = case["split"].as_bool().unwrap();
        assert_eq!(sps.profile, 88);
        assert_eq!(sps.max_num_ref_frames, 3);
        assert_eq!(sps.mb_adaptive_frame_field, mbaff);
        let records = case["frames"].as_array().unwrap();
        let order: Vec<_> = records
            .iter()
            .filter(|r| r["emits"] == true)
            .map(|r| r["presentation"].as_u64().unwrap())
            .collect();
        assert_eq!(order, [0, 3, 1, 2, 4]);
        for (index, record) in records
            .iter()
            .enumerate()
            .filter(|(_, r)| r["role"] == "b1" || r["role"] == "b2")
        {
            let packet = read(record["packet"].as_str().unwrap());
            for nal in NalUnits::new(&packet, 4).unwrap() {
                let nal = nal.unwrap();
                assert!((2..=4).contains(&(nal[0] & 31)));
                assert_eq!(nal[0] & 0x60, 0);
            }
            let selected = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            assert_eq!(selected.len(), if split { 2 } else { 1 });
            for slice in selected {
                let h = &slice.header;
                assert_eq!(h.slice_type, SliceType::B);
                assert_eq!(h.frame_num, 2);
                assert_eq!(
                    h.poc_lsb,
                    Some(
                        record["presentation"].as_u64().unwrap() as u32 * 2
                            + if mbaff { 0 } else { index as u32 % 2 }
                    )
                );
                assert_eq!(h.direct_spatial_mv_pred, case["spatial"].as_bool().unwrap());
                assert_eq!(h.field_pic, !mbaff);
                assert_eq!((h.refs_l0, h.refs_l1), (1, 1));
                if !mbaff {
                    assert_eq!(h.bottom_field, record["bottom"].as_bool().unwrap());
                    assert_eq!(h.modifications_l0, vec![RefModification::Subtract(3)]);
                    assert_eq!(h.modifications_l1, vec![RefModification::Subtract(1)]);
                }
                let p = h.data_partitions.as_ref().unwrap();
                assert_eq!(p.slice_id, h.first_mb);
                let pcm = case["mode"] == "pcm-l1" && (!split || h.first_mb == 0);
                let inter = case["mode"] != "skip" && (!split || !pcm);
                assert_eq!(p.intra.is_some(), pcm);
                assert_eq!(p.inter.is_some(), inter);
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
        for (index, record) in records
            .iter()
            .enumerate()
            .filter(|(_, r)| r["role"] == "successor")
        {
            let packet = read(record["packet"].as_str().unwrap());
            for nal in NalUnits::new(&packet, 4).unwrap() {
                let h =
                    fvid::codec::avc_slice::SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap();
                assert_eq!(h.slice_type, SliceType::P);
                if mbaff || index % 2 == 0 {
                    assert!(h.modifications_l0.is_empty());
                }
            }
        }
        let golden = read(case["oracle"].as_str().unwrap());
        for i in 0..1536 {
            assert_eq!(golden[3 * 1536 + i], golden[i] + 4);
            assert_eq!(golden[4 * 1536 + i], golden[3 * 1536 + i]);
        }
    }
    assert_eq!(checked, 640);
}
