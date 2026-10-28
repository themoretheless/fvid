use fvid::{
    container::mp4::{Limits, Mp4Reader},
    media::CopyOptions,
};
use std::{fs::File, io::BufReader, path::PathBuf};
fn source(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-metadata-admission")
        .join(name)
}
#[test]
fn parser_metadata_limit_rejects_padding_before_loading_it() {
    let error = match Mp4Reader::open(
        BufReader::new(File::open(source("padded.mp4")).unwrap()),
        Limits {
            metadata_bytes: 64 << 10,
            ..Default::default()
        },
    ) {
        Ok(_) => panic!("metadata parser must reject oversized moov"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
}
#[test]
fn controlled_remux_plan_does_not_bypass_parser_metadata_limit() {
    let options = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let error =
        fvid_media::owned_mp4_remux::plan_remux(&source("padded.mp4"), &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn small_metadata_and_ample_budget_controls_are_valid() {
    for (name, budget) in [("small.mp4", 64 << 10), ("padded.mp4", 2 << 20)] {
        let options = CopyOptions {
            max_controlled_bytes: Some(budget),
            ..Default::default()
        };
        assert!(fvid_media::owned_mp4_remux::plan_remux(&source(name), &options).is_ok());
    }
}
#[test]
fn route_inspection_and_concat_propagate_metadata_admission() {
    let padded = source("padded.mp4");
    let options = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let error = fvid::native_export::is_native_mp4_matroska_with_memory_limit(
        &padded,
        options.max_controlled_bytes,
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("metadata exceeds budget"),
        "{error}"
    );
    let error = fvid::media::plan_remux(&padded, &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
    let inputs = [padded.clone(), padded];
    let result = fvid::container::mp4_concat::open_with_options(&inputs, &options);
    assert!(matches!(result,Err(e) if e.to_string().contains("metadata exceeds budget")));
    let error = fvid::media::plan_concat(&inputs, &options).unwrap_err();
    assert!(error.contains("metadata exceeds budget"), "{error}");
}
#[test]
fn remux_budget_errors_remain_specific_and_preserve_destination() {
    let dir = std::env::temp_dir().join(format!("fvid-metadata-admission-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let out = dir.join("untouched.mkv");
    std::fs::write(&out, b"preserve existing destination").unwrap();
    let options = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    for route in 0..3 {
        let result = match route {
            0 => fvid_media::owned_mp4_remux::remux(&source("padded.mp4"), &out, &options),
            1 => fvid::media::remux(&source("padded.mp4"), &out, &options),
            _ => fvid_media::remux(&source("padded.mp4"), &out, &options),
        };
        let error = result.unwrap_err();
        assert!(error.contains("metadata exceeds budget"), "{error}");
        assert_eq!(
            std::fs::read(&out).unwrap(),
            b"preserve existing destination"
        );
    }
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "refusal must not leave temporary output"
    );
}
#[cfg(feature = "player")]
#[test]
fn ample_budget_remux_retains_pixels_timestamps_and_seek() {
    use fvid::playback_native::NativeReader;
    use std::time::Duration;
    let padded = source("padded.mp4");
    let mut original =
        NativeReader::software(BufReader::new(File::open(&padded).unwrap()), 32 << 20).unwrap();
    let expected = original
        .read_frame_raw()
        .unwrap()
        .unwrap()
        .into_rgb(32 << 20)
        .unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-metadata-ample-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let output = dir.join("out.mkv");
    let options = CopyOptions {
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    assert_eq!(
        fvid::media::remux(&padded, &output, &options)
            .unwrap()
            .packets,
        1
    );
    let mut result =
        NativeReader::software(BufReader::new(File::open(output).unwrap()), 32 << 20).unwrap();
    assert_eq!(result.duration(), Some(Duration::from_millis(40)));
    assert_eq!(
        result
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        expected
    );
    assert!(result.read_frame_raw().unwrap().is_none());
    assert!(result.read_frame_raw().unwrap().is_none());
    result.rewind().unwrap();
    assert_eq!(
        result
            .read_frame_raw()
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        expected
    );
    for ms in [0, 20, 39, 40, 100] {
        assert_eq!(
            result
                .seek_raw(Duration::from_millis(ms))
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            expected
        );
    }
}
#[test]
fn unsupported_sample_entry_keeps_its_existing_refusal_route() {
    let input = source("unsupported-entry.mp4");
    let error = match fvid_media::owned_mp4::Mp4Reader::open(
        BufReader::new(File::open(&input).unwrap()),
        Default::default(),
    ) {
        Ok(_) => panic!("unknown entry must remain unsupported"),
        Err(error) => error,
    };
    assert!(error.is_unsupported(), "{error}");
    let dir =
        std::env::temp_dir().join(format!("fvid-metadata-unsupported-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let error =
        fvid_media::owned_mp4_remux::remux(&input, &dir.join("out.mkv"), &CopyOptions::default())
            .unwrap_err();
    assert_eq!(error, "request is not an owned MP4 Matroska remux");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}
#[test]
fn lossless_subtitle_burn_and_y4m_routes_propagate_mp4_metadata_admission() {
    let padded = source("padded.mp4");
    let small = source("small.mp4");
    let tight = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let ample = CopyOptions {
        max_controlled_bytes: Some(2 << 20),
        ..Default::default()
    };
    let dir = std::env::temp_dir().join(format!(
        "fvid-metadata-video-routes-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let srt = dir.join("cue.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:00,040\nHi\n").unwrap();
    let out_mkv = dir.join("out.mkv");
    let out_y4m = dir.join("out.y4m");

    // 1. transcode-lossless and plan transcode-lossless
    for err in [
        fvid::media::plan_transcode_lossless(&padded, &Default::default(), &tight, None)
            .unwrap_err(),
        fvid::media::transcode_lossless(&padded, &out_mkv, Default::default(), &tight).unwrap_err(),
        fvid_media::plan_transcode_lossless(&padded, &Default::default(), &tight, None)
            .unwrap_err(),
        fvid_media::transcode_lossless(&padded, &out_mkv, Default::default(), &tight).unwrap_err(),
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(!out_mkv.exists());
    for (input, opts) in [(&small, &tight), (&padded, &ample)] {
        fvid::media::plan_transcode_lossless(input, &Default::default(), opts, None).unwrap();
        let stats =
            fvid::media::transcode_lossless(input, &out_mkv, Default::default(), opts).unwrap();
        assert_eq!(stats.video_frames, 1);
        std::fs::remove_file(&out_mkv).unwrap();
    }

    // 2. burn-subtitles and plan burn-subtitles
    for err in [
        fvid_media::plan_burn_subtitles(&padded, &srt, &tight).unwrap_err(),
        fvid_media::burn_subtitles(&padded, &out_mkv, &srt, &tight).unwrap_err(),
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(!out_mkv.exists());
    for (input, opts) in [(&small, &tight), (&padded, &ample)] {
        fvid_media::plan_burn_subtitles(input, &srt, opts).unwrap();
        let stats = fvid_media::burn_subtitles(input, &out_mkv, &srt, opts).unwrap();
        assert_eq!(stats.video_frames, 1);
        std::fs::remove_file(&out_mkv).unwrap();
    }

    // 3. export-y4m, trim-y4m, and concat-y4m
    let pair_padded = [padded.clone(), padded.clone()];
    for err in [
        fvid::native_plan::export_y4m_with_options(
            &padded,
            None,
            &Default::default(),
            &Default::default(),
            &tight,
        )
        .unwrap_err(),
        fvid::native_export::export_y4m_pipeline_with_options(
            &padded,
            &out_y4m,
            None,
            &Default::default(),
            &Default::default(),
            &tight,
        )
        .unwrap_err()
        .to_string(),
        fvid::media::plan_trim_y4m(&padded, 0, 40_000, &tight).unwrap_err(),
        fvid::media::trim(&padded, &out_y4m, 0, 40_000, &tight).unwrap_err(),
        fvid::media::plan_concat_y4m(&pair_padded, &tight).unwrap_err(),
        fvid::media::concat(&pair_padded, &out_y4m, &tight).unwrap_err(),
        match fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&padded).unwrap()),
            64 << 10,
        ) {
            Ok(_) => panic!("NativeReader::software must reject oversized MP4 metadata"),
            Err(e) => e.to_string(),
        },
        match fvid::playback_native::NativeReader::new(
            BufReader::new(File::open(&padded).unwrap()),
            64 << 10,
        ) {
            Ok(_) => panic!("NativeReader::new must reject oversized MP4 metadata"),
            Err(e) => e.to_string(),
        },
    ] {
        assert!(err.contains("metadata exceeds budget"), "{err}");
    }
    assert!(!out_y4m.exists());
    for (input, opts) in [(&small, &tight), (&padded, &ample)] {
        fvid::native_plan::export_y4m_with_options(
            input,
            None,
            &Default::default(),
            &Default::default(),
            opts,
        )
        .unwrap();
        let event = fvid::native_export::export_y4m_pipeline_with_options(
            input,
            &out_y4m,
            None,
            &Default::default(),
            &Default::default(),
            opts,
        )
        .unwrap();
        assert_eq!(event.packets, 1);
        std::fs::remove_file(&out_y4m).unwrap();
        fvid::media::plan_trim_y4m(input, 0, 40_000, opts).unwrap();
        let stats = fvid::media::trim(input, &out_y4m, 0, 40_000, opts).unwrap();
        assert_eq!(stats.packets, 1);
        std::fs::remove_file(&out_y4m).unwrap();
        let pair = [input.clone(), input.clone()];
        fvid::media::plan_concat_y4m(&pair, opts).unwrap();
        let stats = fvid::media::concat(&pair, &out_y4m, opts).unwrap();
        assert_eq!(stats.packets, 2);
        std::fs::remove_file(&out_y4m).unwrap();
    }
}

#[test]
fn matroska_packet_index_budget_is_enforced_across_video_subtitle_and_playback_entrypoints() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-matroska-packet-index-admission-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-six-frames.mkv");
    let srt = dir.join("cue.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:00,500\nHi\n").unwrap();
    let out_mkv = dir.join("out.mkv");
    let out_y4m = dir.join("out.y4m");

    // ffv1-six-frames.mkv has 6 packets; each Packet entry is 56 bytes (6 * 56 = 336 bytes).
    // A 200-byte controlled memory budget allows at most 3 indexed packets.
    let tight = CopyOptions {
        max_controlled_bytes: Some(200),
        ..Default::default()
    };
    let ample = CopyOptions {
        max_controlled_bytes: Some(512 << 10),
        ..Default::default()
    };
    let pair = [mkv.clone(), mkv.clone()];

    for err in [
        fvid_media::plan_transcode_lossless(&mkv, &Default::default(), &tight, None).unwrap_err(),
        fvid_media::transcode_lossless(&mkv, &out_mkv, Default::default(), &tight).unwrap_err(),
        fvid::media::plan_transcode_lossless(&mkv, &Default::default(), &tight, None).unwrap_err(),
        fvid::media::transcode_lossless(&mkv, &out_mkv, Default::default(), &tight).unwrap_err(),
        fvid_media::plan_burn_subtitles(&mkv, &srt, &tight).unwrap_err(),
        fvid_media::burn_subtitles(&mkv, &out_mkv, &srt, &tight).unwrap_err(),
        fvid::native_plan::export_y4m_with_options(
            &mkv,
            None,
            &Default::default(),
            &Default::default(),
            &tight,
        )
        .unwrap_err(),
        fvid::native_export::export_y4m_pipeline_with_options(
            &mkv,
            &out_y4m,
            None,
            &Default::default(),
            &Default::default(),
            &tight,
        )
        .unwrap_err()
        .to_string(),
        fvid::media::plan_trim_y4m(&mkv, 0, 100_000, &tight).unwrap_err(),
        fvid::media::trim(&mkv, &out_y4m, 0, 100_000, &tight).unwrap_err(),
        fvid::media::plan_concat_y4m(&pair, &tight).unwrap_err(),
        fvid::media::concat(&pair, &out_y4m, &tight).unwrap_err(),
        match fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&mkv).unwrap()),
            200,
        ) {
            Ok(_) => panic!("NativeReader::software must reject Matroska packet index over budget"),
            Err(e) => e.to_string(),
        },
        match fvid::playback_native::NativeReader::new(
            BufReader::new(File::open(&mkv).unwrap()),
            200,
        ) {
            Ok(_) => panic!("NativeReader::new must reject Matroska packet index over budget"),
            Err(e) => e.to_string(),
        },
    ] {
        assert!(
            err.contains("controlled memory budget exceeded: Matroska packet index limit"),
            "unexpected error message: {err}"
        );
    }
    assert!(!out_mkv.exists());
    assert!(!out_y4m.exists());

    // Positive controls with ample 512 KiB budget
    fvid_media::plan_transcode_lossless(&mkv, &Default::default(), &ample, None).unwrap();
    let stats =
        fvid_media::transcode_lossless(&mkv, &out_mkv, Default::default(), &ample).unwrap();
    assert_eq!(stats.video_frames, 6);
    std::fs::remove_file(&out_mkv).unwrap();

    fvid_media::plan_burn_subtitles(&mkv, &srt, &ample).unwrap();
    let stats = fvid_media::burn_subtitles(&mkv, &out_mkv, &srt, &ample).unwrap();
    assert_eq!(stats.video_frames, 6);
    std::fs::remove_file(&out_mkv).unwrap();

    fvid::native_plan::export_y4m_with_options(
        &mkv,
        None,
        &Default::default(),
        &Default::default(),
        &ample,
    )
    .unwrap();
    let event = fvid::native_export::export_y4m_pipeline_with_options(
        &mkv,
        &out_y4m,
        None,
        &Default::default(),
        &Default::default(),
        &ample,
    )
    .unwrap();
    assert_eq!(event.packets, 6);
    std::fs::remove_file(&out_y4m).unwrap();
}

#[test]
fn matroska_packet_index_budget_is_enforced_in_remux_subtitle_convert_and_audio_concat() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-mkv-packet-index-remux-subtitle-concat-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-six-frames.mkv");
    let out_mkv = dir.join("remux.mkv");

    // 1. Matroska identity remux (6 packets * 56 bytes = 336 bytes > 200 bytes)
    let tight = CopyOptions {
        max_controlled_bytes: Some(200),
        ..Default::default()
    };
    let ample = CopyOptions {
        max_controlled_bytes: Some(512 << 10),
        ..Default::default()
    };
    for err in [
        fvid_media::plan_remux(&mkv, &tight).unwrap_err(),
        fvid_media::remux(&mkv, &out_mkv, &tight).unwrap_err(),
        fvid::media::plan_remux(&mkv, &tight).unwrap_err(),
        fvid::media::remux(&mkv, &out_mkv, &tight).unwrap_err(),
        fvid_media::owned_webm_probe::probe_webm_with_packet_and_memory_limit(
            &mkv,
            tight.max_packet_bytes,
            tight.max_controlled_bytes,
        )
        .unwrap_err(),
    ] {
        assert!(
            err.contains("controlled memory budget exceeded: Matroska packet index limit"),
            "unexpected remux error: {err}"
        );
    }
    assert!(!out_mkv.exists());
    // When max_controlled_bytes is 70 KiB (enough for the 64 KiB + 96 B copy index of 6 packets,
    // and 70 KiB / 56 = 1280 packets), plan_remux must NOT clamp packet_bytes to 70 KiB if a
    // legitimate video packet is 100 KiB (< max_packet_bytes), nor fail on valid 6-frame FFV1.
    let tight_seventy_kib = CopyOptions {
        max_controlled_bytes: Some(70 << 10),
        ..Default::default()
    };
    fvid_media::plan_remux(&mkv, &tight_seventy_kib).unwrap();
    fvid::native_plan::remux_with_options(&mkv, &tight_seventy_kib)
        .unwrap()
        .unwrap();
    // And in concat_mp4_matroska_with_options, when sample-domain audio concat is planned
    // (small.m4a + padded.m4a, which are `sowt` PCM in MP4 so mp4_concat::open_with_options
    // returns Ok(None)), concat_mp4_matroska_with_options must propagate options.max_controlled_bytes
    // to concat_eligible_with_options and probe_with_options so that a budget violation returns
    // `Err(...)` (not `Ok(None)`)!
    let audio_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-metadata-admission");
    let small_m4a = audio_root.join("small.m4a");
    let padded_m4a = audio_root.join("padded.m4a");
    let one_packet_tight_m4a = CopyOptions {
        max_packets: Some(1),
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let small_then_padded_m4a = vec![small_m4a.clone(), padded_m4a.clone()];
    let pair_padded_m4a = vec![padded_m4a.clone(), padded_m4a.clone()];
    for inputs in [&small_then_padded_m4a, &pair_padded_m4a] {
        let concat_plan_err = fvid::native_plan::concat_mp4_matroska_with_options(
            inputs,
            &one_packet_tight_m4a,
        )
        .unwrap_err();
        assert!(
            concat_plan_err.contains("metadata exceeds budget"),
            "{concat_plan_err}"
        );
    }
    fvid_media::plan_remux(&mkv, &ample).unwrap();
    let event = fvid_media::remux(&mkv, &out_mkv, &ample).unwrap();
    assert_eq!(event.packets, 6);
    std::fs::remove_file(&out_mkv).unwrap();

    // 2. Matroska subtitle conversion (4 subtitle cues * 56 bytes = 224 bytes > 150 bytes)
    let srt_in = dir.join("four-cues.srt");
    std::fs::write(
        &srt_in,
        "1\n00:00:00,000 --> 00:00:00,500\nOne\n\n\
         2\n00:00:01,000 --> 00:00:01,500\nTwo\n\n\
         3\n00:00:02,000 --> 00:00:02,500\nThree\n\n\
         4\n00:00:03,000 --> 00:00:03,500\nFour\n",
    )
    .unwrap();
    let sub_mkv = dir.join("four-cues.mkv");
    let sub_opts = Default::default();
    fvid_media::convert_subtitles_with_controls(&srt_in, &sub_mkv, &sub_opts, &ample).unwrap();
    let srt_out = dir.join("converted.mkv");
    let sub_tight = CopyOptions {
        max_controlled_bytes: Some(150),
        ..Default::default()
    };
    for err in [
        fvid_media::plan_convert_subtitles_with_controls(&sub_mkv, &sub_opts, &sub_tight)
            .unwrap_err(),
        fvid_media::convert_subtitles_with_controls(&sub_mkv, &srt_out, &sub_opts, &sub_tight)
            .unwrap_err(),
        fvid::media::plan_convert_subtitles_with_controls(&sub_mkv, &sub_opts, &sub_tight)
            .unwrap_err(),
        fvid::media::convert_subtitles_with_controls(&sub_mkv, &srt_out, &sub_opts, &sub_tight)
            .unwrap_err(),
    ] {
        assert!(
            err.contains("controlled memory budget exceeded: Matroska packet index limit"),
            "unexpected subtitle error: {err}"
        );
    }
    assert!(!srt_out.exists());
    fvid_media::plan_convert_subtitles_with_controls(&sub_mkv, &sub_opts, &ample).unwrap();
    let sub_stats =
        fvid_media::convert_subtitles_with_controls(&sub_mkv, &srt_out, &sub_opts, &ample).unwrap();
    assert_eq!(sub_stats.cues, 4);
    std::fs::remove_file(&srt_out).unwrap();

    // 3. Matroska audio concat (pcm-int.mkv has 4 audio packets * 56 bytes = 224 bytes)
    let pcm_mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/pcm-int.mkv");
    let concat_pair = [pcm_mkv.clone(), pcm_mkv.clone()];
    let out_mka = dir.join("concat.mka");
    let concat_tight = CopyOptions {
        max_controlled_bytes: Some(150),
        max_packets: Some(2),
        ..Default::default()
    };
    for err in [
        fvid::media::plan_concat(&concat_pair, &concat_tight).unwrap_err(),
        fvid::media::concat(&concat_pair, &out_mka, &concat_tight).unwrap_err(),
    ] {
        assert!(
            err.contains("controlled memory budget exceeded: Matroska packet index limit"),
            "unexpected concat error: {err}"
        );
    }
    assert!(!out_mka.exists());
}

#[test]
fn y4m_export_trim_and_concat_propagate_controlled_memory_budget_to_native_reader() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-y4m-native-reader-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let small = source("small.mp4");
    let out_y4m = dir.join("out.y4m");

    // `small.mp4` is a 1-frame AVC video whose metadata (<1 KiB), encoded packet (<2 KiB),
    // and Y4M output frame (6 bytes) all fit well within a 4 KiB budget, whereas `NativeReader::software`
    // requires at least ~16 KiB (`budget * 3 / 8` for `work_budget`) for AVC reference + motion + picture
    // working storage. All Y4M planning and execution routes (`export_y4m_with_options`,
    // `export_y4m_pipeline_with_options`, `plan_trim_y4m`, `trim`, `plan_concat_y4m`, `concat`) must
    // propagate `options.max_controlled_bytes` to `NativeReader::software` rather than hardcoding `usize::MAX`.
    let tight_decoder = CopyOptions {
        max_controlled_bytes: Some(4096),
        ..Default::default()
    };
    let ample_decoder = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let pair_small = [small.clone(), small.clone()];
    for err in [
        fvid::native_plan::export_y4m_with_options(
            &small,
            None,
            &Default::default(),
            &Default::default(),
            &tight_decoder,
        )
        .unwrap_err(),
        fvid::native_export::export_y4m_pipeline_with_options(
            &small,
            &out_y4m,
            None,
            &Default::default(),
            &Default::default(),
            &tight_decoder,
        )
        .unwrap_err()
        .to_string(),
        fvid::media::plan_trim_y4m(&small, 0, 40_000, &tight_decoder).unwrap_err(),
        fvid::media::trim(&small, &out_y4m, 0, 40_000, &tight_decoder).unwrap_err(),
        fvid::media::plan_concat_y4m(&pair_small, &tight_decoder).unwrap_err(),
        fvid::media::concat(&pair_small, &out_y4m, &tight_decoder).unwrap_err(),
        match fvid::playback_native::NativeReader::software(
            BufReader::new(File::open(&small).unwrap()),
            4096,
        ) {
            Ok(mut reader) => match reader.read_frame_raw() {
                Ok(_) => panic!("NativeReader::software(4096) must reject AVC frame decode"),
                Err(e) => e.to_string(),
            },
            Err(e) => e.to_string(),
        },
    ] {
        assert!(
            err.contains("exceeds decoder memory budget")
                || err.contains("exceeds preparation byte limit")
                || err.contains("exceeds controlled memory budget"),
            "unexpected error for tight Y4M decoder budget: {err}"
        );
    }
    assert!(!out_y4m.exists());

    // Positive controls with ample 64 KiB budget succeed across plan and execute
    fvid::native_plan::export_y4m_with_options(
        &small,
        None,
        &Default::default(),
        &Default::default(),
        &ample_decoder,
    )
    .unwrap();
    let event = fvid::native_export::export_y4m_pipeline_with_options(
        &small,
        &out_y4m,
        None,
        &Default::default(),
        &Default::default(),
        &ample_decoder,
    )
    .unwrap();
    assert_eq!(event.packets, 1);
    std::fs::remove_file(&out_y4m).unwrap();
    fvid::media::plan_trim_y4m(&small, 0, 40_000, &ample_decoder).unwrap();
    let trim_event = fvid::media::trim(&small, &out_y4m, 0, 40_000, &ample_decoder).unwrap();
    assert_eq!(trim_event.packets, 1);
    std::fs::remove_file(&out_y4m).unwrap();
    fvid::media::plan_concat_y4m(&pair_small, &ample_decoder).unwrap();
    let concat_event = fvid::media::concat(&pair_small, &out_y4m, &ample_decoder).unwrap();
    assert_eq!(concat_event.packets, 2);
    std::fs::remove_file(&out_y4m).unwrap();

    // Direct callers of `export_audio_pcm_selected_with_controls` (used by mix/merge/concat spooling)
    // must also enforce `max_controlled_bytes` against MP4 metadata and Matroska packet indices.
    let audio_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-metadata-admission");
    let padded_m4a = audio_root.join("padded.m4a");
    let small_m4a = audio_root.join("small.m4a");
    let pcm_mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/pcm-int.mkv");
    let out_wav = dir.join("out.wav");
    let out_mka = dir.join("out.mka");
    let tight_mp4_audio = CopyOptions {
        max_controlled_bytes: Some(1 << 20),
        ..Default::default()
    };
    let tight_mkv_audio = CopyOptions {
        max_controlled_bytes: Some(150),
        ..Default::default()
    };
    let mp4_audio_err = fvid::native_export::export_audio_pcm_selected_with_controls(
        &padded_m4a,
        &out_wav,
        None,
        &tight_mp4_audio,
    )
    .unwrap_err()
    .to_string();
    assert!(
        mp4_audio_err.contains("metadata exceeds budget"),
        "{mp4_audio_err}"
    );
    for dest in [&out_wav, &out_mka] {
        let mkv_audio_err = fvid::native_export::export_audio_pcm_selected_with_controls(
            &pcm_mkv,
            dest,
            None,
            &tight_mkv_audio,
        )
        .unwrap_err()
        .to_string();
        assert!(
            mkv_audio_err.contains("controlled memory budget exceeded: Matroska packet index limit"),
            "{mkv_audio_err}"
        );
    }
    assert!(!out_wav.exists());
    assert!(!out_mka.exists());
    let stats_wav = fvid::native_export::export_audio_pcm_selected_with_controls(
        &small_m4a,
        &out_wav,
        None,
        &ample_decoder,
    )
    .unwrap();
    assert_eq!(stats_wav.sample_frames, 136);
    std::fs::remove_file(&out_wav).unwrap();
}

#[test]
fn video_decode_and_lossless_routes_propagate_controlled_memory_budget_to_codec_decoders() {
    let small_mp4 = source("small.mp4");
    let ffv1_mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-six-frames.mkv");

    // 4096 bytes is enough for container metadata and packet index/payload of both
    // `small.mp4` and `ffv1-six-frames.mkv` (6 packets * 56 bytes = 336 bytes, 6-byte frames),
    // but smaller than the AVC decoder reference/picture working set (~16 KiB) and FFV1
    // quantizer/context state (~20 KiB).
    let tight_codec = CopyOptions {
        max_controlled_bytes: Some(4096),
        ..Default::default()
    };
    let ample_codec = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };

    let dir = std::env::temp_dir().join(format!(
        "fvid-codec-decoder-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let out_mkv = dir.join("out.mkv");
    let srt = dir.join("cue.srt");
    std::fs::write(&srt, "1\n00:00:00,000 --> 00:00:00,040\nHi\n").unwrap();

    for source in [&ffv1_mkv, &small_mp4] {
        for err in [
            fvid_media::plan_transcode_lossless(source, &Default::default(), &tight_codec, None)
                .unwrap_err(),
            fvid_media::transcode_lossless(source, &out_mkv, Default::default(), &tight_codec)
                .unwrap_err(),
            fvid::media::plan_transcode_lossless(source, &Default::default(), &tight_codec, None)
                .unwrap_err(),
            fvid::media::transcode_lossless(source, &out_mkv, Default::default(), &tight_codec)
                .unwrap_err(),
            fvid_media::plan_burn_subtitles(source, &srt, &tight_codec).unwrap_err(),
            fvid_media::burn_subtitles(source, &out_mkv, &srt, &tight_codec).unwrap_err(),
        ] {
            assert!(
                err.contains("memory budget")
                    || err.contains("exceeds preparation byte limit")
                    || err.contains("exceeds controlled memory budget"),
                "unexpected error for {}: {err}",
                source.display()
            );
        }
        assert!(!out_mkv.exists());
    }

    for source in [&ffv1_mkv, &small_mp4] {
        fvid_media::plan_transcode_lossless(source, &Default::default(), &ample_codec, None)
            .unwrap();
        let stats =
            fvid_media::transcode_lossless(source, &out_mkv, Default::default(), &ample_codec)
                .unwrap();
        assert!(stats.video_frames >= 1);
        std::fs::remove_file(&out_mkv).unwrap();
        fvid_media::plan_burn_subtitles(source, &srt, &ample_codec).unwrap();
        let burn_stats =
            fvid_media::burn_subtitles(source, &out_mkv, &srt, &ample_codec).unwrap();
        assert!(burn_stats.video_frames >= 1);
        std::fs::remove_file(&out_mkv).unwrap();
    }
}

#[test]
fn probe_with_options_and_subtitle_burn_prepare_enforce_packet_and_memory_budgets_before_allocating() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-probe-and-burn-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());

    let y4m_burn = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn.y4m");
    let srt_burn = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn.srt");
    let out_mkv = dir.join("burn-out.mkv");

    // 1. Y4M subtitle burn (`subtitle-burn.y4m` is 96x64 YUV420p -> frame_len = 9216, mask_bytes = 6144).
    // With an invalid-font cue (e.g. unsupported glyph '\u{2603}'), `plan_burn_subtitles` used to
    // allocate and rasterize `cues` BEFORE checking whether `frame_len + mask_bytes > max_controlled_bytes`
    // or `frame_len > max_packet_bytes`! And `burn_subtitles` used to open the output Matroska writer
    // before checking `frame_len + mask_bytes > max_controlled_bytes`.
    let bad_glyph_srt = dir.join("bad-glyph.srt");
    std::fs::write(
        &bad_glyph_srt,
        "1\n00:00:00,000 --> 00:00:00,500\n\u{2603}\n",
    )
    .unwrap();
    let tight_burn_mem = CopyOptions {
        max_controlled_bytes: Some(12_000),
        ..Default::default()
    };
    let tight_burn_pkt = CopyOptions {
        max_packet_bytes: 4096,
        ..Default::default()
    };
    for (opts, expected) in [
        (&tight_burn_mem, "subtitle burn frame exceeds controlled memory budget"),
        (&tight_burn_pkt, "Y4M frame byte length exceeds packet byte limit"),
    ] {
        for err in [
            fvid_media::plan_burn_subtitles(&y4m_burn, &bad_glyph_srt, opts).unwrap_err(),
            fvid_media::burn_subtitles(&y4m_burn, &out_mkv, &bad_glyph_srt, opts).unwrap_err(),
        ] {
            assert!(
                err.contains(expected),
                "expected `{expected}` before cue rasterization, got: {err}"
            );
        }
        assert!(!out_mkv.exists());
    }

    // 2. WAVE `LIST INFO` metadata allocation in `probe_with_options` respects `max_controlled_bytes`
    let wav_info = dir.join("info.wav");
    fvid_media::owned_wav_file::write_wav_f32le(&wav_info, 48_000, 1, &[0.0; 480]).unwrap();
    let mut wav_bytes = std::fs::read(&wav_info).unwrap();
    let comment = vec![b'A'; 4096];
    let list_payload_len = 4 + 8 + comment.len();
    wav_bytes.extend_from_slice(b"LIST");
    wav_bytes.extend_from_slice(&(list_payload_len as u32).to_le_bytes());
    wav_bytes.extend_from_slice(b"INFOICMT");
    wav_bytes.extend_from_slice(&(comment.len() as u32).to_le_bytes());
    wav_bytes.extend_from_slice(&comment);
    let riff_size = u32::try_from(wav_bytes.len() - 8).unwrap();
    wav_bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
    std::fs::write(&wav_info, &wav_bytes).unwrap();

    let tight_wav_info = CopyOptions {
        max_controlled_bytes: Some(1024),
        ..Default::default()
    };
    let ample_wav_info = CopyOptions {
        max_controlled_bytes: Some(64 << 10),
        ..Default::default()
    };
    let wav_probe_err =
        fvid_media::owned_probe::probe_with_options(&wav_info, &tight_wav_info).unwrap_err();
    assert!(
        wav_probe_err.contains("controlled memory budget exceeded"),
        "{wav_probe_err}"
    );
    let wav_probed =
        fvid_media::owned_probe::probe_with_options(&wav_info, &ample_wav_info).unwrap();
    assert_eq!(wav_probed.metadata.get("comment").map(String::len), Some(4096));

    // 3. Bare FLAC, bare MPEG, Ogg, ADTS, and Y4M in `probe_with_options` honor `max_packet_bytes`,
    // `max_packets`, and `max_controlled_bytes`
    let flac_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-streaminfo/valid_baseline.flac");
    let mp3_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3");
    let ogg_fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ogg-g711-loudnorm-budget/ogg-opus-mono-4byte-pkt.ogg");
    let adts_fixture =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");

    let one_packet = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let tiny_packet = CopyOptions {
        max_packet_bytes: 8,
        ..Default::default()
    };
    let tiny_memory = CopyOptions {
        max_controlled_bytes: Some(32),
        ..Default::default()
    };
    for path in [&flac_fixture, &mp3_fixture, &ogg_fixture, &adts_fixture, &y4m_burn] {
        assert!(
            fvid_media::owned_probe::probe_with_options(path, &tiny_packet).is_err(),
            "tiny max_packet_bytes must fail for {}",
            path.display()
        );
        assert!(
            fvid_media::owned_probe::probe_with_options(path, &tiny_memory).is_err(),
            "tiny max_controlled_bytes must fail for {}",
            path.display()
        );
    }
    for path in [&adts_fixture, &y4m_burn] {
        let err = fvid_media::owned_probe::probe_with_options(path, &one_packet).unwrap_err();
        assert!(
            err.contains("packet") || err.contains("frame"),
            "max_packets=1 must fail for {}: {err}",
            path.display()
        );
    }
    let _ = srt_burn;
}

#[test]
fn ffv1_and_webm_multitrack_supports_checks_enforce_copy_options_before_full_decode() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-ffv1-supports-options-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());

    let ffv1_mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");
    let small_mp4 = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-intra-420-8bit.mp4");
    let srt = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn.srt");

    // 1. `owned_lossless::supports` and `owned_subtitle_burn::supports` must honor `options.cancel`
    // immediately rather than running an uncancelled full-stream decode in `owned_ffv1_export::supports`.
    let cancelled = fvid_control::CancelFlag::new();
    cancelled.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancelled),
        ..Default::default()
    };
    assert!(
        !fvid_media::owned_lossless::supports(&ffv1_mkv, &Default::default(), &cancelled_opts),
        "owned_lossless::supports must return false when options.cancel is already cancelled"
    );
    assert!(
        !fvid_media::owned_lossless::supports(&small_mp4, &Default::default(), &cancelled_opts),
        "owned_lossless::supports must return false for MP4 when options.cancel is already cancelled"
    );
    assert!(
        !fvid_media::owned_subtitle_burn::supports(&ffv1_mkv, &srt, &cancelled_opts),
        "owned_subtitle_burn::supports must return false when options.cancel is already cancelled"
    );

    // 2. When a Matroska/WebM file has a valid first frame and a corrupt second frame (or a Y4M
    // overlay whose frame exceeds `max_packet_bytes` or `max_controlled_bytes`), calling
    // `transcode_lossless` or `plan_transcode_lossless` with `max_packets: Some(1)` (or tight
    // `max_packet_bytes` / `max_controlled_bytes`) must enforce those limits during qualification
    // (`owned_ffv1_export::supports_with_options`) instead of decoding the entire stream with
    // `options = None` first!
    let mut corrupt_second_mkv = std::fs::read(&ffv1_mkv).unwrap();
    let len = corrupt_second_mkv.len();
    // Corrupt the tail bytes of the second packet in the 2-frame FFV1 Matroska fixture:
    for byte in &mut corrupt_second_mkv[len - 6..] {
        *byte ^= 0xff;
    }
    let corrupt_tail_path = dir.join("corrupt-second-frame.mkv");
    std::fs::write(&corrupt_tail_path, &corrupt_second_mkv).unwrap();

    let one_packet_opts = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    let out_mkv = dir.join("out.mkv");
    let plan_err = fvid_media::plan_transcode_lossless(
        &corrupt_tail_path,
        &Default::default(),
        &one_packet_opts,
        None,
    )
    .unwrap_err();
    assert!(
        plan_err.contains("packet count exceeds limit"),
        "expected packet count limit error before decoding packet 2, got: {plan_err}"
    );
    let export_err = fvid_media::transcode_lossless(
        &corrupt_tail_path,
        &out_mkv,
        Default::default(),
        &one_packet_opts,
    )
    .unwrap_err();
    assert!(
        export_err.contains("packet count exceeds limit"),
        "expected packet count limit error before decoding packet 2, got: {export_err}"
    );
    assert!(!out_mkv.exists());
}

