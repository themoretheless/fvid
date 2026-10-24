use fvid_media::{
    AudioDecodeTransform, CopyOptions, MixAudioOptions, apply_loudnorm, decode_audio,
    decode_audio_transformed, measure_loudness, merge_audio, mix_audio, plan_decode_audio,
    plan_merge_audio, plan_mix_audio,
};
use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-ogg-flac-plan")
        .join(name)
}

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn temp_dir(label: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!(
        "fvid-wave-ogg-flac-plan-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    TempDir(dir)
}

#[test]
fn wave_24in32_nonzero_padding_bits_rejected_across_export_loudness_and_loudnorm() {
    let dir = temp_dir("wave-padding");
    let valid = fixture_path("wave-24in32-valid.wav");
    let bad = fixture_path("wave-24in32-nonzero-padding.wav");

    let valid_out = dir.0.join("valid-out.wav");
    let stats = decode_audio(&valid, &valid_out, &CopyOptions::default())
        .expect("24-in-32 WAVE with zero low padding bits must decode cleanly");
    assert_eq!(stats.sample_frames, 480);

    let bad_out = dir.0.join("bad-out.wav");
    let err = decode_audio(&bad, &bad_out, &CopyOptions::default())
        .expect_err("24-in-32 WAVE with nonzero low padding bits must be rejected by decode_audio");
    assert!(
        err.contains("nonzero PCM padding bits"),
        "unexpected decode_audio error: {err}"
    );
    assert!(!bad_out.exists());

    let valid_loudness = measure_loudness(&valid, &CopyOptions::default())
        .expect("24-in-32 WAVE with zero low padding bits must measure loudness");
    assert_eq!(valid_loudness.sample_frames, 480);

    let loud_err = measure_loudness(&bad, &CopyOptions::default())
        .expect_err("24-in-32 WAVE with nonzero low padding bits must be rejected by measure_loudness");
    assert!(
        loud_err.contains("nonzero PCM padding bits"),
        "unexpected measure_loudness error: {loud_err}"
    );

    let linear_args = "I=-16:TP=-1.5:LRA=11:measured_I=-22:measured_TP=-12:measured_LRA=2:measured_thresh=-32:linear=true";
    let bad_ln_linear = dir.0.join("bad-ln-linear.wav");
    let ln_linear_err = apply_loudnorm(&bad, &bad_ln_linear, Some(linear_args), &CopyOptions::default())
        .expect_err("24-in-32 WAVE with nonzero low padding bits must be rejected by linear loudnorm");
    assert!(
        ln_linear_err.contains("nonzero PCM padding bits"),
        "unexpected linear loudnorm error: {ln_linear_err}"
    );
    assert!(!bad_ln_linear.exists());

    let bad_ln_dyn = dir.0.join("bad-ln-dyn.wav");
    let ln_dyn_err = apply_loudnorm(&bad, &bad_ln_dyn, None, &CopyOptions::default())
        .expect_err("24-in-32 WAVE with nonzero low padding bits must be rejected by dynamic loudnorm");
    assert!(
        ln_dyn_err.contains("nonzero PCM padding bits"),
        "unexpected dynamic loudnorm error: {ln_dyn_err}"
    );
    assert!(!bad_ln_dyn.exists());
}

#[test]
fn g711_and_ogg_inputs_supported_in_plan_decode_mix_and_merge_audio() {
    let dir = temp_dir("g711-ogg-plan");
    let alaw = fixture_path("g711-alaw-48k-mono.wav");
    let mulaw = fixture_path("g711-mulaw-48k-mono.wav");
    let alaw_empty = fixture_path("g711-alaw-empty.wav");
    let ogg_opus = fixture_path("ogg-opus-stereo.ogg");
    let ogg_vorbis = fixture_path("ogg-vorbis-stereo.ogg");
    let ogg_joint = fixture_path("ogg-joint-vorbis-opus.ogg");

    for (path, expected_codec) in [
        (&alaw, "pcm_alaw"),
        (&mulaw, "pcm_mulaw"),
        (&ogg_opus, "opus"),
        (&ogg_vorbis, "vorbis"),
        (&ogg_joint, "vorbis_opus"),
    ] {
        let plan = plan_decode_audio(path, &AudioDecodeTransform::default(), &CopyOptions::default())
            .unwrap_or_else(|e| panic!("plan_decode_audio failed for {}: {e}", path.display()));
        assert_eq!(plan.command, "decode-audio");
        assert_eq!(plan.streams[0].codec, expected_codec);
    }

    // G.711 mix and merge plans + execution
    let g711_mix_plan = plan_mix_audio(
        &[alaw.clone(), mulaw.clone()],
        &MixAudioOptions::default(),
    )
    .expect("plan_mix_audio must accept G.711 WAVE inputs");
    assert_eq!(g711_mix_plan.command, "mix-audio");

    let g711_merge_plan = plan_merge_audio(&[alaw.clone(), mulaw.clone()])
        .expect("plan_merge_audio must accept G.711 WAVE inputs");
    assert_eq!(g711_merge_plan.command, "merge-audio");

    let empty_err = plan_mix_audio(
        &[alaw.clone(), alaw_empty],
        &MixAudioOptions::default(),
    )
    .expect_err("plan_mix_audio must reject empty G.711 WAVE input");
    assert!(
        empty_err.contains("nonempty"),
        "unexpected empty G.711 mix error: {empty_err}"
    );

    let g711_mixed = dir.0.join("g711-mixed.wav");
    let mix_stats = mix_audio(&[alaw.clone(), mulaw.clone()], &g711_mixed, &MixAudioOptions::default())
        .expect("mix_audio on G.711 WAVE must succeed");
    assert_eq!(mix_stats.sample_frames, 480);

    let g711_merged = dir.0.join("g711-merged.wav");
    let merge_stats = merge_audio(&[alaw, mulaw], &g711_merged)
        .expect("merge_audio on G.711 WAVE must succeed");
    assert_eq!(merge_stats.sample_frames, 480);
    assert_eq!(merge_stats.channels, 2);

    // Ogg Opus / Vorbis / Joint mix and merge plans + execution
    let ogg_mix_plan = plan_mix_audio(
        &[ogg_opus.clone(), ogg_vorbis.clone(), ogg_joint.clone()],
        &MixAudioOptions::default(),
    )
    .expect("plan_mix_audio must accept Ogg Opus, Vorbis, and Joint inputs");
    assert_eq!(ogg_mix_plan.command, "mix-audio");

    let ogg_merge_plan = plan_merge_audio(&[ogg_opus.clone(), ogg_vorbis.clone()])
        .expect("plan_merge_audio must accept Ogg inputs");
    assert_eq!(ogg_merge_plan.command, "merge-audio");

    let ogg_mixed = dir.0.join("ogg-mixed.wav");
    let ogg_mix_stats = mix_audio(
        &[ogg_opus, ogg_vorbis],
        &ogg_mixed,
        &MixAudioOptions::default(),
    )
    .expect("mix_audio on Ogg Opus + Vorbis must succeed");
    assert!(ogg_mix_stats.sample_frames > 0);
}

#[test]
fn ogg_chained_vorbis_and_joint_speaker_mask_admission_and_dangling_symlinks() {
    let dir = temp_dir("ogg-rematrix-symlink");
    let vorbis_5ch_0x37 = fixture_path("ogg-vorbis-chained-5ch-0x37.ogg");
    let joint_5ch_0x37 = fixture_path("ogg-joint-chained-5ch-0x37.ogg");
    let vorbis_9ch_0x73f = fixture_path("ogg-vorbis-chained-8ch-0x73f.ogg");
    let joint_9ch_0x73f = fixture_path("ogg-joint-chained-8ch-0x73f.ogg");

    for path in [&vorbis_5ch_0x37, &joint_5ch_0x37] {
        // Identity 5-channel export and plan are supported; 5->2 stereo downmix is also supported (mask 0x37).
        let id_out = dir.0.join(format!(
            "id-{}.wav",
            path.file_stem().unwrap().to_str().unwrap()
        ));
        plan_decode_audio(path, &AudioDecodeTransform::default(), &CopyOptions::default())
            .expect("identity 5ch chained Ogg plan must succeed");
        let id_stats = decode_audio(path, &id_out, &CopyOptions::default())
            .expect("identity 5ch chained Ogg export must succeed");
        assert_eq!(id_stats.channels, 5);

        let stereo_transform = AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        plan_decode_audio(path, &stereo_transform, &CopyOptions::default())
            .expect("5ch mask 0x37 stereo downmix plan must succeed");
        let stereo_out = dir.0.join(format!(
            "stereo-{}.wav",
            path.file_stem().unwrap().to_str().unwrap()
        ));
        let stereo_stats =
            decode_audio_transformed(path, &stereo_out, stereo_transform, &CopyOptions::default())
                .expect("5ch mask 0x37 stereo downmix export must succeed");
        assert_eq!(stereo_stats.channels, 2);
    }

    for (path, expected_ch) in [(&vorbis_9ch_0x73f, 9), (&joint_9ch_0x73f, 4)] {
        // Identity export and plan are supported.
        let id_out = dir.0.join(format!(
            "id-{}.wav",
            path.file_stem().unwrap().to_str().unwrap()
        ));
        plan_decode_audio(path, &AudioDecodeTransform::default(), &CopyOptions::default())
            .expect("identity chained Ogg plan must succeed");
        let id_stats = decode_audio(path, &id_out, &CopyOptions::default())
            .expect("identity chained Ogg export must succeed");
        assert_eq!(id_stats.channels, expected_ch);

        // Downmix to stereo must be rejected by both plan_decode_audio() and decode_audio_transformed()
        // because discrete multichannel (mask 0 / 9ch) has no speaker layout for stereo rematrix.
        let stereo_transform = AudioDecodeTransform {
            channels: Some(2),
            ..Default::default()
        };
        let plan_err = plan_decode_audio(path, &stereo_transform, &CopyOptions::default())
            .expect_err("plan_decode_audio must reject stereo downmix for discrete multichannel Ogg");
        assert!(
            plan_err.contains("channel conversion") || plan_err.contains("speaker layout"),
            "unexpected plan error: {plan_err}"
        );
        let stereo_out = dir.0.join(format!(
            "stereo-{}.wav",
            path.file_stem().unwrap().to_str().unwrap()
        ));
        let export_err =
            decode_audio_transformed(path, &stereo_out, stereo_transform, &CopyOptions::default())
                .expect_err("decode_audio_transformed must reject stereo downmix for discrete multichannel Ogg");
        assert!(
            export_err.contains("channel conversion") || export_err.contains("speaker layout"),
            "unexpected export error: {export_err}"
        );
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        for (idx, src) in [
            fixture_path("g711-alaw-48k-mono.wav"),
            fixture_path("ogg-vorbis-stereo.ogg"),
            fixture_path("ogg-joint-vorbis-opus.ogg"),
        ]
        .iter()
        .enumerate()
        {
            let dangling = dir.0.join(format!("dangling-{idx}.wav"));
            symlink(dir.0.join("nonexistent-target.wav"), &dangling).expect("create dangling symlink");
            let err = decode_audio(src, &dangling, &CopyOptions::default())
                .expect_err("dangling symlink destination must be rejected upfront");
            assert!(
                err.contains("already exists"),
                "unexpected dangling symlink error for {}: {err}",
                src.display()
            );
        }
    }
}

#[test]
fn raw_flac_and_mp3_export_supports_preserve_budget_policy_and_rematrix_admission() {
    let dir = temp_dir("raw-flac-mp3");
    let flac_quad = fixture_path("flac-4ch-quad.flac");
    let flac_unassigned = fixture_path("flac-4ch-unassigned.flac");
    let mp3 = fixture_path("raw-stereo.mp3");
    let out = dir.0.join("out.wav");

    let tight_budget = CopyOptions {
        max_controlled_bytes: Some(1),
        ..Default::default()
    };
    for src in [&flac_quad, &flac_unassigned, &mp3] {
        let err = decode_audio(src, &out, &tight_budget)
            .expect_err("decode_audio must fail with controlled memory budget exceeded");
        assert!(
            err.contains("controlled memory budget exceeded"),
            "unexpected budget error for {}: {err}",
            src.display()
        );
    }

    let downmix = AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    let quad_out = dir.0.join("quad-downmix.wav");
    let quad_stats = decode_audio_transformed(&flac_quad, &quad_out, downmix, &CopyOptions::default())
        .expect("4ch FLAC with standard quad mask 0x33 must downmix to stereo");
    assert_eq!(quad_stats.channels, 2);

    let flac_err = decode_audio_transformed(&flac_unassigned, &out, downmix, &CopyOptions::default())
        .expect_err("4ch FLAC with mask 0x0 must fail stereo downmix");
    assert!(flac_err.contains("speaker layout"));

    let invalid_upmix = AudioDecodeTransform {
        channels: Some(6),
        ..Default::default()
    };
    let upmix_err =
        decode_audio_transformed(&flac_quad, &out, invalid_upmix, &CopyOptions::default())
            .expect_err("4ch FLAC must reject 4->6 channel conversion");
    assert!(
        upmix_err.contains("channel"),
        "unexpected 4->6 FLAC error: {upmix_err}"
    );
    let mp3_out = dir.0.join("mp3-upmix.wav");
    let mp3_stats = decode_audio_transformed(&mp3, &mp3_out, invalid_upmix, &CopyOptions::default())
        .expect("2ch MP3 supports 2->6 upmix");
    assert_eq!(mp3_stats.channels, 6);
}
