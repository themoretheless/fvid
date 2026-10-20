use fvid::codec::hevc_vps::LayeredVps;
fn graph() -> fvid::codec::hevc_vps::LayerDependencyPrefix {
    let bytes = include_bytes!("fixtures/playback-errors/hevc-wedge/wedge-picture.vps");
    LayeredVps::parse(bytes, 1 << 20)
        .unwrap()
        .extension
        .unwrap()
        .graph
}
#[test]
fn encoded_depth_graph_resolves_texture_and_depth_in_the_same_view() {
    let graph = graph();
    assert_eq!(graph.view_component_layer(3, false).unwrap(), Some(0));
    assert_eq!(graph.view_component_layer(0, true).unwrap(), Some(3));
    assert_eq!(graph.view_component_layer(3, true).unwrap(), Some(3));
    assert_eq!(graph.reference_list_layers(3).unwrap(), 0);
    assert_eq!(graph.direct[1], 1);
    assert_eq!(
        graph
            .view_component_layer(62, false)
            .unwrap_err()
            .to_string(),
        "HEVC view component names an unknown layer"
    );
}
#[test]
fn component_derivation_excludes_dependency_and_auxiliary_variants() {
    let mut graph = graph();
    graph.layer_ids = vec![0, 3, 7, 11, 16, 23];
    graph.scalability_ids = vec![[0; 16]; 6];
    graph.scalability_ids[1][0] = 1;
    for index in 2..6 {
        graph.scalability_ids[index][1] = 5;
    }
    graph.scalability_ids[2][2] = 1; // dependency representation, not view component
    graph.scalability_ids[4][0] = 1;
    graph.scalability_ids[5][3] = 1; // auxiliary picture, not TexturePic
    graph.view_ids = vec![91, 17]; // IDs are unrelated to the view-order index.
    assert_eq!(graph.view_component_layer(16, false).unwrap(), Some(11));
    assert_eq!(graph.view_component_layer(11, true).unwrap(), Some(16));
    assert_eq!(graph.view_component_layer(3, false).unwrap(), Some(0));
    graph.scalability_ids[3][2] = 2;
    assert_eq!(graph.view_component_layer(16, false).unwrap(), None);
    graph.scalability_ids[3][2] = 0;
    graph.layer_ids.push(24);
    let mut later = [0; 16];
    later[1] = 5;
    graph.scalability_ids.push(later);
    assert_eq!(graph.view_component_layer(16, false).unwrap(), Some(24));
    graph.scalability_ids[0][0] = 1;
    assert!(graph.view_component_layer(16, false).is_err());
}

#[test]
fn inter_layer_reference_mask_filters_roles_without_removing_dependencies() {
    let mut graph = graph();
    graph.layer_ids = vec![0, 3, 7];
    graph.scalability_ids = vec![[0; 16]; 3];
    graph.scalability_ids[1][0] = 1;
    graph.scalability_ids[2][0] = 1;
    graph.direct = vec![0, 1, 1 | (1 << 3)];
    assert_eq!(graph.reference_list_layers(7).unwrap(), 1 << 3);
    assert_eq!(graph.direct[2], 1 | (1 << 3));
    graph.scalability_ids[2][0] = 0;
    assert_eq!(graph.reference_list_layers(7).unwrap(), 1);
    graph.direct[2] |= 1 << 7;
    assert!(graph.reference_list_layers(7).is_err());
}
