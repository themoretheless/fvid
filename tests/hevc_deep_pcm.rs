//! Active HEVC PCM at 14/16 bits survives software playback and owned exports.
use std::{io::Cursor, path::Path};
#[test]
fn deep_pcm_playback_and_exports_preserve_every_hm_sample() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for depth in [14u8, 16] {
        for mode in ["input8", "full", "mixed", "dependent", "parallel"] {
            let stem = format!("hevc-pcm-444-deep-{mode}-rext{depth}");
            let source = root.join(format!("{stem}.mp4"));
            let oracle = std::fs::read(root.join(format!("{stem}.yuv"))).unwrap();
            let dims = if mode == "parallel" {
                [128, 96]
            } else {
                [64, 64]
            };
            let verify = |path: &Path| {
                let mut reader = fvid::playback_native::NativeReader::software(
                    Cursor::new(std::fs::read(path).unwrap()),
                    16 << 20,
                )
                .unwrap();
                for _ in 0..2 {
                    let mut actual = Vec::new();
                    let mut count = 0;
                    while let Some(frame) = reader.read_frame_raw().unwrap() {
                        let fvid::playback_native::RawFrame::Planar(p) = frame else {
                            panic!("PCM sample precision must be retained")
                        };
                        assert_eq!(p.depth, depth);
                        assert_eq!(p.frame.subsampling, Some([1, 1]));
                        assert_eq!([p.frame.width, p.frame.height], dims);
                        actual.extend_from_slice(&p.frame.data);
                        count += 1;
                    }
                    assert_eq!(count, 3);
                    assert_eq!(actual, oracle, "{stem}: sample mismatch");
                    reader.rewind().unwrap();
                }
            };
            verify(&source);
            let output = std::env::temp_dir()
                .join(format!("fvid-deep-pcm-{}-{stem}.mkv", std::process::id()));
            for export in [
                fvid::media::transcode_lossless,
                fvid_media::transcode_lossless,
            ] {
                export(&source, &output, Default::default(), &Default::default()).unwrap();
                verify(&output);
                std::fs::remove_file(&output).unwrap();
            }
        }
    }
}