#[test]
fn adts_and_matroska_remux_and_concat_enforce_memory_budget_and_cancellation() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-adts-matroska-remux-concat-budget-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());

    let adts = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-mono-44k.aac");
    let mkv = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");

    // 1. `owned_adts_remux::plan_remux` and `owned_concat::plan_concat` must enforce
    // `max_controlled_bytes` against accumulated Matroska packet-index / working-set growth
    // (`packets * 32 + 16384 + meta_bytes > limit`, or `sources.len() * 8192 + 16384 + packets * 32 + meta_bytes > limit`)
    // just as `write_adts_with_options`, `concat_adts_with_options`, `native_plan::remux_adts_with_options`,
    // and `native_plan::concat_adts_with_options` do.
    // `aac-mono-44k.aac` has 7 packets -> 7 * 32 = 224 bytes.
    // With `max_controlled_bytes: Some(16_384 + 128)`, the initial `16384 + meta_bytes` check passes,
    // and packet scanning must fail with `controlled memory budget exceeded`!
    let remux_mem_opts = CopyOptions {
        max_controlled_bytes: Some(16_384 + 128),
        ..Default::default()
    };
    let plan_remux_err = fvid_media::owned_adts_remux::plan_remux(&adts, &remux_mem_opts).unwrap_err();
    assert!(
        plan_remux_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from owned_adts_remux::plan_remux, got: {plan_remux_err}"
    );
    let exec_remux_err =
        fvid_media::owned_adts_remux::remux(&adts, &dir.join("out.mka"), &remux_mem_opts).unwrap_err();
    assert!(
        exec_remux_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from owned_adts_remux::remux, got: {exec_remux_err}"
    );

    // For 2-input concat, `base_bytes = 2 * 8192 + 16384 = 32768`.
    // With `max_controlled_bytes: Some(32_768 + 128)`, `open_readers` passes, and packet scanning
    // in `owned_concat::plan_concat` must fail with `controlled memory budget exceeded` (matching `owned_concat::concat`).
    let concat_mem_opts = CopyOptions {
        max_controlled_bytes: Some(32_768 + 128),
        ..Default::default()
    };
    let plan_concat_err =
        fvid_media::owned_concat::plan_concat(&[adts.clone(), adts.clone()], &concat_mem_opts)
            .unwrap_err();
    assert!(
        plan_concat_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from owned_concat::plan_concat, got: {plan_concat_err}"
    );
    let exec_concat_err = fvid_media::owned_concat::concat(
        &[adts.clone(), adts.clone()],
        &dir.join("concat.mka"),
        &concat_mem_opts,
    )
    .unwrap_err();
    assert!(
        exec_concat_err.contains("controlled memory budget exceeded"),
        "expected `controlled memory budget exceeded` from owned_concat::concat, got: {exec_concat_err}"
    );

    // 2. Pre-cancelled `CopyOptions` on `owned_remux::remux` and `owned_remux_plan::plan_remux`
    // for Matroska and ADTS inputs must fail with `cancelled`, rather than `request is not an owned ...`
    // or `owned WAVE remux requires .wav output`.
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    for (src, dst) in [(&adts, dir.join("adts.mka")), (&mkv, dir.join("mkv.mkv"))] {
        let remux_err = fvid_media::remux(src, &dst, &cancelled_opts).unwrap_err();
        assert!(
            remux_err.contains("cancelled"),
            "expected cancellation error from fvid_media::remux on {}, got: {remux_err}",
            src.display()
        );
        let plan_err = fvid_media::plan_remux(src, &cancelled_opts).unwrap_err();
        assert!(
            plan_err.contains("cancelled"),
            "expected cancellation error from fvid_media::plan_remux on {}, got: {plan_err}",
            src.display()
        );
    }
}

