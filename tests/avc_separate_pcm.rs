use fvid::codec::avc_decoder::AvcDecoder;
use std::path::Path;

#[test]
fn separate_colour_pcm_preserves_all_planes_and_arbitrary_slice_order() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-pcm");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for name in manifest["cases"].as_array().unwrap() {
        let name = name.as_str().unwrap();
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
            .unwrap()
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            for frame in 0..2 {
                let packet = std::fs::read(root.join(format!("{name}-{frame}.packet"))).unwrap();
                let picture = decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name} frame {frame}: {e}"))
                    .unwrap();
                assert_eq!(picture.chroma_format().unwrap(), 3);
                assert_eq!(picture.crop, [1; 4]);
                for (c, plane) in [&picture.y, &picture.cb, &picture.cr]
                    .into_iter()
                    .enumerate()
                {
                    let start = frame * 1536 + c * 512;
                    assert_eq!(plane, &golden[start..start + 512], "{name} plane {c}");
                }
            }
            decoder.reset();
        }
    }
}

#[test]
fn separate_plane_mp4_preserves_visible_samples_timestamps_and_rewind() {
    use fvid::{container::mp4::Limits, playback::video::mp4::Mp4VideoReader};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-pcm");
    for depth in [8, 10, 14] {
        for reverse in [0, 1] {
            let name = format!("separate-pcm-{depth}-{reverse}");
            let golden: Vec<u16> = std::fs::read(root.join(format!("{name}.y16")))
                .unwrap()
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();
            let data = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
            let mut reader =
                Mp4VideoReader::open(std::io::Cursor::new(data), Limits::default(), 16 << 20)
                    .unwrap();
            for _ in 0..2 {
                for frame in 0..2 {
                    let decoded = reader
                        .read_frame()
                        .unwrap_or_else(|e| panic!("{name}: {e}"))
                        .unwrap();
                    let packed = decoded.packed.as_ref().unwrap();
                    assert_eq!(packed.depth, depth);
                    assert_eq!(packed.frame.subsampling, Some([1, 1]));
                    assert_eq!(
                        decoded.presentation_time.nanoseconds().unwrap(),
                        frame as i128 * 40_000_000
                    );
                    let mut expected = Vec::new();
                    for c in 0..3 {
                        for y in 1..15 {
                            for x in 1..31 {
                                let value = golden[frame * 1536 + c * 512 + y * 32 + x];
                                if depth == 8 {
                                    expected.push(value as u8);
                                } else {
                                    expected.extend_from_slice(&value.to_le_bytes());
                                }
                            }
                        }
                    }
                    assert_eq!(packed.frame.data, expected, "{name} frame {frame}");
                }
                assert!(reader.read_frame().unwrap().is_none());
                reader.rewind();
            }
        }
    }
}

#[test]
fn separate_plane_coverage_rejects_missing_duplicate_and_nonzero_first_slices() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-pcm");
    let config = std::fs::read(root.join("separate-pcm-8-0.avcc")).unwrap();
    let packet = std::fs::read(root.join("separate-pcm-8-0-0.packet")).unwrap();
    let mut nals = Vec::new();
    let mut at = 0;
    while at < packet.len() {
        let size = u32::from_be_bytes(packet[at..at + 4].try_into().unwrap()) as usize;
        nals.push(&packet[at..at + 4 + size]);
        at += 4 + size;
    }
    for (indices, expected) in [
        (vec![0, 1, 2, 3], "missing a colour plane"),
        (vec![0, 0, 2, 3, 4, 5], "overlap"),
        (
            vec![0, 1, 2, 3, 5],
            "colour plane must cover macroblock zero",
        ),
    ] {
        let bad: Vec<u8> = indices
            .into_iter()
            .flat_map(|i| nals[i].iter().copied())
            .collect();
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        let error = decoder.decode_order(&bad).unwrap_err().to_string();
        assert!(error.contains(expected), "expected {expected}, got {error}");
        assert!(
            decoder
                .decode_order(&packet)
                .unwrap_err()
                .to_string()
                .contains("requires reset")
        );
        decoder.reset();
        assert!(decoder.decode_order(&packet).unwrap().is_some());
    }
}
