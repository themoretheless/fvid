//! Annex F header and header-to-DPB binding acceptance, not scalable playback.
use fvid::codec::{
    hevc_cabac::SliceType,
    hevc_inter_layer::PictureStatus,
    hevc_nal::NalHeader,
    hevc_pps::{LayeredPps, LayeredPpsContext},
    hevc_reference_list::{LayeredReference, PictureIdentity, ReferenceKind},
    hevc_slice::{LayeredSliceContext, SliceHeader},
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn layered_slice_headers_read_exact_boundaries_inherit_segments_and_bind_reference_pixels() {
    let m: serde_json::Value = serde_json::from_slice(&data("hevc-layered-slices.json")).unwrap();
    let source = &m;
    let atlas = data(m["atlas"].as_str().unwrap());
    let magic = b"YUV4MPEG2 W64 H32 F25:1 Ip A1:1 C420\n";
    assert!(atlas.starts_with(magic));
    let mut at = magic.len();
    let frames: Vec<[Vec<u8>; 3]> = source["pictures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            assert_eq!(&atlas[at..at + 6], b"FRAME\n");
            at += 6;
            let planes = std::array::from_fn(|c| {
                let n = if c == 0 { 2048 } else { 512 };
                let v = atlas[at..at + n].to_vec();
                at += n;
                v
            });
            assert_eq!(serde_json::to_value(&planes).unwrap(), p["planes"]);
            planes
        })
        .collect();
    assert_eq!(at, atlas.len());
    for c in m["cases"].as_array().unwrap() {
        let prefix = c["prefix"].as_str().unwrap();
        let vps = LayeredVps::parse(&data(c["vps"].as_str().unwrap()), 1 << 20)
            .unwrap_or_else(|e| panic!("{prefix}: VPS: {e}"));
        let sps = LayeredSps::parse(
            &data(c["sps"].as_str().unwrap()),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: c["layer"].as_u64().unwrap() as u8,
                output_layer_set: c["ols"].as_u64().unwrap() as usize,
                active_sps: &[],
            },
            1 << 20,
        )
        .unwrap_or_else(|e| panic!("{prefix}: SPS: {e}"));
        let pps = LayeredPps::parse(
            &data(c["pps"].as_str().unwrap()),
            &LayeredPpsContext {
                vps: &vps,
                sps: &sps,
                active_sps: &[],
                active_pps: &[],
            },
            1 << 20,
        )
        .unwrap_or_else(|e| panic!("{prefix}: PPS: {e}"));
        let ctx = LayeredSliceContext {
            vps: &vps,
            sps: &sps,
            pps: &pps,
        };
        let first = data(&format!("{prefix}.nal"));
        let next = c["next"].as_bool().unwrap();
        let bytes = if next {
            data(&format!("{prefix}.next.nal"))
        } else {
            first.clone()
        };
        let previous = if next {
            Some(
                SliceHeader::parse_layered(&first, &ctx, 1 << 20)
                    .unwrap_or_else(|e| panic!("{prefix}: first header: {e}")),
            )
        } else {
            None
        };
        let result = if next {
            SliceHeader::parse_layered_with_previous(
                &bytes,
                &ctx,
                1 << 20,
                if c["missing_previous"].as_bool().unwrap() {
                    None
                } else {
                    previous.as_ref()
                },
            )
        } else {
            SliceHeader::parse_layered(&bytes, &ctx, 1 << 20)
        };
        if let Some(error) = c["error"].as_str() {
            let e = result.unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)), "{prefix}: {e}");
            assert!(
                e.to_string().contains(error),
                "{prefix}: {e}; expected {error}"
            );
            continue;
        }
        let header = result.unwrap_or_else(|e| panic!("{prefix}: header: {e}"));
        if header.layered.is_none() {
            assert_eq!(c["config"].as_str().unwrap(), "single-layer");
            let ordinary = if next {
                SliceHeader::parse_with_previous(
                    &bytes,
                    &sps.sps,
                    &pps.pps,
                    1 << 20,
                    previous.as_ref(),
                )
                .unwrap()
            } else {
                SliceHeader::parse(&bytes, &sps.sps, &pps.pps, 1 << 20).unwrap()
            };
            assert_eq!(
                format!("{header:?}"),
                format!("{ordinary:?}"),
                "{prefix}: ordinary parsing must be unchanged"
            );
            continue;
        }
        let syntax = header.layered.as_ref().unwrap();
        let expected = &c["expected"];
        if let Some(reset) = expected.get("reset") {
            use fvid::codec::hevc_poc::Reset;
            let actual = syntax.poc_reset;
            let idc = match actual.reset {
                None => 0,
                Some(Reset::Msb) => 1,
                Some(Reset::Full) => 2,
                Some(Reset::Signalled { .. }) => 3,
            };
            assert_eq!(idc, reset["idc"].as_u64().unwrap(), "{prefix}: reset IDC");
            assert_eq!(
                serde_json::to_value(actual.period).unwrap(),
                reset["period"],
                "{prefix}: reset period"
            );
            assert_eq!(
                serde_json::to_value(actual.msb_cycle).unwrap(),
                reset["cycle"],
                "{prefix}: reset MSB cycle"
            );
            if let Some(Reset::Signalled { full, lsb }) = actual.reset {
                assert_eq!(
                    full,
                    reset["full"].as_bool().unwrap(),
                    "{prefix}: full reset"
                );
                assert_eq!(
                    u64::from(lsb),
                    reset["lsb"].as_u64().unwrap(),
                    "{prefix}: signalled LSB"
                );
            }
        }
        assert_eq!(
            serde_json::to_value(&syntax.reference_layers).unwrap(),
            expected["selected"],
            "{prefix}: RefPicLayerId"
        );
        assert_eq!(
            serde_json::to_value(syntax.inter_layer_prediction).unwrap(),
            expected["flag"],
            "{prefix}: prediction flag"
        );
        assert_eq!(
            serde_json::to_value(syntax.coded_count_minus1).unwrap(),
            expected["count"],
            "{prefix}: coded count"
        );
        assert_eq!(
            syntax.poc_lsb_present,
            expected["poc_present"].as_bool().unwrap(),
            "{prefix}"
        );
        assert_eq!(
            syntax.discardable,
            c["discardable"].as_bool().unwrap(),
            "{prefix}"
        );
        assert_eq!(
            syntax.cross_layer_bla,
            c["cross"].as_bool().unwrap(),
            "{prefix}"
        );
        assert_eq!(
            header.poc_lsb,
            if syntax.poc_lsb_present {
                expected["poc"].as_u64().unwrap_or(100) as u32
            } else {
                0
            },
            "{prefix}: POC LSB"
        );
        assert_eq!(header.qp, 27, "{prefix}: QP marker");
        assert_eq!(header.sao, [true, false], "{prefix}: SAO marker");
        assert_eq!(header.picture_output, true, "{prefix}: output marker");
        assert_eq!(header.first, !next, "{prefix}");
        assert_eq!(header.address, u32::from(next), "{prefix}");
        assert_eq!(
            header.dependent,
            c["dependent"].as_bool().unwrap(),
            "{prefix}"
        );
        assert_eq!(
            header.entropy_byte_offset,
            expected["entropy_offset"].as_u64().unwrap() as usize,
            "{prefix}: entropy position"
        );
        assert_eq!(
            &header.rbsp[header.entropy_byte_offset..],
            &[0xa5, 0x5a, 0x80],
            "{prefix}: opaque entropy marker"
        );
        assert_eq!(
            serde_json::to_value(&header.extension).unwrap(),
            expected["extension"],
            "{prefix}: extension"
        );
        let stype = match c["stype"].as_u64().unwrap() {
            0 => SliceType::B,
            1 => SliceType::P,
            _ => SliceType::I,
        };
        assert_eq!(header.slice_type, stype, "{prefix}");
        let r0 = c["refs"][0].as_u64().unwrap() as u8;
        let r1 = c["refs"][1].as_u64().unwrap() as u8;
        assert_eq!(
            header.references,
            if stype == SliceType::I {
                [0, 0]
            } else {
                [r0, if stype == SliceType::B { r1 } else { 0 }]
            },
            "{prefix}: active reference counts"
        );
        let modifications = if c["mods"].is_null() {
            serde_json::json!([null, null])
        } else {
            c["mods"].clone()
        };
        assert_eq!(
            serde_json::to_value(&header.list_modification).unwrap(),
            modifications,
            "{prefix}: list modifications"
        );
        assert_eq!(header.mvd_l1_zero, stype == SliceType::B, "{prefix}");
        assert_eq!(
            header.short_term.iter().filter(|r| r.used).count(),
            usize::from(c["temporal"].as_bool().unwrap() && !header.nal.is_idr()),
            "{prefix}: temporal RPS"
        );
        assert_eq!(
            header.max_merge_candidates,
            if stype == SliceType::I { 0 } else { 3 },
            "{prefix}"
        );
        assert_eq!(header.cabac_init, stype != SliceType::I, "{prefix}");
        assert_eq!(
            header.temporal_mvp,
            c["mvp"].as_bool().unwrap() && !header.nal.is_idr(),
            "{prefix}"
        );
        if c["weighted"].as_bool().unwrap() {
            let weights = header.weights.as_ref().unwrap();
            assert_eq!(weights.denominators, [1, 1]);
            for list in 0..if stype == SliceType::B { 2 } else { 1 } {
                assert_eq!(weights.lists[list].len(), 2, "{prefix}");
                for (i, w) in weights.lists[list].iter().enumerate() {
                    assert_eq!(w.values, [2; 3]);
                    assert_eq!(w.offsets, [if i % 2 == 0 { 1 } else { 0 }, 0, 0]);
                }
            }
            assert_eq!(header.collocated_list, usize::from(stype == SliceType::B));
            assert_eq!(header.collocated_ref, 1);
        }
        let original_sources: Vec<_> = source["pictures"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let s = &p["status"];
                let layer = s["layer"].as_u64().unwrap() as u8;
                let kind = if layer == 0 && c["source_irap"].as_bool().unwrap() {
                    21
                } else {
                    1
                };
                (
                    PictureStatus {
                        identity: PictureIdentity {
                            layer_id: layer,
                            poc: 100,
                        },
                        access_unit: s["au"].as_u64().unwrap(),
                        nal: NalHeader {
                            layer_id: layer,
                            unit_type: kind,
                            temporal_id: header.nal.temporal_id,
                        },
                        discardable: false,
                    },
                    &frames[i],
                )
            })
            .collect();
        let binding = header.bind_inter_layer_references(
            &ctx,
            41,
            header.poc_lsb as i32,
            &original_sources,
            1 << 20,
        );
        if let Some(error) = c["binding_error"].as_str() {
            let e = match binding {
                Err(e) => e,
                Ok(_) => panic!("{prefix}: binding unexpectedly accepted"),
            };
            assert!(e.to_string().contains(error), "{prefix}: binding: {e}");
            continue;
        }
        let binding = binding.unwrap_or_else(|e| panic!("{prefix}: binding: {e}"));
        let selected: Vec<_> = binding.pictures().into_iter().flatten().copied().collect();
        assert_eq!(selected.len(), syntax.reference_layers.len());
        for id in selected {
            let frame = [0u8, 2, 5, 9]
                .iter()
                .position(|&layer| layer == id.layer_id)
                .unwrap()
                + 4;
            let actual = binding
                .resolve(LayeredReference {
                    picture: id,
                    kind: ReferenceKind::InterLayer,
                })
                .unwrap();
            assert!(
                std::ptr::eq(*actual, &frames[frame]),
                "{prefix}: frame was copied or selected from wrong AU"
            );
            assert_eq!(**actual, frames[frame], "{prefix}: exact source samples");
        }
        // Source-buffer ownership stays with the caller. Header and new binding
        // allocations are retained together, including later list construction.
        let binding_only = 2
            * syntax.reference_layers.len()
            * (std::mem::size_of::<PictureIdentity>() + std::mem::size_of::<&[Vec<u8>; 3]>())
            + 4 * std::mem::size_of::<Vec<u8>>();
        assert!(
            header
                .bind_inter_layer_references(
                    &ctx,
                    41,
                    header.poc_lsb as i32,
                    &original_sources,
                    binding_only
                )
                .is_err(),
            "{prefix}: retained header budget was ignored"
        );
        assert!(
            header.storage_bytes().unwrap()
                >= std::mem::size_of::<SliceHeader>() + header.rbsp.len()
        );
        let tiny =
            header.rbsp.len() + std::mem::size_of::<Vec<u8>>() + syntax.reference_layers.len() - 1;
        if !next {
            let e = SliceHeader::parse_layered(&bytes, &ctx, tiny).unwrap_err();
            assert!(
                e.to_string().contains("metadata exceeds memory budget"),
                "{prefix}: {e}"
            );
        }
        if header.nal.layer_id != 0 {
            let e = SliceHeader::parse(&bytes, &sps.sps, &pps.pps, 1 << 20).unwrap_err();
            assert!(
                e.to_string()
                    .contains("multilayer decoding is not implemented"),
                "{prefix}: ordinary parser refusal: {e}"
            );
        }
        if header.dependent && header.nal.layer_id == 0 {
            let ordinary = SliceHeader::parse(&first, &sps.sps, &pps.pps, 1 << 20).unwrap();
            assert!(ordinary.layered.is_none());
            let e =
                SliceHeader::parse_layered_with_previous(&bytes, &ctx, 1 << 20, Some(&ordinary))
                    .unwrap_err();
            assert!(
                e.to_string().contains("disagrees with preceding header"),
                "{prefix}: {e}"
            );
        }
        let mut wrong = pps.clone();
        wrong.vps_id ^= 1;
        assert!(
            SliceHeader::parse_layered(
                &first,
                &LayeredSliceContext {
                    vps: &vps,
                    sps: &sps,
                    pps: &wrong
                },
                1 << 20
            )
            .is_err()
        );
    }
}
