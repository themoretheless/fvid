use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-interlaced")
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
fn partitioned_interlaced_pictures_match_physical_pixels_and_combined_controls() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 240);
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
fn partitioned_interlaced_mp4_preserves_frame_pairing_and_clocks_through_seek() {
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
            for index in 0..3 {
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
                assert_eq!(r.seek_to_sync(4), 0);
            }
        }
    }
}

#[test]
fn partitioned_pcm_fields_use_partition_b_in_the_direct_pcm_api() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        avc_field_picture::decode_pcm_slices,
        config::AvcConfig,
    };
    let mut checked = 0;
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["mode"] == "intra" && c["mbaff"] == false && c["fmo"] == false)
    {
        let configuration = read(case["configuration"].as_str().unwrap());
        let config = AvcConfig::parse(&configuration).unwrap();
        let sps = Sps::parse(config.sps[0]).unwrap();
        let pps = Pps::parse(config.pps[0], &sps).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for record in case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["presentation"] != 0)
        {
            let packet = read(record["packet"].as_str().unwrap());
            let selected = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            let headers: Vec<_> = selected.iter().map(|s| &s.header).collect();
            let actual = decode_pcm_slices(&headers, &sps, &pps, 2 << 20)
                .unwrap_or_else(|e| panic!("{} {}: {e}", case["name"], record["packet"]));
            let index = record["presentation"].as_u64().unwrap() as usize;
            let full = &oracle[index * 1536..(index + 1) * 1536];
            let bottom = record["bottom"].as_bool().unwrap();
            let mut expected = Vec::new();
            for (offset, width, height) in [(0, 32, 32), (1024, 16, 16), (1280, 16, 16)] {
                for row in (usize::from(bottom)..height).step_by(2) {
                    expected
                        .extend_from_slice(&full[offset + row * width..offset + (row + 1) * width]);
                }
            }
            assert_eq!(pixels(&actual.picture), expected);
            assert_eq!(actual.bottom, bottom);
            checked += 1;
        }
    }
    assert_eq!(checked, 32);
}

fn seed_fields(d: &mut AvcDecoder) {
    assert!(
        d.decode_order(&read(
            "paff-fmo0-split0-aso0-intra-seed0-topology00-0-0.packet"
        ))
        .unwrap()
        .is_none()
    );
    assert!(
        d.decode_order(&read(
            "paff-fmo0-split0-aso0-intra-seed0-topology00-0-1.packet"
        ))
        .unwrap()
        .is_some()
    );
}
#[test]
fn pure_skip_partitioned_fields_keep_copy_path_pixels_without_residual_streams() {
    let configuration = read("paff-fmo0.avcc");
    let mut d = AvcDecoder::new(&configuration, 4 << 20).unwrap();
    let mut control = AvcDecoder::new(&configuration, 4 << 20).unwrap();
    let oracle = read("paff-fmo0-split0-aso0-intra-seed0-topology00.yuv");
    for _ in 0..2 {
        seed_fields(&mut d);
        seed_fields(&mut control);
        assert!(
            d.decode_order(&read("pure-skip-0.packet"))
                .unwrap()
                .is_none()
        );
        assert!(
            control
                .decode_order(&read("pure-skip-0-control.packet"))
                .unwrap()
                .is_none()
        );
        let actual = d
            .decode_order(&read("pure-skip-1.packet"))
            .unwrap()
            .unwrap();
        let expected = control
            .decode_order(&read("pure-skip-1-control.packet"))
            .unwrap()
            .unwrap();
        assert_eq!(pixels(&actual), pixels(&expected));
        assert_eq!(pixels(&actual), oracle[..1536]);
        d.reset();
        control.reset();
    }
}
#[test]
fn malformed_field_partitions_fail_in_the_intended_stream_and_reset_cleanly() {
    for (name, message) in [
        ("skip-extra-b", "without residual syntax"),
        ("skip-extra-c", "without residual syntax"),
        ("coded-missing-c", "bit"),
        ("pcm-missing-b", "bit"),
        ("pcm-extra-c", "without residual syntax"),
    ] {
        let mut d = AvcDecoder::new(&read("paff-fmo0.avcc"), 4 << 20).unwrap();
        seed_fields(&mut d);
        let error = d
            .decode_order(&read(&format!("{name}.packet")))
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{name}: {error}");
        assert!(d.decode_order(&read("pure-skip-0.packet")).is_err());
        d.reset();
        seed_fields(&mut d);
        assert!(
            d.decode_order(&read("pure-skip-0.packet"))
                .unwrap()
                .is_none()
        );
        assert!(
            d.decode_order(&read("pure-skip-1.packet"))
                .unwrap()
                .is_some()
        );
    }
}
#[cfg(feature = "player")]
#[test]
fn software_mp4_refuses_missing_and_surplus_field_partitions_after_valid_seed() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for (name, message) in [
        ("skip-extra-b", "without residual syntax"),
        ("skip-extra-c", "without residual syntax"),
        ("coded-missing-c", "bit"),
        ("pcm-missing-b", "bit"),
        ("pcm-extra-c", "without residual syntax"),
    ] {
        let mut r = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..2 {
            let mut emitted = 0;
            let error = loop {
                match r.read_frame() {
                    Err(e) => break e.to_string(),
                    Ok(Some(f)) => {
                        assert_eq!(emitted, 0);
                        let oracle = read("paff-fmo0-split0-aso0-intra-seed0-topology00.yuv");
                        assert_eq!(pixels(&f.picture), oracle[..1536]);
                        emitted += 1;
                    }
                    Ok(None) => panic!("{name}: invalid field accepted"),
                }
            };
            assert!(error.contains(message), "{name}: {error}");
            if pass == 0 {
                r.rewind();
            }
        }
    }
}

