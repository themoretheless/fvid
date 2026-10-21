use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn nonseparate_444_pcm_loop_filter_matches_owned_samples() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-444-pcm-deblock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 54);
    for c in cases {
        if c["mode"] == 1 {
            assert!(c["changed"].as_array().unwrap().iter().all(|n| n == 0));
        }
        let name = c["name"].as_str().unwrap();
        let read = |ext: &str| std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
        let expected: Vec<_> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut decoder = AvcDecoder::new(&read("avcc"), 1 << 20).unwrap();
        for _ in 0..2 {
            let p = decoder
                .decode_order(&read("packet"))
                .unwrap_or_else(|e| panic!("{name}: {e}"))
                .unwrap();
            assert_eq!(p.y, expected[..1024], "{name} Y");
            assert_eq!(p.cb, expected[1024..2048], "{name} Cb");
            assert_eq!(p.cr, expected[2048..], "{name} Cr");
            assert_eq!(p.bit_depth, c["yd"].as_u64().unwrap() as u8);
            assert_eq!(p.chroma_bit_depth, c["cd"].as_u64().unwrap() as u8);
            assert_eq!(p.crop, [1; 4]);
            decoder.reset();
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(read("mp4")),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let frame = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                .unwrap();
            assert_eq!(frame.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            assert_eq!(frame.sample_index, 0);
            let packed = frame.packed.unwrap();
            let yd = c["yd"].as_u64().unwrap() as u8;
            let cd = c["cd"].as_u64().unwrap() as u8;
            let depth = yd.max(cd);
            assert_eq!(packed.depth, depth);
            assert_eq!(packed.frame.subsampling, Some([1, 1]));
            let crop: [usize; 4] = [1; 4];
            let mut bytes = Vec::new();
            for (p, source_depth) in [yd, cd, cd].into_iter().enumerate() {
                for y in crop[2]..32 - crop[3] {
                    for x in crop[0]..32 - crop[1] {
                        let v = expected[p * 1024 + y * 32 + x] << (depth - source_depth);
                        if depth == 8 {
                            bytes.push(v as u8);
                        } else {
                            bytes.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
            assert_eq!(packed.frame.data, bytes, "MP4 {name}");
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn pcm_filter_metadata_is_charged_beyond_the_plane_only_budget() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-444-pcm-deblock");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let planes = 32 * 32 * 6;
    let motion =
        fvid::codec::avc_reference_motion::ReferenceMotionField::storage_bytes(32, 32).unwrap();
    // One retained reference plus the reconstruction planes is enough only
    // when filtering is disabled. Enabled filtering needs its QP/edge grids.
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let packet = std::fs::read(root.join(format!("{name}.packet"))).unwrap();
        let mut decoder = AvcDecoder::new(&config, 2 * planes + motion).unwrap();
        if c["mode"] == 1 {
            assert!(decoder.decode_order(&packet).unwrap().is_some(), "{name}");
        } else {
            let error = decoder.decode_order(&packet).unwrap_err();
            assert!(error.to_string().contains("budget"), "{name}: {error}");
        }
    }
}
