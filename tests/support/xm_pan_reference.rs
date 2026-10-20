//! Expected pan changes assembled from continuously played static-pan scores.
//! Each reference sets sample pan on its first note; no instrument-only switch
//! or tested pan opcode participates in rendering the expected samples.
use std::{path::Path, time::Duration};
type Packet = (Duration, Vec<u8>);
pub fn expected(
    root: &Path,
    case: &serde_json::Value,
    channels: u16,
    render: impl Fn(Vec<u8>, u16) -> (Vec<Packet>, u32),
) -> (Vec<Packet>, Vec<Packet>) {
    let (placed, ignored) = render(
        std::fs::read(root.join(case["control"].as_str().unwrap())).unwrap(),
        channels,
    );
    let (center, center_ignored) = render(
        std::fs::read(root.join(case["center_control"].as_str().unwrap())).unwrap(),
        channels,
    );
    assert_eq!((ignored, center_ignored), (0, 0));
    let roles = case["reference_rows"].as_array().unwrap();
    assert_eq!(roles.len(), placed.len());
    assert_eq!(center.len(), placed.len());
    let mut expected = Vec::new();
    for (i, role) in roles.iter().enumerate() {
        assert_eq!(center[i].0, placed[i].0);
        assert_eq!(center[i].1.len(), placed[i].1.len());
        expected.push(match role.as_str().unwrap() {
            "center" => center[i].clone(),
            "instrument" => placed[i].clone(),
            other => panic!("unknown static-pan reference role:{other}"),
        });
    }
    (expected, placed)
}
