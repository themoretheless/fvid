//! Regression tests for Batch 248 (`FVID-0255`):
//! 1. `fvid::native_loudnorm::try_plan` and `fvid::native_loudnorm::try_apply` enforce
//!    `options.max_packet_bytes` on ADTS, MP4, and Matroska sources (returning
//!    `"ADTS packet exceeds budget"` / `"MP4 packet exceeds budget"` when `max_packet_bytes`
//!    is smaller than an encoded packet), and validate WAVE stream selection (`options.streams = vec![1]`
//!    or `vec![0, 1]`) and `options.max_packet_bytes` against the WAVE PCM frame size before falling
//!    back to `fvid_media::plan_loudnorm` / `fvid_media::apply_loudnorm`.
//! 2. `fvid::native_audio_mix::validate_and_inspect_source_with_options` (used by `mix_audio_with_controls`,
//!    `plan_mix_audio_with_controls`, `merge_audio_with_controls`, `plan_merge_audio_with_controls`, and
//!    `concat_audio_with_options`) propagates specific budget and packet-limit errors (`"controlled memory budget exceeded"`,
//!    `"PCM packet limit cannot hold one sample frame"`, `"MP4 packet exceeds budget"`, `"ADTS packet exceeds budget"`)
//!    when `options.max_controlled_bytes` or `options.max_packet_bytes` rejects an otherwise-valid single-stream
//!    audio source, instead of masking them with `"source is not a single-stream owned audio input"`.
//! 3. `fvid::native_plan::loudness_with_options` passes `options` to `decode_audio_with_options` instead of
//!    `decode_audio_selected` with default options, so `plan_loudness` and `plan_normalize_loudness`
//!    immediately enforce `options.max_packet_bytes` on WAVE, MP4, Matroska, and ADTS sources.

use std::path::PathBuf;

fn audio_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio")
}

fn write_synthetic_wave_stereo_s16(path: &std::path::Path) {
    // 48 kHz stereo 16-bit PCM (frame_bytes = 4), 24000 sample frames (0.5 s)
    let mut pcm_bytes = Vec::with_capacity(24_000 * 4);
    for i in 0..24_000i32 {
        let l: i16 = if i % 4 < 2 { 4096 } else { -4096 };
        let r: i16 = if i % 4 < 2 { -4096 } else { 4096 };
        pcm_bytes.extend_from_slice(&l.to_le_bytes());
        pcm_bytes.extend_from_slice(&r.to_le_bytes());
    }
    let mut fmt = Vec::with_capacity(16);
    fmt.extend_from_slice(&1u16.to_le_bytes());
    fmt.extend_from_slice(&2u16.to_le_bytes());
    fmt.extend_from_slice(&48_000u32.to_le_bytes());
    fmt.extend_from_slice(&192_000u32.to_le_bytes());
    fmt.extend_from_slice(&4u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());

    let mut riff_body = Vec::new();
    riff_body.extend_from_slice(b"WAVEfmt ");
    riff_body.extend_from_slice(&(fmt.len() as u32).to_le_bytes());
    riff_body.extend_from_slice(&fmt);
    riff_body.extend_from_slice(b"data");
    riff_body.extend_from_slice(&(pcm_bytes.len() as u32).to_le_bytes());
    riff_body.extend_from_slice(&pcm_bytes);

    let mut file = Vec::new();
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&(riff_body.len() as u32).to_le_bytes());
    file.extend_from_slice(&riff_body);
    std::fs::write(path, file).unwrap();
}

