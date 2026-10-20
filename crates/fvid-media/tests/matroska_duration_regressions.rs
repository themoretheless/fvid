use fvid_media::owned_webm::{Limits, WebmReader};
use std::{fs, io::Cursor, path::PathBuf};

fn check(name: &str) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/matroska-duration");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let case = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["file"] == name)
        .unwrap();
    let data = fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
    let mut reader = WebmReader::open(Cursor::new(data), Limits::default()).unwrap();
    assert_eq!(
        reader.duration_ns,
        case["expected_ns"].as_u64(),
        "{}",
        case["file"]
    );
    reader.scan_all().unwrap();
    assert_eq!(reader.packets.len(), 3);
    for index in 0..3 {
        let picture = fvid_codecs::codec::vp9_decoder::Decoder::new(32 << 20)
            .decode(&reader.read_packet(index).unwrap())
            .unwrap()
            .picture;
        assert_eq!(picture.size, [[16, 16], [32, 16], [16, 32]][index]);
    }
}

#[test]
fn ordinary_duration_avoids_double_rounding() {
    check("double-rounding.mkv");
}
#[test]
fn integral_duration_keeps_the_full_scaled_product() {
    check("integer-product.mkv");
}
#[test]
fn representable_upper_duration_is_accepted() {
    check("representable-upper-bound.mkv");
}
