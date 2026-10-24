use std::path::{Path, PathBuf};
use std::process::Command;

use fvid::media::{self, AudioDecodeTransform, CopyOptions, MixAudioOptions};

struct Directory(PathBuf);

impl Directory {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "fvid-media-audio-routing-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/media-audio-routing")
}

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_fvid")
}

#[test]
fn top_level_media_routes_g711_alaw_and_mulaw_wave_decode_plan_mix_and_merge() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k.wav");
    let mulaw = dir.join("g711-mulaw-48k.wav");
    let tmp = Directory::new("g711-wave");

    let transform = AudioDecodeTransform::default();
    let options = CopyOptions::default();

    let plan_alaw = media::plan_decode_audio(&alaw, &transform, &options)
        .expect("plan_decode_audio should route G.711 A-law WAVE to owned fvid_media plan");
    assert_eq!(plan_alaw.command, "decode-audio");
    assert_eq!(plan_alaw.streams.len(), 1);

    let out_alaw = tmp.0.join("alaw-decoded.wav");
    let report_alaw = media::decode_audio(&alaw, &out_alaw, &options)
        .expect("decode_audio should route G.711 A-law WAVE to owned fvid_media export");
    assert_eq!(report_alaw.sample_frames, 4_800);
    assert!(out_alaw.is_file());

    let out_mulaw = tmp.0.join("mulaw-decoded.wav");
    let report_mulaw = media::decode_audio(&mulaw, &out_mulaw, &options)
        .expect("decode_audio should route G.711 mu-law WAVE to owned fvid_media export");
    assert_eq!(report_mulaw.sample_frames, 4_800);
    assert!(out_mulaw.is_file());

    let sources = vec![alaw.clone(), mulaw.clone()];
    let mix_options = MixAudioOptions::default();
    let mix_plan = media::plan_mix_audio(&sources, &mix_options)
        .expect("plan_mix_audio should route G.711 WAVE inputs");
    assert_eq!(mix_plan.command, "mix-audio");

    let out_mix = tmp.0.join("mixed.wav");
    let mix_report = media::mix_audio(&sources, &out_mix, &mix_options)
        .expect("mix_audio should mix G.711 A-law and mu-law WAVE inputs");
    assert_eq!(mix_report.sample_frames, 4_800);
    assert!(out_mix.is_file());

    let merge_plan =
        media::plan_merge_audio(&sources).expect("plan_merge_audio should route G.711 WAVE inputs");
    assert_eq!(merge_plan.command, "merge-audio");

    let out_merge = tmp.0.join("merged.wav");
    let merge_report = media::merge_audio(&sources, &out_merge)
        .expect("merge_audio should merge G.711 A-law and mu-law WAVE inputs");
    assert_eq!(merge_report.sample_frames, 4_800);
    assert_eq!(merge_report.channels, 2);
}

#[test]
fn top_level_media_and_native_loudnorm_route_selected_secondary_flac_and_ima4_tracks() {
    let dir = fixture_dir();
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("multitrack-selected");

    let transform = AudioDecodeTransform::default();
    let selected = CopyOptions {
        streams: vec![1],
        ..CopyOptions::default()
    };

    let mka_plan = media::plan_decode_audio(&mka, &transform, &selected)
        .expect("plan_decode_audio should route secondary Matroska A_FLAC stream to fvid_media");
    assert_eq!(mka_plan.streams[0].index, 1);

    let mka_out = tmp.0.join("mka-track1.wav");
    let mka_report = media::decode_audio_transformed(&mka, &mka_out, transform, &selected)
        .expect("decode_audio_transformed should export secondary Matroska A_FLAC stream");
    assert_eq!(mka_report.sample_frames, 11 * 1_920);
    assert!(mka_out.is_file());

    let mov_plan = media::plan_decode_audio(&mov, &transform, &selected)
        .expect("plan_decode_audio should route secondary QuickTime ima4 stream to fvid_media");
    assert_eq!(mov_plan.streams[0].index, 1);

    let mov_out = tmp.0.join("mov-track1.wav");
    let mov_report = media::decode_audio_transformed(&mov, &mov_out, transform, &selected)
        .expect("decode_audio_transformed should export secondary QuickTime ima4 stream");
    assert_eq!(mov_report.sample_frames, 64);
    assert!(mov_out.is_file());

    let mka_ln_plan = fvid::native_loudnorm::try_plan(&mka, None, false, &selected)
        .expect("native_loudnorm::try_plan should not fail on secondary Matroska A_FLAC stream")
        .expect(
            "native_loudnorm::try_plan should return a plan for secondary Matroska A_FLAC stream",
        );
    assert_eq!(mka_ln_plan.streams[0].index, 1);

    let mka_ln_out = tmp.0.join("mka-track1-ln.wav");
    let mka_ln_report = fvid::native_loudnorm::try_apply(
        &mka,
        &mka_ln_out,
        None,
        false,
        &selected,
    )
    .expect("native_loudnorm::try_apply should not fail on secondary Matroska A_FLAC stream")
    .expect(
        "native_loudnorm::try_apply should return a report for secondary Matroska A_FLAC stream",
    );
    assert_eq!(mka_ln_report.sample_frames, 11 * 1_920 * 4);
    assert_eq!(mka_ln_report.sample_rate, 192_000);

    let mov_ln_plan = fvid::native_loudnorm::try_plan(&mov, None, false, &selected)
        .expect("native_loudnorm::try_plan should not fail on secondary QuickTime ima4 stream")
        .expect(
            "native_loudnorm::try_plan should return a plan for secondary QuickTime ima4 stream",
        );
    assert_eq!(mov_ln_plan.streams[0].index, 1);

    let mov_ln_out = tmp.0.join("mov-track1-ln.wav");
    let mov_ln_report = fvid::native_loudnorm::try_apply(
        &mov,
        &mov_ln_out,
        None,
        false,
        &selected,
    )
    .expect("native_loudnorm::try_apply should not fail on secondary QuickTime ima4 stream")
    .expect(
        "native_loudnorm::try_apply should return a report for secondary QuickTime ima4 stream",
    );
    assert_eq!(mov_ln_report.sample_frames, 64 * 4);
    assert_eq!(mov_ln_report.sample_rate, 192_000);
}

