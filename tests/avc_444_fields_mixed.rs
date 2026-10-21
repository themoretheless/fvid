use fvid::codec::avc_decoder::AvcDecoder;
fn check(pcm: bool) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-444-fields-mixed");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 108);
    let mut checked = 0;
    for c in cases.iter().filter(|c| c["pcm"] == pcm) {
        checked += 1;
        let name = c["name"].as_str().unwrap();
        let yd = c["yd"].as_u64().unwrap() as u8;
        let cd = c["cd"].as_u64().unwrap() as u8;
        let read = |ext: &str| std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
        let cfg = read("avcc");
        let params = fvid::codec::config::AvcConfig::parse(&cfg).unwrap();
        let sps = fvid::codec::avc::Sps::parse(params.sps[0]).unwrap();
        assert_eq!(sps.level, 41);
        assert_eq!(sps.chroma_format, 3);
        assert!(!sps.frame_mbs_only && !sps.mb_adaptive_frame_field && !sps.separate_colour_plane);
        let expected: Vec<_> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(expected.len(), 6144);
        let packet = |i| std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap();
        let mut decoder = AvcDecoder::new(&cfg, 4 << 20).unwrap();
        for _ in 0..2 {
            assert!(
                decoder
                    .decode_order(&packet(0))
                    .unwrap_or_else(|e| panic!("{name} first: {e}"))
                    .is_none()
            );
            assert!(decoder.has_pending_field());
            let p = decoder
                .decode_order(&packet(1))
                .unwrap_or_else(|e| panic!("{name} second: {e}"))
                .unwrap();
            assert!(!decoder.has_pending_field());
            assert!(decoder.output_is_field_pair());
            assert_eq!(p.chroma_format().unwrap(), 3);
            assert_eq!(p.component_depths(), [yd, cd, cd]);
            assert_eq!(p.crop, [1, 1, 2, 2]);
            assert_eq!(p.dimensions(), (30, 60));
            for (component, plane) in [&p.y, &p.cb, &p.cr].into_iter().enumerate() {
                let golden = &expected[component * 2048..(component + 1) * 2048];
                assert_eq!(plane.len(), golden.len());
                if let Some(at) = plane.iter().zip(golden).position(|(a, b)| a != b) {
                    panic!(
                        "{name} component {component} sample {at}: got {}, expected {}",
                        plane[at], golden[at]
                    );
                }
            }
            decoder.reset();
        }
        let depth = yd.max(cd);
        let mut bytes = Vec::new();
        for (p, source_depth) in [yd, cd, cd].into_iter().enumerate() {
            for y in 2..62 {
                for x in 1..31 {
                    let v = expected[p * 2048 + y * 32 + x] << (depth - source_depth);
                    if depth == 8 {
                        bytes.push(v as u8);
                    } else {
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(read("mp4")),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let f = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                .unwrap();
            assert_eq!(f.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(f.duration.nanoseconds().unwrap(), 40_000_000);
            assert_eq!(f.sample_index, 0);
            let p = f.packed.unwrap();
            assert_eq!(p.depth, depth);
            assert_eq!(p.frame.subsampling, Some([1, 1]));
            assert_eq!(p.frame.data, bytes, "MP4 {name}");
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
    assert_eq!(checked, 108);
}
#[test]
fn nonseparate_444_shared_slice_pcm_and_compressed_fields_match_samples() {
    check(false);
}
