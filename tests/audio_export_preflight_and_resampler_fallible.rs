use fvid_control::CopyOptions;
use fvid_media::AudioDecodeTransform;
use std::path::PathBuf;

fn unique_temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch593-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn decode_audio_transformed_rejects_existing_destination_before_opening_source() {
    let dir = unique_temp_dir("decode-audio-transformed-dest");
    let missing_source = dir.join("missing.mkv");
    let existing_dest = dir.join("already_exists.wav");
    std::fs::write(&existing_dest, b"sentinel").unwrap();

    let err = fvid_media::decode_audio_transformed(
        &missing_source,
        &existing_dest,
        AudioDecodeTransform::default(),
        &CopyOptions::default(),
    )
    .unwrap_err();
    assert!(
        err.contains("already exists"),
        "expected existing destination rejection before source recognition, got: {err}"
    );
    assert_eq!(std::fs::read(&existing_dest).unwrap(), b"sentinel");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn container_and_adts_loudnorm_reject_existing_destination_before_policy_check() {
    let dir = unique_temp_dir("loudnorm-dest-order");
    let missing_source = dir.join("missing.mka");
    let existing_dest = dir.join("already_exists.wav");
    std::fs::write(&existing_dest, b"sentinel").unwrap();

    let bad_stream_options = CopyOptions {
        streams: vec![1, 2],
        ..Default::default()
    };

    let err = fvid_media::owned_container_loudnorm::apply(
        &missing_source,
        &existing_dest,
        None,
        false,
        &bad_stream_options,
    )
    .unwrap_err();
    assert!(
        err.contains("already exists"),
        "expected container loudnorm to reject existing destination first, got: {err}"
    );

    let err = fvid_media::owned_adts_loudnorm::apply(
        &missing_source,
        &existing_dest,
        None,
        false,
        &bad_stream_options,
    )
    .unwrap_err();
    assert!(
        err.contains("already exists"),
        "expected ADTS loudnorm to reject existing destination first, got: {err}"
    );

    assert_eq!(std::fs::read(&existing_dest).unwrap(), b"sentinel");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn raw_flac_and_mp3_decode_and_plan_preserve_output() {
    let dir = unique_temp_dir("raw-flac-mp3");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let flac = root.join("tests/fixtures/playback-errors/audio-budget-controls/raw-flac-48k-440ms.flac");
    let mp3 = root.join("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");

    for (idx, source) in [flac, mp3].iter().enumerate() {
        let out = dir.join(format!("out-{idx}.wav"));
        let plan = fvid_media::plan_decode_audio(
            source,
            &AudioDecodeTransform::default(),
            &CopyOptions::default(),
        )
        .unwrap();
        assert_eq!(plan.command, "decode-audio");
        let stats = fvid_media::decode_audio(source, &out, &CopyOptions::default()).unwrap();
        assert!(stats.sample_frames > 0);
        assert!(out.exists());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn resampler_vec_output_preserves_samples_across_passthrough_and_sinc() {
    for (in_rate, out_rate) in [(48_000, 48_000), (44_100, 48_000)] {
        let mut r32 =
            fvid_media::owned_resample::Resampler::new(Vec::new(), in_rate, out_rate, 2).unwrap();
        for _ in 0..128 {
            use std::io::Write;
            r32.write_all(&0.25f32.to_le_bytes()).unwrap();
            r32.write_all(&(-0.5f32).to_le_bytes()).unwrap();
        }
        let frames32 = r32.finish().unwrap();
        let out32 = r32.take_output();
        assert_eq!(out32.len(), frames32 as usize * 8);

        let mut r64 =
            fvid_media::owned_resample_f64::Resampler::new(Vec::new(), in_rate, out_rate, 2)
                .unwrap();
        for _ in 0..128 {
            use std::io::Write;
            r64.write_all(&0.25f64.to_le_bytes()).unwrap();
            r64.write_all(&(-0.5f64).to_le_bytes()).unwrap();
        }
        let frames64 = r64.finish().unwrap();
        let out64 = r64.take_output();
        assert_eq!(out64.len(), frames64 as usize * 16);
    }
}
