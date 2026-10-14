//! Independent seven-frame switching sequences with distinct initialization banks.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("ffv1-v3-quantizer-switches.json")).unwrap())
        .unwrap()
}
#[test]
fn changing_context_count_and_quantizer_shape_preserves_each_authored_sample() {
    for case in manifest()["cases"].as_array().unwrap() {
        let private = std::fs::read(root().join(case["configuration"].as_str().unwrap())).unwrap();
        let mut decoder =
            fvid_media::owned_ffv1_decoder::Decoder::from_configuration(12, 6, &private, 32 << 20)
                .unwrap();
        for _ in 0..2 {
            for frame in case["frames"].as_array().unwrap() {
                let packet = std::fs::read(root().join(frame["packet"].as_str().unwrap())).unwrap();
                let decoded = decoder
                    .decode(&packet)
                    .unwrap_or_else(|e| panic!("{}: {e}", frame["packet"]));
                assert_eq!(decoded.depth, case["depth"].as_u64().unwrap() as u8);
                assert_eq!(decoded.keyframe, frame["keyframe"].as_bool().unwrap());
                assert_eq!(
                    decoded.frame.subsampling.is_none(),
                    case["rgb"].as_bool().unwrap()
                );
                assert_eq!(
                    decoded.frame.data,
                    std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap(),
                    "{}",
                    frame["packet"]
                );
            }
            decoder.reset();
        }
    }
}
#[test]
fn switched_contexts_survive_native_seek_rewind_and_lossless_export() {
    for (index, case) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let source = root().join(case["file"].as_str().unwrap());
        let mut reader = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(&source).unwrap()),
            32 << 20,
        )
        .unwrap();
        let check = |raw: fvid::playback_native::RawFrame, frame: &serde_json::Value| {
            let fvid::playback_native::RawFrame::Planar(p) = raw else {
                panic!("quantizer switching narrowed source samples")
            };
            assert_eq!(p.depth, case["depth"].as_u64().unwrap() as u8);
            assert_eq!(
                p.frame.data,
                std::fs::read(root().join(frame["reference"].as_str().unwrap())).unwrap()
            );
        };
        for frame in case["frames"].as_array().unwrap() {
            check(reader.read_frame_raw().unwrap().unwrap(), frame);
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
        for frame in [4, 1, 3, 6, 0] {
            check(
                reader
                    .seek_raw(std::time::Duration::from_millis(frame * 20))
                    .unwrap()
                    .unwrap(),
                &case["frames"][frame as usize],
            );
            let (start, end, scale) = reader.frame_interval().unwrap();
            assert_eq!(
                (
                    start * 1000 / u128::from(scale),
                    end * 1000 / u128::from(scale)
                ),
                (u128::from(frame) * 20, (u128::from(frame) + 1) * 20)
            );
        }
        reader.rewind().unwrap();
        check(
            reader.read_frame_raw().unwrap().unwrap(),
            &case["frames"][0],
        );
        let target =
            std::env::temp_dir().join(format!("fvid-q-switch-{}-{index}.mkv", std::process::id()));
        let _ = std::fs::remove_file(&target);
        fvid_media::transcode_lossless(&source, &target, Default::default(), &Default::default())
            .unwrap();
        let mut exported = fvid::playback_native::NativeReader::software(
            Cursor::new(std::fs::read(&target).unwrap()),
            32 << 20,
        )
        .unwrap();
        for frame in case["frames"].as_array().unwrap() {
            check(exported.read_frame_raw().unwrap().unwrap(), frame);
        }
        assert!(exported.read_frame_raw().unwrap().is_none());
        std::fs::remove_file(target).unwrap();
    }
}
#[test]
fn enlarged_and_retained_banks_stay_inside_the_aggregate_budget() {
    for case in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["depth"] == 16 && c["rgb"] == true)
    {
        let private = std::fs::read(root().join(case["configuration"].as_str().unwrap())).unwrap();
        let packets: Vec<_> = case["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| std::fs::read(root().join(f["packet"].as_str().unwrap())).unwrap())
            .collect();
        let run = |budget| -> Result<(), String> {
            let mut decoder = fvid_media::owned_ffv1_decoder::Decoder::from_configuration(
                12, 6, &private, budget,
            )?;
            for packet in &packets {
                decoder.decode(packet)?;
            }
            Ok(())
        };
        let (mut low, mut high) = (0, 32 << 20);
        while low < high {
            let middle = low + (high - low) / 2;
            if run(middle).is_ok() {
                high = middle
            } else {
                low = middle + 1
            }
        }
        assert!(low > 0 && low < 32 << 20);
        run(low).unwrap();
        assert!(run(low - 1).unwrap_err().contains("memory budget"));
    }
}
