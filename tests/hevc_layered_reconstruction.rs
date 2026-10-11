//! Coded nonbase-picture acceptance; access-unit/DPB ownership is separate.
use fvid::codec::{
    config::{HevcConfig, NalUnits},
    hevc_decoder::{HevcDecoder, LayeredParameters},
    hevc_nal::NalHeader,
    hevc_picture::{decode_layered, decode_slices},
    hevc_pps::LayeredPpsContext,
    hevc_slice::{LayeredSliceContext, SliceHeader},
};
use fvid::container::mp4::Mp4Reader;
use std::{io::Cursor, path::Path};

#[test]
fn owned_nonbase_coded_pictures_reconstruct_exact_pixels_with_bound_parameters() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("hevc-layered-reconstruction.json")).unwrap(),
    )
    .unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        if case["inter"].as_bool() == Some(true) {
            continue;
        }
        let file = case["file"].as_str().unwrap();
        let mut movie = Mp4Reader::open(
            Cursor::new(std::fs::read(root.join(file)).unwrap()),
            Default::default(),
        )
        .unwrap();
        assert_eq!(movie.tracks()[0].samples.len(), 1, "{file}");
        let configuration = movie.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&configuration).unwrap();
        let mut packet = vec![];
        movie.read_packet(0, 0, &mut packet).unwrap();
        let units: Vec<_> = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let mut parameters =
            LayeredParameters::from_configuration(&configuration, 32 << 20).unwrap();
        let first = |layer| {
            units
                .iter()
                .copied()
                .find(|nal| {
                    let h = NalHeader::parse(nal).unwrap();
                    h.is_vcl() && h.layer_id == layer
                })
                .unwrap()
        };
        let base = parameters.activate_slice(first(0), 1).unwrap();
        let binding = parameters.activate_slice(first(3), 1).unwrap();
        assert_eq!(base.sps.sps.id, 0);
        assert_eq!(base.pps.pps.id, 0);
        assert_eq!(parameters.active(0).unwrap().pps.pps.id, 0);
        if case["shared_pps"].as_bool() == Some(true) {
            let shared_base = parameters.activate(0, 1, 1).unwrap();
            assert_eq!(shared_base.pps.pps, binding.pps.pps);
            assert_eq!(shared_base.pps.nal_layer_id, 0);
            assert_eq!(binding.pps.nal_layer_id, 0);
            assert_eq!(shared_base.pps.layer_id, 0);
            assert_eq!(binding.pps.layer_id, 3);
        }
        let sps = &binding.sps;
        let pps = &binding.pps;
        // Annex F shares parameter IDs across all NAL layers. These fixtures
        // must not overwrite the base layer's SPS/PPS 0 with different content.
        assert_eq!(
            sps.sps.id,
            case["sps_id"]
                .as_u64()
                .map(|i| i as u8)
                .unwrap_or(u8::from(case["independent"].as_bool().unwrap()))
        );
        assert_eq!(
            sps.multilayer_syntax,
            case["multilayer_sps"].as_bool().unwrap_or(false)
        );
        assert_eq!(pps.pps.id, 1);
        assert_eq!(pps.pps.sps_id, sps.sps.id);
        let context = binding.context();
        let mut headers = vec![];
        let mut coded_layers = vec![];
        for nal in NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(Result::unwrap)
        {
            if NalHeader::parse(nal).unwrap().is_vcl() {
                coded_layers.push(NalHeader::parse(nal).unwrap().layer_id);
            }
            if !NalHeader::parse(nal).unwrap().is_vcl()
                || NalHeader::parse(nal).unwrap().layer_id != 3
            {
                continue;
            }
            let previous = headers.iter().rev().find(|h: &&SliceHeader| {
                // A dependent segment inherits only within its colour plane;
                // these authored separate-plane streams use independent slices.
                !sps.sps.separate_colour_plane || h.colour_plane == 0
            });
            headers.push(
                SliceHeader::parse_layered_with_previous(nal, &context, 32 << 20, previous)
                    .unwrap_or_else(|e| panic!("{file}: slice {e}")),
            );
        }
        coded_layers.sort();
        coded_layers.dedup();
        assert_eq!(coded_layers, [0, 3], "{file}");
        assert!(headers.iter().all(|h| h.nal.layer_id == 3));
        let lists = vec![[vec![], vec![]]; headers.len()];
        // Preserve the ordinary entry point's base-layer restriction.
        let refusal = decode_slices(&sps.sps, &pps.pps, &headers, 0, &lists, 32 << 20)
            .err()
            .unwrap()
            .to_string();
        assert!(
            refusal.contains(if file.contains("multislice") {
                "invalid HEVC slice partition start"
            } else {
                "unsupported HEVC picture tools"
            }),
            "{file}: {refusal}"
        );
        let picture = decode_layered(&context, &headers, 0, &lists, 32 << 20)
            .unwrap_or_else(|e| panic!("{file}: reconstruction {e}"));
        let mut source = Mp4Reader::open(
            Cursor::new(std::fs::read(root.join(case["seed"].as_str().unwrap())).unwrap()),
            Default::default(),
        )
        .unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&source.tracks()[0].configuration, 32 << 20).unwrap();
        source.read_packet(0, 0, &mut packet).unwrap();
        let gold = decoder.decode_packet(&packet).unwrap().unwrap().picture;
        assert_eq!(picture.dimensions, gold.dimensions, "{file}");
        assert_eq!(picture.crop, gold.crop, "{file}");
        for (actual, expected) in picture.planes.iter().zip(&gold.planes) {
            assert_eq!(actual.samples(), expected.samples(), "{file}");
        }
        let saved = std::fs::read(
            root.join(case["seed"].as_str().unwrap())
                .with_extension("yuv"),
        )
        .unwrap();
        let mut at = 0;
        assert_eq!(picture.depth, [8; 2]);
        for actual in &picture.planes {
            let n = actual.samples().len();
            let expected: Vec<_> = saved[at..at + n].iter().map(|&v| u16::from(v)).collect();
            assert_eq!(
                actual.samples(),
                expected,
                "{file}: independent saved oracle"
            );
            at += n;
        }
        let repeated = decode_layered(&context, &headers, 0, &lists, 32 << 20).unwrap();
        assert_eq!(repeated.planes[0].samples(), picture.planes[0].samples());
        assert!(decode_layered(&context, &headers, 0, &lists, 1).is_err());
        let mut mismatched = headers.clone();
        mismatched[0].nal.layer_id = 2;
        assert!(decode_layered(&context, &mismatched, 0, &lists, 32 << 20).is_err());
        if file.contains("multislice") && !case["independent"].as_bool().unwrap() {
            let mut mixed_picture = headers.clone();
            mixed_picture.last_mut().unwrap().poc_lsb = 1;
            assert!(
                decode_layered(&context, &mixed_picture, 0, &lists, 32 << 20)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("slices disagree on picture identity")
            );
        }
    }
}

