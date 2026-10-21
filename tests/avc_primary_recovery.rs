use fvid::codec::{
    avc::{Pps, Sps},
    avc_decoder::AvcDecoder,
    avc_picture::IntraPicture,
    config::AvcConfig,
};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-primary-recovery")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn pixels(p: &IntraPicture) -> Vec<u8> {
    let mut b = Vec::new();
    p.write_planar(&mut b).unwrap();
    b
}
#[test]
fn intact_primary_slices_and_redundant_areas_recover_missing_or_truncated_payloads() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 88);
    for c in m["cases"].as_array().unwrap() {
        let cfg = read("configuration.avcc");
        let mut d = AvcDecoder::new(&cfg, 1 << 20).unwrap();
        let mut control = AvcDecoder::new(&cfg, 1 << 20).unwrap();
        let oracle = read(c["oracle"].as_str().unwrap());
        for _ in 0..2 {
            for (i, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                let actual = d
                    .decode_order(&read(f["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} frame{i}: {e}", c["name"]))
                    .unwrap();
                let expected = control
                    .decode_order(&read(f["control"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    pixels(&actual),
                    pixels(&expected),
                    "{} frame{i} control",
                    c["name"]
                );
                assert_eq!(
                    pixels(&actual),
                    oracle[i * 768..(i + 1) * 768],
                    "{} frame{i} scalar",
                    c["name"]
                );
            }
            d.reset();
            control.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn software_mp4_recovery_survives_eof_rewind_and_sync_seek() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for c in manifest()["cases"].as_array().unwrap() {
        let oracle = read(c["oracle"].as_str().unwrap());
        let mut r = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(c["file"].as_str().unwrap())),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        for pass in 0..3 {
            let mut count = 0;
            while let Some(f) = r.read_frame().unwrap() {
                assert!(count < oracle.len() / 768);
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    count as i128 * 40_000_000
                );
                assert_eq!(f.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(
                    pixels(&f.picture),
                    oracle[count * 768..(count + 1) * 768],
                    "{} frame{count}",
                    c["name"]
                );
                count += 1;
            }
            assert_eq!(count, oracle.len() / 768);
            if pass == 0 {
                r.rewind();
            } else {
                assert_eq!(r.seek_to_sync((oracle.len() / 768 - 1) as i64), 0);
            }
        }
    }
}

#[test]
fn primary_payload_failure_without_redundancy_is_not_hidden() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
        if c["recovery_index"].as_u64().unwrap() == 1 {
            d.decode_order(&read(c["frames"][0]["packet"].as_str().unwrap()))
                .unwrap();
        }
        assert!(
            d.decode_order(&read(c["primary"].as_str().unwrap()))
                .is_err(),
            "{}",
            c["name"]
        );
    }
}

#[test]
fn recovered_metadata_preserves_primary_and_redundant_wire_offsets() {
    use fvid::codec::{avc_access_unit::prepare_for_decode, avc_slice::SliceHeader};
    let cfg = read("configuration.avcc");
    let avc = AvcConfig::parse(&cfg).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let bank = vec![(sps.clone(), pps.clone())];
    for c in manifest()["cases"].as_array().unwrap() {
        let index = c["recovery_index"].as_u64().unwrap() as usize;
        let packet = read(c["frames"][index]["packet"].as_str().unwrap());
        let prepared =
            prepare_for_decode(&packet, 4, &sps, &pps, &bank, packet.len(), 1 << 20).unwrap();
        assert_eq!(prepared.source_redundant_pic_cnt, 0);
        assert_eq!(prepared.slices.len(), 2);
        for slice in prepared.slices {
            let wire = SliceHeader::parse(slice.nal, &sps, &pps).unwrap();
            assert_eq!(
                wire.redundant_pic_cnt,
                if u64::from(wire.first_mb) == c["good_mb"].as_u64().unwrap() {
                    0
                } else {
                    1
                }
            );
            assert_eq!(slice.header.redundant_pic_cnt, 0);
            assert_eq!(slice.header.entropy_bit_offset, wire.entropy_bit_offset);
        }
    }
}

#[test]
fn truncated_fixture_has_valid_primary_headers_and_specific_entropy_failure() {
    use fvid::codec::{
        avc_access_unit::prepare_with_parameters, avc_inter_slice::InterCavlcSlice,
        avc_macroblock::IntraCavlcReader, avc_slice::SliceType,
    };
    let cfg = read("configuration.avcc");
    let avc = AvcConfig::parse(&cfg).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    let bank = vec![(sps.clone(), pps.clone())];
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["damage"] == "truncated")
    {
        let packet = read(c["primary"].as_str().unwrap());
        let slices = prepare_with_parameters(&packet, 4, &sps, &pps, &bank, packet.len()).unwrap();
        assert_eq!(slices.len(), 2);
        for slice in slices {
            let h = &slice.header;
            let outcome = (|| -> fvid::Result<usize> {
                let mut count = 0;
                if h.slice_type == SliceType::I {
                    let mut r = IntraCavlcReader::new_fmo(h, &sps, &pps, 1024)?;
                    while r.read_macroblock()?.is_some() {
                        count += 1;
                    }
                } else {
                    let mut r = InterCavlcSlice::new_fmo(h, &sps, &pps, 1 << 20)?;
                    while r.read_macroblock()?.is_some() {
                        count += 1;
                    }
                }
                Ok(count)
            })();
            if u64::from(h.first_mb) == c["good_mb"].as_u64().unwrap() {
                assert_eq!(outcome.unwrap(), 1);
            } else {
                let error = outcome.unwrap_err().to_string();
                assert!(error.contains("truncated"), "{}: {error}", c["name"]);
            }
        }
    }
}
