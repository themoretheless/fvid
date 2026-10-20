use fvid::playback_webm::WebmVideoReader;
use std::{fs, io::Cursor, path::Path};

#[test]
fn header_stripped_video_matches_pixels_clock_seek_rewind_and_eof() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-header-stripping");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let open = |key: &str| {
            WebmVideoReader::open_software(
                Cursor::new(fs::read(root.join(case[key].as_str().unwrap())).unwrap()),
                32 << 20,
            )
            .unwrap()
        };
        let mut encoded = open("file");
        let mut control = open("control");
        for _ in 0..2 {
            for index in 0..3 {
                assert!(encoded.read_frame().unwrap());
                assert!(control.read_frame().unwrap());
                let size: [usize; 2] =
                    serde_json::from_value(case["sizes"][index].clone()).unwrap();
                assert_eq!(encoded.dimensions(), size, "{} frame {index}", case["file"]);
                assert_eq!(encoded.rgb(), control.rgb());
                assert!(encoded.rgb().iter().all(|&sample| sample == 130));
                assert_eq!(encoded.frame_interval(), control.frame_interval());
                assert_eq!(
                    encoded.frame_interval().unwrap().0,
                    u128::from(case["pts_ns"][index].as_u64().unwrap())
                );
            }
            assert!(!encoded.read_frame().unwrap());
            assert!(!control.read_frame().unwrap());
            encoded.rewind();
            control.rewind();
        }
        for index in [2, 0, 1] {
            let pts = case["pts_ns"][index].as_i64().unwrap();
            encoded.seek_to_frame(pts).unwrap();
            control.seek_to_frame(pts).unwrap();
            assert_eq!(encoded.rgb(), control.rgb());
            assert_eq!(encoded.dimensions(), control.dimensions());
            assert_eq!(encoded.frame_interval(), control.frame_interval());
            if index == 2 {
                assert!(!encoded.read_frame().unwrap());
            }
        }
    }
}

#[test]
fn wholly_stripped_frames_can_be_losslessly_transcoded() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-header-stripping");
    for (index, name) in ["whole-frame.mkv", "whole-laced.mkv", "whole-late-track.mkv"]
        .into_iter()
        .enumerate()
    {
        let out = std::env::temp_dir().join(format!(
            "fvid-header-strip-lossless-{}-{index}.mkv",
            std::process::id()
        ));
        fvid_media::transcode_lossless(
            &root.join(name),
            &out,
            fvid_media::LosslessTransform {
                horizontal_flip: true,
                ..Default::default()
            },
            &Default::default(),
        )
        .unwrap();
        {
            let mut video =
                WebmVideoReader::open_software(Cursor::new(fs::read(&out).unwrap()), 32 << 20)
                    .unwrap();
            assert_eq!(video.codec(), "FFV1");
            let mut frames = 0;
            while video.read_frame().unwrap() {
                assert_eq!(video.dimensions(), [16, 16]);
                assert!(video.rgb().iter().all(|&sample| sample == 130));
                frames += 1;
            }
            assert_eq!(frames, 3);
        }
        fs::remove_file(out).unwrap();
    }
}

#[test]
fn identity_copy_preserves_compression_metadata_and_stored_zero_byte_frames() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-header-stripping");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let input = fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let mut output = Vec::new();
        let stats = fvid_media::owned_matroska_copy::copy(
            &mut Cursor::new(&input),
            &mut output,
            false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(stats.packets, case["packet_count"].as_u64().unwrap());
        assert_eq!(output, input, "{}", case["file"]);
    }
}
