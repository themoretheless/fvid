use fvid::playback::video::mp4::LayeredMp4VideoReader;
use std::{io::Cursor, path::Path};

#[test]
fn temporal_merge_switches_target_reference_class_without_affecting_default_controls() {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/hevc-wedge");
    for (size, direct) in [(32usize, false), (128, false), (128, true)] {
        for alternate in [false, true] {
            for explicit in [false, true] {
                let name = format!(
                    "temporal-alt-ref-{size}-{}{}{}.mp4",
                    if alternate { "on" } else { "off" },
                    if direct { "-direct" } else { "" },
                    if explicit { "-explicit" } else { "" }
                );
                let mut reader = LayeredMp4VideoReader::open(
                    Cursor::new(std::fs::read(root.join(&name)).unwrap()),
                    Default::default(),
                    1,
                    0,
                    64 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut seen = Vec::new();
                    while let Some(frame) = reader
                        .read_frame()
                        .unwrap_or_else(|e| panic!("{name} after {seen:?}: {e}"))
                    {
                        let id = frame.decoded.layer_id;
                        let poc = frame.decoded.poc;
                        seen.push((id, poc));
                        if id != 8 || poc != 1 {
                            continue;
                        }
                        let picture = &frame.decoded.picture;
                        for y in 0..size {
                            for x in 0..size {
                                let clamp = |v: i32| v.clamp(0, size as i32 - 1) as usize;
                                let a = ((x * 7
                                    + clamp(y as i32 + if explicit && alternate { 1 } else { 2 })
                                        * 11
                                    + 37)
                                    % 256) as u16;
                                let b = ((x * 7 + clamp(y as i32 - 2) * 11 + 37 + 97) % 256) as u16;
                                assert_eq!(
                                    picture.planes[0].samples()[y * size + x],
                                    (a + b + 1) / 2,
                                    "{name}: pixel{x},{y}"
                                );
                                let motion =
                                    picture.prediction_motion_at([x as u32, y as u32]).unwrap();
                                for (list, mv) in (if explicit && alternate {
                                    [0, 0]
                                } else {
                                    [8, -8]
                                })
                                .into_iter()
                                .enumerate()
                                {
                                    let v = motion[list].unwrap();
                                    assert_eq!(
                                        v.reference,
                                        u8::from(alternate && !explicit),
                                        "{name}: list{list}"
                                    );
                                    assert_eq!(v.mv, [0, mv], "{name}: list{list}");
                                }
                            }
                        }
                    }
                    assert_eq!(
                        seen,
                        [
                            (0, 0),
                            (3, 0),
                            (5, 0),
                            (8, 0),
                            (0, 1),
                            (3, 1),
                            (5, 1),
                            (8, 1),
                            (0, 2),
                            (3, 2),
                            (5, 2),
                            (8, 2)
                        ]
                    );
                    reader.rewind();
                }
            }
        }
    }
}
