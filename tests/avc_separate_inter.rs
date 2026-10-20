use fvid::codec::avc_decoder::AvcDecoder;
use std::path::Path;

#[test]
fn separate_plane_inter_keeps_independent_motion_references_and_b_reordering() {
    use fvid::{container::mp4::Limits, playback::video::mp4::Mp4VideoReader};
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-inter");
    let cases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 72);
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
        assert_eq!(golden.len(), 6144);
        let mut decoder = AvcDecoder::new(&config, 1 << 20).unwrap();
        for _ in 0..2 {
            for (frame, packet) in packets.iter().enumerate() {
                let picture = decoder
                    .decode_order(packet)
                    .unwrap_or_else(|e| panic!("{name} frame {frame}: {e}"))
                    .unwrap();
                assert_eq!(picture.chroma_format().unwrap(), 3);
                assert_eq!(picture.crop, [1; 4]);
                assert_eq!(picture.component_depths(), [depth; 3]);
                for (c, plane) in [&picture.y, &picture.cb, &picture.cr]
                    .into_iter()
                    .enumerate()
                {
                    assert_eq!(plane.len(), 512);
                    let offset = frame * 1536 + c * 512;
                    if let Some(i) = plane
                        .iter()
                        .zip(&golden[offset..offset + 512])
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
                assert_eq!(packed.frame.data, expected, "MP4 {name} frame {frame}");
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}

#[test]
fn separate_plane_decoder_reserves_all_three_output_planes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-separate-inter");
    let config = std::fs::read(root.join("separate-inter-8-e0-w0-temporal.avcc")).unwrap();
    let packet = std::fs::read(root.join("separate-inter-8-e0-w0-temporal-0.packet")).unwrap();
    // Three coded 32x16 u16 planes cannot fit in a 3071-byte budget. Keep
    // packet preparation below that threshold so this tests output storage.
    let budget = 32 * 16 * 3 * 2 - 1;
    assert!(packet.len() < budget);
    let mut decoder = AvcDecoder::new(&config, budget).unwrap();
    let error = decoder.decode_order(&packet).unwrap_err().to_string();
    assert!(
        error.contains("separate-plane decoder exceeds memory budget"),
        "{error}"
    );
    assert!(
        decoder
            .decode_order(&packet)
            .unwrap_err()
            .to_string()
            .contains("requires reset")
    );
    decoder.reset();
    assert!(!decoder.has_pending_field());
    assert!(
        decoder
            .decode_order(&packet)
            .unwrap_err()
            .to_string()
            .contains("memory budget")
    );
}

#[test]
fn complete_idr_boundaries_switch_between_separate_and_monochrome_state() {
    use fvid::codec::config::AvcConfig;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let colour_config =
        std::fs::read(root.join("avc-separate-inter/separate-inter-8-e0-w0-temporal.avcc"))
            .unwrap();
    let colour_packet =
        std::fs::read(root.join("avc-separate-inter/separate-inter-8-e0-w0-temporal-0.packet"))
            .unwrap();
    let mono_config =
        std::fs::read(root.join("avc-pcm-formats/avc-pcm-format0-y8-c8-0.avcc")).unwrap();
    let mono_packet =
        std::fs::read(root.join("avc-pcm-formats/avc-pcm-format0-y8-c8-0-0.packet")).unwrap();
    let inband = |configuration: &[u8], packet: &[u8]| {
        let config = AvcConfig::parse(configuration).unwrap();
        let mut out = Vec::new();
        for nal in config.sps.iter().chain(config.pps.iter()) {
            out.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            out.extend_from_slice(nal);
        }
        out.extend_from_slice(packet);
        out
    };
    let mut decoder = AvcDecoder::new(&colour_config, 1 << 20).unwrap();
    let first = decoder.decode_order(&colour_packet).unwrap().unwrap();
    assert_eq!(first.chroma_format().unwrap(), 3);
    let mono = decoder
        .decode_order(&inband(&mono_config, &mono_packet))
        .unwrap()
        .unwrap();
    assert_eq!(mono.chroma_format().unwrap(), 0);
    let again = decoder
        .decode_order(&inband(&colour_config, &colour_packet))
        .unwrap()
        .unwrap();
    assert_eq!(again.chroma_format().unwrap(), 3);
    assert_eq!(
        (&first.y, &first.cb, &first.cr),
        (&again.y, &again.cb, &again.cr)
    );
    decoder.reset();
    let reset = decoder.decode_order(&colour_packet).unwrap().unwrap();
    assert_eq!(
        (&first.y, &first.cb, &first.cr),
        (&reset.y, &reset.cb, &reset.cr)
    );
}
