use fvid::codec::avc_decoder::AvcDecoder;
use serde_json::Value;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-switching-fields.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(s: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(s),
    )
    .unwrap()
}
#[test]
fn complementary_sp_si_fields_match_normative_pixels_and_reset() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut d = AvcDecoder::new(&hex(c["configuration"].as_str().unwrap()), 4 << 20).unwrap();
        let expected = fixture(c["reference"].as_str().unwrap());
        for _ in 0..2 {
            let mut actual = vec![];
            let mut emitted = 0;
            for (i, p) in c["packets"].as_array().unwrap().iter().enumerate() {
                let picture = d
                    .decode(&hex(p.as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} packet{i}: {e}", c["file"]));
                assert_eq!(picture.is_some(), i % 2 == 1, "{} packet{i}", c["file"]);
                if let Some(picture) = picture {
                    assert_eq!(picture.dimensions(), (32, 32));
                    picture.write_planar(&mut actual).unwrap();
                    emitted += 1;
                }
            }
            assert_eq!(emitted, c["frame_count"].as_u64().unwrap());
            let difference = actual.iter().zip(&expected).position(|(a, b)| a != b);
            assert_eq!(actual.len(), expected.len());
            assert!(
                difference.is_none(),
                "{} first differing byte {:?}",
                c["file"],
                difference
            );
            assert!(!d.has_pending_field());
            d.reset();
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn switching_field_mp4_software_seek_and_rewind_emit_complete_pairs() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(fixture(c["file"].as_str().unwrap())),
            Default::default(),
            4 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        let expected = fixture(c["reference"].as_str().unwrap());
        for target in [None, Some(2), Some(5), None] {
            if let Some(target) = target {
                assert_eq!(reader.seek_to_sync(target), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            let mut count = 0;
            while let Some(f) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{}: {e}", c["file"]))
            {
                assert_eq!(f.picture.dimensions(), (32, 32));
                actual.extend(
                    f.picture
                        .y
                        .iter()
                        .chain(&f.picture.cb)
                        .chain(&f.picture.cr)
                        .map(|v| *v as u8),
                );
                count += 1;
            }
            assert_eq!(count, c["frame_count"].as_u64().unwrap());
            assert_eq!(actual, expected, "{}", c["file"]);
        }
    }
}
