use fvid::codec::avc_decoder::AvcDecoder;
use serde_json::Value;

fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-primary-sp-multimb.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
#[test]
fn primary_sp_cross_macroblock_and_slice_filtering_matches_normative_oracle() {
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let reference = fixture(c["reference"].as_str().unwrap());
        let mut decoder =
            AvcDecoder::new(&hex(c["configuration"].as_str().unwrap()), 1 << 20).unwrap();
        for _ in 0..2 {
            let mut actual = vec![];
            for p in c["packets"].as_array().unwrap() {
                decoder
                    .decode(&hex(p.as_str().unwrap()))
                    .unwrap()
                    .unwrap()
                    .write_planar(&mut actual)
                    .unwrap();
            }
            assert_eq!(actual, reference, "{}", c["file"]);
            decoder.reset();
        }
    }
    let reference = |slices, mode| {
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["slices"] == slices && c["mode"] == mode)
            .unwrap();
        fixture(c["reference"].as_str().unwrap())
    };
    // A single slice permits identical boundaries in modes0/2. Four slices
    // suppress external boundaries only in mode2; mode1 disables all filtering.
    assert_eq!(reference(1, 0), reference(1, 2));
    assert_ne!(reference(4, 0), reference(4, 2));
    assert_ne!(reference(4, 1), reference(4, 2));
}

#[cfg(feature = "player")]
#[test]
fn primary_sp_multislice_software_player_rewind_and_seek_match_normative_oracle() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mut reader = fvid::playback_mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(fixture(c["file"].as_str().unwrap())),
            Default::default(),
            1 << 20,
        )
        .unwrap();
        assert!(!reader.hardware_accelerated());
        let expected = fixture(c["reference"].as_str().unwrap());
        for target in [None, Some(2), Some(3), None] {
            if let Some(target) = target {
                assert_eq!(reader.seek_to_sync(target), 0);
            } else {
                reader.rewind();
            }
            let mut actual = vec![];
            while let Some(f) = reader.read_frame().unwrap() {
                actual.extend(
                    f.picture
                        .y
                        .iter()
                        .chain(&f.picture.cb)
                        .chain(&f.picture.cr)
                        .map(|v| *v as u8),
                );
            }
            assert_eq!(actual, expected, "{}", c["file"]);
        }
    }
}
