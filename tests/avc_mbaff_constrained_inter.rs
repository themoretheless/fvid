use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn mbaff_constrained_intra_excludes_inter_samples() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-mbaff-constrained-inter");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 16);
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let read = |ext: &str| std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
        let cfg = read("avcc");
        let params = fvid::codec::config::AvcConfig::parse(&cfg).unwrap();
        let sps = fvid::codec::avc::Sps::parse(params.sps[0]).unwrap();
        let pps = fvid::codec::avc::Pps::parse(params.pps[0], &sps).unwrap();
        assert_eq!(sps.chroma_format, 0);
        assert!(sps.mb_adaptive_frame_field);
        assert_eq!(
            pps.constrained_intra_pred,
            c["constrained"].as_bool().unwrap()
        );
        assert_eq!(pps.cabac, c["entropy"].as_i64().unwrap() >= 0);
        let expected: Vec<_> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(expected.len(), 4096);
        let mut decoder = AvcDecoder::new(&cfg, 4 << 20).unwrap();
        for _ in 0..2 {
            for i in 0..2 {
                let packet = std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap();
                let p = decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name} frame{i}: {e}"))
                    .unwrap();
                let golden = &expected[i * 2048..(i + 1) * 2048];
                if let Some(at) = p.y.iter().zip(golden).position(|(a, b)| a != b) {
                    panic!(
                        "{name} frame{i} sample{at}: got {}, expected {}",
                        p.y[at], golden[at]
                    );
                }
                assert_eq!(p.y.len(), 2048);
            }
            decoder.reset();
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(read("mp4")),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for i in 0..2 {
                let f = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    i as i128 * 40_000_000
                );
                let packed = f.packed.unwrap();
                let golden: Vec<_> = expected[i * 2048..(i + 1) * 2048]
                    .iter()
                    .map(|&v| v as u8)
                    .collect();
                assert_eq!(&packed.frame.data[..2048], golden, "MP4 {name} frame{i}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