#[test]
fn loudnorm_plan_and_apply_enforce_packet_budgets_and_wave_stream_selection() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-loudnorm-budget-parity-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let wav = dir.join("stereo.wav");
    write_synthetic_wave_stereo_s16(&wav);
    let adts = audio_fixture_dir().join("aac-stereo.aac");
    let mp4 = audio_fixture_dir().join("aac-native-edit.m4a");

    // 1. ADTS loudnorm with tight max_packet_bytes = 2 (smaller than the first ADTS packet payload)
    let mut tight_adts = fvid::media::CopyOptions::default();
    tight_adts.max_packet_bytes = 2;

    let plan_adts_err = fvid::media::plan_loudnorm(&adts, None, false, &tight_adts).unwrap_err();
    assert!(
        plan_adts_err.contains("ADTS packet exceeds budget"),
        "plan_loudnorm on ADTS with max_packet_bytes=2 must report 'ADTS packet exceeds budget', got: {plan_adts_err}"
    );
    let out_adts = dir.join("out-adts.wav");
    let apply_adts_err =
        fvid::media::apply_loudnorm(&adts, &out_adts, None, &tight_adts).unwrap_err();
    assert!(
        apply_adts_err.contains("ADTS packet exceeds budget"),
        "apply_loudnorm on ADTS with max_packet_bytes=2 must report 'ADTS packet exceeds budget', got: {apply_adts_err}"
    );
    assert!(!out_adts.exists());

    // 2. MP4 loudnorm with tight max_packet_bytes = 2 (smaller than MP4 AAC sample sizes)
    let mut tight_mp4 = fvid::media::CopyOptions::default();
    tight_mp4.max_packet_bytes = 2;

    let plan_mp4_err = fvid::media::plan_loudnorm(&mp4, None, false, &tight_mp4).unwrap_err();
    assert!(
        plan_mp4_err.contains("MP4 packet exceeds budget"),
        "plan_loudnorm on MP4 with max_packet_bytes=2 must report 'MP4 packet exceeds budget', got: {plan_mp4_err}"
    );
    let out_mp4 = dir.join("out-mp4.wav");
    let apply_mp4_err =
        fvid::media::apply_loudnorm(&mp4, &out_mp4, None, &tight_mp4).unwrap_err();
    assert!(
        apply_mp4_err.contains("MP4 packet exceeds budget"),
        "apply_loudnorm on MP4 with max_packet_bytes=2 must report 'MP4 packet exceeds budget', got: {apply_mp4_err}"
    );
    assert!(!out_mp4.exists());

    let dual_mp4_err =
        fvid::media::apply_loudnorm_dual(&mp4, &out_mp4, None, &tight_mp4).unwrap_err();
    assert!(
        dual_mp4_err.contains("MP4 packet exceeds budget"),
        "apply_loudnorm_dual on MP4 with max_packet_bytes=2 must report 'MP4 packet exceeds budget', got: {dual_mp4_err}"
    );
    assert!(!out_mp4.exists());

    // 3. WAVE loudnorm with non-zero stream selection (streams = [1] and streams = [0, 1])
    for bad_streams in [vec![1], vec![0, 1]] {
        let mut opts = fvid::media::CopyOptions::default();
        opts.streams = bad_streams.clone();

        let plan_wav_err = fvid::media::plan_loudnorm(&wav, None, false, &opts).unwrap_err();
        assert!(
            plan_wav_err.contains("WAVE has only stream 0")
                || plan_wav_err.contains("loudnorm requires one selected audio stream"),
            "plan_loudnorm on WAVE with streams={bad_streams:?} must reject invalid stream selection, got: {plan_wav_err}"
        );
        let out_wav = dir.join("out-wav-stream.wav");
        let apply_wav_err =
            fvid::media::apply_loudnorm(&wav, &out_wav, None, &opts).unwrap_err();
        assert!(
            apply_wav_err.contains("WAVE has only stream 0")
                || apply_wav_err.contains("loudnorm requires one selected audio stream"),
            "apply_loudnorm on WAVE with streams={bad_streams:?} must reject invalid stream selection, got: {apply_wav_err}"
        );
        assert!(!out_wav.exists());
    }

    // 4. WAVE loudnorm with max_packet_bytes = 2 (smaller than 4-byte stereo 16-bit PCM frame)
    let mut tight_wav = fvid::media::CopyOptions::default();
    tight_wav.max_packet_bytes = 2;
    let plan_wav_pkt_err =
        fvid::media::plan_loudnorm(&wav, None, false, &tight_wav).unwrap_err();
    assert!(
        plan_wav_pkt_err.contains("PCM packet limit cannot hold one sample frame"),
        "plan_loudnorm on stereo s16 WAVE with max_packet_bytes=2 must fail frame check, got: {plan_wav_pkt_err}"
    );
    let out_wav_pkt = dir.join("out-wav-pkt.wav");
    let apply_wav_pkt_err =
        fvid::media::apply_loudnorm(&wav, &out_wav_pkt, None, &tight_wav).unwrap_err();
    assert!(
        apply_wav_pkt_err.contains("PCM packet limit cannot hold one sample frame"),
        "apply_loudnorm on stereo s16 WAVE with max_packet_bytes=2 must fail frame check, got: {apply_wav_pkt_err}"
    );
    assert!(!out_wav_pkt.exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn audio_mix_and_merge_propagate_controlled_memory_and_packet_budget_errors() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-mix-merge-budget-parity-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let wav_a = dir.join("a.wav");
    let wav_b = dir.join("b.wav");
    write_synthetic_wave_stereo_s16(&wav_a);
    write_synthetic_wave_stereo_s16(&wav_b);
    let wav_sources = vec![wav_a.clone(), wav_b.clone()];
    let mix_opts = fvid::media::MixAudioOptions::default();

    // 1. Tight controlled memory budget (max_controlled_bytes = Some(1)) on WAVE inputs
    let mut tight_mem = fvid::media::CopyOptions::default();
    tight_mem.max_controlled_bytes = Some(1);

    let plan_mix_err =
        fvid::media::plan_mix_audio_with_controls(&wav_sources, &mix_opts, &tight_mem)
            .unwrap_err();
    assert!(
        plan_mix_err.contains("controlled memory budget exceeded"),
        "plan_mix_audio_with_controls with max_controlled_bytes=Some(1) must report 'controlled memory budget exceeded', got: {plan_mix_err}"
    );

    let out_mix = dir.join("mix-mem.wav");
    let mix_err =
        fvid::media::mix_audio_with_controls(&wav_sources, &out_mix, &mix_opts, &tight_mem)
            .unwrap_err();
    assert!(
        mix_err.contains("controlled memory budget exceeded"),
        "mix_audio_with_controls with max_controlled_bytes=Some(1) must report 'controlled memory budget exceeded', got: {mix_err}"
    );
    assert!(!out_mix.exists());

    let plan_merge_err =
        fvid::media::plan_merge_audio_with_controls(&wav_sources, &tight_mem).unwrap_err();
    assert!(
        plan_merge_err.contains("controlled memory budget exceeded"),
        "plan_merge_audio_with_controls with max_controlled_bytes=Some(1) must report 'controlled memory budget exceeded', got: {plan_merge_err}"
    );

    let out_merge = dir.join("merge-mem.wav");
    let merge_err =
        fvid::media::merge_audio_with_controls(&wav_sources, &out_merge, &tight_mem).unwrap_err();
    assert!(
        merge_err.contains("controlled memory budget exceeded"),
        "merge_audio_with_controls with max_controlled_bytes=Some(1) must report 'controlled memory budget exceeded', got: {merge_err}"
    );
    assert!(!out_merge.exists());

    // 2. Tight controlled memory budget on MP4 AAC inputs
    let mp4 = audio_fixture_dir().join("aac-native-edit.m4a");
    let mp4_sources = vec![mp4.clone(), mp4.clone()];
    let plan_mp4_mix_err =
        fvid::media::plan_mix_audio_with_controls(&mp4_sources, &mix_opts, &tight_mem)
            .unwrap_err();
    assert!(
        plan_mp4_mix_err.contains("controlled memory budget exceeded"),
        "plan_mix_audio_with_controls on MP4 with max_controlled_bytes=Some(1) must report 'controlled memory budget exceeded', got: {plan_mp4_mix_err}"
    );

    // 3. Tight packet byte budget + controlled memory budget on MP4 AAC inputs
    let mut tight_mp4_pkt = fvid::media::CopyOptions::default();
    tight_mp4_pkt.max_controlled_bytes = Some(64 * 1024 * 1024);
    tight_mp4_pkt.max_packet_bytes = 2;

    let plan_mp4_pkt_err =
        fvid::media::plan_mix_audio_with_controls(&mp4_sources, &mix_opts, &tight_mp4_pkt)
            .unwrap_err();
    assert!(
        plan_mp4_pkt_err.contains("MP4 packet exceeds budget"),
        "plan_mix_audio_with_controls on MP4 with max_packet_bytes=2 must report 'MP4 packet exceeds budget', got: {plan_mp4_pkt_err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn loudness_and_normalize_plans_enforce_packet_bytes_via_options() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-loudness-plan-pkt-parity-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Multi-stream MP4 with video + 2 audio tracks: stream 1 is mono AAC, stream 2 is stereo AAC
    let mp4_multi = audio_fixture_dir().join("two-audio.mp4");
    let target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: -24.0,
        sample_peak_dbfs: -1.0,
    };

    // 1. Multi-stream selection (streams = [1, 2]) must be rejected by default_weights_with_options
    // and measure_loudness_file_with_options with "owned loudness requires at most one audio stream"
    let mut multi_opts = fvid::media::CopyOptions::default();
    multi_opts.streams = vec![1, 2];

    let weights_err =
        fvid::native_pcm::loudness_channel_weights_with_options(&mp4_multi, &multi_opts)
            .unwrap_err();
    assert!(
        weights_err.to_string().contains("owned loudness requires at most one audio stream"),
        "loudness_channel_weights_with_options with streams=[1, 2] must reject multiple streams, got: {weights_err}"
    );

    let out_norm = dir.join("norm-multi.wav");
    let norm_multi_err =
        fvid::media::normalize_loudness(&mp4_multi, &out_norm, &multi_opts, None, target)
            .unwrap_err();
    assert!(
        norm_multi_err.contains("owned loudness requires at most one audio stream"),
        "normalize_loudness with streams=[1, 2] must reject multiple streams, got: {norm_multi_err}"
    );
    assert!(!out_norm.exists());

    // 2. When explicit weights [1.0, 1.0] do not match the selected mono stream (stream 1 has 1 channel),
    // loudness_with_options and plan_normalize_loudness with tight max_packet_bytes=2 must report
    // "MP4 packet exceeds budget" (from decode_audio_with_options) rather than a weight mismatch.
    let mut tight_mp4 = fvid::media::CopyOptions::default();
    tight_mp4.streams = vec![1];
    tight_mp4.max_packet_bytes = 2;

    let mp4_loudness_err =
        fvid::native_plan::loudness_with_options(&mp4_multi, &tight_mp4, Some(&[1.0, 1.0]))
            .unwrap_err();
    assert!(
        mp4_loudness_err.contains("MP4 packet exceeds budget"),
        "loudness_with_options with max_packet_bytes=2 must fail with 'MP4 packet exceeds budget' before weight check, got: {mp4_loudness_err}"
    );

    let mp4_norm_err = fvid::media::plan_normalize_loudness(
        &mp4_multi,
        &tight_mp4,
        Some(&[1.0, 1.0]),
        target,
    )
    .unwrap_err();
    assert!(
        mp4_norm_err.contains("MP4 packet exceeds budget"),
        "plan_normalize_loudness with max_packet_bytes=2 must fail with 'MP4 packet exceeds budget' before weight check, got: {mp4_norm_err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
