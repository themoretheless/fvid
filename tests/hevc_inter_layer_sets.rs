//! Access-unit/DPB selection acceptance, not scalable slice playback.
use fvid::codec::{
    hevc_inter_layer::{InterLayerSet, PictureStatus},
    hevc_nal::NalHeader,
    hevc_reference_list::{LayeredReference, PictureIdentity, ReferenceKind},
    hevc_vps::LayeredVps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn status(v: &serde_json::Value) -> PictureStatus {
    let layer_id = v["layer"].as_u64().unwrap() as u8;
    PictureStatus {
        identity: PictureIdentity {
            layer_id,
            poc: v["poc"].as_i64().unwrap() as i32,
        },
        access_unit: v["au"].as_u64().unwrap(),
        nal: NalHeader {
            unit_type: v["kind"].as_u64().unwrap() as u8,
            layer_id,
            temporal_id: v["tid"].as_u64().unwrap() as u8,
        },
        discardable: v["discardable"].as_bool().unwrap(),
    }
}
#[test]
fn inter_layer_dpb_selection_binds_access_units_views_temporal_admission_and_exact_samples() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-inter-layer-sets.json")).unwrap();
    let atlas = data("hevc-inter-layer-set-atlas.y4m");
    let header = b"YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C420\n";
    assert!(atlas.starts_with(header));
    let mut at = header.len();
    let frames: Vec<[Vec<u8>; 3]> = manifest["pictures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            assert_eq!(&atlas[at..at + 6], b"FRAME\n");
            at += 6;
            let planes = std::array::from_fn(|c| {
                let n = if c == 0 { 64 } else { 16 };
                let out = atlas[at..at + n].to_vec();
                at += n;
                out
            });
            assert_eq!(serde_json::to_value(&planes).unwrap(), p["planes"]);
            planes
        })
        .collect();
    assert_eq!(at, atlas.len());
    for c in manifest["cases"].as_array().unwrap() {
        let label = c["label"].as_str().unwrap();
        let vps = LayeredVps::parse(&data(c["vps"].as_str().unwrap()), 1 << 20)
            .unwrap_or_else(|e| panic!("{label}: VPS: {e}"));
        let current = status(&c["current"]);
        let selected: Vec<u8> = c["selected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let decoded: Vec<_> = c["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (status(p), &frames[p["frame"].as_u64().unwrap() as usize]))
            .collect();
        let snapshot: Vec<_> = decoded.iter().map(|p| p.0).collect();
        let result = InterLayerSet::derive(&vps, current, &selected, &decoded, 1 << 20);
        assert_eq!(decoded.iter().map(|p| p.0).collect::<Vec<_>>(), snapshot);
        if let Some(message) = c["error"].as_str() {
            let e = match result {
                Err(e) => e,
                Ok(_) => panic!("{label}: unexpectedly admitted"),
            };
            assert!(matches!(e, fvid::Error::Invalid(_)), "{label}: {e}");
            assert!(
                e.to_string().contains(message),
                "{label}: {e}, expected {message}"
            );
            continue;
        }
        let set = result.unwrap_or_else(|e| panic!("{label}: {e}"));
        for group in 0..2 {
            let expected: Vec<_> = c["groups"][group]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| PictureIdentity {
                    layer_id: id[0].as_u64().unwrap() as u8,
                    poc: id[1].as_i64().unwrap() as i32,
                })
                .collect();
            assert_eq!(set.pictures()[group], expected, "{label}: group {group}");
            for (i, &id) in set.pictures()[group].iter().enumerate() {
                let selected = set
                    .resolve(LayeredReference {
                        picture: id,
                        kind: ReferenceKind::InterLayer,
                    })
                    .unwrap();
                let frame = c["frames"][group][i].as_u64().unwrap() as usize;
                assert_eq!(**selected, frames[frame], "{label}: source image");
                assert!(
                    std::ptr::eq(*selected, &frames[frame]),
                    "{label}: source was copied"
                );
            }
        }
        for list in 0..2 {
            let cycle: Vec<_> = if list == 0 {
                set.pictures()[0]
                    .iter()
                    .chain(set.pictures()[1])
                    .copied()
                    .collect()
            } else {
                set.pictures()[1]
                    .iter()
                    .chain(set.pictures()[0])
                    .copied()
                    .collect()
            };
            let count = selected.len().min(7);
            let actual = set
                .select([&[], &[], &[]], false, count, None, list, 1 << 20)
                .unwrap();
            let expected: Vec<_> = cycle
                .iter()
                .cycle()
                .take(count)
                .map(|&picture| LayeredReference {
                    picture,
                    kind: ReferenceKind::InterLayer,
                })
                .collect();
            assert_eq!(actual, expected, "{label}: list {list}");
            for r in actual {
                assert!(r.long_term());
                set.resolve(r).unwrap();
            }
            assert!(
                set.select([&[], &[], &[]], false, count, None, list, 0)
                    .is_err()
            );
        }
        assert!(InterLayerSet::derive(&vps, current, &selected, &decoded, 0).is_err());
        let storage = 2
            * selected.len()
            * (std::mem::size_of::<PictureIdentity>() + std::mem::size_of::<&[Vec<u8>; 3]>())
            + 2 * std::mem::size_of::<Vec<PictureIdentity>>()
            + 2 * std::mem::size_of::<Vec<&[Vec<u8>; 3]>>();
        assert!(
            InterLayerSet::derive(&vps, current, &selected, &decoded, storage).is_ok(),
            "{label}: exact budget"
        );
        assert!(
            InterLayerSet::derive(&vps, current, &selected, &decoded, storage - 1).is_err(),
            "{label}: undersized budget"
        );
        // The aggregate list gate must retain the DPB binding's buffer budget.
        let only_list_bytes = selected.len() * std::mem::size_of::<LayeredReference>()
            + 3 * std::mem::size_of::<Vec<LayeredReference>>();
        assert!(
            set.select([&[], &[], &[]], false, 0, None, 0, only_list_bytes)
                .is_err()
        );
        assert!(
            set.resolve(LayeredReference {
                picture: current.identity,
                kind: ReferenceKind::Current
            })
            .is_err()
        );
        assert!(
            set.resolve(LayeredReference {
                picture: current.identity,
                kind: ReferenceKind::InterLayer
            })
            .is_err()
        );
        let mut wrong = current;
        wrong.nal.layer_id = 0;
        assert!(InterLayerSet::derive(&vps, wrong, &selected, &decoded, 1 << 20).is_err());
    }
}

