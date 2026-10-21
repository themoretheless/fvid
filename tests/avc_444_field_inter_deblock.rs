use fvid::codec::avc_decoder::AvcDecoder;
#[test]
fn shared_444_field_inter_filter_keeps_component_flags_and_mapped_qps() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-444-field-inter-deblock");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = cases.as_array().unwrap();
    assert_eq!(cases.len(), 432);
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
        let pps = fvid::codec::avc::Pps::parse(params.pps[0], &sps).unwrap();
        assert_eq!(sps.level, 41);
        assert_eq!(sps.chroma_format, 3);
        assert!(!sps.frame_mbs_only && !sps.separate_colour_plane && !sps.mb_adaptive_frame_field);
        let entropy = c["entropy"].as_i64().unwrap();
        assert_eq!(pps.cabac, entropy >= 0);
        let expected: Vec<_> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(expected.len(), 6144);
        let mut decoder = AvcDecoder::new(&cfg, 4 << 20).unwrap();
        for _ in 0..2 {
            for i in 0..4 {
                let packet = std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap();
                if i >= 2 {
                    let header =
                        fvid::codec::avc_slice::SliceHeader::parse(&packet[4..], &sps, &pps)
                            .unwrap();
                    assert!(header.field_pic);
                    assert_eq!(
                        header.bottom_field,
                        c["reverse"].as_bool().unwrap() ^ (i % 2 != 0)
                    );
                    if entropy >= 0 {
                        assert_eq!(header.cabac_init_idc as i64, entropy);
                    }
                }
                let frame = decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name} field{i}: {e}"));
                if i % 2 == 0 {
                    assert!(frame.is_none());
                    assert!(decoder.has_pending_field());
                } else {
                    let p = frame.unwrap();
                    assert!(!decoder.has_pending_field());
                    assert!(decoder.output_is_field_pair());
                    assert_eq!(p.component_depths(), [yd, cd, cd]);
                    assert_eq!(p.dimensions(), (30, 28));
                    assert_eq!(p.crop, [1, 1, 2, 2]);
                    for (component, plane) in [&p.y, &p.cb, &p.cr].into_iter().enumerate() {
                        let at = i / 2 * 3072 + component * 1024;
                        let golden = &expected[at..at + 1024];
                        assert_eq!(plane.len(), 1024);
                        if let Some(pos) = plane.iter().zip(golden).position(|(a, b)| a != b) {
                            panic!(
                                "{name} pair{} component{component} sample{pos}: got {}, expected {}",
                                i / 2,
                                plane[pos],
                                golden[pos]
                            );
                        }
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
            for pair in 0..2 {
                let f = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                assert_eq!(
                    f.presentation_time.nanoseconds().unwrap(),
                    pair as i128 * 40_000_000
                );
                assert_eq!(f.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(f.sample_index, pair * 2);
                let packed = f.packed.unwrap();
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                let depth = yd.max(cd);
                assert_eq!(packed.depth, depth);
                let mut bytes = Vec::new();
                for (component, source_depth) in [yd, cd, cd].into_iter().enumerate() {
                    for y in 2..30 {
                        for x in 1..31 {
                            let v = expected[pair * 3072 + component * 1024 + y * 32 + x]
                                << (depth - source_depth);
                            if depth == 8 {
                                bytes.push(v as u8);
                            } else {
                                bytes.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
                assert_eq!(packed.frame.data, bytes, "MP4 {name} pair{pair}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
