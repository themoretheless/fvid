//! F.8.3.4 list/identity acceptance using an original Y4M reference atlas.
//! This is not coded scalable slice or multilayer playback acceptance.
use fvid::codec::hevc_reference_list::{
    LayeredReference, PictureIdentity, ReferenceKind, ReferenceSets,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn id(v: &serde_json::Value) -> PictureIdentity {
    PictureIdentity {
        layer_id: v[0].as_u64().unwrap() as u8,
        poc: v[1].as_i64().unwrap() as i32,
    }
}
#[test]
fn layered_lists_preserve_normative_order_current_picture_and_exact_frame_identity() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-layered-reference-lists.json")).unwrap();
    let bytes = data("hevc-layered-reference-atlas.y4m");
    let header = b"YUV4MPEG2 W8 H8 F25:1 Ip A1:1 C420\n";
    assert!(bytes.starts_with(header));
    let mut at = header.len();
    let mut pictures = Vec::new();
    for p in manifest["pictures"].as_array().unwrap() {
        assert_eq!(&bytes[at..at + 6], b"FRAME\n");
        at += 6;
        let planes: [Vec<u8>; 3] = std::array::from_fn(|c| {
            let n = if c == 0 { 64 } else { 16 };
            let pixels = bytes[at..at + n].to_vec();
            at += n;
            pixels
        });
        assert_eq!(serde_json::to_value(&planes).unwrap(), p["planes"]);
        pictures.push((id(&p["id"]), planes));
    }
    assert_eq!(at, bytes.len());
    let all: [Vec<PictureIdentity>; 5] = [
        (0..3).map(|i| pictures[i].0).collect(),
        (3..6).map(|i| pictures[i].0).collect(),
        (6..8).map(|i| pictures[i].0).collect(),
        (8..11).map(|i| pictures[i].0).collect(),
        (11..13).map(|i| pictures[i].0).collect(),
    ];
    for (index, c) in manifest["cases"].as_array().unwrap().iter().enumerate() {
        let groups: [Vec<PictureIdentity>; 5] = std::array::from_fn(|g| {
            if let Some(values) = c["overrides"][g.to_string()].as_array() {
                values.iter().map(id).collect()
            } else {
                all[g][..c["counts"][g].as_u64().unwrap() as usize].to_vec()
            }
        });
        let snapshot = groups.clone();
        let sets = ReferenceSets {
            current: PictureIdentity {
                layer_id: c["current_layer"].as_u64().unwrap_or(7) as u8,
                poc: 100,
            },
            before: &groups[0],
            after: &groups[1],
            long_term: &groups[2],
            inter_layer: [&groups[3], &groups[4]],
            current_picture: c["current"].as_bool().unwrap(),
        };
        let modification = c["modification"].as_array().map(|v| {
            v.iter()
                .map(|i| i.as_u64().unwrap() as u8)
                .collect::<Vec<_>>()
        });
        let active = c["active"].as_u64().unwrap() as usize;
        let list = c["list"].as_u64().unwrap() as usize;
        let result = sets.select(active, modification.as_deref(), list, 1 << 20);
        assert_eq!(groups, snapshot);
        if let Some(message) = c["error"].as_str() {
            let e = result.unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)), "case {index}: {e}");
            assert!(
                e.to_string().contains(message),
                "case {index}: {e}, expected {message}"
            );
            continue;
        }
        let expected: Vec<LayeredReference> = c["expected"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| LayeredReference {
                picture: id(&p["picture"]),
                kind: match p["kind"].as_str().unwrap() {
                    "ShortTerm" => ReferenceKind::ShortTerm,
                    "LongTerm" => ReferenceKind::LongTerm,
                    "InterLayer" => ReferenceKind::InterLayer,
                    "Current" => ReferenceKind::Current,
                    _ => panic!(),
                },
            })
            .collect();
        let output = result.unwrap_or_else(|e| panic!("case {index}: {e}"));
        assert_eq!(output, expected, "case {index}");
        assert_eq!(
            sets.select(active, modification.as_deref(), list, 1 << 20)
                .unwrap(),
            output
        );
        for (actual, wanted) in output.iter().zip(&expected) {
            let pixels = actual.picture.resolve(&pictures).unwrap();
            let saved = manifest["pictures"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| id(&p["id"]) == wanted.picture)
                .unwrap();
            assert_eq!(
                serde_json::to_value(pixels).unwrap(),
                saved["planes"],
                "case {index}: exact layer/POC image {wanted:?}"
            );
            assert_eq!(actual.long_term(), wanted.kind != ReferenceKind::ShortTerm);
        }
        assert!(
            sets.select(active, modification.as_deref(), list, 0)
                .unwrap_err()
                .to_string()
                .contains("budget")
        );
    }
    let missing = PictureIdentity {
        layer_id: 7,
        poc: 123,
    };
    assert!(
        missing
            .resolve(&pictures)
            .unwrap_err()
            .to_string()
            .contains("missing")
    );
    let mut ambiguous = pictures.clone();
    ambiguous.push(pictures[0].clone());
    assert!(
        pictures[0]
            .0
            .resolve(&ambiguous)
            .unwrap_err()
            .to_string()
            .contains("ambiguous")
    );
    assert!(
        PictureIdentity {
            layer_id: 63,
            poc: 100
        }
        .resolve(&pictures)
        .is_err()
    );
}
