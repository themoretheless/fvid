use fvid::media::{CopyOptions, MixAudioOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name)
}

#[test]
fn mix_merge_and_matroska_concat_support_container_and_stream_metadata_and_stream_selection() {
    let wav_a = fixture("audio-mix-merge-concat-metadata/tone-a-48k-stereo.wav");
    let wav_b = fixture("audio-mix-merge-concat-metadata/tone-b-48k-stereo.wav");
    let mp4_av = fixture("shared-mp4-av-multiple.mp4");

    let temp = std::env::temp_dir().join(format!(
        "fvid-mix-merge-concat-metadata-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();

    // 1. mix_audio_with_controls and plan_mix_audio_with_controls support WAVE INFO and Matroska
    //    container/stream metadata mutations and reject invalid metadata or out-of-range stream indices.
    let wav_meta_controls = CopyOptions {
        metadata_set: vec![
            ("title".into(), "Mixed Wave Title".into()),
            ("artist".into(), "Mixed Artist".into()),
        ],
        metadata_delete: vec!["comment".into()],
        ..Default::default()
    };
    let mix_plan = fvid::media::plan_mix_audio_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &MixAudioOptions::default(),
        &wav_meta_controls,
    )
    .expect("plan_mix_audio_with_controls must accept valid metadata mutations");
    assert!(
        mix_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in mix-audio plan: {:?}",
        mix_plan.steps
    );

    let mixed_wav = temp.join("mixed-tagged.wav");
    fvid::media::mix_audio_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &mixed_wav,
        &MixAudioOptions::default(),
        &wav_meta_controls,
    )
    .expect("mix_audio_with_controls to .wav with INFO metadata failed");
    let probed_mixed_wav = fvid::media::probe(&mixed_wav).unwrap();
    assert_eq!(
        probed_mixed_wav.metadata.get("title").map(String::as_str),
        Some("Mixed Wave Title")
    );
    assert_eq!(
        probed_mixed_wav.metadata.get("artist").map(String::as_str),
        Some("Mixed Artist")
    );

    let mkv_meta_controls = CopyOptions {
        metadata_set: vec![("title".into(), "Mixed Matroska Title".into())],
        stream_metadata_set: vec![
            (0, "title".into(), "Mixed Track".into()),
            (0, "language".into(), "eng".into()),
        ],
        ..Default::default()
    };
    let mixed_mka = temp.join("mixed-tagged.mka");
    fvid::media::mix_audio_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &mixed_mka,
        &MixAudioOptions::default(),
        &mkv_meta_controls,
    )
    .expect("mix_audio_with_controls to .mka with container and stream metadata failed");
    let probed_mixed_mka = fvid::media::probe(&mixed_mka).unwrap();
    assert_eq!(
        probed_mixed_mka.metadata.get("title").map(String::as_str),
        Some("Mixed Matroska Title")
    );
    assert_eq!(
        probed_mixed_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Mixed Track")
    );
    assert_eq!(
        probed_mixed_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("eng")
    );

    // Stream metadata on .wav output must be rejected cleanly.
    let bad_wav_stream_meta = temp.join("bad-stream-meta.wav");
    assert!(
        fvid::media::mix_audio_with_controls(
            &[wav_a.clone(), wav_b.clone()],
            &bad_wav_stream_meta,
            &MixAudioOptions::default(),
            &mkv_meta_controls,
        )
        .is_err(),
        "WAVE mix-audio output must reject stream metadata mutations"
    );
    assert!(!bad_wav_stream_meta.exists());

    // Out-of-range stream metadata index (> 0) must be rejected in plan and execution.
    let bad_stream_idx = CopyOptions {
        stream_metadata_set: vec![(1, "title".into(), "OutOfRange".into())],
        ..Default::default()
    };
    assert!(
        fvid::media::plan_mix_audio_with_controls(
            &[wav_a.clone(), wav_b.clone()],
            &MixAudioOptions::default(),
            &bad_stream_idx,
        )
        .is_err(),
        "plan_mix_audio_with_controls must reject stream metadata index > 0"
    );

    // 2. merge_audio_with_controls and plan_merge_audio_with_controls support WAVE INFO and
    //    Matroska container/stream metadata mutations.
    let merge_plan =
        fvid::media::plan_merge_audio_with_controls(&[wav_a.clone(), wav_b.clone()], &mkv_meta_controls)
            .expect("plan_merge_audio_with_controls must accept valid metadata mutations");
    assert!(
        merge_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in merge-audio plan: {:?}",
        merge_plan.steps
    );

    let merged_wav = temp.join("merged-tagged.wav");
    fvid::media::merge_audio_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &merged_wav,
        &wav_meta_controls,
    )
    .expect("merge_audio_with_controls to .wav with INFO metadata failed");
    let probed_merged_wav = fvid::media::probe(&merged_wav).unwrap();
    assert_eq!(
        probed_merged_wav.metadata.get("title").map(String::as_str),
        Some("Mixed Wave Title")
    );

    let merged_mka = temp.join("merged-tagged.mka");
    fvid::media::merge_audio_with_controls(
        &[wav_a.clone(), wav_b.clone()],
        &merged_mka,
        &mkv_meta_controls,
    )
    .expect("merge_audio_with_controls to .mka with container and stream metadata failed");
    let probed_merged_mka = fvid::media::probe(&merged_mka).unwrap();
    assert_eq!(
        probed_merged_mka.metadata.get("title").map(String::as_str),
        Some("Mixed Matroska Title")
    );
    assert_eq!(
        probed_merged_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Mixed Track")
    );

    // 3. Compressed MP4->Matroska concat and plan_concat support --streams selection/reordering
    //    and container/stream metadata mutations, including audio-only selection into .mka!
    let mp4_concat_controls = CopyOptions {
        streams: vec![2, 0],
        metadata_set: vec![("title".into(), "Concat Audio Only".into())],
        stream_metadata_set: vec![
            (2, "title".into(), "Second Audio First".into()),
            (0, "language".into(), "fra".into()),
        ],
        ..Default::default()
    };
    let mp4_concat_plan =
        fvid::media::plan_concat(&[mp4_av.clone(), mp4_av.clone()], &mp4_concat_controls)
            .expect("plan_concat on MP4 segments must support stream selection and metadata");
    assert_eq!(mp4_concat_plan.streams.len(), 2);
    assert_eq!(mp4_concat_plan.streams[0].index, 2);
    assert_eq!(mp4_concat_plan.streams[1].index, 0);
    assert!(
        mp4_concat_plan.steps.iter().any(|s| s.action == "metadata"),
        "expected metadata step in MP4 concat plan: {:?}",
        mp4_concat_plan.steps
    );

    let concat_mka = temp.join("mp4-audio-selected.mka");
    fvid::media::concat(
        &[mp4_av.clone(), mp4_av.clone()],
        &concat_mka,
        &mp4_concat_controls,
    )
    .expect("MP4 concat with audio-only --streams 2,0 to .mka failed");
    let probed_concat_mka = fvid::media::probe(&concat_mka).unwrap();
    assert_eq!(probed_concat_mka.streams.len(), 2);
    assert_eq!(
        probed_concat_mka.metadata.get("title").map(String::as_str),
        Some("Concat Audio Only")
    );
    assert_eq!(
        probed_concat_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Second Audio First")
    );
    assert_eq!(
        probed_concat_mka.streams[1]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    // 4. CLI routing for mix-audio, merge-audio, and concat with metadata and stream controls.
    let bin = env!("CARGO_BIN_EXE_fvid");
    let cli_mix_wav = temp.join("cli-mix.wav");
    let out = Command::new(bin)
        .args([
            "media",
            "mix-audio",
            cli_mix_wav.to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--metadata",
            "title=CLIMixed",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "CLI mix-audio --metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        fvid::media::probe(&cli_mix_wav)
            .unwrap()
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLIMixed")
    );

    let cli_merge_mka = temp.join("cli-merge.mka");
    let out = Command::new(bin)
        .args([
            "media",
            "merge-audio",
            cli_merge_mka.to_str().unwrap(),
            wav_a.to_str().unwrap(),
            wav_b.to_str().unwrap(),
            "--metadata",
            "title=CLIMerged",
            "--stream-metadata",
            "0:title=MergedStream",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "CLI merge-audio --metadata --stream-metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let probed_cli_merge = fvid::media::probe(&cli_merge_mka).unwrap();
    assert_eq!(
        probed_cli_merge.metadata.get("title").map(String::as_str),
        Some("CLIMerged")
    );
    assert_eq!(
        probed_cli_merge.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MergedStream")
    );

    let cli_concat_mkv = temp.join("cli-concat.mkv");
    let out = Command::new(bin)
        .args([
            "media",
            "concat",
            cli_concat_mkv.to_str().unwrap(),
            mp4_av.to_str().unwrap(),
            mp4_av.to_str().unwrap(),
            "--streams",
            "1,0",
            "--metadata",
            "title=CLIConcat",
            "--stream-metadata",
            "1:title=MainVideo",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "CLI concat MP4->MKV with --streams and --metadata failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let probed_cli_concat = fvid::media::probe(&cli_concat_mkv).unwrap();
    assert_eq!(probed_cli_concat.streams.len(), 2);
    assert_eq!(probed_cli_concat.streams[0].media_type, "video");
    assert_eq!(probed_cli_concat.streams[1].media_type, "audio");
    assert_eq!(
        probed_cli_concat.metadata.get("title").map(String::as_str),
        Some("CLIConcat")
    );
    assert_eq!(
        probed_cli_concat.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MainVideo")
    );

    let _ = std::fs::remove_dir_all(&temp);
}
