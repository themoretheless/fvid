use fvid::codec::avc_decoder::AvcDecoder;
use serde_json::Value;
fn manifest() -> Value {
    let mut base: Value = serde_json::from_slice(&fixture("avc-switching-fields.json")).unwrap();
    for name in [
        "avc-mbaff-mixed-si-inter-reproducer.json",
        "avc-mbaff-mixed-si-inter.json",
        "avc-mbaff-si-filter.json",
        "avc-mbaff-sp-filter-offsets.json",
        "avc-mbaff-switching-filter.json",
        "avc-mbaff-sp-motion-chroma.json",
        "avc-mbaff-si-chroma.json",
        "avc-si-mbaff.json",
        "avc-switching-mbaff.json",
        "avc-mixed-pb-fields.json",
        "avc-switching-field-motion.json",
        "avc-switching-field-filter.json",
        "avc-switching-field-rows.json",
        "avc-mixed-sp-fields.json",
        "avc-uniform-type-fields.json",
        "avc-mixed-intra-sp-fields.json",
        "avc-mixed-si-sp-fields.json",
        "avc-mixed-intra-b-fields.json",
        "avc-mixed-cabac-intra-p-fields.json",
        "avc-mixed-cabac-intra-b-fields.json",
    ] {
        let extra: Value = serde_json::from_slice(&fixture(name)).unwrap();
        base["cases"]
            .as_array_mut()
            .unwrap()
            .extend(extra["cases"].as_array().unwrap().iter().cloned());
    }
    base
}

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(s: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(s),
    )
    .unwrap()
}
#[test]
fn complementary_sp_si_fields_match_normative_pixels_and_reset() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = AvcDecoder::new(&hex(c["configuration"].as_str().unwrap()), 4 << 20).unwrap();
        let expected = fixture(c["reference"].as_str().unwrap());
        for _ in 0..2 {
            let mut actual = vec![];
            let mut emitted = 0;
            for (i, p) in c["packets"].as_array().unwrap().iter().enumerate() {
                let picture = d
                    .decode(&hex(p.as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} packet{i}: {e}", c["file"]));
                assert_eq!(
                    picture.is_some(),
                    c["frame_picture"].as_bool().unwrap_or(false) || i % 2 == 1,
                    "{} packet{i}",
                    c["file"]
                );
                if let Some(picture) = picture {
                    assert_eq!(
                        picture.dimensions(),
                        (
                            c["width"].as_u64().unwrap_or(32) as usize,
                            c["height"].as_u64().unwrap_or(32) as usize
                        )
                    );
                    picture.write_planar(&mut actual).unwrap();
                    emitted += 1;
                }
            }
            assert_eq!(emitted, c["frame_count"].as_u64().unwrap());
            let difference = actual.iter().zip(&expected).position(|(a, b)| a != b);
            assert_eq!(actual.len(), expected.len());
            assert!(
                difference.is_none(),
                "{} first differing byte {:?}",
                c["file"],
                difference
            );
            assert!(!d.has_pending_field());
            d.reset();
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn switching_field_mp4_software_seek_and_rewind_emit_complete_pairs() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(fixture(c["file"].as_str().unwrap())),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        let expected = fixture(c["reference"].as_str().unwrap());
        for target in [None, Some(2), Some(5), None] {
            if let Some(target) = target {
                assert_eq!(reader.seek_to_sync(target), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            let mut count = 0;
            while let Some(f) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{}: {e}", c["file"]))
            {
                assert_eq!(
                    f.picture.dimensions(),
                    (
                        c["width"].as_u64().unwrap_or(32) as usize,
                        c["height"].as_u64().unwrap_or(32) as usize
                    )
                );
                actual.extend(
                    f.picture
                        .y
                        .iter()
                        .chain(&f.picture.cb)
                        .chain(&f.picture.cr)
                        .map(|v| *v as u8),
                );
                count += 1;
            }
            assert_eq!(count, c["frame_count"].as_u64().unwrap());
            assert_eq!(actual, expected, "{}", c["file"]);
        }
    }
}

#[test]
fn mbaff_sp_si_public_inter_reconstruction_matches_oracle_and_refuses_non_extended() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_decoder::AvcDecoder,
        avc_inter_picture::{
            decode_inter_picture_with_motion, decode_inter_resolved_slices_with_motion,
            decode_inter_slices_with_motion,
        },
        avc_slice::{SliceHeader, SliceType},
        config::{AvcConfig, NalUnits},
    };
    let repro: Value =
        serde_json::from_slice(&fixture("avc-mbaff-mixed-si-inter-reproducer.json")).unwrap();
    for c in repro["cases"].as_array().unwrap() {
        let cfg_bytes = hex(c["configuration"].as_str().unwrap());
        let cfg = AvcConfig::parse(&cfg_bytes).unwrap();
        let sps = Sps::parse(cfg.sps[0]).unwrap();
        let pps = Pps::parse(cfg.pps[0], &sps).unwrap();
        let expected = fixture(c["reference"].as_str().unwrap());
        let frame_bytes = 32 * 64 * 3 / 2;
        let packets = c["packets"].as_array().unwrap();
        let mut dec = AvcDecoder::new(&cfg_bytes, 4 << 20).unwrap();
        let mut previous = dec
            .decode(&hex(packets[0].as_str().unwrap()))
            .unwrap()
            .unwrap();
        for (idx, pkt_hex) in packets.iter().enumerate().skip(1) {
            let pkt = hex(pkt_hex.as_str().unwrap());
            let mut headers = Vec::new();
            for nal in NalUnits::new(&pkt, cfg.length_size).unwrap() {
                headers.push(SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap());
            }
            headers.sort_by_key(|h| h.first_mb);
            let header_refs: Vec<&SliceHeader> = headers.iter().collect();
            let prev_list = [previous.as_ref()];
            let refs: Vec<[&[&fvid::codec::avc_picture::IntraPicture]; 2]> = header_refs
                .iter()
                .map(|h| {
                    if matches!(h.slice_type, SliceType::I | SliceType::Si) {
                        [&[][..], &[][..]]
                    } else {
                        [&prev_list[..], &[][..]]
                    }
                })
                .collect();
            let direct = vec![None; header_refs.len()];
            let (picture, _) = decode_inter_resolved_slices_with_motion(
                &header_refs,
                &sps,
                &pps,
                &refs,
                &direct,
                4 << 20,
            )
            .unwrap();
            let mut planar = Vec::new();
            picture.write_planar(&mut planar).unwrap();
            assert_eq!(
                planar.as_slice(),
                &expected[idx * frame_bytes..(idx + 1) * frame_bytes],
                "{} frame {idx}",
                c["file"]
            );
            if idx == 1 {
                let mut bad_sps = sps.clone();
                bad_sps.profile = 100;
                let err = decode_inter_resolved_slices_with_motion(
                    &header_refs,
                    &bad_sps,
                    &pps,
                    &refs,
                    &direct,
                    4 << 20,
                )
                .err()
                .unwrap();
                assert_eq!(
                    err.to_string(),
                    "AVC SP/SI requires eight-bit Extended-profile 4:2:0 CAVLC"
                );
            }
            previous = std::sync::Arc::new(picture);
        }
    }
    let sp_manifest: Value = serde_json::from_slice(&fixture("avc-switching-mbaff.json")).unwrap();
    let sp_case = &sp_manifest["cases"].as_array().unwrap()[0];
    let cfg_bytes = hex(sp_case["configuration"].as_str().unwrap());
    let cfg = AvcConfig::parse(&cfg_bytes).unwrap();
    let sps = Sps::parse(cfg.sps[0]).unwrap();
    let pps = Pps::parse(cfg.pps[0], &sps).unwrap();
    let expected = fixture(sp_case["reference"].as_str().unwrap());
    let packets = sp_case["packets"].as_array().unwrap();
    let mut dec = AvcDecoder::new(&cfg_bytes, 4 << 20).unwrap();
    let anchor = dec
        .decode(&hex(packets[0].as_str().unwrap()))
        .unwrap()
        .unwrap();
    let pkt1 = hex(packets[1].as_str().unwrap());
    let mut headers = Vec::new();
    for nal in NalUnits::new(&pkt1, cfg.length_size).unwrap() {
        headers.push(SliceHeader::parse(nal.unwrap(), &sps, &pps).unwrap());
    }
    headers.sort_by_key(|h| h.first_mb);
    let header_refs: Vec<&SliceHeader> = headers.iter().collect();
    let anchor_list = [anchor.as_ref()];
    let (pic, _) = decode_inter_slices_with_motion(
        &header_refs,
        &sps,
        &pps,
        [&anchor_list[..], &[][..]],
        None,
        4 << 20,
    )
    .unwrap();
    let mut planar = Vec::new();
    pic.write_planar(&mut planar).unwrap();
    assert_eq!(planar.as_slice(), &expected[1536..3072]);
    let _ = decode_inter_picture_with_motion;
}

