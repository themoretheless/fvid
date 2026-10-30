use fvid_media::owned_aac::{
    aac_ld_mps_entropy::Kind, aac_ld_mps_parameters::Parameters, bits::BitReader,
};
use serde_json::Value;
use std::path::PathBuf;
fn corpus() -> (Value, Vec<u8>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ld-mps-parameters");
    (
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap(),
        std::fs::read(root.join("packets.bin")).unwrap(),
    )
}
fn state(c: &Value) -> Parameters {
    Parameters::new(
        match c["kind"].as_str().unwrap() {
            "CLD" => Kind::Cld,
            "ICC" => Kind::Icc,
            "CPC" => Kind::Cpc,
            _ => unreachable!(),
        },
        c["bands"].as_u64().unwrap() as usize,
        c["default"].as_i64().unwrap() as i16,
    )
    .unwrap()
}
fn slots(f: &Value) -> Vec<u8> {
    f["slots"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_u64().unwrap() as u8)
        .collect()
}
fn expected(f: &Value) -> Vec<Vec<i16>> {
    f["expected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            r.as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap() as i16)
                .collect()
        })
        .collect()
}
fn shifted(blob: &[u8], f: &Value, at: usize) -> Vec<u8> {
    let offset = f["offset"].as_u64().unwrap() as usize;
    let n = f["bits"].as_u64().unwrap() as usize;
    let mut bytes = vec![0; (at + n + 8).div_ceil(8)];
    for i in 0..n {
        bytes[(at + i) / 8] |= ((blob[offset + i / 8] >> (7 - i % 8)) & 1) << (7 - (at + i) % 8);
    }
    for i in 0..8 {
        bytes[(at + n + i) / 8] |= ((0x5au8 >> (7 - i)) & 1) << (7 - (at + n + i) % 8);
    }
    bytes
}
#[test]
fn saved_sequences_cover_modes_strides_quantization_and_checkpoint_replay() {
    let (manifest, blob) = corpus();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 552);
    for c in cases {
        for at in 0..8 {
            let initial = state(c);
            let mut decoder = initial.clone();
            for f in c["frames"].as_array().unwrap() {
                let saved = decoder.clone();
                let bytes = shifted(&blob, f, at);
                let mut bits = BitReader::new(&bytes);
                bits.skip(at).unwrap();
                let rows = decoder
                    .decode(
                        &mut bits,
                        &slots(f),
                        16,
                        f["independent"].as_bool().unwrap(),
                    )
                    .unwrap_or_else(|e| panic!("{} {:?}: {e}", c["name"], f["modes"]));
                assert_eq!(rows, expected(f), "{} {:?}", c["name"], f["modes"]);
                assert_eq!(bits.position(), at + f["bits"].as_u64().unwrap() as usize);
                assert_eq!(bits.read(8).unwrap(), 0x5a);
                let mut replay = saved;
                let mut bits = BitReader::new(&bytes);
                bits.skip(at).unwrap();
                assert_eq!(
                    replay
                        .decode(
                            &mut bits,
                            &slots(f),
                            16,
                            f["independent"].as_bool().unwrap()
                        )
                        .unwrap(),
                    rows
                );
                assert_eq!(replay, decoder, "checkpoint replay state");
            }
            decoder.reset();
            assert_eq!(
                decoder, initial,
                "reset includes both quantization histories"
            );
        }
    }
}
#[test]
fn leading_interpolation_keeps_first_anchor_and_final_slot_limits_are_accepted() {
    let (manifest, blob) = corpus();
    for c in manifest["cases"].as_array().unwrap() {
        let seed = &c["frames"][0];
        let bytes = shifted(&blob, seed, 0);
        let mut decoder = state(c);
        let rows = decoder
            .decode(&mut BitReader::new(&bytes), &[0, 31], 32, true)
            .unwrap();
        let previous = rows[1].clone();
        for (packet, right) in [
            (0x90, previous.clone()),
            (
                0x80,
                vec![c["default"].as_i64().unwrap() as i16; previous.len()],
            ),
        ] {
            let mut candidate = decoder.clone();
            let mut bits = BitReader::new(std::slice::from_ref(&packet));
            assert_eq!(
                candidate.decode(&mut bits, &[0, 31], 32, false).unwrap(),
                vec![previous.clone(), right]
            );
            assert_eq!(bits.position(), 4);
        }
        assert_eq!(
            state(c)
                .decode(&mut BitReader::new(&[0]), &[0], 1, true)
                .unwrap(),
            vec![vec![c["default"].as_i64().unwrap() as i16; previous.len()]]
        );
    }
}
#[test]
fn every_block_prefix_preserves_cursor_and_both_histories() {
    let (manifest, blob) = corpus();
    let mut checked = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let mut decoder = state(c);
        for f in c["frames"].as_array().unwrap() {
            let n = f["bits"].as_u64().unwrap() as usize;
            for at in [0, 3, 7] {
                let bytes = shifted(&blob, f, at);
                for end in 0..n {
                    let mut bits = BitReader::with_bit_len(&bytes, at + end).unwrap();
                    bits.skip(at).unwrap();
                    let mut candidate = decoder.clone();
                    assert!(
                        candidate
                            .decode(
                                &mut bits,
                                &slots(f),
                                16,
                                f["independent"].as_bool().unwrap()
                            )
                            .is_err(),
                        "{} {:?} prefix {end}/{n}",
                        c["name"],
                        f["modes"]
                    );
                    assert_eq!(bits.position(), at);
                    assert_eq!(candidate, decoder);
                    checked += 1;
                }
            }
            let bytes = shifted(&blob, f, 0);
            decoder
                .decode(
                    &mut BitReader::new(&bytes),
                    &slots(f),
                    16,
                    f["independent"].as_bool().unwrap(),
                )
                .unwrap();
        }
    }
    assert_eq!(checked, 280_719);
    eprintln!("{checked} stateful truncated prefixes checked");
}
#[test]
fn invalid_modes_pair_geometry_and_slots_are_transactional() {
    let initial = Parameters::new(Kind::Cld, 5, 15).unwrap();
    // modes keep/interpolate forbidden on independent first set; final
    // interpolation forbidden; a pair needs a second coded data set.
    for (packet, count, independent, reason) in [
        (0x40, 1, true, "independent LD-MPS set requires"),
        (0x80, 1, true, "independent LD-MPS set requires"),
        (0x80, 1, false, "interpolation lacks a following anchor"),
        (0xe0, 1, false, "data pair lacks a second coded set"),
        (0xd8, 2, false, "data pair lacks a second coded set"),
        (0xc8, 2, false, "data pair lacks a second coded set"),
        (0xe0, 2, false, "interpolation lacks a following anchor"),
    ] {
        let mut s = initial.clone();
        let mut b = BitReader::new(std::slice::from_ref(&packet));
        let slots: &[u8] = if count == 1 { &[15] } else { &[7, 15] };
        let error = s
            .decode(&mut b, slots, 16, independent)
            .unwrap_err()
            .to_string();
        assert!(error.contains(reason), "{packet:x}: {error}");
        assert_eq!(s, initial);
        assert_eq!(b.position(), 0);
    }
    for slots in [vec![], vec![7, 7], vec![15, 7], vec![16], vec![0, 1, 2]] {
        let mut s = initial.clone();
        let mut b = BitReader::new(&[0xff]);
        assert!(s.decode(&mut b, &slots, 16, false).is_err());
        assert_eq!(s, initial);
        assert_eq!(b.position(), 0);
    }
    for frame_slots in [0, 33, 255] {
        let mut s = initial.clone();
        let mut b = BitReader::new(&[0]);
        assert!(s.decode(&mut b, &[0], frame_slots, false).is_err());
        assert_eq!(s, initial);
        assert_eq!(b.position(), 0);
    }
    assert!(Parameters::new(Kind::Cld, 0, 0).is_ok());
    assert!(Parameters::new(Kind::Icc, 24, 0).is_err());
    assert!(Parameters::new(Kind::Icc, 2, -1).is_err());
    assert!(Parameters::new(Kind::Cpc, 2, 31).is_err());
}
