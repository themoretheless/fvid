use fvid::native_export::export_audio_pcm_selected as export;
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn dir() -> Dir {
    let p = std::env::temp_dir().join(format!(
        "fvid-alac-media-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/alac")
        .join(name)
}
#[test]
fn cli_plan_intervals_gain_selection_and_api_use_owned_alac() {
    let d = dir();
    let source = fixture("stereo-24.m4a");
    assert!(fvid::native_media::is_alac_source(&source).unwrap());
    let full = d.0.join("full.f32le");
    let stats = export(&source, &full, None, 1.0, None, None, None, None, None).unwrap();
    let bytes = std::fs::read(&full).unwrap();
    assert_eq!(
        bytes.len() as u64,
        stats.sample_frames * u64::from(stats.channels) * 4
    );
    let dest = d.0.join("window.f32le");
    let from = Duration::from_millis(10);
    let to = Duration::from_millis(50);
    let window = export(
        &source,
        &dest,
        Some((from, to)),
        0.5,
        Some(1),
        None,
        Some(0),
        None,
        None,
    )
    .unwrap();
    let start = (from.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let end = (to.as_nanos() * u128::from(stats.sample_rate)).div_ceil(1_000_000_000) as usize;
    let samples: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
        .collect();
    let expected: Vec<u8> = (start..end)
        .flat_map(|i| ((samples[2 * i] + samples[2 * i + 1]) * 0.5 * 0.5).to_le_bytes())
        .collect();
    assert_eq!(window.sample_frames, (end - start) as u64);
    assert_eq!(std::fs::read(&dest).unwrap(), expected);
    let cli = d.0.join("cli.f32le");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "decode-audio"])
        .arg(&source)
        .arg(&cli)
        .args([
            "--from",
            "0.01",
            "--to",
            "0.05",
            "--channels",
            "1",
            "--volume",
            "0.5",
            "--streams",
            "0",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(std::fs::read(&cli).unwrap(), expected);
    let plan = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "decode-audio"])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan["streams"][0]["codec"], "alac");
    assert!(plan["graph"].is_null());
    #[cfg(feature = "media")]
    {
        let output = d.0.join("api.f32le");
        let transform = fvid::media::AudioDecodeTransform {
            interval: Some((10000, 50000)),
            volume: Some(0.5),
            channels: Some(1),
            ..Default::default()
        };
        fvid::media::decode_audio_transformed(&source, &output, transform, &Default::default())
            .unwrap();
        assert_eq!(std::fs::read(output).unwrap(), expected);
    }
    let invalid = d.0.join("invalid.f32le");
    assert!(
        export(
            &source,
            &invalid,
            None,
            1.0,
            None,
            None,
            Some(1),
            None,
            None
        )
        .is_err()
    );
    assert!(!invalid.exists());
    assert!(fvid::native_export::export_aac_pcm(&source, &invalid).is_err());
    assert!(!invalid.exists());
    assert!(export(&source, &full, None, 1.0, None, None, None, None, None).is_err());
    assert_eq!(std::fs::read(full).unwrap(), bytes);
    let cancel = fvid::media_control::CancelFlag::default();
    cancel.cancel();
    assert!(
        export(
            &source,
            &invalid,
            None,
            1.0,
            None,
            None,
            None,
            Some(&cancel),
            None
        )
        .is_err()
    );
    assert!(!invalid.exists());
}
#[test]
fn owned_mix_and_merge_accept_alac_inputs() {
    let d = dir();
    let source = fixture("mono-16.m4a");
    let pcm = d.0.join("baseline.f32le");
    let original = export(&source, &pcm, None, 1.0, None, None, None, None, None).unwrap();
    let bytes = std::fs::read(&pcm).unwrap();
    let sources = vec![source.clone(), source];
    assert!(fvid::native_audio_mix::eligible(&sources).unwrap());
    let mixed = d.0.join("mix.wav");
    let stats = fvid::native_audio_mix::mix_audio(&sources, &mixed, &Default::default()).unwrap();
    assert_eq!(stats.backend, "fvid");
    assert_eq!(stats.sample_frames, original.sample_frames);
    let decoded = d.0.join("mix.f32le");
    export(&mixed, &decoded, None, 1.0, None, None, None, None, None).unwrap();
    assert_eq!(std::fs::read(decoded).unwrap(), bytes);
    let merged = d.0.join("merge.wav");
    fvid::native_audio_mix::merge_audio(&sources, &merged).unwrap();
    let decoded = d.0.join("merge.f32le");
    export(&merged, &decoded, None, 1.0, None, None, None, None, None).unwrap();
    let expected: Vec<u8> = bytes
        .chunks_exact(4)
        .flat_map(|sample| sample.iter().chain(sample).copied())
        .collect();
    assert_eq!(std::fs::read(decoded).unwrap(), expected);
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn reference_decoder_matches_all_alac_mp4_fixtures() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let d = dir();
    for name in [
        "mono-16.m4a",
        "mono-24.m4a",
        "stereo-16.m4a",
        "stereo-24.m4a",
        "stereo-pair-24.m4a",
        "silence-16.m4a",
        "noise-24.m4a",
    ] {
        let source = fixture(name);
        let output = d.0.join(format!("{name}.f32le"));
        export(&source, &output, None, 1.0, None, None, None, None, None).unwrap();
        let result = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(&source)
            .args(["-f", "f32le", "pipe:1"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(std::fs::read(output).unwrap(), result.stdout, "{name}");
    }
}

#[test]
fn damaged_packet_and_cookie_fail_without_publishing() {
    let d = dir();
    let bytes = std::fs::read(fixture("stereo-24.m4a")).unwrap();
    let reader =
        fvid::container::mp4::Mp4Reader::open(std::io::Cursor::new(&bytes), Default::default())
            .unwrap();
    let sample = &reader.tracks()[0].samples.get(1).unwrap();
    let mut corrupt = bytes.clone();
    let at = sample.offset as usize;
    corrupt[at..at + sample.size as usize].fill(0xff);
    let input = d.0.join("bad.m4a");
    std::fs::write(&input, corrupt).unwrap();
    assert!(fvid::native_media::is_owned_audio_source(&input).unwrap());
    #[cfg(feature = "media")]
    {
        let output = d.0.join("api-bad.f32le");
        assert!(fvid::media::decode_audio(&input, &output, &Default::default()).is_err());
        assert!(!output.exists());
    }
    let output = d.0.join("bad.f32le");
    assert!(export(&input, &output, None, 1.0, None, None, None, None, None).is_err());
    assert!(!output.exists());
    let cookie = &reader.tracks()[0].configuration;
    let at = bytes
        .windows(cookie.len())
        .position(|p| p == cookie)
        .unwrap();
    let mut corrupt = bytes.clone();
    corrupt[at + 20..at + 24].copy_from_slice(&1u32.to_be_bytes());
    std::fs::write(&input, corrupt).unwrap();
    assert!(!fvid::native_media::is_owned_audio_source(&input).unwrap());
    assert!(export(&input, &output, None, 1.0, None, None, None, None, None).is_err());
    assert!(!output.exists());
    assert!(!std::fs::read_dir(&d.0).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-")
    }));
}
