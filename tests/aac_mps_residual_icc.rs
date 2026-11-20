//! Residual ICC mapping/matrices only; native residual playback remains refused.
use fvid_media::owned_aac::{
    aac_ld_mps_assemble,
    aac_ld_mps_config::{Config, Extension, ExtensionValue},
    aac_ld_mps_entropy::Kind,
    aac_ld_mps_frame::ParameterId,
    aac_ld_mps_parameters::Parameters,
    aac_ld_mps_residual::{BoxData, Frame},
    aac_ld_mps_residual_icc as correction,
    bits::BitReader,
};
use serde_json::Value;
fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-mps-residual-icc")
}
fn n(v: &Value) -> usize {
    v.as_u64().unwrap() as usize
}
fn json(name: &str) -> Value {
    serde_json::from_slice(&std::fs::read(root().join(name)).unwrap()).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}
fn rows(v: &Value) -> Vec<Vec<i16>> {
    v.as_array()
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
#[test]
fn quantization_tags_and_residual_indices_match_unmodified_iso_without_history_pollution() {
    let m = json("parameters.json");
    let oracle = json("reference.json");
    let packets = std::fs::read(root().join("parameters.bin")).unwrap();
    let mut at = 0;
    let mut values = 0;
    for c in m["cases"].as_array().unwrap() {
        let mut decoder = Parameters::new(Kind::Icc, n(&c["bands"]), 0).unwrap();
        let mut legacy = decoder.clone();
        for f in c["frames"].as_array().unwrap() {
            let data = &packets[n(&f["offset"])..n(&f["offset"]) + n(&f["bytes"])];
            let mut b = BitReader::with_bit_len(data, n(&f["bits"])).unwrap();
            let saved = decoder.clone();
            let decoded = decoder
                .decode_with_quantization(&mut b, &[7, 15], 16, f["independent"].as_bool().unwrap())
                .unwrap();
            assert_eq!(b.position(), n(&f["bits"]));
            assert_eq!(decoded.values, rows(&f["base"]));
            assert_eq!(
                decoded.coarse,
                f["coarse"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_bool().unwrap())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                legacy
                    .decode(
                        &mut BitReader::new(data),
                        &[7, 15],
                        16,
                        f["independent"].as_bool().unwrap()
                    )
                    .unwrap(),
                decoded.values
            );
            assert_eq!(decoder, legacy);
            let mut corrected = decoded.values.clone();
            for (set, row) in corrected.iter_mut().enumerate() {
                for (band, value) in row.iter_mut().enumerate() {
                    *value = correction::index(
                        *value,
                        decoded.coarse[set],
                        n(&f["differences"][set][band]) as u8,
                    )
                    .unwrap();
                    values += 1;
                }
            }
            let expected = &oracle["frames"][at];
            assert_eq!(corrected, rows(&expected["rows"]));
            assert_eq!(n(&expected["bits"]), n(&f["bits"]));
            at += 1;
            let mut bad = BitReader::with_bit_len(data, n(&f["bits"]) - 1).unwrap();
            let mut retry = saved.clone();
            assert!(
                retry
                    .decode_with_quantization(
                        &mut bad,
                        &[7, 15],
                        16,
                        f["independent"].as_bool().unwrap()
                    )
                    .is_err()
            );
            assert_eq!(bad.position(), 0);
            assert_eq!(retry, saved);
            assert_eq!(
                retry
                    .decode_with_quantization(
                        &mut BitReader::new(data),
                        &[7, 15],
                        16,
                        f["independent"].as_bool().unwrap()
                    )
                    .unwrap(),
                decoded
            );
            assert_eq!(retry, decoder);
        }
    }
    assert_eq!(at, 84);
    eprintln!("Residual ICC original-reference frames {at}, corrected indices {values}");
}
#[test]
fn invalid_residual_icc_domains_are_rejected_instead_of_clamped() {
    for coarse in [false, true] {
        for base in -1..=8 {
            for diff in 0..=8 {
                let expected = base >= 0
                    && base <= 7
                    && diff <= 7
                    && (!coarse || base % 2 == 0 && base <= 6)
                    && base + i16::from(diff) * if coarse { 2 } else { 1 } <= 7;
                assert_eq!(correction::index(base, coarse, diff).is_ok(), expected);
            }
        }
    }
}
#[test]
fn corrected_shared_and_per_box_matrices_match_explicit_fine_controls_and_rollback() {
    let m = json("matrix.json");
    let packets = std::fs::read(root().join("matrix-packets.bin")).unwrap();
    let mut matrices = 0;
    let mut changed = 0;
    let mut shared = 0;
    let mut held = 0;
    for c in m["cases"].as_array().unwrap() {
        let mut cfg = Config::parse(&hex(c["config"].as_str().unwrap())).unwrap();
        assert!(cfg.extensions.is_empty());
        let bands = cfg
            .tree
            .ott_lfe
            .iter()
            .map(|&lfe| if lfe { None } else { Some(1) })
            .chain(std::iter::repeat_n(None, usize::from(cfg.tree.ttt_boxes)))
            .collect::<Vec<_>>();
        let extension = ExtensionValue::Residual {
            rate: cfg.sample_rate,
            frames_code: 0,
            bands: bands.clone(),
        };
        let mut payload_bits = format!("{:04b}00", if cfg.sample_rate == 24000 { 6 } else { 3 });
        for band in &bands {
            payload_bits.push_str(if band.is_some() { "100001" } else { "0" });
        }
        while payload_bits.len() % 8 != 0 {
            payload_bits.push('0');
        }
        let payload = payload_bits
            .as_bytes()
            .chunks_exact(8)
            .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 2).unwrap())
            .collect();
        cfg.extensions.push(Extension {
            kind: 0,
            payload,
            value: extension.clone(),
        });
        let mut control_cfg = cfg.clone();
        if cfg.tree.code != 7 {
            control_cfg.one_icc = Some(false);
        }
        let mut decoder = aac_ld_mps_assemble::Decoder::new(&cfg).unwrap();
        let mut control = aac_ld_mps_assemble::Decoder::new(&control_cfg).unwrap();
        let mut baseline = aac_ld_mps_assemble::Decoder::new(&cfg).unwrap();
        for row in c["frames"].as_array().unwrap() {
            let data = &packets[n(&row["offset"])..n(&row["offset"]) + n(&row["bytes"])];
            let residual = Frame {
                boxes: bands
                    .iter()
                    .enumerate()
                    .map(|(box_index, band)| {
                        band.map(|_| BoxData {
                            icc_differences: row["differences"][box_index]
                                .as_array()
                                .unwrap()
                                .iter()
                                .map(|v| Some(vec![n(v) as u8]))
                                .collect(),
                            blocks: vec![],
                        })
                    })
                    .collect(),
            };
            let before = decoder.clone();
            let mut bits = BitReader::new(data);
            let actual = decoder
                .decode_residual_icc(&mut bits, &extension, &residual)
                .unwrap();
            let ordinary = baseline.decode(&mut BitReader::new(data)).unwrap();
            assert_eq!(actual.prepared.syntax, ordinary.prepared.syntax);
            assert_eq!(bits.remaining(), 0);
            let expected = control
                .decode(&mut BitReader::new(&hex(row["control"].as_str().unwrap())))
                .unwrap();
            assert_eq!(
                actual.m1, expected.m1,
                "tree{} coarse{} held{} M1",
                c["tree"], c["coarse"], c["held"]
            );
            assert_eq!(actual.dry, expected.dry);
            assert_eq!(actual.wet, expected.wet);
            matrices += 1;
            if actual.m1 != ordinary.m1 || actual.dry != ordinary.dry || actual.wet != ordinary.wet
            {
                changed += 1;
            }
            let again = correction::apply(&cfg, &extension, &actual.prepared, &residual).unwrap();
            assert_eq!(again, actual.prepared);
            for (box_index, lfe) in cfg.tree.ott_lfe.iter().enumerate() {
                if *lfe {
                    continue;
                }
                let level = actual
                    .prepared
                    .parameters
                    .iter()
                    .find(|p| p.id == ParameterId::OttIcc(Some(box_index as u8)))
                    .unwrap();
                let expected = rows(&row["corrected"][box_index.to_string()]);
                assert_eq!(&level.indices[..2], expected.as_slice());
                if c["held"].as_bool().unwrap() {
                    assert_eq!(level.indices[2], expected[1]);
                    held += 1;
                }
            }
            if cfg.one_icc == Some(true) {
                shared += 1;
                assert!(
                    actual
                        .prepared
                        .parameters
                        .iter()
                        .any(|p| p.id == ParameterId::OttIcc(None))
                );
            }
            let mut bad = residual.clone();
            let active = bad.boxes.iter_mut().flatten().next().unwrap();
            active.icc_differences[1] = Some(vec![8]);
            let mut retry = before.clone();
            let mut b = BitReader::new(data);
            assert!(retry.decode_residual_icc(&mut b, &extension, &bad).is_err());
            assert_eq!(retry, before);
            assert_eq!(b.position(), 0);
            assert_eq!(
                retry
                    .decode_residual_icc(&mut BitReader::new(data), &extension, &residual)
                    .unwrap(),
                actual
            );
            assert_eq!(retry, decoder);
        }
    }
    assert_eq!(m["cases"].as_array().unwrap().len(), 192);
    assert!(shared > 0 && held > 0);
    assert!(changed > 0, "corrections must affect the matrix path");
    eprintln!(
        "Residual ICC matrix controls {matrices}, shared frames {shared}, held OTT endpoints {held}, changed matrices {changed}"
    );
}

