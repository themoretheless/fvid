use fvid::codec::avc_decoder::AvcDecoder;
use std::path::Path;

#[test]
fn separate_intra_fields_weave_cavlc_cabac_samples_in_both_arrival_orders() {
    use fvid::{container::mp4::Limits, playback::video::mp4::Mp4VideoReader};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-fields");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 120);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let parameters = fvid::codec::config::AvcConfig::parse(&config).unwrap();
        let sequence = fvid::codec::avc::Sps::parse(parameters.sps[0]).unwrap();
        assert_eq!(config[3], 41);
        assert_eq!(sequence.level, 41);
        assert!(
            !sequence.frame_mbs_only
                && !sequence.mb_adaptive_frame_field
                && sequence.separate_colour_plane
        );
        let packets: Vec<_> = (0..2)
            .map(|i| std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
            .collect();
        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            assert!(
                decoder
                    .decode_order(&packets[0])
                    .unwrap_or_else(|e| panic!("{name} first: {e}"))
                    .is_none()
            );
            assert!(decoder.has_pending_field());
            let picture = decoder
                .decode_order(&packets[1])
                .unwrap_or_else(|e| panic!("{name} second: {e}"))
                .unwrap();
            assert!(!decoder.has_pending_field());
            assert!(decoder.output_is_field_pair());
            assert_eq!(picture.chroma_format().unwrap(), 3);
            assert_eq!(picture.crop, [1, 1, 2, 2]);
            assert_eq!(picture.component_depths(), [depth; 3]);
            assert_eq!(picture.dimensions(), (30, 28));
            for (c, plane) in [&picture.y, &picture.cb, &picture.cr]
                .into_iter()
                .enumerate()
            {
                assert_eq!(plane.len(), 1024);
                if let Some(i) = plane
                    .iter()
                    .zip(&golden[c * 1024..(c + 1) * 1024])
                    .position(|(a, b)| a != b)
                {
                    panic!(
                        "{name} component {c} sample {i}: got {}, expected {}",
                        plane[i],
                        golden[c * 1024 + i]
                    );
                }
            }
            decoder.reset();
        }
        let mut expected = Vec::new();
        for c in 0..3 {
            for y in 2..30 {
                for x in 1..31 {
                    let value = golden[c * 1024 + y * 32 + x];
                    if depth == 8 {
                        expected.push(value as u8);
                    } else {
                        expected.extend_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
        let bytes = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(std::io::Cursor::new(bytes), Limits::default(), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            let frame = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                .unwrap();
            assert_eq!(frame.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            let packed = frame.packed.unwrap();
            assert_eq!(packed.depth, depth);
            assert_eq!(packed.frame.subsampling, Some([1, 1]));
            assert_eq!(packed.frame.data, expected, "MP4 {name}");
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
