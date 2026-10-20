use fvid::playback::video::mp4::LayeredMp4VideoReader;
use std::{io::Cursor, path::Path};

#[test]
fn five_initial_candidates_are_followed_by_zero_not_combined_biprediction() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    for direct in [false, true] {
        for index in 0..6 {
            let name = format!(
                "initial-merge-six-{index}{}.mp4",
                if direct { "-direct" } else { "" }
            );
            {
                use fvid::codec::hevc_decoder::{LayeredAccessUnit, LayeredDpb, LayeredParameters};
                let mut input = fvid::container::mp4::Mp4Reader::open(
                    Cursor::new(std::fs::read(root.join(&name)).unwrap()),
                    Default::default(),
                )
                .unwrap();
                let mut params = LayeredParameters::from_configuration(
                    &input.tracks()[0].configuration,
                    64 << 20,
                )
                .unwrap();
                let mut dpb = LayeredDpb::default();
                let mut packet = Vec::new();
                for sample in 0..4 {
                    input.read_packet(0, sample, &mut packet).unwrap();
                    let plan =
                        LayeredAccessUnit::parse(&params, dpb.poc(), &packet, 4, 1, 64 << 20)
                            .unwrap_or_else(|e| panic!("{name} parse AU{sample}: {e}"));
                    if sample == 3 {
                        let header = &plan
                            .pictures()
                            .iter()
                            .find(|p| p.binding.sps.layer_id == 3)
                            .unwrap()
                            .slices[0];
                        assert_eq!(header.max_merge_candidates, 6);
                        assert!(header.temporal_mvp);
                        assert_eq!(header.references, [1, 1]);
                    }
                    let (updated, _, _) = dpb
                        .decode(plan, 64 << 20)
                        .unwrap_or_else(|e| panic!("{name} decode AU{sample}: {e}"));
                    params = updated;
                }
            }
            let mut reader = LayeredMp4VideoReader::open(
                Cursor::new(std::fs::read(root.join(&name)).unwrap()),
                Default::default(),
                1,
                0,
                64 << 20,
            )
            .unwrap();
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name} after {seen:?}: {e}"))
            {
                seen.push((frame.decoded.layer_id, frame.decoded.poc));
                if frame.decoded.layer_id == 3 {
                    let picture = &frame.decoded.picture;
                    for y in 0..64usize {
                        for x in 0..64usize {
                            let clamp = |v: i32| v.clamp(0, 63) as usize;
                            let past = |row: usize| ((x * 7 + row * 11 + 37) % 256) as u16;
                            let (m0, m1) = if (32..48).contains(&x) && (32..48).contains(&y) {
                                [(1, 1), (2, 2), (3, 3), (4, 4), (9, -9), (0, 0)][index]
                            } else {
                                let v = match (x / 16 * 16, y / 16 * 16) {
                                    (16, 32) => 1,
                                    (32, 16) => 2,
                                    (48, 16) => 3,
                                    (16, 48) => 4,
                                    _ => 5,
                                };
                                (v, v)
                            };
                            let expected = match frame.decoded.poc {
                                0 => past(y),
                                1 => past(clamp(y as i32 + 9)),
                                3 => ((x * 7 + y * 11 + 37 + 97) % 256) as u16,
                                2 => {
                                    let row = clamp(y as i32 + m0);
                                    let a = past(clamp(row as i32 + 9));
                                    let b = ((x * 7 + clamp(y as i32 + m1) * 11 + 37 + 97) % 256)
                                        as u16;
                                    (a + b + 1) / 2
                                }
                                _ => unreachable!(),
                            };
                            assert_eq!(
                                picture.planes[0].samples()[y * 64 + x],
                                expected,
                                "{name}: POC{} pixel{x},{y}",
                                frame.decoded.poc
                            );
                            if frame.decoded.poc == 1 || frame.decoded.poc == 2 {
                                let motion =
                                    picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                                assert_eq!(
                                    motion[0].unwrap().mv,
                                    [0, 4 * i64::from(if frame.decoded.poc == 1 { 9 } else { m0 })],
                                    "{name}: POC{} L0 at{x},{y}",
                                    frame.decoded.poc
                                );
                                if frame.decoded.poc == 2 {
                                    assert_eq!(
                                        motion[1].unwrap().mv,
                                        [0, i64::from(m1) * 4],
                                        "{name}: L1 at{x},{y}"
                                    );
                                }
                            }
                        }
                    }
                }
                if frame.decoded.layer_id != 3 || frame.decoded.poc != 2 {
                    continue;
                }
                let expected = [(1, 1), (2, 2), (3, 3), (4, 4), (9, -9), (0, 0)][index];
                let motion = frame
                    .decoded
                    .picture
                    .prediction_motion_at([32, 32])
                    .unwrap();
                for (list, mv) in [expected.0, expected.1].into_iter().enumerate() {
                    let v = motion[list].unwrap();
                    assert_eq!(v.reference, 0, "{name}: list{list}");
                    assert_eq!(v.mv, [0, mv * 4], "{name}: list{list}");
                }
            }
            assert_eq!(
                seen,
                [
                    (0, 0),
                    (3, 0),
                    (0, 1),
                    (3, 1),
                    (0, 2),
                    (3, 2),
                    (0, 3),
                    (3, 3)
                ]
            );
        }
    }
}
