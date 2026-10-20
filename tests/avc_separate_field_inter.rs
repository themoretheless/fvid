use fvid::codec::{
    avc::{Pps, Sps},
    avc_decoder::AvcDecoder,
    avc_field_dpb::FieldBuffer,
    avc_field_picture::{
        ImplicitFieldWeights, PcmField, decode_inter_field_lists_with_motion,
        decode_inter_field_lists_with_order, decode_intra_slices, decode_p_field,
        decode_skip_field, weave_pair,
    },
    avc_slice::{SliceHeader, SliceType},
    config::{AvcConfig, NalUnits},
};
use std::path::Path;

fn bytes(values: impl IntoIterator<Item = u16>, depth: u8) -> Vec<u8> {
    let mut out = Vec::new();
    for v in values {
        if depth == 8 {
            out.push(v as u8);
        } else {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

#[test]
fn separate_plane_paff_inter_fields_decode_via_decoder_mp4_and_direct_entrypoints() {
    use fvid::{container::mp4::Limits, playback::video::mp4::Mp4VideoReader};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-field-inter");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 288);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let decode_order: Vec<usize> = case["decode_order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as usize)
            .collect();
        let pair_count = decode_order.len();
        let field_count = pair_count * 2;
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        assert_eq!(sps.level, 41);
        assert!(
            !sps.frame_mbs_only && !sps.mb_adaptive_frame_field && sps.separate_colour_plane
        );
        let packets: Vec<_> = (0..field_count)
            .map(|i| std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
            .collect();
        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        assert_eq!(golden.len(), pair_count * 3072);

        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            for (field_idx, packet) in packets.iter().enumerate() {
                let frame = decoder
                    .decode_order(packet)
                    .unwrap_or_else(|e| panic!("{name} field {field_idx}: {e}"));
                if field_idx % 2 == 0 {
                    assert!(frame.is_none());
                    assert!(decoder.has_pending_field());
                } else {
                    let picture = frame.unwrap();
                    assert!(!decoder.has_pending_field());
                    assert!(decoder.output_is_field_pair());
                    assert_eq!(picture.chroma_format().unwrap(), 3);
                    assert_eq!(picture.crop, [1, 1, 2, 2]);
                    assert_eq!(picture.component_depths(), [depth; 3]);
                    assert_eq!(picture.dimensions(), (30, 28));
                    let shown_pair = decode_order[field_idx / 2];
                    let base = shown_pair * 3072;
                    for (c, plane) in [&picture.y, &picture.cb, &picture.cr]
                        .into_iter()
                        .enumerate()
                    {
                        assert_eq!(
                            plane.as_slice(),
                            &golden[base + c * 1024..base + (c + 1) * 1024],
                            "{name} pair {} component {c}",
                            field_idx / 2
                        );
                    }
                }
            }
            decoder.reset();
        }

        // Direct avc_field_picture entrypoint verification on separate-colour-plane fields
        if kind == "p" {
            let mut buffer: FieldBuffer<PcmField> =
                FieldBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames).unwrap();
            let mut pending: Option<std::sync::Arc<PcmField>> = None;
            for (field_idx, packet) in packets.iter().enumerate() {
                let headers: Vec<_> = NalUnits::new(packet, avc.length_size)
                    .unwrap()
                    .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
                    .collect();
                let views: Vec<_> = headers.iter().collect();
                let first = &headers[0];
                let field = if first.slice_type == SliceType::I {
                    decode_intra_slices(&views, &sps, &pps, 1 << 20).unwrap()
                } else {
                    let lists: Vec<_> = headers
                        .iter()
                        .map(|h| buffer.lists(h, h.poc_lsb.unwrap() as i32).unwrap())
                        .collect();
                    let resolved: Vec<_> = lists
                        .iter()
                        .map(|l| {
                            (
                                buffer.get(l.l0[0].id, l.l0[0].bottom).unwrap().as_ref(),
                                l.l0[0].id * 2 + u64::from(l.l0[0].bottom),
                            )
                        })
                        .collect();
                    let same_reference = resolved.iter().all(|(r, _)| std::ptr::eq(*r, resolved[0].0));
                    assert!(same_reference);
                    if field_idx >= 4 && !pps.cabac && !pps.weighted_pred {
                        let skip_copy =
                            decode_skip_field(&views, &sps, &pps, resolved[0].0, 1 << 20)
                                .unwrap_or_else(|e| panic!("direct skip {name}: {e}"));
                        let p_copy = decode_p_field(&views, &sps, &pps, resolved[0].0, 1 << 20)
                            .unwrap_or_else(|e| panic!("direct p {name}: {e}"));
                        assert_eq!(skip_copy.picture.y, p_copy.picture.y);
                        assert_eq!(skip_copy.picture.cb, p_copy.picture.cb);
                        assert_eq!(skip_copy.picture.cr, p_copy.picture.cr);
                        p_copy
                    } else {
                        decode_p_field(&views, &sps, &pps, resolved[0].0, 1 << 20)
                            .unwrap_or_else(|e| panic!("direct p {name} field {field_idx}: {e}"))
                    }
                };
                let field = std::sync::Arc::new(field);
                if let Some(prev) = pending.take() {
                    let woven = weave_pair(&prev, &field, 1 << 20).unwrap();
                    let shown_pair = decode_order[field_idx / 2];
                    let base = shown_pair * 3072;
                    assert_eq!(woven.y, golden[base..base + 1024], "direct {name} Y");
                    assert_eq!(woven.cb, golden[base + 1024..base + 2048], "direct {name} Cb");
                    assert_eq!(woven.cr, golden[base + 2048..base + 3072], "direct {name} Cr");
                } else {
                    pending = Some(std::sync::Arc::clone(&field));
                }
                buffer
                    .finish(
                        first,
                        first.poc_lsb.unwrap() as i32,
                        10 + field_idx as u64,
                        field,
                    )
                    .unwrap();
            }
        } else if matches!(case["mode"].as_str().unwrap(), "spatial" | "bi") {
            let mut buffer: FieldBuffer<PcmField> =
                FieldBuffer::new(sps.frame_num_bits, sps.max_num_ref_frames).unwrap();
            let mut pending: Option<std::sync::Arc<PcmField>> = None;
            for (field_idx, packet) in packets.iter().enumerate() {
                let headers: Vec<_> = NalUnits::new(packet, avc.length_size)
                    .unwrap()
                    .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
                    .collect();
                let views: Vec<_> = headers.iter().collect();
                let first = &headers[0];
                let poc = first.poc_lsb.unwrap() as i32;
                let field = if first.slice_type == SliceType::I {
                    decode_intra_slices(&views, &sps, &pps, 1 << 20).unwrap()
                } else {
                    let lists: Vec<_> = headers
                        .iter()
                        .map(|h| buffer.lists(h, poc).unwrap())
                        .collect();
                    let references: Vec<_> = lists
                        .iter()
                        .map(|l| {
                            [
                                l.l0.iter()
                                    .map(|r| {
                                        (
                                            buffer.get(r.id, r.bottom).unwrap().as_ref(),
                                            r.id * 2 + u64::from(r.bottom),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                                l.l1.iter()
                                    .map(|r| {
                                        (
                                            buffer.get(r.id, r.bottom).unwrap().as_ref(),
                                            r.id * 2 + u64::from(r.bottom),
                                        )
                                    })
                                    .collect::<Vec<_>>(),
                            ]
                        })
                        .collect();
                    let contexts: Vec<_> = references
                        .iter()
                        .map(|l| [l[0].as_slice(), l[1].as_slice()])
                        .collect();
                    let orders: Vec<_> = lists
                        .iter()
                        .map(|l| {
                            [
                                l.l0.iter()
                                    .map(|r| buffer.order(r.id, r.bottom).unwrap())
                                    .collect::<Vec<_>>(),
                                l.l1.iter()
                                    .map(|r| buffer.order(r.id, r.bottom).unwrap())
                                    .collect::<Vec<_>>(),
                            ]
                        })
                        .collect();
                    let order_views: Vec<_> = orders
                        .iter()
                        .map(|l| [l[0].as_slice(), l[1].as_slice()])
                        .collect();
                    let implicit = ImplicitFieldWeights {
                        poc,
                        references: &order_views,
                    };
                    let implicit_ref = if pps.weighted_bipred == 2 && first.slice_type == SliceType::B {
                        Some(&implicit)
                    } else {
                        None
                    };
                    if case["mode"] == "bi" {
                        let (picture, _motion) = decode_inter_field_lists_with_motion(
                            &views,
                            &sps,
                            &pps,
                            &contexts,
                            implicit_ref,
                            1 << 20,
                        )
                        .unwrap_or_else(|e| panic!("direct bi {name} field {field_idx}: {e}"));
                        let without_motion = decode_inter_field_lists_with_order(
                            &views,
                            &sps,
                            &pps,
                            &contexts,
                            implicit_ref,
                            1 << 20,
                        )
                        .unwrap();
                        assert_eq!(picture.picture.y, without_motion.picture.y);
                        assert_eq!(picture.picture.cb, without_motion.picture.cb);
                        assert_eq!(picture.picture.cr, without_motion.picture.cr);
                        picture
                    } else {
                        // Spatial direct without direct motion context returns Unsupported on direct legacy entrypoint
                        if first.slice_type == SliceType::B {
                            let err = decode_inter_field_lists_with_order(
                                &views,
                                &sps,
                                &pps,
                                &contexts,
                                implicit_ref,
                                1 << 20,
                            )
                            .err()
                            .unwrap();
                            assert!(
                                err.to_string().contains(
                                    "B field direct prediction requires reference motion context"
                                ),
                                "{name}: {err}"
                            );
                            continue;
                        }
                        decode_inter_field_lists_with_order(
                            &views,
                            &sps,
                            &pps,
                            &contexts,
                            implicit_ref,
                            1 << 20,
                        )
                        .unwrap()
                    }
                };
                let field = std::sync::Arc::new(field);
                if let Some(prev) = pending.take() {
                    let woven = weave_pair(&prev, &field, 1 << 20).unwrap();
                    let shown_pair = decode_order[field_idx / 2];
                    let base = shown_pair * 3072;
                    assert_eq!(woven.y, golden[base..base + 1024], "direct {name} Y");
                    assert_eq!(woven.cb, golden[base + 1024..base + 2048], "direct {name} Cb");
                    assert_eq!(woven.cr, golden[base + 2048..base + 3072], "direct {name} Cr");
                } else {
                    pending = Some(std::sync::Arc::clone(&field));
                }
                buffer
                    .finish(
                        first,
                        poc,
                        10 + field_idx as u64,
                        field,
                    )
                    .unwrap();
            }
        }

        let mp4 = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(std::io::Cursor::new(mp4), Limits::default(), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            for shown in 0..pair_count {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name} pair {shown}: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    shown as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                let expected_sample = if kind == "b" {
                    [0, 4, 2, 6][shown]
                } else {
                    shown * 2
                };
                assert_eq!(frame.sample_index, expected_sample);
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                let mut visible = Vec::new();
                for c in 0..3 {
                    for y in 2..30 {
                        for x in 1..31 {
                            visible.push(golden[shown * 3072 + c * 1024 + y * 32 + x]);
                        }
                    }
                }
                assert_eq!(packed.frame.data, bytes(visible, depth), "MP4 {name} pair {shown}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
