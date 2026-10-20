use fvid::{
    codec::avc_decoder::AvcDecoder,
    playback_native::{NativeReader, RawFrame},
};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-mbaff-mixed-component-slices")
}
fn bytes(value: &serde_json::Value) -> Vec<u8> {
    fs::read(root().join(value.as_str().unwrap())).unwrap()
}
fn verify(
    picture: &fvid::codec::avc_picture::IntraPicture,
    case: &serde_json::Value,
    index: usize,
) {
    assert_eq!(
        picture.component_depths(),
        [
            case["luma_depth"].as_u64().unwrap() as u8,
            case["chroma_depth"].as_u64().unwrap() as u8,
            case["chroma_depth"].as_u64().unwrap() as u8
        ]
    );
    for (actual, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
        let expected: Vec<u16> = bytes(&case["frames"][index][name])
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        assert_eq!(actual, &expected, "{} frame{index} {name}", case["file"]);
    }
}
fn verify_raw(raw: &RawFrame, case: &serde_json::Value, index: usize) {
    match raw {
        RawFrame::Avc { picture, .. } => verify(picture, case, index),
        RawFrame::Planar8(picture) => {
            assert_eq!(
                (
                    case["luma_depth"].as_u64().unwrap(),
                    case["chroma_depth"].as_u64().unwrap()
                ),
                (8, 8)
            );
            for (actual, name) in [(&picture.y, "y"), (&picture.cb, "cb"), (&picture.cr, "cr")] {
                let expected: Vec<u16> = bytes(&case["frames"][index][name])
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                assert_eq!(
                    actual.iter().map(|&v| u16::from(v)).collect::<Vec<_>>(),
                    expected
                );
            }
        }
        _ => panic!("unexpected frame precision"),
    }
}
#[test]
fn mixed_intra_inter_slices_keep_per_component_qps_samples_playback_and_seek() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let mut decoder = AvcDecoder::new(&bytes(&case["configuration"]), 32 << 20).unwrap();
        for _ in 0..2 {
            for index in 0..2 {
                verify(
                    &decoder
                        .decode(&bytes(&case["frames"][index]["packet"]))
                        .unwrap()
                        .unwrap(),
                    case,
                    index,
                );
            }
            decoder.reset();
        }
        let mut reader =
            NativeReader::software(Cursor::new(bytes(&case["file"])), 32 << 20).unwrap();
        for _ in 0..2 {
            let mut first_rgb = Vec::new();
            for index in 0..2 {
                let raw = reader.read_frame_raw().unwrap().unwrap();
                verify_raw(&raw, case, index);
                assert_eq!(
                    reader.frame_interval().unwrap(),
                    (index as u128, index as u128 + 1, 50)
                );
                let rgb = raw.into_rgb(32 << 20).unwrap();
                assert_eq!(rgb.len(), 32 * 32 * 3);
                if index == 0 {
                    first_rgb = rgb;
                } else {
                    for row in 0..32 {
                        assert!(rgb[row * 96..row * 96 + 48].iter().all(|&v| v == 130));
                        assert_eq!(
                            &rgb[row * 96 + 48..(row + 1) * 96],
                            &first_rgb[row * 96 + 48..(row + 1) * 96]
                        );
                    }
                }
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
        for index in [1, 0, 1] {
            let raw = reader
                .seek_raw(std::time::Duration::from_millis(index as u64 * 20))
                .unwrap()
                .unwrap();
            verify_raw(&raw, case, index);
            assert_eq!(
                reader.frame_interval().unwrap(),
                (index as u128, index as u128 + 1, 50)
            );
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}
