//! Metadata binding acceptance. Active depth-picture reconstruction still refuses.
use fvid::codec::{
    hevc_pps::{LayeredPps, LayeredPpsContext},
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
use std::path::Path;
#[test]
fn depth_scalability_context_binds_tables_without_reinterpreting_texture_layers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-depth-pps-binding");
    let read = |name: &str| std::fs::read(root.join(name)).unwrap();
    let vps = LayeredVps::parse(&read("depth.vps"), 1 << 20).unwrap();
    let graph = &vps.extension.as_ref().unwrap().graph;
    assert_eq!(
        graph.depth_layer_ids().unwrap().collect::<Vec<_>>(),
        vec![3]
    );
    let texture = LayeredSps::parse(
        &read("texture.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 0,
            output_layer_set: 0,
            active_sps: &[],
        },
        1 << 20,
    )
    .unwrap();
    let depth = LayeredSps::parse(
        &read("depth.sps"),
        &LayeredSpsContext {
            vps: &vps,
            layer_id: 3,
            output_layer_set: 1,
            active_sps: std::slice::from_ref(&texture),
        },
        1 << 20,
    )
    .unwrap();
    let active = [texture.clone(), depth.clone()];
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hevc/main-ipb.mp4");
    let mut reader = fvid::container::mp4::Mp4Reader::open(
        std::fs::File::open(fixture).unwrap(),
        Default::default(),
    )
    .unwrap();
    let decoder = fvid::codec::hevc_decoder::HevcDecoder::from_configuration(
        &reader.tracks()[0].configuration,
        16 << 20,
    )
    .unwrap();
    let mut packet = Vec::new();
    reader.read_packet(0, 0, &mut packet).unwrap();
    let header = decoder.slice_headers(&packet).unwrap().remove(0);
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        for (layer, sps, suffix, ols) in [(3, &depth, "", 1), (0, &texture, "-texture", 0)] {
            let result = LayeredPps::parse(
                &read(&format!("{name}{suffix}.pps")),
                &LayeredPpsContext {
                    vps: &vps,
                    sps,
                    active_sps: &active,
                    active_pps: &[],
                },
                1 << 20,
            );
            if let Some(error) = c["error"].as_str() {
                assert_eq!(result.unwrap_err().to_string(), error, "{name}");
                continue;
            }
            let pps = result.unwrap();
            assert_eq!(pps.layer_id, layer);
            assert_eq!(pps.output_layer_set, ols);
            let tables = pps.pps.depth_tables.as_ref().unwrap();
            let expected: Vec<_> = c["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u16)
                .collect();
            assert_eq!(tables.layers[0].id, 3);
            assert_eq!(tables.layers[0].values, Some(expected));
            assert_eq!(pps.pps.depth_lookup_active, layer == 3);
            if layer == 3 {
                let error = fvid::codec::hevc_picture::decode(
                    &sps.sps,
                    &pps.pps,
                    &header,
                    0,
                    &[Vec::new(), Vec::new()],
                    16 << 20,
                )
                .err()
                .expect("ordinary reconstruction must not ignore active DLT");
                assert_eq!(
                    error.to_string(),
                    "HEVC depth lookup reconstruction requires a resolved depth layer"
                );
            }
            assert!(tables.heap_bytes() >= tables.layers[0].values.as_ref().unwrap().len() * 2);
        }
    }
    let mut invalid = graph.clone();
    invalid.scalability_ids[1][0] = 2;
    assert!(invalid.depth_layer_ids().is_err());
    let mut invalid = graph.clone();
    invalid.layer_ids[1] = 63;
    assert!(invalid.depth_layer_ids().is_err());
}

#[test]
fn base_texture_picture_playback_preserves_pixels_with_other_layer_depth_tables() {
    use fvid::{
        codec::hevc_decoder::{HevcDecoder, LayeredDecoder},
        container::mp4::Mp4Reader,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut original = Mp4Reader::open(
        std::fs::File::open(root.join("hevc/main-ipb.mp4")).unwrap(),
        Default::default(),
    )
    .unwrap();
    let mut movie = Mp4Reader::open(
        std::fs::File::open(
            root.join("playback-errors/hevc-depth-pps-binding/texture-playback.mp4"),
        )
        .unwrap(),
        Default::default(),
    )
    .unwrap();
    let mut baseline =
        HevcDecoder::from_configuration(&original.tracks()[0].configuration, 32 << 20).unwrap();
    let mut decoder =
        LayeredDecoder::from_configuration(&movie.tracks()[0].configuration, 0, 32 << 20).unwrap();
    assert_eq!(original.tracks()[0].samples.len(), 17);
    assert_eq!(movie.tracks()[0].samples.len(), 17);
    let mut wire = Vec::new();
    let mut control = Vec::new();
    for _ in 0..2 {
        for index in 0..17 {
            original.read_packet(0, index, &mut control).unwrap();
            movie.read_packet(0, index, &mut wire).unwrap();
            assert_eq!(
                wire, control,
                "coded pictures must remain authored controls"
            );
            let expected = baseline.decode_packet(&control).unwrap().unwrap();
            let (pictures, _) = decoder.decode_packet(&wire).unwrap();
            assert_eq!(pictures.len(), 1);
            let actual = &pictures[0];
            assert_eq!(actual.layer_id, 0);
            assert_eq!(actual.poc, expected.poc);
            assert!(!actual.binding.pps.pps.depth_lookup_active);
            assert_eq!(
                actual.binding.pps.pps.depth_tables.as_ref().unwrap().layers[0].values,
                Some(vec![1, 2, 4, 9])
            );
            for (actual, expected) in actual.picture.planes.iter().zip(&expected.picture.planes) {
                assert_eq!(actual.dimensions(), expected.dimensions());
                assert_eq!(actual.samples(), expected.samples());
            }
        }
        baseline.reset();
        decoder.reset();
    }
}
