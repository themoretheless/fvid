//! Three-picture PCM/P/B inter-DC regressions with independent sample-domain oracles.
use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn inter_dc_only_p_and_b_play_merge_explicit_skip_and_disabled_controls() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    for size in [32usize, 64, 128] {
        for mode in [
            "merge",
            "explicit",
            "skip",
            "disabled",
            "partitioned",
            "merge-residual",
        ] {
            let scenarios: &[&str] = if matches!(mode, "merge" | "explicit") {
                &["unit", "positive", "negative", "clip"]
            } else {
                &["identity"]
            };
            for scenario in scenarios {
                let stem = format!("inter-dc-{size}-{mode}-{scenario}");
                let expected: Vec<u16> = std::fs::read(root.join(format!("{stem}.y16")))
                    .unwrap()
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                for dlt in [false, true] {
                    for direct in if size == 128 {
                        &[false, true][..]
                    } else {
                        &[false][..]
                    } {
                        let movie = format!(
                            "{stem}{}{}.mp4",
                            if dlt { "-dlt" } else { "" },
                            if *direct { "-direct" } else { "" }
                        );
                        let mut reader = LayeredMp4VideoReader::open(
                            std::io::Cursor::new(std::fs::read(root.join(&movie)).unwrap()),
                            Limits::default(),
                            1,
                            0,
                            64 << 20,
                        )
                        .unwrap();
                        for _ in 0..2 {
                            let mut layers = Vec::new();
                            let mut count = 0;
                            while let Some(frame) = reader
                                .read_frame()
                                .unwrap_or_else(|e| panic!("{movie}: {e}"))
                            {
                                layers.push(frame.decoded.layer_id);
                                if frame.decoded.layer_id == 3 {
                                    assert_eq!(frame.decoded.poc, count as i32, "{movie}");
                                    let actual = frame.decoded.picture.planes[0].samples();
                                    let wanted =
                                        &expected[count * size * size..(count + 1) * size * size];
                                    assert_eq!(actual.len(), wanted.len());
                                    assert_eq!(
                                        actual
                                            .iter()
                                            .zip(wanted)
                                            .enumerate()
                                            .find(|(_, (a, b))| a != b),
                                        None,
                                        "{movie}: POC{count} first mismatching pixel"
                                    );
                                    count += 1;
                                }
                            }
                            assert_eq!(count, 3, "{movie}");
                            assert_eq!(layers, [0, 3, 0, 3, 0, 3], "{movie}");
                            reader.rewind();
                        }
                    }
                }
            }
        }
    }
}
