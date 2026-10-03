use fvid_control::CopyOptions;
use std::io::Cursor;
#[test]
fn ima4_owned_mp4_export_preserves_ramp_silence_repeat_and_budget() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ima4-ramp-edits.mov");
    let options = CopyOptions {
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..Default::default()
    };
    let plan = fvid_media::plan_decode_audio(&source, &Default::default(), &options).unwrap();
    assert_eq!(plan.streams[0].codec, "adpcm_ima_qt");
    let ramp: Vec<f32> = (1..=64).map(|n| (n * 2) as f32 / 32768.0).collect();
    let expected: Vec<u8> = [0f32, 0.0]
        .iter()
        .chain(&ramp)
        .chain(&ramp[2..])
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let encoded = std::fs::read(&source).unwrap();
    let mut actual = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&encoded),
        &mut actual,
        None,
        &options,
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 128);
    assert_eq!(actual, expected);
    let mut rejected = Vec::new();
    assert!(
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&encoded),
            &mut rejected,
            None,
            &CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            }
        )
        .unwrap_err()
        .to_string()
        .contains("controlled memory budget exceeded")
    );
    assert!(rejected.is_empty());
    let output = std::env::temp_dir().join(format!("fvid-owned-ima4-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&output);
    fvid_media::decode_audio(&source, &output, &options).unwrap();
    let bytes = std::fs::read(&output).unwrap();
    std::fs::remove_file(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
    assert_eq!(
        &bytes[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
        expected
    );
}