#[test]
fn layered_irap_lists_restrict_temporal_and_current_picture_references() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-inter-layer-sets.json")).unwrap();
    let decoded: Vec<_> = manifest["pictures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (status(&p["status"]), p["planes"].clone()))
        .collect();
    for (index, c) in manifest["list_cases"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let vps = LayeredVps::parse(&data(c["vps"].as_str().unwrap()), 1 << 20).unwrap();
        let selected: Vec<u8> = c["selected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u8)
            .collect();
        let set = InterLayerSet::derive(&vps, status(&c["current"]), &selected, &decoded, 1 << 20)
            .unwrap();
        let temporal: [Vec<PictureIdentity>; 3] = std::array::from_fn(|g| {
            c["temporal"][g]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| PictureIdentity {
                    layer_id: id[0].as_u64().unwrap() as u8,
                    poc: id[1].as_i64().unwrap() as i32,
                })
                .collect()
        });
        let modification = c["modification"].as_array().map(|values| {
            values
                .iter()
                .map(|v| v.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        });
        let result = set.select(
            temporal.each_ref().map(|v| v.as_slice()),
            c["current_picture"].as_bool().unwrap(),
            c["active"].as_u64().unwrap() as usize,
            modification.as_deref(),
            c["list"].as_u64().unwrap() as usize,
            1 << 20,
        );
        if let Some(error) = c["error"].as_str() {
            let e = result.unwrap_err();
            assert!(
                e.to_string().contains(error),
                "case {index}: {e}, expected {error}"
            );
        } else {
            let expected: Vec<_> = c["expected"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| LayeredReference {
                    picture: PictureIdentity {
                        layer_id: v["layer"].as_u64().unwrap() as u8,
                        poc: v["poc"].as_i64().unwrap() as i32,
                    },
                    kind: match v["kind"].as_str().unwrap() {
                        "Current" => ReferenceKind::Current,
                        "ShortTerm" => ReferenceKind::ShortTerm,
                        kind => panic!("{kind}"),
                    },
                })
                .collect();
            assert_eq!(result.unwrap(), expected, "case {index}");
        }
    }
}
