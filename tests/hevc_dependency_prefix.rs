use fvid::codec::hevc_vps::{LayerDependencyPrefix, LayerSets};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
#[test]
fn annex_f_dependency_prefix_graphs() {
    let cases: serde_json::Value =
        serde_json::from_slice(&data("hevc-dependency-prefix.json")).unwrap();
    for c in cases.as_array().unwrap() {
        let bytes = data(c["file"].as_str().unwrap());
        let ids: Vec<u8> = c["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let layers = LayerSets {
            base_internal: true,
            base_available: true,
            max_layers_minus1: ids.len() as u8 - 1,
            max_layer_id: *ids.iter().max().unwrap(),
            sets: vec![1],
        };
        let result = LayerDependencyPrefix::parse(&bytes, 0, &layers, 0, 1 << 20);
        if c["error"].as_bool().unwrap() {
            assert!(result.unwrap_err().to_string().contains("not increasing"));
            continue;
        }
        let p = result.unwrap();
        assert_eq!(p.layer_ids, ids);
        assert_eq!(p.end_bit, c["end"].as_u64().unwrap() as usize);
        let mut reach = vec![vec![false; ids.len()]; ids.len()];
        for e in c["edges"].as_array().unwrap() {
            let i = e[0].as_u64().unwrap() as usize;
            let j = e[1].as_u64().unwrap() as usize;
            reach[i][j] = true;
        }
        for i in 0..ids.len() {
            let direct = (0..ids.len())
                .filter(|&j| reach[i][j])
                .fold(0u64, |m, j| m | (1u64 << ids[j]));
            assert_eq!(p.direct[i], direct);
        }
        // Independent Floyd-Warshall oracle rather than the decoder's forward recurrence.
        for k in 0..ids.len() {
            for i in 0..ids.len() {
                for j in 0..ids.len() {
                    reach[i][j] |= reach[i][k] && reach[k][j];
                }
            }
        }
        for i in 0..ids.len() {
            let mask = (0..ids.len())
                .filter(|&j| reach[i][j])
                .fold(0u64, |m, j| m | (1u64 << ids[j]));
            assert_eq!(p.transitive[i], mask);
        }
        let mut claimed = std::collections::BTreeSet::new();
        let mut expected = Vec::new();
        for i in 0..ids.len() {
            if p.direct[i] != 0 {
                continue;
            }
            let mut tree = vec![ids[i]];
            for j in 0..ids.len() {
                if reach[j][i] && claimed.insert(ids[j]) {
                    tree.push(ids[j]);
                }
            }
            expected.push(tree);
        }
        assert_eq!(p.tree_partitions, expected);
        if c["split"].as_bool().unwrap() {
            for (i, &id) in ids.iter().enumerate() {
                assert_eq!(p.scalability_ids[i][1], id & 7);
                assert_eq!(p.scalability_ids[i][2], id >> 3);
            }
        }
        assert!(LayerDependencyPrefix::parse(&bytes, 0, &layers, 0, 0).is_err());
        assert!(
            LayerDependencyPrefix::parse(&bytes[..bytes.len() - 1], 0, &layers, 0, 1 << 20)
                .is_err()
        );
    }
}
