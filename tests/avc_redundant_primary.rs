use fvid::codec::{
    avc::{Pps, Sps},
    avc_decoder::AvcDecoder,
    config::AvcConfig,
};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-redundant-primary")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn bytes(p: &fvid::codec::avc_picture::IntraPicture) -> Vec<u8> {
    let mut out = Vec::new();
    p.write_planar(&mut out).unwrap();
    out
}
#[test]
fn redundant_representations_do_not_replace_or_duplicate_primary_pictures() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 12);
    let oracle = read("oracle.yuv");
    for c in m["cases"].as_array().unwrap() {
        let cfg = read(c["configuration"].as_str().unwrap());
        let mut d = AvcDecoder::new(&cfg, 1 << 20).unwrap();
        let mut control = AvcDecoder::new(&cfg, 1 << 20).unwrap();
        for _ in 0..2 {
            for (i, f) in c["frames"].as_array().unwrap().iter().enumerate() {
                let p = read(f["packet"].as_str().unwrap());
                let actual = d
                    .decode_order(&p)
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
                    .unwrap();
                let expected = control
                    .decode_order(&read(f["primary"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    bytes(&actual),
                    oracle[i * 768..(i + 1) * 768],
                    "{}",
                    c["name"]
                );
                assert_eq!(bytes(&actual), bytes(&expected));
            }
            d.reset();
            control.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn mp4_redundant_slices_preserve_primary_pixels_after_rewind_and_sync_seek() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    let oracle = read("oracle.yuv");
    for c in manifest()["cases"].as_array().unwrap() {
        let mut r = Mp4VideoReader::open_software(
            Cursor::new(read(c["file"].as_str().unwrap())),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        for pass in 0..3 {
            let mut count = 0;
            while let Some(f) = r.read_frame().unwrap() {
                assert!(count < 2);
                assert_eq!(bytes(&f.picture), oracle[count * 768..(count + 1) * 768]);
                count += 1;
            }
            assert_eq!(count, 2);
            if pass == 0 {
                r.rewind();
            } else {
                assert_eq!(r.seek_to_sync(1), 0);
            }
        }
    }
}

#[test]
fn preparation_keeps_only_bounded_primary_slices_and_preserves_uniform_promises() {
    use fvid::codec::avc_access_unit::{prepare, prepare_with_parameters};
    for c in manifest()["cases"].as_array().unwrap() {
        let config = read(c["configuration"].as_str().unwrap());
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let params: Vec<_> = avc
            .pps
            .iter()
            .map(|nal| (sps.clone(), Pps::parse(nal, &sps).unwrap()))
            .collect();
        let pps = &params[0].1;
        for f in c["frames"].as_array().unwrap() {
            let packet = read(f["packet"].as_str().unwrap());
            let primary = read(f["primary"].as_str().unwrap());
            let selected =
                prepare_with_parameters(&packet, 4, &sps, pps, &params, packet.len()).unwrap();
            let expected = prepare(&primary, 4, &sps, pps, primary.len()).unwrap();
            assert_eq!(selected.len(), f["slices"].as_u64().unwrap() as usize);
            for (a, b) in selected.iter().zip(&expected) {
                assert_eq!(a.header.redundant_pic_cnt, 0);
                assert_eq!(a.nal, b.nal);
                assert_eq!(a.macroblocks, b.macroblocks);
            }
            let error = prepare_with_parameters(&packet, 4, &sps, pps, &params, packet.len() - 1)
                .err()
                .unwrap();
            assert!(error.to_string().contains("byte limit"));
        }
    }
}
#[test]
fn invalid_redundant_headers_fail_before_reference_state_reuse() {
    let m = manifest();
    assert_eq!(m["invalid"].as_array().unwrap().len(), 5);
    for c in m["invalid"].as_array().unwrap() {
        let config = read(c["configuration"].as_str().unwrap());
        let mut d = AvcDecoder::new(&config, 1 << 20).unwrap();
        let seed = read("seed.packet");
        let good = read("good-p.packet");
        assert!(d.decode_order(&seed).unwrap().is_some());
        let error = d
            .decode_order(&read(c["file"].as_str().unwrap()))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["message"].as_str().unwrap()),
            "{}: {error}",
            c["file"]
        );
        assert!(
            d.decode_order(&good)
                .unwrap_err()
                .to_string()
                .contains("reset")
        );
        d.reset();
        assert!(d.decode_order(&seed).unwrap().is_some());
        let picture = d.decode_order(&good).unwrap().unwrap();
        assert_eq!(bytes(&picture), read("oracle.yuv")[..768]);
    }
}

#[test]
fn redundant_fixture_payloads_are_valid_distinct_pcm_areas() {
    use fvid::codec::{
        avc_slice::{SliceHeader, SliceType},
        bits::BitReader,
        config::NalUnits,
    };
    let m = manifest();
    let mut total = 0;
    for c in m["cases"].as_array().unwrap() {
        let cfg = read(c["configuration"].as_str().unwrap());
        let avc = AvcConfig::parse(&cfg).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pairs: Vec<_> = avc
            .pps
            .iter()
            .map(|nal| Pps::parse(nal, &sps).unwrap())
            .collect();
        for f in c["frames"].as_array().unwrap() {
            let packet = read(f["packet"].as_str().unwrap());
            let mut count = 0;
            for nal in NalUnits::new(&packet, 4).unwrap() {
                let nal = nal.unwrap();
                let id = SliceHeader::parameter_set_id(nal).unwrap();
                let pps = pairs.iter().find(|p| p.id == id).unwrap();
                let h = SliceHeader::parse(nal, &sps, pps).unwrap();
                if h.redundant_pic_cnt == 0 {
                    continue;
                }
                count += 1;
                assert_eq!(h.slice_type, SliceType::I);
                let mut bits = BitReader::new(&h.rbsp);
                bits.skip(h.entropy_bit_offset).unwrap();
                let mut mb = h.first_mb as usize;
                while bits.more_rbsp_data() {
                    assert!(mb < 2);
                    assert_eq!(bits.unsigned_golomb().unwrap(), 25);
                    while bits.position() % 8 != 0 {
                        assert_eq!(bits.read(1).unwrap(), 0);
                    }
                    for (plane, (bw, bh)) in [(16, 16), (8, 8), (8, 8)].into_iter().enumerate() {
                        for y in 0..bh {
                            for x in 0..bw {
                                let wanted =
                                    (((mb * bw + x) * 11 + y * 17 + plane * 41) % 251 + 1) as u32;
                                assert_eq!(
                                    bits.read(8).unwrap(),
                                    wanted,
                                    "{} rank{} mb{mb} plane{plane}",
                                    c["name"],
                                    h.redundant_pic_cnt
                                );
                            }
                        }
                    }
                    mb += 1;
                }
                assert_eq!(mb, 2);
                assert_eq!(bits.read(1).unwrap(), 1);
                while bits.remaining() != 0 {
                    assert_eq!(bits.read(1).unwrap(), 0);
                }
            }
            assert_eq!(count, f["redundants"].as_u64().unwrap() as usize);
            total += count;
        }
    }
    assert_eq!(total, 48);
}

#[test]
fn formerly_refused_redundant_only_picture_is_recovered() {
    let mut d = AvcDecoder::new(&read("invalid.avcc"), 1 << 20).unwrap();
    for _ in 0..2 {
        d.decode_order(&read("seed.packet")).unwrap().unwrap();
        let p = d.decode_order(&read("redundant-only.packet")).unwrap().unwrap();
        let expected: Vec<_> = read("oracle.yuv")[..768].iter().map(|v| v + 1).collect();
        assert_eq!(bytes(&p), expected);
        d.reset();
    }
}
