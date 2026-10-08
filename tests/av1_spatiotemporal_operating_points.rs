//! Owned three-spatial/three-temporal-layer SVC, checked against dual oracles.
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
fn all_nine_operating_points_pixels_filtering_reset_configuration_and_display() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-spatiotemporal-operating-points-generated.json")).unwrap(),
    )
    .unwrap();
    let data = std::fs::read(root.join(m["file"].as_str().unwrap())).unwrap();
    let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
    let sequence = obus.iter().find(|o| o.kind == 1).unwrap();
    let s = Sequence::parse(sequence.payload).unwrap();
    assert_eq!(
        s.operating_points.iter().map(|p| p.idc).collect::<Vec<_>>(),
        vec![
            0x707, 0x703, 0x701, 0x307, 0x303, 0x301, 0x107, 0x103, 0x101
        ]
    );
    let tids = [0, 2, 1, 2, 0, 2, 1, 2];
    assert_eq!(
        obus.iter()
            .filter(|o| o.kind == 6)
            .map(|o| {
                assert!(o.has_extension);
                (o.spatial_id, o.temporal_id)
            })
            .collect::<Vec<_>>(),
        (0..24)
            .map(|i| ((i % 3) as u8, tids[i / 3]))
            .collect::<Vec<_>>()
    );
    let mut boundaries: Vec<_> = obus
        .iter()
        .filter(|o| o.kind == 2)
        .map(|o| {
            let end = o.payload.as_ptr() as usize - data.as_ptr() as usize;
            assert_eq!(&data[end - 2..end], &[0x12, 0]);
            end - 2
        })
        .collect();
    assert_eq!(boundaries.len(), 8);
    boundaries.push(data.len());
    let end = sequence.payload.as_ptr() as usize - data.as_ptr() as usize + sequence.payload.len();
    let mut config = vec![0x81, 0, 0, 0];
    config.extend_from_slice(&data[2..end]);
    let mut coverage = [[0u64; 3]; 3];
    let mut temporal_coverage = [0u64; 3];
    for point in m["points"].as_array().unwrap() {
        let op = point["point"].as_u64().unwrap() as usize;
        let idc = point["idc"].as_u64().unwrap() as u16;
        let indices: Vec<_> = point["indices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap() as usize)
            .collect();
        assert_eq!(
            indices,
            (0..24)
                .filter(|i| idc & (1 << (i % 3 + 8)) != 0 && idc & (1 << tids[i / 3]) != 0)
                .collect::<Vec<_>>()
        );
        let bytes = std::fs::read(root.join(point["reference"].as_str().unwrap())).unwrap();
        let mut at = 0;
        let golden: Vec<_> = indices
            .iter()
            .map(|i| {
                let sid = i % 3;
                let len = (128 >> (2 - sid)) * (96 >> (2 - sid)) * 3 / 2;
                let p = bytes[at..at + len].to_vec();
                at += len;
                p
            })
            .collect();
        assert_eq!(at, bytes.len());
        for seeded in [false, true] {
            let mut decoder = if seeded {
                Decoder::from_configuration_with_operating_point(&config, 64 << 20, op).unwrap()
            } else {
                Decoder::with_operating_point(64 << 20, op).unwrap()
            };
            for fragmented in [false, true] {
                let mut frames = Vec::new();
                if fragmented {
                    let mut start = 0;
                    for obu in &obus {
                        let end = obu.payload.as_ptr() as usize - data.as_ptr() as usize
                            + obu.payload.len();
                        frames.extend(decoder.decode_packet(&data[start..end]).unwrap());
                        start = end;
                    }
                    assert_eq!(start, data.len());
                } else {
                    for pair in boundaries.windows(2) {
                        frames.extend(decoder.decode_packet(&data[pair[0]..pair[1]]).unwrap());
                    }
                }
                assert_eq!(frames.len(), golden.len());
                for (n, frame) in frames.iter().enumerate() {
                    let i = indices[n];
                    let sid = i % 3;
                    assert!(frame.show);
                    assert_eq!(frame.spatial_id, sid as u8);
                    assert_eq!(
                        frame.picture.size,
                        [(128 >> (2 - sid)) as u32, (96 >> (2 - sid)) as u32]
                    );
                    assert_eq!(
                        pixels(&frame.picture),
                        golden[n],
                        "op={op} seeded={seeded} fragmented={fragmented} frame={i}"
                    );
                    if op == 0 && !seeded && !fragmented {
                        coverage[sid][tids[i / 3] as usize] +=
                            frame.picture.inter_prediction.scaled_reference_blocks as u64;
                    }
                    if sid == 0 {
                        assert_eq!(frame.picture.inter_prediction.scaled_reference_blocks, 0);
                        if op == 0 && !seeded && !fragmented {
                            temporal_coverage[tids[i / 3] as usize] +=
                                frame.picture.inter_prediction.single_reference_blocks as u64;
                        }
                    }
                }
                decoder.finish().unwrap();
                decoder.reset();
            }
            for (unit, pair) in boundaries.windows(2).enumerate() {
                let shown = decoder
                    .decode_temporal_unit(&data[pair[0]..pair[1]])
                    .unwrap();
                let last = indices
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| **i / 3 == unit)
                    .last();
                match (shown, last) {
                    (None, None) => {}
                    (Some(frame), Some((n, i))) => {
                        assert_eq!(frame.spatial_id, (*i % 3) as u8);
                        assert_eq!(pixels(&frame.picture), golden[n]);
                    }
                    _ => panic!("op {op}: incorrect display selection at unit {unit}"),
                }
            }
            decoder.finish().unwrap();
        }
    }
    for (tid, count) in temporal_coverage.iter().enumerate() {
        assert!(
            *count > 0,
            "no actual base-layer temporal reference blocks for tid={tid}"
        );
    }
    for (sid, row) in coverage.iter().enumerate().skip(1) {
        for (tid, count) in row.iter().enumerate() {
            assert!(
                *count > 0,
                "no actual interlayer scaled reconstruction for sid={sid} tid={tid}; counts={coverage:?}"
            );
        }
    }
}