#[test]
fn media_cli_routes_owned_audio_formats_for_decode_mix_merge_and_loudnorm() {
    let dir = fixture_dir();
    let alaw = dir.join("g711-alaw-48k.wav");
    let ogg = dir.join("ogg-opus-mono-48k.ogg");
    let mka = dir.join("multitrack-pcm-flac.mka");
    let mov = dir.join("multitrack-pcm-ima4.mov");
    let tmp = Directory::new("cli-routing");

    let plan_ogg = Command::new(bin_path())
        .args([
            "media",
            "plan",
            "decode-audio",
            ogg.to_str().unwrap(),
        ])
        .output()
        .expect("run cli plan decode-audio ogg");
    assert!(
        plan_ogg.status.success(),
        "cli plan decode-audio ogg failed: {}",
        String::from_utf8_lossy(&plan_ogg.stderr)
    );

    let decoded_ogg = tmp.0.join("ogg.wav");
    let decode_ogg = Command::new(bin_path())
        .args([
            "media",
            "decode-audio",
            ogg.to_str().unwrap(),
            decoded_ogg.to_str().unwrap(),
        ])
        .output()
        .expect("run cli decode-audio ogg");
    assert!(
        decode_ogg.status.success(),
        "cli decode-audio ogg failed: {}",
        String::from_utf8_lossy(&decode_ogg.stderr)
    );
    assert!(decoded_ogg.is_file());

    let decoded_mka = tmp.0.join("mka1.wav");
    let decode_mka = Command::new(bin_path())
        .args([
            "media",
            "decode-audio",
            mka.to_str().unwrap(),
            decoded_mka.to_str().unwrap(),
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli decode-audio mka stream 1");
    assert!(
        decode_mka.status.success(),
        "cli decode-audio mka stream 1 failed: {}",
        String::from_utf8_lossy(&decode_mka.stderr)
    );
    assert!(decoded_mka.is_file());

    let mixed_wav = tmp.0.join("cli-mixed.wav");
    let mix_cli = Command::new(bin_path())
        .args([
            "media",
            "mix-audio",
            mixed_wav.to_str().unwrap(),
            alaw.to_str().unwrap(),
            ogg.to_str().unwrap(),
        ])
        .output()
        .expect("run cli mix-audio");
    assert!(
        mix_cli.status.success(),
        "cli mix-audio failed: {}",
        String::from_utf8_lossy(&mix_cli.stderr)
    );
    assert!(mixed_wav.is_file());

    let merged_wav = tmp.0.join("cli-merged.wav");
    let merge_cli = Command::new(bin_path())
        .args([
            "media",
            "merge-audio",
            merged_wav.to_str().unwrap(),
            alaw.to_str().unwrap(),
            ogg.to_str().unwrap(),
        ])
        .output()
        .expect("run cli merge-audio");
    assert!(
        merge_cli.status.success(),
        "cli merge-audio failed: {}",
        String::from_utf8_lossy(&merge_cli.stderr)
    );
    assert!(merged_wav.is_file());

    let ln_mov = tmp.0.join("cli-mov-ln.wav");
    let loudnorm_cli = Command::new(bin_path())
        .args([
            "media",
            "loudnorm",
            mov.to_str().unwrap(),
            ln_mov.to_str().unwrap(),
            "--streams",
            "1",
        ])
        .output()
        .expect("run cli loudnorm mov stream 1");
    assert!(
        loudnorm_cli.status.success(),
        "cli loudnorm mov stream 1 failed: {}",
        String::from_utf8_lossy(&loudnorm_cli.stderr)
    );
    assert!(ln_mov.is_file());
}
