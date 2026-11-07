#![cfg(feature = "player")]

use std::path::Path;
use std::time::Duration;

use fvid_control::CopyOptions;
use fvid::playback::audio::wav::{self, Wav};
use fvid_media::owned_audio_mix::{
    MixAudioOptions, mix_audio_with_controls, plan_mix_audio_with_controls,
};
use fvid_media::owned_subtitle::try_convert_with_controls;
use fvid_media_info::{SubtitleCodec, SubtitleConvertOptions};

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wav-adpcm-mp3-plan-and-subtitle-validation")
}

#[test]
fn wav_rejects_extensible_4bit_adpcm_with_16bit_storage_width() {
    let bytes = std::fs::read(fixture_dir().join("wav-extensible-g722-16bit-storage.wav"))
        .expect("read wav fixture");
    let err = Wav::parse(&bytes, &wav::Limits::default())
        .expect_err("extensible G.722 with 16-bit container storage must be rejected");
    assert!(
        err.to_string()
            .contains("extensible four-bit ADPCM requires four-bit storage and precision"),
        "unexpected error: {err}"
    );
}

#[test]
fn plan_mix_audio_bounds_bare_mp3_frame_estimate_by_max_packets() {
    let mp3 = fixture_dir().join("mp3-multi-frame-max-packets.mp3");
    let controls = CopyOptions {
        max_packets: Some(1),
        // 1 frame = 1152 samples * 4 bytes = 4608 bytes per input + 4608 out = 13824 bytes,
        // whereas all 50 frames would be 57600 * 4 * 3 = 691200 bytes of PCM and exceed 400 KiB.
        max_controlled_bytes: Some(400 * 1024),
        ..CopyOptions::default()
    };
    let plan = plan_mix_audio_with_controls(
        &[mp3.clone(), mp3.clone()],
        &MixAudioOptions::default(),
        &controls,
    )
    .expect("plan_mix_audio_with_controls should bound bare MP3 frames by max_packets");
    assert_eq!(plan.command, "mix-audio");

    let out = std::env::temp_dir().join(format!(
        "fvid-mp3-max-packets-mix-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);
    let stats = mix_audio_with_controls(
        &[mp3.clone(), mp3],
        &out,
        &MixAudioOptions::default(),
        &controls,
    )
    .expect("mix_audio_with_controls should succeed under the same budget");
    let _ = std::fs::remove_file(&out);
    assert_eq!(stats.sample_frames, 1152);
}

#[test]
fn matroska_subtitle_conversion_rejects_malformed_ass_layer_and_read_order() {
    let mkv = fixture_dir().join("mkv-ass-malformed-layer.mkv");
    let out = std::env::temp_dir().join(format!(
        "fvid-mkv-ass-malformed-layer-{}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);
    let options = SubtitleConvertOptions {
        codec: SubtitleCodec::Ass,
        streams: Vec::new(),
    };
    let err = try_convert_with_controls(&mkv, &out, &options, &CopyOptions::default())
        .expect_err("malformed Matroska ASS layer must be rejected");
    let _ = std::fs::remove_file(&out);
    assert!(
        err.to_string().contains("invalid Matroska ASS event"),
        "unexpected error: {err}"
    );
}

#[test]
fn webvtt_inline_header_cue_enforces_max_packets_and_max_controlled_bytes() {
    let vtt = fixture_dir().join("vtt-header-inline-cue.vtt");
    let out = std::env::temp_dir().join(format!(
        "fvid-vtt-inline-header-{}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out);
    let options = SubtitleConvertOptions {
        codec: SubtitleCodec::Ass,
        streams: Vec::new(),
    };

    let packet_limited = CopyOptions {
        max_packets: Some(1),
        ..CopyOptions::default()
    };
    let err = try_convert_with_controls(&vtt, &out, &options, &packet_limited)
        .expect_err("2-cue VTT with inline header cue must exceed max_packets=1");
    let _ = std::fs::remove_file(&out);
    assert!(
        err.to_string().contains("packet work limit exceeded"),
        "unexpected error: {err}"
    );

    // Tight memory limit that passes initial file_len*3 + HEADER + 16384 check,
    // but fails once the inline header cue's event_bytes + 80 are charged.
    let single_cue_dir = std::env::temp_dir().join(format!(
        "fvid-vtt-single-inline-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&single_cue_dir);
    std::fs::create_dir_all(&single_cue_dir).unwrap();
    let single_vtt = single_cue_dir.join("single.vtt");
    std::fs::write(
        &single_vtt,
        b"WEBVTT\n00:00:00.000 --> 00:00:01.000\nInline header cue\n",
    )
    .unwrap();
    // Initial preflight working bytes = 55 * 3 + 414 + 0 + 16384 = 16963.
    // After inline cue: 55 + 55 + 414 + 36 + 80 + 0 + 16384 = 17024.
    let mem_limited = CopyOptions {
        max_controlled_bytes: Some(17000),
        ..CopyOptions::default()
    };
    let mem_err = try_convert_with_controls(&single_vtt, &out, &options, &mem_limited)
        .expect_err("inline header cue must enforce max_controlled_bytes");
    let _ = std::fs::remove_file(&out);
    let _ = std::fs::remove_dir_all(&single_cue_dir);
    assert!(
        mem_err.to_string().contains("controlled memory budget exceeded"),
        "unexpected error: {mem_err}"
    );
}

#[test]
fn subtitle_clock_rejects_missing_colon_and_out_of_range_minutes_or_seconds() {
    let srt_bytes = std::fs::read_to_string(fixture_dir().join("srt-invalid-clock-fields.srt"))
        .expect("read srt fixture");
    let cues = fvid::subtitles::parse_srt(&srt_bytes);
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "Valid cue");
    assert_eq!(cues[0].start, Duration::from_secs(1));
    assert_eq!(cues[0].end, Duration::from_secs(2));
}