#[test]
fn probe_and_probe_as_with_options_enforce_packet_count_and_per_packet_bytes_across_containers() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let flac_fixture = root.join("tests/fixtures/playback-errors/flac-rate-escape/rate-8000.flac");
    let mp3_fixture = root.join("tests/fixtures/playback-errors/mp3-id3-metadata/tagged-id3v24.mp3");
    let ogg_fixture =
        root.join("tests/fixtures/playback-errors/ogg-g711-loudnorm-budget/ogg-opus-mono-4byte-pkt.ogg");
    let adts_fixture = root.join("tests/fixtures/audio/aac-mono-44k.aac");
    let y4m_fixture = root.join("tests/fixtures/playback-errors/subtitle-burn.y4m");
    let mkv_fixture = root.join("tests/fixtures/playback-errors/ffv1-gray-8.mkv");
    let mp4_fixture = root.join("tests/fixtures/playback-errors/avc-intra-420-8bit.mp4");

    // 1. `probe_ogg_with_options` must check `options.max_packet_bytes` against individual Ogg packet
    // lengths rather than the entire Ogg file byte length (`limits.file_bytes`).
    // `ogg-opus-mono-4byte-pkt.ogg` is > 100 bytes total, its largest packet (OpusHead/OpusTags) is <= 64 bytes,
    // and its audio packets are 4 bytes.
    let ogg_file_len = std::fs::metadata(&ogg_fixture).unwrap().len() as usize;
    assert!(ogg_file_len > 96, "expected Ogg fixture > 96 bytes, got {ogg_file_len}");
    let per_packet_64 = CopyOptions {
        max_packet_bytes: 64,
        ..Default::default()
    };
    let ogg_info = fvid_media::owned_probe::probe_ogg_with_options(&ogg_fixture, &per_packet_64)
        .expect("probe_ogg_with_options must succeed when max_packet_bytes >= largest packet even if < file_bytes");
    assert_eq!(ogg_info.streams[0].codec, "opus");
    let ogg_info_pub = fvid::media::probe_as_with_options(&ogg_fixture, Some("ogg"), &per_packet_64)
        .expect("fvid::media::probe_as_with_options must succeed when max_packet_bytes >= largest packet");
    assert_eq!(ogg_info_pub.streams[0].codec, "opus");

    // 2. `max_packets: Some(1)` must be enforced across multi-packet FLAC, MP3, Ogg, ADTS, Y4M, and Matroska
    // in both `fvid_media::owned_probe::probe_with_options` and `fvid::media::probe_as_with_options`:
    let one_packet = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    for (path, hint) in [
        (&flac_fixture, "flac"),
        (&mp3_fixture, "mp3"),
        (&ogg_fixture, "ogg"),
        (&adts_fixture, "aac"),
        (&y4m_fixture, "y4m"),
        (&mkv_fixture, "matroska"),
    ] {
        let err1 = fvid_media::owned_probe::probe_with_options(path, &one_packet).unwrap_err();
        assert!(
            err1.contains("packet") || err1.contains("frame"),
            "probe_with_options(max_packets=1) must fail for {}: {err1}",
            path.display()
        );
        let err2 = fvid::media::probe_as_with_options(path, Some(hint), &one_packet).unwrap_err();
        assert!(
            err2.contains("packet") || err2.contains("frame"),
            "probe_as_with_options({hint}, max_packets=1) must fail for {}: {err2}",
            path.display()
        );
    }

    // 3. `max_packets: Some(0)` and `max_packet_bytes: 0` must also fail for MP4 in `probe_with_options`
    // and `probe_as_with_options`:
    let zero_packets = CopyOptions {
        max_packets: Some(0),
        ..Default::default()
    };
    let zero_packet_bytes = CopyOptions {
        max_packet_bytes: 0,
        ..Default::default()
    };
    for opts in [&zero_packets, &zero_packet_bytes] {
        assert!(
            fvid_media::owned_probe::probe_with_options(&mp4_fixture, opts).is_err(),
            "MP4 probe_with_options must reject {:?}",
            (opts.max_packets, opts.max_packet_bytes)
        );
        assert!(
            fvid::media::probe_as_with_options(&mp4_fixture, Some("mp4"), opts).is_err(),
            "MP4 probe_as_with_options must reject {:?}",
            (opts.max_packets, opts.max_packet_bytes)
        );
    }

    // 4. Pre-cancelled `CopyOptions` must fail in `fvid::media::probe_as_with_options` for every explicit format hint:
    let cancel = fvid_control::CancelFlag::new();
    cancel.cancel();
    let cancelled_opts = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    for (path, hint) in [
        (&mp4_fixture, "mp4"),
        (&mkv_fixture, "matroska"),
        (&flac_fixture, "flac"),
        (&mp3_fixture, "mp3"),
        (&ogg_fixture, "ogg"),
        (&adts_fixture, "aac"),
        (&y4m_fixture, "y4m"),
    ] {
        let err = fvid::media::probe_as_with_options(path, Some(hint), &cancelled_opts).unwrap_err();
        assert!(
            err.contains("cancelled"),
            "expected cancellation error for probe_as_with_options({hint}), got: {err}"
        );
    }
}








