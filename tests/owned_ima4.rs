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
    let ramp: Vec<f32> = (1..=64).map(|n| n as f32 / 32768.0).collect();
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

#[test]
fn ima_wav_owned_mp4_export_preserves_stereo_block_order_and_edits() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ima-wav-stereo-edits.mov");
    let options = CopyOptions {
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..Default::default()
    };
    assert_eq!(
        fvid_media::plan_decode_audio(&source, &Default::default(), &options)
            .unwrap()
            .streams[0]
            .codec,
        "adpcm_ima_wav"
    );
    let ramp: Vec<f32> = (0..9)
        .flat_map(|n| [n as f32 / 32768.0, -128.0 / 32768.0])
        .collect();
    let expected: Vec<u8> = [0f32; 4]
        .iter()
        .chain(&ramp)
        .chain(&ramp[4..])
        .flat_map(|x| x.to_le_bytes())
        .collect();
    let encoded = std::fs::read(&source).unwrap();
    let mut actual = Vec::new();
    assert_eq!(
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&encoded),
            &mut actual,
            None,
            &options
        )
        .unwrap()
        .sample_frames,
        18
    );
    assert_eq!(actual, expected);
    let output =
        std::env::temp_dir().join(format!("fvid-owned-ima-wav-{}.wav", std::process::id()));
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

#[test]
fn ms_adpcm_stereo_export_preserves_header_order_and_repeated_edits() {
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ms-adpcm-stereo-edits.mov");
    let options = CopyOptions {
        max_controlled_bytes: Some(32 * 1024 * 1024),
        ..Default::default()
    };
    assert_eq!(
        fvid_media::plan_decode_audio(&source, &Default::default(), &options)
            .unwrap()
            .streams[0]
            .codec,
        "adpcm_ms"
    );
    let samples: Vec<f32> = [-16.0, 0.0, 16.0, 32.0, 48.0, 64.0]
        .into_iter()
        .flat_map(|x| [x / 32768.0, -128.0 / 32768.0])
        .collect();
    let expected: Vec<u8> = [0f32; 4]
        .iter()
        .chain(&samples)
        .chain(&samples[4..])
        .flat_map(|x| x.to_le_bytes())
        .collect();
    let bytes = std::fs::read(&source).unwrap();
    let mut actual = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(&bytes),
        &mut actual,
        None,
        &options,
    )
    .unwrap();
    assert_eq!((stats.sample_frames, stats.channels), (12, 2));
    assert_eq!(actual, expected);
    let output =
        std::env::temp_dir().join(format!("fvid-owned-ms-adpcm-{}.wav", std::process::id()));
    let _ = std::fs::remove_file(&output);
    fvid_media::decode_audio(&source, &output, &options).unwrap();
    let exported = std::fs::read(&output).unwrap();
    std::fs::remove_file(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&exported), None).unwrap();
    assert_eq!(
        &exported[info.data_offset as usize..info.data_offset as usize + info.data_bytes as usize],
        expected
    );
}