#[test]
fn coded_nonbase_inter_slice_uses_same_access_unit_base_samples() {
    use fvid::codec::{
        hevc_inter_layer::PictureStatus,
        hevc_motion::Reference,
        hevc_reference_list::{LayeredReference, PictureIdentity, ReferenceKind},
        hevc_sao::Sao,
    };
    use std::sync::Arc;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let mut movie = Mp4Reader::open(
        Cursor::new(
            std::fs::read(root.join("hevc-layered-reconstruct-inter-prediction.mp4")).unwrap(),
        ),
        Default::default(),
    )
    .unwrap();
    assert_eq!(movie.tracks()[0].samples.len(), 2);
    let configuration = movie.tracks()[0].configuration.clone();
    let config = HevcConfig::parse(&configuration).unwrap();
    let mut parameters = LayeredParameters::from_configuration(&configuration, 32 << 20).unwrap();
    let base_binding = parameters.activate(0, 0, 1).unwrap();
    let binding = parameters.activate(3, 1, 1).unwrap();
    let active_sps = [base_binding.sps.clone()];
    let sps = &binding.sps;
    let pps = &binding.pps;
    let vps = &binding.vps;
    assert_eq!(sps.sps.id, 0); // Shared base SPS, distinct global PPS ID.
    assert_eq!(pps.pps.id, 1);
    assert_eq!(pps.pps.sps_id, 0);
    let context = LayeredSliceContext {
        vps: &vps,
        sps: &sps,
        pps: &pps,
    };
    let pps_context = LayeredPpsContext {
        vps: &vps,
        sps: &sps,
        active_sps: &active_sps,
        active_pps: &[],
    };
    let seed = root.join("hevc-pcm-reference-rext8.mp4");
    let mut source = Mp4Reader::open(
        Cursor::new(std::fs::read(&seed).unwrap()),
        Default::default(),
    )
    .unwrap();
    let source_config = source.tracks()[0].configuration.clone();
    let mut decoder = HevcDecoder::from_configuration(&source_config, 32 << 20).unwrap();
    let oracle = std::fs::read(seed.with_extension("yuv")).unwrap();
    let mut packet = vec![];
    for index in 0..2 {
        source.read_packet(0, index, &mut packet).unwrap();
        let base = decoder.decode_packet(&packet).unwrap().unwrap();
        assert_eq!(base.poc, index as i32);
        movie.read_packet(0, index, &mut packet).unwrap();
        let units: Vec<_> = NalUnits::new(&packet, config.length_size)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let base_nal = NalHeader::parse(
            units
                .iter()
                .find(|n| NalHeader::parse(n).unwrap().layer_id == 0)
                .unwrap(),
        )
        .unwrap();
        let headers: Vec<_> = units
            .iter()
            .filter(|n| NalHeader::parse(n).unwrap().layer_id == 3)
            .map(|n| SliceHeader::parse_layered(n, &context, 32 << 20).unwrap())
            .collect();
        assert_eq!(headers.len(), 1);
        let header = &headers[0];
        let mut lists = [vec![], vec![]];
        if index == 1 {
            assert!(header.short_term.is_empty());
            assert_eq!(header.layered.as_ref().unwrap().reference_layers, [0]);
            let identity = PictureIdentity {
                layer_id: 0,
                poc: base.poc,
            };
            let decoded = [(
                PictureStatus {
                    identity,
                    access_unit: 1,
                    nal: base_nal,
                    discardable: false,
                },
                base.picture.clone(),
            )];
            let bound = header
                .bind_inter_layer_references(&context, 1, base.poc, &decoded, 32 << 20)
                .unwrap();
            assert!(Arc::ptr_eq(
                bound
                    .resolve(LayeredReference {
                        picture: identity,
                        kind: ReferenceKind::InterLayer
                    })
                    .unwrap(),
                &base.picture
            ));
            let prepared = pps
                .reference_samples(
                    &pps_context,
                    0,
                    std::array::from_fn(|c| base.picture.planes[c].samples()),
                    32 << 20,
                )
                .unwrap();
            for (actual, expected) in prepared.iter().zip(&base.picture.planes) {
                assert_eq!(actual.samples(), expected.samples());
            }
            lists[0].push(Reference {
                layer_id: 0,
                poc: base.poc,
                long_term: true,
                picture: Some(base.picture.clone()),
            });
            if header.references[1] > 0 {
                lists[1] = lists[0].clone();
            }
        }
        let picture =
            decode_layered(&context, &headers, base.poc, &[lists.clone()], 32 << 20).unwrap();
        let mut at = index as usize * 6144;
        for plane in &picture.planes {
            let n = plane.samples().len();
            assert_eq!(
                plane.samples(),
                oracle[at..at + n]
                    .iter()
                    .map(|&v| u16::from(v))
                    .collect::<Vec<_>>()
            );
            at += n;
        }
        if index == 1 {
            let mut alternate_decoder =
                HevcDecoder::from_configuration(&source_config, 32 << 20).unwrap();
            source.read_packet(0, 0, &mut packet).unwrap();
            alternate_decoder.decode_packet(&packet).unwrap();
            source.read_packet(0, 1, &mut packet).unwrap();
            let mut alternate = alternate_decoder
                .decode_packet(&packet)
                .unwrap()
                .unwrap()
                .picture;
            alternate_decoder.reset();
            Arc::get_mut(&mut alternate).unwrap().planes[0]
                .apply_sao(
                    6,
                    &[Sao::Band {
                        position: 0,
                        offsets: [7; 4],
                    }],
                )
                .unwrap();
            assert_ne!(
                alternate.planes[0].samples(),
                base.picture.planes[0].samples()
            );
            for list in &mut lists {
                for reference in list {
                    reference.picture = Some(alternate.clone());
                }
            }
            let changed =
                decode_layered(&context, &headers, base.poc, &[lists.clone()], 32 << 20).unwrap();
            assert_ne!(
                changed.planes[0].samples(),
                picture.planes[0].samples(),
                "coded inter picture must read its inter-layer source samples"
            );
            lists[0][0].poc = base.poc - 1;
            assert!(
                decode_layered(&context, &headers, base.poc, &[lists], 32 << 20)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("invalid inter-layer reference")
            );
        }
    }
}
