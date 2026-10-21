use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn shared_444_inter_deblock_uses_component_counts_and_qps() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-444-inter-deblock");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 54);
    let witnesses: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("witnesses.json")).unwrap()).unwrap();
    let witnesses = witnesses.as_object().unwrap();
    assert_eq!(witnesses.len(), 10);
    assert!(witnesses.values().all(|v| v.as_u64().unwrap() > 0));
    for c in cases {
        let name = c["name"].as_str().unwrap();
        let yd = c["yd"].as_u64().unwrap() as u8;
        let cd = c["cd"].as_u64().unwrap() as u8;
        let read = |ext: &str| std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
        let cfg = read("avcc");
        let params = fvid::codec::config::AvcConfig::parse(&cfg).unwrap();
        let sps = fvid::codec::avc::Sps::parse(params.sps[0]).unwrap();
        assert_eq!(sps.level, 41);
        assert_eq!(sps.chroma_format, 3);
        assert!(sps.frame_mbs_only && !sps.separate_colour_plane);
        let expected: Vec<_> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(expected.len(), 4608);
        let mut decoder = AvcDecoder::new(&cfg, 4 << 20).unwrap();
        for _ in 0..2 {
            for i in 0..3 {
                let packet = std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap();
                let p = decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name} frame {i}: {e}"))
                    .unwrap();
                assert_eq!(p.component_depths(), [yd, cd, cd]);
                assert_eq!(p.dimensions(), (30, 14));
                for (component, plane) in [&p.y, &p.cb, &p.cr].into_iter().enumerate() {
                    let golden =
                        &expected[i * 1536 + component * 512..i * 1536 + (component + 1) * 512];
                    assert_eq!(plane.len(), golden.len());
                    if let Some(at) = plane.iter().zip(golden).position(|(a, b)| a != b) {
                        panic!(
                            "{name} frame {i} component {component} sample {at}: got {}, expected {}",
                            plane[at], golden[at]
                        );
                    }
                }
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
            for i in 0..3 {
                let f = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    i as i128 * 40_000_000
                );
                assert_eq!(f.duration.nanoseconds().unwrap(), 40_000_000);
                let packed = f.packed.unwrap();
                let mut bytes = Vec::new();
                let depth = yd.max(cd);
                for (component, source_depth) in [yd, cd, cd].into_iter().enumerate() {
                    for y in 1..15 {
                        for x in 1..31 {
                            let v = expected[i * 1536 + component * 512 + y * 32 + x]
                                << (depth - source_depth);
                            if depth == 8 {
                                bytes.push(v as u8);
                            } else {
                                bytes.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                assert_eq!(packed.frame.data, bytes, "MP4 {name} frame {i}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
