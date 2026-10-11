use fvid::codec::hevc_vps::{LayerDependencyPrefix, LayerSets};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_representation_geometry_depth_crop_and_mapping() {
    let cases: serde_json::Value =
        serde_json::from_slice(&data("hevc-representation.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let bytes = data(name);
        let layers = LayerSets {
            base_internal: true,
            base_available: true,
            max_layers_minus1: 3,
            max_layer_id: 15,
            sets: vec![1, 32905],
        };
        let graph = LayerDependencyPrefix::parse(&bytes, 0, &layers, 0, 1 << 20).unwrap();
        let extension = graph.layer_sets(&bytes, &layers, 0, 1 << 20).unwrap();
        let output = extension
            .output_layers(&bytes, &graph, &layers, 0, 1 << 20)
            .unwrap();
        let result = output.representation_formats(&bytes, &graph, &layers, 1 << 20);
        if let Some(error) = c["error"].as_str() {
            assert!(result.unwrap_err().to_string().contains(error), "{name}");
            continue;
        }
        let parsed = result.unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(parsed.end_bit, c["end"].as_u64().unwrap() as usize);
        assert_eq!(parsed.formats.len(), c["formats"].as_array().unwrap().len());
        for (f, e) in parsed.formats.iter().zip(c["formats"].as_array().unwrap()) {
            assert_eq!(u64::from(f.width), e["width"].as_u64().unwrap());
            assert_eq!(u64::from(f.height), e["height"].as_u64().unwrap());
            assert_eq!(u64::from(f.chroma), e["chroma"].as_u64().unwrap());
            assert_eq!(f.separate_colour_planes, e["separate"].as_bool().unwrap());
            assert_eq!(u64::from(f.luma_depth), e["luma_depth"].as_u64().unwrap());
            assert_eq!(
                u64::from(f.chroma_depth),
                e["chroma_depth"].as_u64().unwrap()
            );
            assert_eq!(
                f.crop,
                e["crop"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u32)
                    .collect::<Vec<_>>()
                    .as_slice()
            );
            assert_eq!(
                f.display_size().unwrap(),
                (
                    e["display"][0].as_u64().unwrap() as u16,
                    e["display"][1].as_u64().unwrap() as u16
                )
            );
        }
        assert_eq!(
            parsed.layer_format_indices,
            c["mapping"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        );
        assert!(parsed.max_one_active_reference);
        assert!(parsed.poc_lsb_aligned);
        assert_eq!(parsed.poc_lsb_not_present, vec![false; 4]);
        assert!(
            output
                .representation_formats(&bytes, &graph, &layers, 0)
                .is_err()
        );
        assert!(
            output
                .representation_formats(&bytes[..bytes.len() - 1], &graph, &layers, 1 << 20)
                .is_err()
        );
    }
}