#[test]
fn interlaced_headers_bind_field_parity_pair_units_and_residual_categories() {
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
        let split = case["split"].as_bool().unwrap();
        let intra = case["mode"] == "intra";
        assert!(!sps.frame_mbs_only);
        assert_eq!(sps.mb_adaptive_frame_field, mbaff);
        assert_eq!(sps.coded_dimensions(), (32, 32));
        assert_eq!(sps.profile, 88);
        for (index, record) in case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, r)| r["presentation"] != 0)
        {
            let packet = read(record["packet"].as_str().unwrap());
            let selected = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            assert_eq!(selected.len(), if split { 2 } else { 1 });
            for slice in selected {
                let h = &slice.header;
                assert_eq!(
                    h.slice_type,
                    if intra { SliceType::I } else { SliceType::P }
                );
                assert_eq!(h.field_pic, !mbaff);
                assert_eq!(
                    h.bottom_field,
                    record["bottom"].as_bool().unwrap() && !mbaff
                );
                assert_eq!(h.frame_num, record["presentation"].as_u64().unwrap() as u32);
                assert_eq!(
                    h.poc_lsb,
                    Some(h.frame_num * 2 + if mbaff { 0 } else { index as u32 % 2 })
                );
                if !mbaff && !intra {
                    assert_eq!(h.modifications_l0, vec![RefModification::Subtract(1)]);
                }
                let partition = h.data_partitions.as_ref().unwrap();
                assert_eq!(partition.slice_id, h.first_mb);
                assert!(h.first_mb < 2);
                let has_pcm = intra
                    || case["mode"] != "coded"
                        && (!split
                            || case["mode"] == "pcm-left" && h.first_mb == 0
                            || case["mode"] == "pcm-right" && h.first_mb == 1);
                let has_inter = !intra && (case["mode"] == "coded" || !split || !has_pcm);
                assert_eq!(partition.intra.is_some(), has_pcm);
                assert_eq!(partition.inter.is_some(), has_inter);
                for residual in [&partition.intra, &partition.inter].into_iter().flatten() {
                    assert_eq!(
                        residual.entropy_bit_offset,
                        if h.first_mb == 0 { 1 } else { 3 }
                    );
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
    assert_eq!(checked, 960);
}
