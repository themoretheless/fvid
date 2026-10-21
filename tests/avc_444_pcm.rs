use fvid::codec::avc_decoder::AvcDecoder;
use std::path::{Path, PathBuf};

fn fixtures() -> (PathBuf, serde_json::Value, Vec<u16>) {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avc-444-pcm");
    let manifest =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let golden = std::fs::read(root.join("samples.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    (root, manifest, golden)
}

fn expected_bytes(
    case: &serde_json::Value,
    frame: &serde_json::Value,
    golden: &[u16],
    packed: bool,
) -> Vec<u8> {
    let w = case["width"].as_u64().unwrap() as usize;
    let h = case["height"].as_u64().unwrap() as usize;
    let count = w * h;
    let crop: [usize; 4] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as usize);
    let depths = [
        case["luma_depth"].as_u64().unwrap() as u8,
        case["chroma_depth"].as_u64().unwrap() as u8,
    ];
    let output_depth = depths[0].max(depths[1]);
    let mut output = Vec::new();
    let mut at = frame["offset"].as_u64().unwrap() as usize;
    for c in 0..3 {
        let plane = &golden[at..at + count];
        at += count;
        let source_depth = depths[usize::from(c != 0)];
        let depth = if packed { output_depth } else { source_depth };
        for y in crop[2]..h - crop[3] {
            for x in crop[0]..w - crop[1] {
                let value = plane[y * w + x] << (depth - source_depth);
                if depth == 8 {
                    output.push(value as u8);
                } else {
                    output.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
    }
    output
}

#[test]
fn non_separated_444_progressive_cabac_and_mbaff_pcm_decode() {
    let (root, manifest, golden) = fixtures();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 132);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let w = case["width"].as_u64().unwrap() as usize;
        let h = case["height"].as_u64().unwrap() as usize;
        let count = w * h;
        let crop: [usize; 4] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as usize);
        let visible_w = w - crop[0] - crop[1];
        let visible_h = h - crop[2] - crop[3];
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let packed_depth = yd.max(cd);
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let packet = std::fs::read(root.join(frame["packet"].as_str().unwrap())).unwrap();
                let picture = decoder
                    .decode_order(&packet)
                    .unwrap_or_else(|e| panic!("{name} frame {i}: {e}"))
                    .unwrap();
                assert!(!decoder.has_pending_field());
                assert!(!decoder.output_is_field_pair());
                assert_eq!(picture.chroma_format().unwrap(), 3);
                assert_eq!(picture.crop, crop);
                assert_eq!(picture.dimensions(), (visible_w, visible_h));
                assert_eq!(picture.bit_depth, yd);
                assert_eq!(picture.chroma_bit_depth, cd);
                let at = frame["offset"].as_u64().unwrap() as usize;
                assert_eq!(picture.y, golden[at..at + count], "{name} Y");
                assert_eq!(picture.cb, golden[at + count..at + 2 * count], "{name} Cb");
                assert_eq!(
                    picture.cr,
                    golden[at + 2 * count..at + 3 * count],
                    "{name} Cr"
                );
                let mut planar = Vec::new();
                picture.write_planar(&mut planar).unwrap();
                assert_eq!(
                    planar,
                    expected_bytes(case, frame, &golden, false),
                    "{name} planar"
                );
            }
            decoder.reset();
        }
        let mp4 = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(mp4),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            for (i, frame_meta) in case["frames"].as_array().unwrap().iter().enumerate() {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name} frame {i}: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    i as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(frame.sample_index, i);
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, packed_depth);
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                assert_eq!(
                    packed.frame.data,
                    expected_bytes(case, frame_meta, &golden, true),
                    "packed {name}"
                );
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn non_separated_444_pcm_enforces_exact_memory_budgets() {
    let (root, manifest, _) = fixtures();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let w = case["width"].as_u64().unwrap() as usize;
        let h = case["height"].as_u64().unwrap() as usize;
        let storage = w * h * 6;
        let motion =
            fvid::codec::avc_reference_motion::ReferenceMotionField::storage_bytes(w, h).unwrap();
        let refs = if case["kind"] == "mbaff" { 2 } else { 1 };
        let working = if case["kind"] == "mbaff" {
            (w * h / 256) * 256 + 16384
        } else {
            0
        };
        let required = refs * (storage + motion) + storage + working;
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let packet = std::fs::read(root.join(format!("{name}-0.packet"))).unwrap();
        let mut under = AvcDecoder::new(&config, required - 1).unwrap();
        let err = under.decode_order(&packet).unwrap_err();
        assert!(err.to_string().contains("budget"), "{name}: {err}");
        let mut exact = AvcDecoder::new(&config, required).unwrap();
        assert!(exact.decode_order(&packet).unwrap().is_some(), "{name}");
    }
    let unsupported: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("unsupported.json")).unwrap()).unwrap();
    assert!(unsupported.as_array().unwrap().is_empty());
}

#[test]
fn historical_mbaff_refusals_are_complete_component_acceptance() {
    let (root, _, _) = fixtures();
    let accepted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("accepted-mbaff.json")).unwrap()).unwrap();
    assert_eq!(accepted.as_array().unwrap().len(), 4);
    for case in accepted.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let read = |ext: &str| std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
        let expected: Vec<u16> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut decoder = AvcDecoder::new(&read("avcc"), 1 << 20).unwrap();
        let picture = decoder
            .decode_order(&read("packet"))
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .unwrap();
        assert_eq!(picture.y, expected[..2048]);
        assert_eq!(picture.cb, expected[2048..4096]);
        assert_eq!(picture.cr, expected[4096..]);
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(read("mp4")),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        let packed = reader.read_frame().unwrap().unwrap().packed.unwrap();
        let mut bytes = Vec::new();
        for component in 0..3 {
            for y in 2..62 {
                for x in 1..31 {
                    bytes.push(expected[component * 2048 + y * 32 + x] as u8);
                }
            }
        }
        assert_eq!(packed.frame.data, bytes, "{name}");
        assert!(reader.read_frame().unwrap().is_none());
    }
}

#[test]
fn progressive_nonseparate_cabac_refusal_is_now_complete_component_acceptance() {
    let (root, _, _) = fixtures();
    let accepted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("accepted.json")).unwrap()).unwrap();
    assert_eq!(accepted.as_array().unwrap().len(), 1);
    for case in accepted.as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let read = |ext: &str| std::fs::read(root.join(format!("{name}.{ext}"))).unwrap();
        let expected: Vec<u16> = read("y16")
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        let mut decoder = AvcDecoder::new(&read("avcc"), 1 << 20).unwrap();
        for _ in 0..2 {
            let picture = decoder.decode_order(&read("packet")).unwrap().unwrap();
            assert_eq!(picture.y, expected[..512]);
            assert_eq!(picture.cb, expected[512..1024]);
            assert_eq!(picture.cr, expected[1024..]);
            decoder.reset();
        }
        let mut reader = fvid::playback::video::mp4::Mp4VideoReader::open_software(
            std::io::Cursor::new(read("mp4")),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let frame = reader.read_frame().unwrap().unwrap();
            assert_eq!(frame.presentation_time.nanoseconds().unwrap(), 0);
            assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            let packed = frame.packed.unwrap();
            assert_eq!(packed.depth, 8);
            assert_eq!(packed.frame.subsampling, Some([1, 1]));
            assert_eq!(
                packed.frame.data,
                expected.iter().map(|n| *n as u8).collect::<Vec<_>>()
            );
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
