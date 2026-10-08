//! Owned two-spatial-layer SVC pixels; external oracles run only at generation.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_picture::Picture, av1_sequence::Sequence};
fn pixels(p: &Picture) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, plane) in p.planes.iter().enumerate() {
        let d = if i == 0 { 1 } else { 2 };
        for y in 0..(p.size[1] as usize).div_ceil(d) {
            out.extend(
                plane.samples[y * plane.width..y * plane.width + (p.size[0] as usize).div_ceil(d)]
                    .iter()
                    .map(|v| *v as u8),
            );
        }
    }
    out
}
#[test]
fn owned_spatial_operating_points_pixels_scaled_interlayer_reset_and_configuration() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-spatial-operating-points-generated.json")).unwrap(),
    )
    .unwrap();
    let data = std::fs::read(root.join(manifest["file"].as_str().unwrap())).unwrap();
    let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
    let seq = obus.iter().find(|o| o.kind == 1).unwrap();
    let sequence = Sequence::parse(seq.payload).unwrap();
    assert_eq!(
        sequence
            .operating_points
            .iter()
            .map(|p| p.idc)
            .collect::<Vec<_>>(),
        vec![0x301, 0x101]
    );
    assert_eq!(
        obus.iter()
            .filter(|o| o.kind == 6)
            .map(|o| o.spatial_id)
            .collect::<Vec<_>>(),
        vec![0, 1, 0, 1, 0, 1, 0, 1]
    );
    let end = seq.payload.as_ptr() as usize - data.as_ptr() as usize + seq.payload.len();
    assert_eq!(&data[..2], &[0x12, 0]);
    let mut config = vec![0x81, 0, 0, 0];
    config.extend_from_slice(&data[2..end]);
    for point in manifest["points"].as_array().unwrap() {
        let op = point["point"].as_u64().unwrap() as usize;
        let golden = std::fs::read(root.join(point["reference"].as_str().unwrap())).unwrap();
        for seeded in [false, true] {
            let mut decoder = if seeded {
                Decoder::from_configuration_with_operating_point(&config, 64 << 20, op).unwrap()
            } else {
                Decoder::with_operating_point(64 << 20, op).unwrap()
            };
            for packetized in [false, true] {
                let frames = if packetized {
                    let mut frames = Vec::new();
                    let mut start = 0;
                    for obu in &obus {
                        let end = obu.payload.as_ptr() as usize - data.as_ptr() as usize
                            + obu.payload.len();
                        frames.extend(decoder.decode_packet(&data[start..end]).unwrap());
                        start = end;
                    }
                    assert_eq!(start, data.len());
                    frames
                } else {
                    decoder.decode_packet(&data).unwrap()
                };
                assert_eq!(frames.len(), point["sizes"].as_array().unwrap().len());
                let mut offset = 0;
                let mut scaled = 0;
                for (n, (frame, size)) in frames
                    .iter()
                    .zip(point["sizes"].as_array().unwrap())
                    .enumerate()
                {
                    assert!(frame.show);
                    assert_eq!(frame.spatial_id, if op == 0 { (n % 2) as u8 } else { 0 });
                    assert_eq!(
                        frame.picture.size,
                        [
                            size[0].as_u64().unwrap() as u32,
                            size[1].as_u64().unwrap() as u32
                        ]
                    );
                    let actual = pixels(&frame.picture);
                    assert_eq!(actual, &golden[offset..offset + actual.len()]);
                    offset += actual.len();
                    scaled += frame.picture.inter_prediction.scaled_reference_blocks;
                }
                assert_eq!(offset, golden.len());
                if op == 0 {
                    assert!(
                        scaled > 0,
                        "fixture must reconstruct actual scaled interlayer predictions"
                    );
                } else {
                    assert_eq!(scaled, 0);
                }
                decoder.finish().unwrap();
                decoder.reset();
            }
        }
    }
}
