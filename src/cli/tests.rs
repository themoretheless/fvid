mod colorize_cli_tests {
    #[test]
    fn owned_colorize_cli_decode_plan_and_export_need_no_legacy_feature() {
        let directory =
            std::env::temp_dir().join(format!("fvid-colorize-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("colorized-{depth}.mkv"));
            let source = source.to_str().unwrap();
            let tint = "hue=120:saturation=0.5:lightness=0.5:mix=0.5";
            for args in [
                vec!["decode", source, "--colorize", tint, "--quiet"],
                vec![
                    "plan",
                    "transcode-lossless",
                    source,
                    "--colorize",
                    tint,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source,
                    output.to_str().unwrap(),
                    "--colorize",
                    tint,
                    "--quiet",
                ],
            ] {
                super::super::run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod monochrome_cli_tests {
    #[test]
    fn owned_monochrome_cli_decode_plan_and_export_need_no_legacy_feature() {
        let directory =
            std::env::temp_dir().join(format!("fvid-monochrome-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/monochrome-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("monochromed-{depth}.mkv"));
            let source = source.to_str().unwrap();
            let tint = "cb=0.5:cr=-0.5:size=0.2:high=0.75";
            for args in [
                vec!["decode", source, "--monochrome", tint, "--quiet"],
                vec![
                    "plan",
                    "transcode-lossless",
                    source,
                    "--monochrome",
                    tint,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source,
                    output.to_str().unwrap(),
                    "--monochrome",
                    tint,
                    "--quiet",
                ],
            ] {
                super::super::run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod scalar_expression_tests {
    #[test]
    fn arithmetic_filter_parameters_decode_plan_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-scalar-expression-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let source = source.to_str().unwrap();
            for (filter, expression, numeric) in [
                (
                    "--colorize",
                    "hue=60*2:saturation=1/2:lightness=1/2:mix=1/2",
                    "hue=120:saturation=0.5:lightness=0.5:mix=0.5",
                ),
                (
                    "--monochrome",
                    "cb=1/2:cr=-1/2:size=1/5:high=3/4",
                    "cb=0.5:cr=-0.5:size=0.2:high=0.75",
                ),
                (
                    "--eq",
                    "gamma=sqrt(4):brightness=1/10",
                    "gamma=2:brightness=0.1",
                ),
                ("--hue", "h=90*2:s=1/2", "h=180:s=0.5"),
                (
                    "--colorhold",
                    "color=#ff000080@0.5:similarity=0.2:blend=0.5",
                    "red:0.2:0.5",
                ),
                (
                    "--lutyuv",
                    "y=gammaval(2):u=clip(val):v=gammaval709(2)",
                    "y=pow((clipval-minval)/(maxval-minval),2)*(maxval-minval)+minval:u=clipval:v=if(lt((clipval-minval)/(maxval-minval),0.018),4.5*(clipval-minval)/(maxval-minval),1.099*pow((clipval-minval)/(maxval-minval),0.5)-0.099)*(maxval-minval)+minval",
                ),
            ] {
                for command in [
                    vec!["decode", source, filter, expression, "--quiet"],
                    vec![
                        "plan",
                        "transcode-lossless",
                        source,
                        filter,
                        expression,
                        "--quiet",
                    ],
                ] {
                    super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                        .unwrap();
                }
                let mut outputs = Vec::new();
                for (index, args) in [expression, numeric].into_iter().enumerate() {
                    let output = directory.join(format!(
                        "{}-{depth}-{index}.mkv",
                        filter.trim_start_matches('-')
                    ));
                    super::super::run(
                        &[
                            "transcode-lossless",
                            source,
                            output.to_str().unwrap(),
                            filter,
                            args,
                            "--quiet",
                        ]
                        .into_iter()
                        .map(str::to_owned)
                        .collect::<Vec<_>>(),
                    )
                    .unwrap();
                    let mut reader = fvid::playback_native::NativeReader::software(
                        std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                        usize::MAX,
                    )
                    .unwrap();
                    let mut frames = Vec::new();
                    while let Some(raw) = reader.read_frame_raw().unwrap() {
                        let frame = fvid::native_geometry::VideoGeometry::default()
                            .apply(&raw, 3, 3)
                            .unwrap();
                        frames.push((frame.data, reader.frame_interval()));
                    }
                    assert_eq!(frames.len(), 3);
                    outputs.push(frames);
                }
                assert_eq!(outputs[0], outputs[1], "filter={filter} depth={depth}");
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod lutyuv_cli_tests {
    #[test]
    fn owned_lutyuv_cli_decode_plan_and_export_need_no_legacy_feature() {
        let directory =
            std::env::temp_dir().join(format!("fvid-lutyuv-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("lutyuvd-{depth}.mkv"));
            let source = source.to_str().unwrap();
            let tint = "y=minval:u=maxval:v=minval";
            for args in [
                vec!["decode", source, "--lutyuv", tint, "--quiet"],
                vec![
                    "plan",
                    "transcode-lossless",
                    source,
                    "--lutyuv",
                    tint,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source,
                    output.to_str().unwrap(),
                    "--lutyuv",
                    tint,
                    "--quiet",
                ],
            ] {
                super::super::run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            let mut reader = fvid::playback_native::NativeReader::software(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            while let Some(frame) = reader.read_frame_raw().unwrap() {
                let pixels = fvid::native_geometry::VideoGeometry::default()
                    .apply(&frame, 3, 3)
                    .unwrap();
                let samples: Vec<u16> = if depth == 8 {
                    pixels.data.iter().map(|s| *s as u16).collect()
                } else {
                    pixels
                        .data
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| u16::from_le_bytes([s[0], s[1]]))
                        .collect()
                };
                assert_eq!(&samples[..9], &[16u16 << (depth - 8); 9]);
                assert_eq!(&samples[9..13], &[240u16 << (depth - 8); 4]);
                assert_eq!(&samples[13..], &[16u16 << (depth - 8); 4]);
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod lutyuv_full_range_tests {
    #[test]
    fn full_range_y4m_lut_exports_keep_black_and_white() {
        let directory = std::env::temp_dir().join(format!("fvid-lut-full-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/lutyuv-full-range.y4m");
        for extension in ["mkv", "y4m"] {
            let output = directory.join(format!("filtered.{extension}"));
            let command = if extension == "mkv" {
                "transcode-lossless"
            } else {
                "export-y4m"
            };
            let mut args = vec![
                command,
                source.to_str().unwrap(),
                output.to_str().unwrap(),
                "--lutyuv",
                "y=minval:u=maxval:v=minval",
            ];
            if extension == "mkv" {
                args.push("--quiet");
            }
            super::super::run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>()).unwrap();
            let mut reader = fvid::playback_native::NativeReader::software(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            assert!(reader.colour().full_range);
            let mut count = 0;
            while let Some(frame) = reader.read_frame_raw().unwrap() {
                let pixels = fvid::native_geometry::VideoGeometry::default()
                    .apply(&frame, 3, 3)
                    .unwrap();
                assert_eq!(&pixels.data[..9], &[0; 9]);
                assert_eq!(&pixels.data[9..13], &[255; 4]);
                assert_eq!(&pixels.data[13..], &[0; 4]);
                count += 1;
            }
            assert_eq!(count, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod colorhold_cli_tests {
    #[test]
    fn owned_colorhold_cli_decode_plan_and_export_need_no_legacy_feature() {
        let directory =
            std::env::temp_dir().join(format!("fvid-colorhold-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("colorholdd-{depth}.mkv"));
            let y4m = directory.join(format!("colorhold-{depth}.y4m"));
            let source = source.to_str().unwrap();
            let tint = "black:0.00001";
            for args in [
                vec!["export-y4m", source, y4m.to_str().unwrap(), "--colorhold", tint],
                vec!["decode", source, "--colorhold", tint, "--quiet"],
                vec![
                    "plan",
                    "transcode-lossless",
                    source,
                    "--colorhold",
                    tint,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source,
                    output.to_str().unwrap(),
                    "--colorhold",
                    tint,
                    "--quiet",
                ],
            ] {
                super::super::run(&args.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
            let mut reader = fvid::playback_native::NativeReader::software(
                std::io::BufReader::new(std::fs::File::open(&output).unwrap()),
                usize::MAX,
            )
            .unwrap();
            let reference = if depth > 8 {
                Some(
                    std::fs::read(
                        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                            "tests/fixtures/playback-errors/colorhold-black-reference-{depth}.raw"
                        )),
                    )
                    .unwrap()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
                )
            } else {
                None
            };
            let mut index = 0;
            while let Some(frame) = reader.read_frame_raw().unwrap() {
                let pixels = fvid::native_geometry::VideoGeometry::default()
                    .apply(&frame, 3, 3)
                    .unwrap();
                let samples: Vec<u16> = if depth == 8 {
                    pixels.data.iter().map(|s| *s as u16).collect()
                } else {
                    pixels
                        .data
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|s| u16::from_le_bytes([s[0], s[1]]))
                        .collect()
                };

                if let Some(expected) = &reference {
                    assert_eq!(&samples, &expected[index * 17..index * 17 + 17]);
                } else {
                    assert_eq!(&samples[9..], &[128; 8]);
                }
                index += 1;
            }
            assert_eq!(index, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod colorcontrast_cli_tests {
    #[test]
    fn owned_colorcontrast_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-contrast-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "rc=1:rcw=1:pl=0.5";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--colorcontrast",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--colorcontrast",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--colorcontrast",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod vibrance_cli_tests {
    #[test]
    fn owned_vibrance_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-vibrance-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "intensity=1";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--vibrance",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--vibrance",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--vibrance",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod colorlevels_cli_tests {
    #[test]
    fn owned_colorlevels_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-colorlevels-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "romin=0.1:bomax=0.7";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--colorlevels",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--colorlevels",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--colorlevels",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod colorchannelmixer_cli_tests {
    #[test]
    fn owned_colorchannelmixer_decode_and_export_without_legacy() {
        let directory = std::env::temp_dir()
            .join(format!("fvid-colorchannelmixer-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "rr=0.5:rg=0.2:bb=0.7";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--colorchannelmixer",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--colorchannelmixer",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--colorchannelmixer",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod exposure_cli_tests {
    #[test]
    fn owned_exposure_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-exposure-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "exposure=1";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--exposure",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--exposure",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--exposure",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod colorbalance_cli_tests {
    #[test]
    fn owned_colorbalance_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-colorbalance-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "rs=0.2:gm=-0.1:bh=0.3:pl=1";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--colorbalance",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--colorbalance",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--colorbalance",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod colorcorrect_cli_tests {
    #[test]
    fn owned_colorcorrect_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-colorcorrect-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorcorrect-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "analyze=median:saturation=0.5";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--colorcorrect",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--colorcorrect",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--colorcorrect",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod cas_cli_tests {
    #[test]
    fn owned_cas_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-cas-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/cas-grid-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "strength=0.8:planes=5";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--cas",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--cas",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--cas",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}

mod grayworld_cli_tests {
    #[test]
    fn timeline_controls_decode_y4m_and_ffv1_export_without_legacy() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/grayworld-gamut-8.y4m");
        let read = |path: &std::path::Path| {
            let input = std::fs::read(path).unwrap();
            let mut frames = Vec::new();
            fvid_media::owned_y4m_decode::visit_reader_transformed(
                std::io::Cursor::new(input),
                &Default::default(),
                |_, frame, _, _| {
                    frames.push(frame.to_vec());
                    Ok(())
                },
            )
            .unwrap();
            frames
        };
        let original = read(&source);
        let directory = std::env::temp_dir().join(format!(
            "fvid-grayworld-timeline-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        for (case, args, selected) in [
            (0, "enable=lt(n,1)", 0),
            (1, "enable='gte(t,1)*eq(w,5)*eq(h,5)'", 2),
        ] {
            let y4m = directory.join(format!("selected-{case}.y4m"));
            let mkv = directory.join(format!("selected-{case}.mkv"));
            let decoded = directory.join(format!("decoded-{case}.y4m"));
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--grayworld",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--grayworld",
                    args,
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    mkv.to_str().unwrap(),
                    "--grayworld",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    mkv.to_str().unwrap(),
                    decoded.to_str().unwrap(),
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            for output in [&y4m, &decoded] {
                let actual = read(output);
                assert_eq!(actual.len(), 3);
                for i in 0..3 {
                    if i == selected {
                        assert_ne!(actual[i], original[i]);
                    } else {
                        assert_eq!(actual[i], original[i]);
                    }
                }
            }
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn native_timeline_options_are_admitted() {
        let args = vec![
            "decode".into(),
            "synthetic.y4m".into(),
            "--grayworld".into(),
            "enable=between(n,1,2)".into(),
        ];
        let (remaining, filters) = super::super::decode_video::pixel_decode_args(&args).unwrap();
        assert_eq!(
            remaining,
            vec!["decode".to_owned(), "synthetic.y4m".to_owned()]
        );
        assert!(!filters.is_empty());
        let request = fvid::media_info::DecodeTransform {
            grayworld: Some("enable=between(n,1,2)".into()),
            ..Default::default()
        };
        assert!(fvid::native_pixels::PixelFilters::from_request(&request).is_ok());
        let transform = fvid::media_info::LosslessTransform {
            grayworld: request.grayworld,
            ..Default::default()
        };
        assert!(fvid::native_lossless::supports(&transform));
    }

    #[test]
    fn owned_grayworld_decode_and_export_without_legacy() {
        let directory =
            std::env::temp_dir().join(format!("fvid-grayworld-cli-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for depth in [8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/grayworld-gamut-{depth}.y4m"
            ));
            let output = directory.join(format!("contrast-{depth}.mkv"));
            let y4m = directory.join(format!("contrast-{depth}.y4m"));
            let args = "0";
            for command in [
                vec![
                    "decode",
                    source.to_str().unwrap(),
                    "--grayworld",
                    args,
                    "--quiet",
                ],
                vec![
                    "transcode-lossless",
                    source.to_str().unwrap(),
                    output.to_str().unwrap(),
                    "--grayworld",
                    args,
                    "--quiet",
                ],
                vec![
                    "export-y4m",
                    source.to_str().unwrap(),
                    y4m.to_str().unwrap(),
                    "--grayworld",
                    args,
                ],
            ] {
                super::super::run(&command.into_iter().map(str::to_owned).collect::<Vec<_>>())
                    .unwrap();
            }
            assert_eq!(fvid_media::decode_video(&output).unwrap().video_frames, 3);
            assert_eq!(fvid_media::decode_video(&y4m).unwrap().video_frames, 3);
        }
        std::fs::remove_dir_all(directory).unwrap();
    }
}
