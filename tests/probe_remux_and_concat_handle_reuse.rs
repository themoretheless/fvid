use fvid_media::{CopyOptions, concat, plan_concat, plan_remux, remux};
use std::{path::Path, time::Instant};

#[test]
fn probe_remux_and_concat_reuse_open_file_handles() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mp4 = root.join("tests/fixtures/audio/aac-native-edit.m4a");
    let mkv = root.join("tests/fixtures/playback-errors/webm-probe-clear.mkv");
    let adts = root.join("tests/fixtures/audio/aac-mono-44k.aac");
    let wav = root.join("tests/fixtures/playback-errors/loudness-normalize-budget-parity/stereo-48k.wav");
    let ogg = root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-vorbis-chained-5ch-0x37.ogg");
    let flac = root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    let mp3 = root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");

    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-probe-remux-concat-reuse-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let y4m = temp_dir.join("synthetic.y4m");
    std::fs::write(&y4m, b"YUV4MPEG2 W2 H2 F24:1 Ip A1:1 C420\nFRAME\n\x10\x10\x10\x10\x80\x80").unwrap();

    let start = Instant::now();

    // FVID-0960 & FVID-0961: owned_probe::probe_as_with_options and native_probe::try_probe_as
    // reuse open File handle across 12-byte signature dispatch and container probes.
    let opts = CopyOptions::default();
    for path in [&mp4, &mkv, &adts, &wav, &ogg, &flac, &mp3, &y4m] {
        let info_owned = fvid_media::owned_probe::probe_as_with_options(path, None, &opts)
            .expect("owned probe should succeed");
        let info_frontend = fvid::native_probe::probe_as(path, None)
            .expect("frontend probe should succeed");
        assert_eq!(info_owned.streams.len(), info_frontend.streams.len());
        assert_eq!(info_owned.streams[0].codec, info_frontend.streams[0].codec);
    }

    // FVID-0962 & FVID-0963: owned_mp4_remux, owned_adts_remux, owned_matroska_remux,
    // owned_remux, and owned_remux_plan reuse open File handles and Mp4Reader/TrackPlans.
    let mp4_plan = plan_remux(&mp4, &opts).expect("MP4 remux plan should succeed");
    assert_eq!(mp4_plan.command, "remux");
    assert_eq!(mp4_plan.streams[0].codec, "aac");

    let adts_plan = plan_remux(&adts, &opts).expect("ADTS remux plan should succeed");
    assert_eq!(adts_plan.command, "remux");
    assert_eq!(adts_plan.streams[0].codec, "aac");

    let mkv_plan = plan_remux(&mkv, &opts).expect("Matroska remux plan should succeed");
    assert_eq!(mkv_plan.command, "remux");
    assert_eq!(mkv_plan.streams[0].codec, "ffv1");

    let out_mp4_mkv = temp_dir.join("from-mp4.mkv");
    let stats_mp4 = remux(&mp4, &out_mp4_mkv, &opts).expect("MP4 remux should succeed");
    assert!(stats_mp4.packets > 0);

    let out_adts_mka = temp_dir.join("from-adts.mka");
    let stats_adts = remux(&adts, &out_adts_mka, &opts).expect("ADTS remux should succeed");
    assert!(stats_adts.packets > 0);

    let out_mkv_mkv = temp_dir.join("from-mkv.mkv");
    let stats_mkv = remux(&mkv, &out_mkv_mkv, &opts).expect("Matroska remux should succeed");
    assert!(stats_mkv.packets > 0);

    // FVID-0964: owned_concat reuses opened ADTS File handles across qualification and SequenceReader.
    let concat_sources = vec![adts.clone(), adts.clone()];
    let concat_plan =
        plan_concat(&concat_sources, &opts).expect("ADTS concat plan should succeed");
    assert_eq!(concat_plan.command, "concat");
    assert_eq!(concat_plan.inputs.len(), 2);

    let out_concat_mka = temp_dir.join("concat-adts.mka");
    let stats_concat =
        concat(&concat_sources, &out_concat_mka, &opts).expect("ADTS concat should succeed");
    assert_eq!(stats_concat.segments, 2);
    assert_eq!(stats_concat.packets, stats_adts.packets * 2);

    let _ = std::fs::remove_dir_all(&temp_dir);
    println!("elapsed_ms={}", start.elapsed().as_millis());
}
