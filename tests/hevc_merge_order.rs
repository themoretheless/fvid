use fvid::codec::hevc_motion::{
    merge_order::{self, Selection, Specials},
    texture::MergeKind,
};
fn identity(selection: Selection) -> usize {
    match selection {
        Selection::Initial(i) => i,
        Selection::Texture => 6,
        Selection::InterView => 7,
        Selection::InterViewShift => 8,
        Selection::Disparity => 9,
        Selection::DisparityShift => 10,
        Selection::ViewSynthesis => 11,
    }
}
#[test]
fn extended_merge_order_and_small_pb_selection_match_independent_full_list_oracle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/extended-merge-order-oracles.json");
    let oracle: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let cases = oracle["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 3072);
    for case in cases {
        let layout = case[0].as_u64().unwrap() as usize;
        let maximum = case[1].as_u64().unwrap() as usize;
        let mask = case[2].as_u64().unwrap();
        let depth = case[3].as_bool().unwrap();
        let left = case[4].as_bool().unwrap();
        let initial: Vec<_> = oracle["layouts"][layout]
            .as_array()
            .unwrap()
            .iter()
            .take(maximum)
            .map(|item| {
                let kind = match item[0].as_str().unwrap() {
                    "A1" => MergeKind::A1,
                    "B1" => MergeKind::B1,
                    "B0" => MergeKind::B0,
                    "A0" => MergeKind::A0,
                    "B2" => MergeKind::B2,
                    "Other" => MergeKind::Other,
                    _ => panic!("unknown oracle kind"),
                };
                (kind, item[1].as_i64().unwrap() as i32)
            })
            .collect();
        let texture = (mask & 1 != 0).then_some(1);
        let iv = (mask & 2 != 0).then_some(if mask & 8 != 0 { 1 } else { 2 });
        let iv_shift = (mask & 4 != 0).then_some(if mask & 16 != 0 {
            if mask & 8 != 0 { 1 } else { 2 }
        } else {
            9
        });
        let di = (mask & 8 != 0).then_some(if mask & 4 != 0 { 1 } else { 3 });
        let di_shift = (mask & 16 != 0).then_some(9);
        let special = Specials {
            texture: texture.as_ref(),
            inter_view: iv.as_ref(),
            inter_view_shift: iv_shift.as_ref(),
            disparity: di.as_ref(),
            disparity_shift: di_shift.as_ref(),
            view_synthesis: mask & 32 != 0,
            left_view_synthesis: left,
            depth,
        };
        let expected = |field: usize| {
            case[field]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as usize)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            merge_order::order(&initial, &special, maximum)
                .unwrap()
                .into_iter()
                .map(identity)
                .collect::<Vec<_>>(),
            expected(5),
            "{case}"
        );
        for (field, size) in [(6, [8, 8]), (7, [4, 8])] {
            let selected = (0..maximum)
                .map(|index| {
                    identity(merge_order::select(&initial, &special, maximum, index, size).unwrap())
                })
                .collect::<Vec<_>>();
            assert_eq!(selected, expected(field), "{case}");
        }
    }
}
