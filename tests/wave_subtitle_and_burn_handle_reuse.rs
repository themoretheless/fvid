use fvid_media::{
    CopyOptions, SubtitleCodec, SubtitleConvertOptions, burn_subtitles,
    convert_subtitles_with_controls, plan_burn_subtitles, plan_convert_subtitles_with_controls,
    plan_trim, trim,
};
use std::fs;
use std::path::PathBuf;

#[test]
fn wave_remux_subtitle_convert_and_burn_reuse_handles() {
    let temp = std::env::temp_dir().join(format!(
        "fvid-wave-sub-burn-handle-reuse-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp);
    fs::create_dir_all(&temp).unwrap();

    // 1. WAVE trim & plan_trim with LIST INFO chunk (unbounded and bounded memory)
    let wav_in = temp.join("in.wav");
    let mut wav_bytes = Vec::new();
    wav_bytes.extend_from_slice(b"RIFF");
    wav_bytes.extend_from_slice(&0u32.to_le_bytes()); // placeholder
    wav_bytes.extend_from_slice(b"WAVE");
    wav_bytes.extend_from_slice(b"fmt ");
    wav_bytes.extend_from_slice(&16u32.to_le_bytes());
    wav_bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav_bytes.extend_from_slice(&1u16.to_le_bytes()); // 1 channel
    wav_bytes.extend_from_slice(&1000u32.to_le_bytes()); // 1000 Hz
    wav_bytes.extend_from_slice(&2000u32.to_le_bytes()); // byte rate
    wav_bytes.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav_bytes.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav_bytes.extend_from_slice(b"LIST");
    let info_payload = b"INFOINAM\x06\x00\x00\x00hello\x00";
    wav_bytes.extend_from_slice(&(info_payload.len() as u32).to_le_bytes());
    wav_bytes.extend_from_slice(info_payload);
    wav_bytes.extend_from_slice(b"data");
    let pcm = vec![0u8; 200]; // 100 frames = 100_000 us
    wav_bytes.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    wav_bytes.extend_from_slice(&pcm);
    let riff_len = (wav_bytes.len() - 8) as u32;
    wav_bytes[4..8].copy_from_slice(&riff_len.to_le_bytes());
    fs::write(&wav_in, &wav_bytes).unwrap();

    let wav_plan = plan_trim(&wav_in, 0, 50_000, &CopyOptions::default()).unwrap();
    assert_eq!(wav_plan.command, "trim");

    let wav_out = temp.join("trimmed.wav");
    let wav_stats = trim(&wav_in, &wav_out, 0, 50_000, &CopyOptions::default()).unwrap();
    assert_eq!(wav_stats.payload_bytes, 100);

    // 2. Subtitle conversion plan & convert across SRT, ASS, and MKV inputs
    let srt_in = temp.join("test.srt");
    fs::write(
        &srt_in,
        "1\n00:00:00,000 --> 00:00:00,500\nHello world\n",
    )
    .unwrap();
    let sub_opts = SubtitleConvertOptions {
        streams: Vec::new(),
        codec: SubtitleCodec::Ass,
    };
    let srt_mkv = temp.join("from-srt.mkv");
    let srt_plan =
        plan_convert_subtitles_with_controls(&srt_in, &sub_opts, &CopyOptions::default()).unwrap();
    assert_eq!(srt_plan.command, "convert-subtitles");
    let srt_stats = convert_subtitles_with_controls(
        &srt_in,
        &srt_mkv,
        &sub_opts,
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(srt_stats.cues, 1);

    // Convert from MKV subtitle source
    let mkv_mkv = temp.join("from-mkv.mkv");
    let mkv_plan =
        plan_convert_subtitles_with_controls(&srt_mkv, &sub_opts, &CopyOptions::default()).unwrap();
    assert_eq!(mkv_plan.command, "convert-subtitles");
    let mkv_stats = convert_subtitles_with_controls(
        &srt_mkv,
        &mkv_mkv,
        &sub_opts,
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(mkv_stats.cues, 1);

    // 3. Subtitle burn-in from Y4M and Matroska sources
    let burn_y4m = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn.y4m");
    let burn_srt = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitle-burn.srt");
    let burn_out = temp.join("burned.mkv");
    let burn_plan = plan_burn_subtitles(&burn_y4m, &burn_srt, &CopyOptions::default()).unwrap();
    assert_eq!(burn_plan.command, "burn-subtitles");
    let burn_stats =
        burn_subtitles(&burn_y4m, &burn_out, &burn_srt, &CopyOptions::default()).unwrap();
    assert_eq!(burn_stats.video_frames, 3);

    let _ = fs::remove_dir_all(&temp);
}
