use fvid::codec::{
    avc::{Pps, Sps},
    avc_decoder::AvcDecoder,
    avc_picture::IntraPicture,
    config::AvcConfig,
};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-redundant-recovery")
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
fn lowest_complete_redundant_picture_replaces_missing_primary_and_becomes_reference() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 84);
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
#[test]
fn preparation_preserves_wire_rank_and_selects_complete_representation_with_bounds() {
    use fvid::codec::{avc_access_unit::prepare_for_decode, avc_slice::SliceHeader};
    let cfg = read("configuration.avcc");
    let avc = AvcConfig::parse(&cfg).unwrap();
    let sps = Sps::parse(avc.sps[0]).unwrap();
    let bank: Vec<_> = avc
        .pps
        .iter()
        .map(|n| (sps.clone(), Pps::parse(n, &sps).unwrap()))
        .collect();
    for c in manifest()["cases"].as_array().unwrap() {
        let index = c["recovery_index"].as_u64().unwrap_or(1) as usize;
        let packet = read(c["frames"][index]["packet"].as_str().unwrap());
        // The initial PPS comes from the first received slice, as in AvcDecoder.
        let first = fvid::codec::config::NalUnits::new(&packet, 4)
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let id = SliceHeader::parameter_set_id(first).unwrap();
        let pps = &bank.iter().find(|(_, p)| p.id == id).unwrap().1;
        assert!(
            fvid::codec::avc_access_unit::prepare_with_parameters(
                &packet,
                4,
                &sps,
                pps,
                &bank,
                packet.len()
            )
            .err()
            .unwrap()
            .to_string()
            .contains("primary coded picture")
        );
        let p = prepare_for_decode(&packet, 4, &sps, pps, &bank, packet.len(), 1 << 20).unwrap();
        assert_eq!(
            u64::from(p.source_redundant_pic_cnt),
            c["rank"].as_u64().unwrap()
        );
        assert_eq!(u64::from(p.pps_id), c["pps"].as_u64().unwrap());
        for s in p.slices {
            assert_eq!(s.header.redundant_pic_cnt, 0);
            let wire = SliceHeader::parse(s.nal, &sps, &bank[p.pps_id as usize].1).unwrap();
            let ranks: Vec<u64> = c["ranks"]
                .as_array()
                .map(|a| a.iter().map(|v| v.as_u64().unwrap()).collect())
                .unwrap_or_else(|| vec![c["rank"].as_u64().unwrap()]);
            assert!(
                ranks.contains(&u64::from(wire.redundant_pic_cnt)),
                "{} rank{}",
                c["name"],
                wire.redundant_pic_cnt
            );
            assert_eq!(wire.entropy_bit_offset, s.header.entropy_bit_offset);
        }
        assert!(
            prepare_for_decode(&packet, 4, &sps, pps, &bank, packet.len() - 1, 1 << 20)
                .err()
                .unwrap()
                .to_string()
                .contains("byte limit")
        );
        assert!(prepare_for_decode(&packet, 4, &sps, pps, &bank, packet.len(), 1).is_err());
    }
}
#[test]
fn incomplete_or_foreign_representations_are_refused_and_reset_restores_decoder() {
    for c in manifest()["invalid"].as_array().unwrap() {
        let mut d = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
        let seed = read("seed.packet");
        d.decode_order(&seed).unwrap();
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
            d.decode_order(&seed)
                .unwrap_err()
                .to_string()
                .contains("reset")
        );
        d.reset();
        assert!(d.decode_order(&seed).unwrap().is_some());
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
fn formerly_refused_disjoint_ranks_form_a_complete_picture() {
    let mut d = AvcDecoder::new(&read("configuration.avcc"), 1 << 20).unwrap();
    let seed = d.decode_order(&read("seed.packet")).unwrap().unwrap();
    let actual = d
        .decode_order(&read("disjoint-ranks.packet"))
        .unwrap()
        .unwrap();
    assert_eq!(pixels(&actual), pixels(&seed));
}
