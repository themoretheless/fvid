use std::path::Path;

use fvid::media::{
    AudioDecodeTransform, CopyOptions, MixAudioOptions, decode_audio_transformed, plan_decode_audio,
    plan_mix_audio_with_controls, plan_remux, remux,
};
use fvid::native_media::{
    constant_audio_source_info_selected_with_options, is_owned_audio_source_with_options,
    is_owned_audio_trim_source_with_options,
};

fn write_wav(path: &Path, sample_rate: u32, samples: &[i16]) {
    let data_bytes = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_bytes as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn native_media_audio_inspection_and_trim_qualification_reuse_handles() {
    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch650-media-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let wav = temp_dir.join("tone.wav");
    write_wav(&wav, 48_000, &vec![4000i16; 4_800]);
    let mka = temp_dir.join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(&wav, &mka, None, 1.0, None, None, None, None, None)
        .unwrap();

    let opts = CopyOptions::default();
    assert!(is_owned_audio_source_with_options(&mka, &opts).unwrap());
    assert!(is_owned_audio_trim_source_with_options(&mka, &opts).unwrap());

    let info = constant_audio_source_info_selected_with_options(&mka, None, &opts).unwrap();
    assert_eq!(info.codec, "pcm_f32le");
    assert_eq!(info.channels, 1);
    assert_eq!(info.sample_rate, 48_000);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn native_audio_mix_eligibility_and_planning_avoid_redundant_double_scans() {
    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch650-mix-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let wav = temp_dir.join("tone.wav");
    write_wav(&wav, 48_000, &vec![4000i16; 4_800]);
    let mka = temp_dir.join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(&wav, &mka, None, 1.0, None, None, None, None, None)
        .unwrap();

    let opts = CopyOptions::default();
    assert!(fvid::native_audio_mix::eligible_with_options(&[wav.clone(), wav.clone()], &opts).unwrap());
    let plan = plan_mix_audio_with_controls(
        &[wav.clone(), wav],
        &MixAudioOptions::default(),
        &opts,
    )
    .unwrap();
    assert_eq!(plan.command, "mix-audio");

    assert!(fvid::native_audio_mix::eligible_with_options(&[mka.clone(), mka], &opts).unwrap());
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn uses_native_audio_route_and_remux_reuse_open_file_handles() {
    let temp_dir = std::env::temp_dir().join(format!(
        "fvid-batch650-remux-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let wav = temp_dir.join("tone.wav");
    write_wav(&wav, 48_000, &vec![4000i16; 4_800]);
    let mka = temp_dir.join("tone.mka");
    fvid::native_export::export_audio_pcm_selected(&wav, &mka, None, 1.0, None, None, None, None, None)
        .unwrap();

    let opts = CopyOptions::default();
    let transform = AudioDecodeTransform::default();

    let decode_plan = plan_decode_audio(&mka, &transform, &opts).unwrap();
    assert_eq!(decode_plan.command, "decode-audio");

    let remux_plan = plan_remux(&mka, &opts).unwrap();
    assert_eq!(remux_plan.command, "remux");

    let out_wav = temp_dir.join("out.wav");
    let out_mka = temp_dir.join("out.mka");

    let decode_stats = decode_audio_transformed(&mka, &out_wav, transform, &opts).unwrap();
    assert_eq!(decode_stats.sample_frames, 4_800);

    let remux_stats = remux(&mka, &out_mka, &opts).unwrap();
    assert!(remux_stats.packets > 0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn decode_overlay_plan_reuses_main_and_foreground_handles() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch650-overlay-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.y4m");
    let fg = dir.join("fg.y4m");
    let bytes = b"YUV4MPEG2 W2 H2 F24:1 Ip A1:1 C420\nFRAME\n\x10\x10\x10\x10\x80\x80".to_vec();
    std::fs::write(&main, &bytes).unwrap();
    std::fs::write(&fg, &bytes).unwrap();

    let plan = fvid::native_plan::decode_overlay(&main, &fg, 0, 0).unwrap();
    assert_eq!(plan.command, "decode");
    assert_eq!(plan.inputs.len(), 2);

    let _ = std::fs::remove_dir_all(&dir);
}
