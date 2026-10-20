use fvid::codec::avc_decoder::AvcDecoder;
use std::path::{Path, PathBuf};

fn fixtures() -> (PathBuf, serde_json::Value, Vec<u16>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-non420-paff-filtered-pcm");
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
    let fmt = case["format"].as_u64().unwrap() as usize;
    let w = case["width"].as_u64().unwrap() as usize;
    let h = case["height"].as_u64().unwrap() as usize;
    let crop: [usize; 4] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as usize);
    let depths = [
        case["luma_depth"].as_u64().unwrap() as u8,
        case["chroma_depth"].as_u64().unwrap() as u8,
    ];
    let output_depth = if fmt == 0 {
        depths[0]
    } else {
        depths[0].max(depths[1])
    };
    let sx = if fmt == 2 { 2 } else { 1 };
    let mut output = Vec::new();
    let mut at = frame["offset"].as_u64().unwrap() as usize;
    for c in 0..3 {
        if c != 0 && fmt == 0 {
            if packed {
                let cw = (w - crop[0] - crop[1]).div_ceil(2);
                let ch = (h - crop[2] - crop[3]).div_ceil(2);
                let v = 1u16 << (output_depth - 1);
                for _ in 0..cw * ch {
                    if output_depth == 8 {
                        output.push(v as u8);
                    } else {
                        output.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            continue;
        }
        let sub_x = if c == 0 { 1 } else { sx };
        let stride = w / sub_x;
        let count = stride * h;
        let plane = &golden[at..at + count];
        at += count;
        let source_depth = depths[usize::from(c != 0)];
        let depth = if packed { output_depth } else { source_depth };
        for y in crop[2]..h - crop[3] {
            for x in crop[0] / sub_x..(w - crop[1]) / sub_x {
                let value = plane[y * stride + x] << (depth - source_depth);
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
fn non_420_paff_frames_and_filtered_pcm_decode() {
    let (root, manifest, golden) = fixtures();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 336);
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let kind = case["kind"].as_str().unwrap();
        let fmt = case["format"].as_u64().unwrap() as u8;
        let w = case["width"].as_u64().unwrap() as usize;
        let h = case["height"].as_u64().unwrap() as usize;
        let count = w * h;
        let sx = if fmt == 2 { 2 } else { 1 };
        let chroma_count = if fmt == 0 { 0 } else { count / sx };
        let crop: [usize; 4] = std::array::from_fn(|i| case["crop"][i].as_u64().unwrap() as usize);
        let visible_w = w - crop[0] - crop[1];
        let visible_h = h - crop[2] - crop[3];
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let packed_depth = if fmt == 0 { yd } else { yd.max(cd) };
        let config = std::fs::read(root.join(format!("{name}.avcc"))).unwrap();
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            for (i, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let picture = if kind == "paff_field" {
                    let packets = frame["packets"].as_array().unwrap();
                    let p0 = std::fs::read(root.join(packets[0].as_str().unwrap())).unwrap();
                    let p1 = std::fs::read(root.join(packets[1].as_str().unwrap())).unwrap();
                    assert!(decoder
                        .decode_order(&p0)
                        .unwrap_or_else(|e| panic!("{name} field 0: {e}"))
                        .is_none());
                    assert!(decoder.has_pending_field());
                    let pic = decoder
                        .decode_order(&p1)
                        .unwrap_or_else(|e| panic!("{name} field 1: {e}"))
                        .unwrap();
                    assert!(!decoder.has_pending_field());
                    assert!(decoder.output_is_field_pair());
                    pic
                } else {
                    let packet =
                        std::fs::read(root.join(frame["packet"].as_str().unwrap())).unwrap();
                    let pic = decoder
                        .decode_order(&packet)
                        .unwrap_or_else(|e| panic!("{name} frame {i}: {e}"))
                        .unwrap();
                    assert!(!decoder.has_pending_field());
                    assert!(!decoder.output_is_field_pair());
                    pic
                };
                assert_eq!(picture.chroma_format().unwrap(), fmt);
                assert_eq!(picture.crop, crop);
                assert_eq!(picture.dimensions(), (visible_w, visible_h));
                assert_eq!(picture.bit_depth, yd);
                assert_eq!(picture.chroma_bit_depth, cd);
                let at = frame["offset"].as_u64().unwrap() as usize;
                assert_eq!(picture.y, golden[at..at + count], "{name} Y");
                assert_eq!(
                    picture.cb,
                    golden[at + count..at + count + chroma_count],
                    "{name} Cb"
                );
                assert_eq!(
                    picture.cr,
                    golden[at + count + chroma_count..at + count + 2 * chroma_count],
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
        let frames = case["frames"].as_array().unwrap();
        let presentation_order: Vec<(usize, &serde_json::Value)> = if kind == "paff_frame_422_b" {
            vec![(0, &frames[0]), (2, &frames[2]), (1, &frames[1])]
        } else {
            frames.iter().enumerate().collect()
        };
        for _ in 0..2 {
            for (i, (sample_index, frame_meta)) in presentation_order.iter().enumerate() {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("MP4 {name} frame {i}: {e}"))
                    .unwrap();
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    i as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
                assert_eq!(frame.sample_index, *sample_index);
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, packed_depth);
                assert_eq!(
                    packed.frame.subsampling,
                    Some(if fmt == 2 {
                        [2, 1]
                    } else if fmt == 3 {
                        [1, 1]
                    } else {
                        [2, 2]
                    })
                );
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

