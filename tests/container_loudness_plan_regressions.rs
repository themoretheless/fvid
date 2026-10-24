use fvid::media::{AudioDecodeTransform, CopyOptions};
use std::path::Path;

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/container-loudness-plan")
}

#[test]
fn matroska_flac_and_mpeg_loudness_and_loudnorm_succeed() {
    let dir = fixture_dir();
    for name in ["flac-loudness.mka", "mp1-loudness.mka"] {
        let source = dir.join(name);
        let options = CopyOptions::default();
        let plan = fvid_media::plan_loudness(&source, &options).expect("plan_loudness should succeed");
        assert_eq!(plan.command, "loudness");
        let stats = fvid_media::measure_loudness(&source, &options)
            .unwrap_or_else(|e| panic!("measure_loudness failed for {name}: {e}"));
        assert_eq!(stats.backend, "owned container audio loudness");
        assert_eq!(stats.sample_rate, 48_000);
        assert_eq!(stats.channels, 1);
        assert!(stats.integrated_lufs.is_finite(), "expected finite integrated LUFS for {name}");

        let out_single = std::env::temp_dir().join(format!(
            "fvid-batch200-loudnorm-{}-{}.wav",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_single);
        fvid_media::apply_loudnorm(&source, &out_single, None, &options)
            .unwrap_or_else(|e| panic!("apply_loudnorm failed for {name}: {e}"));
        let _ = std::fs::remove_file(&out_single);

        let out_dual = std::env::temp_dir().join(format!(
            "fvid-batch200-loudnorm-dual-{}-{}.wav",
            name,
            std::process::id()
        ));
        let _ = std::fs::remove_file(&out_dual);
        fvid_media::apply_loudnorm_dual(&source, &out_dual, None, &options)
            .unwrap_or_else(|e| panic!("apply_loudnorm_dual failed for {name}: {e}"));
        let _ = std::fs::remove_file(&out_dual);
    }
}

#[test]
fn matroska_opus_and_flac_and_pcm_layout_masks_govern_plan_and_export_admission() {
    let dir = fixture_dir();
    let options = CopyOptions::default();

    // 1. 4-channel family 255 discrete Opus has wave_channel_mask() == 0:
    //    - loudness is refused by plan_loudness()/measure_loudness()
    //    - rematrixing to stereo is refused by plan_decode_audio()/decode_audio_transformed()
    //    - identity 4-channel decode is accepted
    let opus_discrete = dir.join("opus-discrete-family255-4ch.mka");
    assert!(fvid_media::plan_loudness(&opus_discrete, &options).is_err());
    assert!(fvid_media::measure_loudness(&opus_discrete, &options).is_err());

    let to_stereo = AudioDecodeTransform {
        channels: Some(2),
        ..Default::default()
    };
    assert!(fvid_media::plan_decode_audio(&opus_discrete, &to_stereo, &options).is_err());
    let dummy_wav = std::env::temp_dir().join(format!("fvid-batch200-opus-disc-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&dummy_wav);
    assert!(fvid_media::decode_audio_transformed(&opus_discrete, &dummy_wav, to_stereo, &options).is_err());

    // Identity 4ch plan on discrete Opus succeeds:
    assert!(fvid_media::plan_decode_audio(
        &opus_discrete,
        &AudioDecodeTransform::default(),
        &options
    )
    .is_ok());

    // 2. 4-channel family 1 Vorbis-mapped Opus has Quad mask 0x33:
    //    - loudness plan and stereo fold plan/decode are supported!
    let opus_quad = dir.join("opus-quad-family1.mka");
    assert!(fvid_media::plan_loudness(&opus_quad, &options).is_ok());
    assert!(fvid_media::plan_decode_audio(&opus_quad, &to_stereo, &options).is_ok());
    fvid_media::decode_audio_transformed(&opus_quad, &dummy_wav, to_stereo, &options)
        .expect("4ch family-1 Opus to stereo decode should succeed");
    let _ = std::fs::remove_file(&dummy_wav);

    // 3. Matroska FLAC with WAVEFORMATEXTENSIBLE_CHANNEL_MASK=0x0 refuses rematrixing in plan and export
    let flac_unassigned = dir.join("flac-unassigned-mask.mka");
    assert!(fvid_media::plan_decode_audio(&flac_unassigned, &to_stereo, &options).is_err());
    assert!(fvid_media::decode_audio_transformed(&flac_unassigned, &dummy_wav, to_stereo, &options).is_err());

    // 4. Matroska 4-channel PCM has no channel mask element in Matroska (mask == 0 for >2 channels):
    //    - refuses loudness and refuses rematrixing in owned plan and export
    let pcm_discrete = dir.join("pcm-discrete-4ch.mka");
    assert!(fvid_media::plan_loudness(&pcm_discrete, &options).is_err());
    assert!(fvid_media::plan_decode_audio(&pcm_discrete, &to_stereo, &options).is_err());
    assert!(fvid_media::decode_audio_transformed(&pcm_discrete, &dummy_wav, to_stereo, &options).is_err());
}

#[test]
fn quicktime_sowt_omitted_bit_depth_probes_as_pcm_s16le() {
    let mov = fixture_dir().join("mov-sowt-zero-bits.mov");
    let info = fvid::media::probe(&mov).expect("MP4/MOV probe should succeed");
    assert_eq!(info.streams.len(), 1);
    assert_eq!(info.streams[0].codec, "pcm_s16le");
}

#[test]
fn native_pcm_loudness_negotiates_in_band_ps_for_mp4_matroska_and_adts() {
    let pb = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let mp4_ps = pb.join("he-aac-ps-inband-export-lc-1024-synthetic.mp4");
    let mka_ps = pb.join("he-aac-ps-output-clock-lc-1024-synthetic.mkv");
    let adts_ps = pb.join("adts-implicit-ps-111-synthetic.aac");

    for source in [&mp4_ps, &mka_ps, &adts_ps] {
        let weights = fvid::native_pcm::loudness_channel_weights(source, None)
            .unwrap_or_else(|e| panic!("loudness_channel_weights failed for {}: {e}", source.display()));
        assert_eq!(weights, vec![1.0, 1.0]);
        let report = fvid::native_pcm::measure_loudness_file(source, None, &weights, None)
            .unwrap_or_else(|e| panic!("measure_loudness_file failed for {}: {e}", source.display()));
        assert!(report.sample_frames > 0);
        let pub_report = fvid::media::measure_loudness(source, &CopyOptions::default())
            .unwrap_or_else(|e| panic!("fvid::media::measure_loudness failed for {}: {e}", source.display()));
        assert_eq!(pub_report.sample_frames, report.sample_frames);
    }
}
