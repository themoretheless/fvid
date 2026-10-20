use fvid::codec::avc_decoder::AvcDecoder;
use std::path::Path;

#[test]
fn separate_mbaff_b_uses_each_planes_colocated_motion_and_vertical_units() {
    use fvid::{container::mp4::Limits, playback::video::mp4::Mp4VideoReader};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-mbaff-b");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 240);
    for case in cases.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let depth = case["depth"].as_u64().unwrap() as u8;
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let packets: Vec<_> = (0..4)
            .map(|i| std::fs::read(root.join(format!("{name}-{i}.packet"))).unwrap())
            .collect();
        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect();
        assert_eq!(golden.len(), 24576);
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            for (frame, packet) in packets.iter().enumerate() {
                let picture = decoder
                    .decode_order(packet)
                    .unwrap_or_else(|e| panic!("{name} frame {frame}: {e}"))
                    .unwrap();
                assert_eq!(picture.chroma_format().unwrap(), 3);
                assert_eq!(picture.crop, [1, 1, 2, 2]);
                assert_eq!(picture.component_depths(), [depth; 3]);
                for (c, plane) in [&picture.y, &picture.cb, &picture.cr]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(plane.len(), 2048);
                    let offset = frame * 6144 + c * 2048;
                    if let Some(i) = plane
                        .iter()
                        .zip(&golden[offset..offset + 2048])
                        .position(|(a, b)| a != b)
                    {
                        panic!(
                            "{name} frame {frame} component {c} sample {i}: got {}, expected {}",
                            plane[i],
                            golden[offset + i]
                        );
                    }
                }
            }
            decoder.reset();
        }
        let bytes = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(std::io::Cursor::new(bytes), Limits::default(), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            for (shown, frame) in [0, 2, 1, 3].into_iter().enumerate() {
                let decoded = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name}: {e}"))
                    .unwrap();
                assert_eq!(decoded.sample_index, frame);
                assert_eq!(
                    decoded.presentation_time.nanoseconds().unwrap(),
                    shown as i128 * 40_000_000
                );
                assert_eq!(decoded.duration.nanoseconds().unwrap(), 40_000_000);
                let packed = decoded.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                let mut expected = Vec::new();
                for c in 0..3 {
                    for y in 2..62 {
                        for x in 1..31 {
                            let value = golden[frame * 6144 + c * 2048 + y * 32 + x];
                            if depth == 8 {
                                expected.push(value as u8);
                            } else {
                                expected.extend_from_slice(&value.to_le_bytes());
                            }
                        }
                    }
                }
                assert_eq!(packed.frame.data, expected, "MP4 {name} frame {frame}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
