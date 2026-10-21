use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-fmo")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn pixels(picture: &IntraPicture) -> Vec<u8> {
    let mut bytes = Vec::new();
    picture.write_planar(&mut bytes).unwrap();
    bytes
}
#[test]
fn partitioned_fmo_matches_scalar_pixels_and_combined_streams() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 120);
    for case in m["cases"].as_array().unwrap() {
        let mut actual =
            AvcDecoder::new(&read(case["configuration"].as_str().unwrap()), 2 << 20).unwrap();
        let mut control =
            AvcDecoder::new(&read(case["configuration"].as_str().unwrap()), 2 << 20).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for _ in 0..2 {
            for d in [&mut actual, &mut control] {
                d.decode_order(&read(case["seed"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
            }
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let picture = actual
                    .decode_order(&read(frame["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} {}: {e}", case["name"], frame["packet"]))
                    .unwrap();
                let expected = control
                    .decode_order(&read(frame["control"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                let index = index + 1;
                assert_eq!(
                    pixels(&picture),
                    pixels(&expected),
                    "{} combined frame{index}",
                    case["name"]
                );
                assert_eq!(
                    pixels(&picture),
                    oracle[index * 1536..(index + 1) * 1536],
                    "{} scalar frame{index}",
                    case["name"]
                );
            }
            actual.reset();
            control.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn partitioned_fmo_mp4_preserves_pixels_and_clocks_through_rewind_and_seek() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for case in manifest()["cases"].as_array().unwrap() {
        let mut reader = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(case["file"].as_str().unwrap())),
            Default::default(),
            8 << 20,
        )
        .unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for pass in 0..3 {
            for index in 0..3 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"]))
                    .unwrap();
                assert_eq!(
                    pixels(&frame.picture),
                    oracle[index * 1536..(index + 1) * 1536],
                    "{} presentation{index}",
                    case["name"]
                );
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    index as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            }
            assert!(reader.read_frame().unwrap().is_none());
            if pass == 0 {
                reader.rewind();
            } else {
                assert_eq!(reader.seek_to_sync(2), 0);
            }
        }
    }
}

#[test]
fn fmo_maps_slice_ids_partition_binding_and_cycle_fields_match_literal_groups() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        avc_slice::SliceType,
        avc_slice_group_map::map_units,
        config::{AvcConfig, NalUnits},
    };
    let mut checked = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let configuration = read(case["configuration"].as_str().unwrap());
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        assert_eq!(sps.profile, 88);
        assert_eq!(sps.coded_dimensions(), (32, 32));
        let expected: Vec<_> = case["mapping"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let cycle = if (3..=5).contains(&case["map_type"].as_u64().unwrap()) {
            Some(2)
        } else {
            None
        };
        assert_eq!(
            map_units(&pps.slice_groups, 2, 2, cycle, 4).unwrap(),
            expected
        );
        let multislice = case["multislice"].as_bool().unwrap();
        for frame in case["frames"].as_array().unwrap() {
            let packet = read(frame["packet"].as_str().unwrap());
            let selected = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            assert_eq!(selected.len(), if multislice { 4 } else { 2 });
            let mut ids = std::collections::BTreeSet::new();
            for slice in selected {
                let h = &slice.header;
                assert_eq!(h.slice_type, SliceType::P);
                assert_eq!(h.slice_group_change_cycle, cycle);
                let partitions = h.data_partitions.as_ref().unwrap();
                let id = if multislice {
                    h.first_mb
                } else {
                    u32::from(expected[h.first_mb as usize])
                };
                assert_eq!(partitions.slice_id, id);
                assert!(ids.insert(id));
                let offset = (2 * (32 - (id + 1).leading_zeros()) - 1) as usize;
                for residual in [&partitions.intra, &partitions.inter].into_iter().flatten() {
                    assert_eq!(residual.entropy_bit_offset, offset);
                }
                if case["mode"] == "coded" {
                    assert!(partitions.intra.is_none());
                    assert!(partitions.inter.is_some());
                }
                if case["mode"] == "skipmixed" {
                    assert!(partitions.inter.is_none());
                }
                checked += 1;
            }
            if case["aso"].as_bool().unwrap() {
                let types: Vec<_> = NalUnits::new(&packet, 4)
                    .unwrap()
                    .map(|n| n.unwrap()[0] & 31)
                    .collect();
                assert!(types.windows(2).all(|pair| pair[0] <= pair[1]));
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
    assert_eq!(checked, 720);
    let golden = read("map0-dir0-coded-aso0.yuv");
    assert_eq!((golden[1536], golden[1536 + 16]), (5, 173));
    assert_eq!((golden[3072], golden[3072 + 16]), (9, 169));
}
