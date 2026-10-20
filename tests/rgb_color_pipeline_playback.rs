use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path};

#[test]
fn exported_rgb_exposure_plays_exact_pixels_clocks_seek_rewind_and_eof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rgb-color-pipeline");
    let output = std::env::temp_dir().join(format!(
        "fvid-rgb-exposure-playback-{}.mkv",
        std::process::id()
    ));
    fvid_media::transcode_lossless(
        &root.join("rgb-8.mkv"),
        &output,
        fvid_media::LosslessTransform {
            exposure: Some("1".into()),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let expected: Vec<Vec<u8>> = (0..3)
        .map(|i| {
            fs::read(root.join(format!("rgb-8-{i}.color")))
                .unwrap()
                .into_iter()
                .map(|v| v.saturating_mul(2))
                .collect()
        })
        .collect();
    let mut video =
        WebmVideoReader::open_software(Cursor::new(fs::read(&output).unwrap()), 32 << 20).unwrap();
    for _ in 0..2 {
        for (i, pixels) in expected.iter().enumerate() {
            assert!(video.read_frame().unwrap());
            assert_eq!(video.dimensions(), [4, 2]);
            assert_eq!(video.rgb(), pixels);
            assert_eq!(
                {
                    let interval = video.frame_interval().unwrap();
                    (interval.0, interval.1)
                },
                (i as u128 * 20_000_000, (i as u128 + 1) * 20_000_000)
            );
        }
        assert!(!video.read_frame().unwrap());
        video.rewind();
    }
    video.seek_to_frame(20_000_000).unwrap();
    assert_eq!(video.rgb(), &expected[1]);
    assert!(video.read_frame().unwrap());
    assert_eq!(video.rgb(), &expected[2]);
    assert!(!video.read_frame().unwrap());
    fs::remove_file(output).unwrap();
}

#[test]
fn sharpened_rgb_export_plays_and_seeks_to_exact_filtered_frames() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/rgb-color-pipeline");
    let output = std::env::temp_dir().join(format!("fvid-cas-playback-{}.mkv", std::process::id()));
    fvid_media::transcode_lossless(
        &root.join("rgb-8.mkv"),
        &output,
        fvid_media::LosslessTransform {
            cas: Some("strength=0.8".into()),
            ..Default::default()
        },
        &Default::default(),
    )
    .unwrap();
    let expected: Vec<Vec<u8>> = (0..3)
        .map(|i| {
            let mut data = fs::read(root.join(format!("rgb-8-{i}.color"))).unwrap();
            fvid_media::owned_cas::Cas::parse("strength=0.8")
                .unwrap()
                .apply_rgb(&mut data, 4, 2, 8, 3)
                .unwrap();
            data
        })
        .collect();
    let mut video =
        WebmVideoReader::open_software(Cursor::new(fs::read(&output).unwrap()), 32 << 20).unwrap();
    for _ in 0..2 {
        for (i, pixels) in expected.iter().enumerate() {
            assert!(video.read_frame().unwrap());
            assert_eq!(video.rgb(), pixels);
            let interval = video.frame_interval().unwrap();
            assert_eq!(
                (interval.0, interval.1),
                (i as u128 * 20_000_000, (i as u128 + 1) * 20_000_000)
            );
        }
        assert!(!video.read_frame().unwrap());
        video.rewind();
    }
    video.seek_to_frame(40_000_000).unwrap();
    assert_eq!(video.rgb(), &expected[2]);
    assert!(!video.read_frame().unwrap());
    fs::remove_file(output).unwrap();
}