#[test]
fn compressed_native_candidate_residual_prefix_reaches_the_same_icc_correction() {
    use fvid_media::owned_aac::{aac_ld_mps_prepare, aac_ld_mps_residual};
    let parent = root().parent().unwrap().join("aac-ld-mps-residual");
    let cfg = Config::parse(&std::fs::read(parent.join("native.ssc")).unwrap()).unwrap();
    let metadata: Value =
        serde_json::from_slice(&std::fs::read(parent.join("manifest.json")).unwrap()).unwrap();
    let ext = &cfg.extensions[0].value;
    let mut decoder = aac_ld_mps_prepare::Decoder::new(&cfg).unwrap();
    for row in metadata["native"]["frames"].as_array().unwrap() {
        let wire = hex(row["spatial"].as_str().unwrap());
        let prepared = decoder.decode(&mut BitReader::new(&wire)).unwrap();
        let mut bits = BitReader::new(&prepared.syntax.extensions[0].payload);
        let residual =
            aac_ld_mps_residual::read(&mut bits, &cfg, ext, prepared.syntax.slots.len()).unwrap();
        let corrected = correction::apply(&cfg, ext, &prepared, &residual).unwrap();
        let raw = prepared
            .syntax
            .parameters
            .iter()
            .find(|p| p.id == ParameterId::OttIcc(Some(0)))
            .unwrap();
        assert_eq!(raw.coarse, vec![false, false]);
        assert_eq!(raw.values[0][0], 4);
        assert_eq!(raw.values[1][0], 1);
        let physical = corrected
            .parameters
            .iter()
            .find(|p| p.id == ParameterId::OttIcc(Some(0)))
            .unwrap();
        assert_eq!(physical.indices[0][0], 7);
        assert_eq!(physical.indices[1][0], 1);
        assert_eq!(physical.values[0][0], -0.99);
        assert_eq!(physical.values[1][0], 0.937);
        assert_eq!(corrected.syntax, prepared.syntax);
    }
}
