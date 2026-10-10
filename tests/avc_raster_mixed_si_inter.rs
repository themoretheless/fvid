use fvid::codec::avc_decoder::AvcDecoder;
use serde_json::Value;
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest(name: &str) -> Value {
    serde_json::from_slice(&fixture(name)).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn assert_pixels(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}: output length");
    assert_eq!(
        actual.iter().zip(expected).position(|(a, b)| a != b),
        None,
        "{name}: first differing byte"
    );
}
fn decode_case(case: &Value) {
    let name = case["file"].as_str().unwrap();
    let mut decoder =
        AvcDecoder::new(&hex(case["configuration"].as_str().unwrap()), 4 << 20).unwrap();
    let expected = fixture(case["reference"].as_str().unwrap());
    for _ in 0..2 {
        let mut actual = vec![];
        for (index, packet) in case["packets"].as_array().unwrap().iter().enumerate() {
            let picture = decoder
                .decode(&hex(packet.as_str().unwrap()))
                .unwrap_or_else(|e| panic!("{name} packet{index}: {e}"))
                .unwrap_or_else(|| panic!("{name} packet{index}: missing raster picture"));
            assert_eq!(picture.dimensions(), (32, 32), "{name}");
            picture.write_planar(&mut actual).unwrap();
        }
        assert_pixels(&actual, &expected, name);
        assert!(!decoder.has_pending_field());
        decoder.reset();
    }
}
#[test]
fn raster_reproducer_si_top_accepts_mixed_picture() {
    decode_case(&manifest("avc-raster-mixed-si-inter-reproducer.json")["cases"][0]);
}
#[test]
fn raster_reproducer_si_bottom_accepts_mixed_picture() {
    decode_case(&manifest("avc-raster-mixed-si-inter-reproducer.json")["cases"][1]);
}
#[test]
fn raster_mixed_si_inter_normative_pixels_and_reset() {
    for case in manifest("avc-raster-mixed-si-inter.json")["cases"]
        .as_array()
        .unwrap()
    {
        decode_case(case);
    }
}
#[cfg(feature = "player")]
#[test]
fn raster_mixed_si_inter_software_seek_and_rewind() {
    for case in manifest("avc-raster-mixed-si-inter.json")["cases"]
        .as_array()
        .unwrap()
    {
        let name = case["file"].as_str().unwrap();
        let expected = fixture(case["reference"].as_str().unwrap());
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(fixture(name)),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        for target in [None, Some(2), Some(5), None] {
            if let Some(t) = target {
                assert_eq!(reader.seek_to_sync(t), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            let mut count = 0;
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                assert_eq!(frame.picture.dimensions(), (32, 32), "{name}");
                actual.extend(
                    frame
                        .picture
                        .y
                        .iter()
                        .chain(&frame.picture.cb)
                        .chain(&frame.picture.cr)
                        .map(|v| *v as u8),
                );
                count += 1;
            }
            assert_eq!(count, case["frame_count"].as_u64().unwrap());
            assert_pixels(&actual, &expected, name);
        }
    }
}
