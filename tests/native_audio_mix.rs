use fvid::{media_info::MixAudioOptions, native_audio_mix};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
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
        "fvid-mix-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&p).unwrap();
    Dir(p)
}
fn wave(path: &Path, samples: &[f32], rate: u32, channels: u16) {
    let bytes = (samples.len() * 4) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 4).to_le_bytes());
    out.extend_from_slice(&(channels * 4).to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, out).unwrap();
}
fn labelled_wave(path: &Path, samples: &[f32], rate: u32, channels: u16) {
    wave(path, samples, rate, channels);
    if channels <= 2 {
        return;
    }
    let mut bytes = std::fs::read(path).unwrap();
    let mask: u32 = match channels {
        3 => 7,
        4 => 0x107,
        5 => 0x37,
        6 => 0x3f,
        _ => 0,
    };
    bytes[16..20].copy_from_slice(&40u32.to_le_bytes());
    bytes[20..22].copy_from_slice(&0xfffeu16.to_le_bytes());
    let mut extra = Vec::new();
    extra.extend_from_slice(&22u16.to_le_bytes());
    extra.extend_from_slice(&32u16.to_le_bytes());
    extra.extend_from_slice(&mask.to_le_bytes());
    extra.extend_from_slice(&[
        3, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71,
    ]);
    bytes.splice(36..36, extra);
    let len = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&len.to_le_bytes());
    std::fs::write(path, bytes).unwrap();
}
fn pcm(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        if &bytes[at..at + 4] == b"data" {
            return bytes[at + 8..at + 8 + size].to_vec();
        }
        at += 8 + size + (size & 1);
    }
    panic!("no data chunk");
}
fn fixtures(d: &Dir) -> (Vec<PathBuf>, Vec<Vec<f32>>) {
    let samples: Vec<Vec<f32>> = [9003, 9009, 5001]
        .into_iter()
        .enumerate()
        .map(|(i, n)| {
            (0..n * 2)
                .map(|x| ((x * 37 + i * 117) % 1024) as f32 / 128.0 - 4.0)
                .collect()
        })
        .collect();
    let paths = (0..3)
        .map(|i| d.0.join(format!("{i}.wav")))
        .collect::<Vec<_>>();
    for (p, s) in paths.iter().zip(&samples) {
        wave(p, s, 48000, 2);
    }
    (paths, samples)
}
#[test]
fn weights_shortest_block_boundaries_and_publication() {
    let d = dir();
    let (paths, samples) = fixtures(&d);
    for (case, normalize, weights) in [
        (0, true, vec![]),
        (1, true, vec![0.25, 0.75]),
        (2, false, vec![0.0, 2.0, 0.5]),
    ] {
        let options = MixAudioOptions {
            normalize,
            weights,
            ..Default::default()
        };
        let dest = d.0.join(format!("out-{case}.wav"));
        let stats = native_audio_mix::mix_audio(&paths, &dest, &options).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.sample_frames, 5001);
        assert_eq!(stats.channels, 2);
        let sum: f32 = stats.weights.iter().sum();
        let scales: Vec<_> = stats
            .weights
            .iter()
            .map(|&w| if normalize { w / sum } else { w })
            .collect();
        let expected: Vec<_> = (0..10002)
            .flat_map(|i| {
                let mut value = 0f32;
                for (p, &s) in samples.iter().zip(&scales) {
                    value = (f64::from(value) + f64::from(p[i]) * f64::from(s)) as f32;
                }
                value.to_le_bytes()
            })
            .collect();
        assert_eq!(pcm(&dest), expected);
        let before = std::fs::read(&dest).unwrap();
        assert!(native_audio_mix::mix_audio(&paths, &dest, &options).is_err());
        assert_eq!(std::fs::read(&dest).unwrap(), before);
    }
    for weights in [
        vec![0.0],
        vec![-1.0],
        vec![f32::NAN],
        vec![f32::INFINITY],
        vec![1.0; 4],
    ] {
        let options = MixAudioOptions {
            weights,
            ..Default::default()
        };
        assert!(native_audio_mix::plan(&paths, &options).is_err());
        assert!(native_audio_mix::mix_audio(&paths, &d.0.join("bad.wav"), &options).is_err());
    }
    wave(&paths[2], &samples[2], 44100, 2);
    assert!(
        native_audio_mix::mix_audio(&paths, &d.0.join("bad.wav"), &Default::default()).is_err()
    );
    wave(&paths[2], &[f32::NAN; 20], 48000, 2);
    assert!(
        native_audio_mix::mix_audio(&paths, &d.0.join("bad.wav"), &Default::default()).is_err()
    );
    assert!(!d.0.join("bad.wav").exists());
    assert!(!std::fs::read_dir(&d.0).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-")
    }));
}
#[test]
fn aac_wave_cli_plan_and_api_use_owned_pipeline() {
    let d = dir();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let decoded = d.0.join("decoded.wav");
    fvid::native_export::export_audio_pcm_selected(
        &source, &decoded, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    let sources = vec![source, decoded.clone()];
    let dest = d.0.join("mixed.wav");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "mix-audio"])
        .arg(&dest)
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    assert_eq!(pcm(&dest), pcm(&decoded));
    let plan = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "mix-audio"])
        .args(&sources)
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan["command"], "mix-audio");
    assert!(plan["graph"].is_null());
    #[cfg(feature = "media")]
    {
        let api = d.0.join("api.wav");
        let stats = fvid::media::mix_audio(&sources, &api, &Default::default()).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(pcm(&api), pcm(&decoded));
        assert!(
            fvid::media::plan_mix_audio(&sources, &Default::default())
                .unwrap()
                .graph
                .is_none()
        );
    }
}
#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn reference_amix_matches_float_pcm() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let d = dir();
    let (sources, _) = fixtures(&d);
    for normalize in [false, true] {
        let options = MixAudioOptions {
            normalize,
            weights: vec![0.25, 0.75],
            ..Default::default()
        };
        let dest = d.0.join(format!("{normalize}.wav"));
        native_audio_mix::mix_audio(&sources, &dest, &options).unwrap();
        let mut command = std::process::Command::new(&ffmpeg);
        command.args(["-v", "error"]);
        for source in &sources {
            command.arg("-i").arg(source);
        }
        let result=command.args(["-filter_complex",&format!("amix=inputs=3:duration=shortest:dropout_transition=0:normalize={}:weights=0.25 0.75",u8::from(normalize)),"-f","f32le","pipe:1"]).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(pcm(&dest), result.stdout, "normalize={normalize}");
    }
}

#[test]
fn unsupported_wave_geometry_keeps_legacy_admission() {
    let d = dir();
    let p = d.0.join("sixty-five.wav");
    wave(&p, &[0.0; 130], 48000, 65);
    assert!(!native_audio_mix::eligible(&[p.clone(), p]).unwrap());
}

#[test]
fn merge_channel_order_shortest_and_legacy_wav_contract() {
    let d = dir();
    for (left, right) in [(1u16, 1u16), (1, 2), (2, 1), (2, 2), (6, 6)] {
        let a = d.0.join(format!("a-{left}-{right}.wav"));
        let b = d.0.join(format!("b-{left}-{right}.wav"));
        let x: Vec<f32> = (0..9003 * usize::from(left))
            .map(|i| (i % 997) as f32 / 128.0 - 3.0)
            .collect();
        let y: Vec<f32> = (0..5001 * usize::from(right))
            .map(|i| (i % 457) as f32 / 256.0 + 0.125)
            .collect();
        labelled_wave(&a, &x, 48000, left);
        labelled_wave(&b, &y, 48000, right);
        let sources = vec![a, b];
        let destination = d.0.join(format!("out-{left}-{right}.wav"));
        let stats = native_audio_mix::merge_audio(&sources, &destination).unwrap();
        assert_eq!(stats.backend, "fvid");
        assert_eq!(stats.channels, i32::from(left + right));
        assert_eq!(stats.sample_frames, 5001);
        let mut expected = Vec::new();
        for i in 0..5001 {
            for &s in &x[i * usize::from(left)..(i + 1) * usize::from(left)] {
                expected.extend_from_slice(&s.to_le_bytes());
            }
            for &s in &y[i * usize::from(right)..(i + 1) * usize::from(right)] {
                expected.extend_from_slice(&s.to_le_bytes());
            }
        }
        assert_eq!(pcm(&destination), expected);
        let before = std::fs::read(&destination).unwrap();
        assert_eq!(&before[20..22], &3u16.to_le_bytes());
        assert_eq!(before.len(), 44 + expected.len());
        assert!(native_audio_mix::merge_audio(&sources, &destination).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), before);
        #[cfg(feature = "media")]
        {
            let legacy = d.0.join(format!("legacy-{left}-{right}.wav"));
            fvid_media::merge_audio(&sources, &legacy).unwrap();
            assert_eq!(std::fs::read(&legacy).unwrap(), before);
            let api = d.0.join(format!("api-{left}-{right}.wav"));
            let stats = fvid::media::merge_audio(&sources, &api).unwrap();
            assert_eq!(stats.backend, "fvid");
            assert_eq!(std::fs::read(api).unwrap(), before);
            assert!(
                fvid::media::plan_merge_audio(&sources)
                    .unwrap()
                    .graph
                    .is_none()
            );
        }
    }
}

#[test]
fn merge_aac_cli_and_failure_cleanup() {
    let d = dir();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio/aac-stereo.aac");
    let baseline = d.0.join("baseline.wav");
    let decoded = fvid::native_export::export_audio_pcm_selected(
        &source, &baseline, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    let bytes = pcm(&baseline);
    let stride = usize::from(decoded.channels) * 4;
    let expected: Vec<u8> = bytes
        .chunks_exact(stride)
        .flat_map(|frame| frame.iter().chain(frame.iter()).copied())
        .collect();
    let destination = d.0.join("merged.wav");
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "merge-audio"])
        .arg(&destination)
        .arg(&source)
        .arg(&baseline)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(pcm(&destination), expected);
    let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(stats["backend"], "fvid");
    let plan = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
        .args(["media", "plan", "merge-audio"])
        .arg(&source)
        .arg(&baseline)
        .output()
        .unwrap();
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(plan["command"], "merge-audio");
    assert!(plan["graph"].is_null());
    let invalid = d.0.join("invalid.wav");
    wave(&invalid, &[0.0; 20], 8000, 1);
    assert!(native_audio_mix::plan_merge(&[baseline.clone(), invalid.clone()]).is_err());
    wave(&invalid, &[f32::NAN; 20], decoded.sample_rate, 1);
    let bad = d.0.join("bad.wav");
    assert!(native_audio_mix::merge_audio(&[baseline.clone(), invalid], &bad).is_err());
    assert!(!bad.exists());
    assert!(native_audio_mix::merge_audio(&[baseline], &bad).is_err());
    assert!(!std::fs::read_dir(&d.0).unwrap().any(|p| {
        p.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".fvid-")
    }));
}

#[test]
#[ignore = "requires FVID_REFERENCE_FFMPEG"]
fn reference_amerge_overlapping_layouts_matches_channel_concatenation() {
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap();
    let d = dir();
    let (sources, _) = fixtures(&d);
    let sources = &sources[..2];
    let destination = d.0.join("merged.wav");
    native_audio_mix::merge_audio(sources, &destination).unwrap();
    let result = std::process::Command::new(ffmpeg)
        .args(["-v", "error", "-i"])
        .arg(&sources[0])
        .arg("-i")
        .arg(&sources[1])
        .args([
            "-filter_complex",
            "amerge=inputs=2",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(pcm(&destination), result.stdout);
}

#[test]
fn merge_preserves_aac_container_edits_and_rejects_ambiguous_selection() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/audio");
    let d = dir();
    for name in [
        "aac-stereo.aac",
        "aac-stereo.mka",
        "aac-native-edit.m4a",
        "aac-960-48000.m4a",
    ] {
        let source = root.join(name);
        let baseline = d.0.join(format!("{name}.wav"));
        let decoded = fvid::native_export::export_audio_pcm_selected(
            &source, &baseline, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        let bytes = pcm(&baseline);
        let stride = usize::from(decoded.channels) * 4;
        let expected: Vec<u8> = bytes
            .chunks_exact(stride)
            .flat_map(|frame| frame.iter().chain(frame.iter()).copied())
            .collect();
        let output = d.0.join(format!("{name}-merged.wav"));
        let sources = vec![source, baseline];
        assert!(native_audio_mix::eligible(&sources).unwrap());
        let stats = native_audio_mix::merge_audio(&sources, &output).unwrap();
        assert_eq!(stats.sample_frames, decoded.sample_frames);
        assert_eq!(pcm(&output), expected, "{name}");
    }
    for name in ["two-audio.mp4", "two-audio.mka"] {
        let p = root.join(name);
        assert!(!native_audio_mix::eligible(&[p.clone(), p]).unwrap());
    }
}

#[test]
fn merged_unlabelled_channels_roundtrip_and_mix_without_layout_inference() {
    let d = dir();
    let channels = 32u16;
    let frames = 4193;
    let data: Vec<f32> = (0..frames * usize::from(channels))
        .map(|i| (i % 127) as f32 / 64.0 - 0.75)
        .collect();
    let a = d.0.join("a.wav");
    wave(&a, &data, 48000, channels);
    let sources = vec![a.clone(), a];
    assert!(native_audio_mix::eligible(&sources).unwrap());
    let merged = d.0.join("merged.wav");
    let stats = native_audio_mix::merge_audio(&sources, &merged).unwrap();
    assert_eq!(stats.channels, 64);
    let decoded = d.0.join("decoded.f32le");
    fvid::native_export::export_audio_pcm_selected(
        &merged, &decoded, None, 1.0, None, None, None, None, None,
    )
    .unwrap();
    assert_eq!(std::fs::read(decoded).unwrap(), pcm(&merged));
    let mixed = d.0.join("mixed.wav");
    let sources = vec![merged.clone(), merged.clone()];
    let stats = native_audio_mix::mix_audio(&sources, &mixed, &Default::default()).unwrap();
    assert_eq!(stats.channels, 64);
    assert_eq!(pcm(&mixed), pcm(&merged));
    let info = fvid::native_pcm::inspect(&mut std::fs::File::open(&mixed).unwrap(), None).unwrap();
    assert_eq!(info.channel_mask, 0);
}

#[test]
fn mix_and_merge_matroska_match_wave_samples_and_channel_order() {
    let d = dir();
    let a = d.0.join("a.wav");
    let b = d.0.join("b.wav");
    let first: Vec<f32> = (0..9003)
        .flat_map(|i| [i as f32 / 20000.0, -(i as f32) / 30000.0])
        .collect();
    let second: Vec<f32> = (0..9011)
        .flat_map(|i| [-(i as f32) / 25000.0, i as f32 / 40000.0])
        .collect();
    wave(&a, &first, 44100, 2);
    wave(&b, &second, 44100, 2);
    let sources = vec![a, b];
    for merge in [false, true] {
        let name = if merge { "merge" } else { "mix" };
        let wav = d.0.join(format!("{name}.wav"));
        let mka = d.0.join(format!("{name}.mka"));
        let raw = d.0.join(format!("{name}.f32le"));
        if merge {
            native_audio_mix::merge_audio(&sources, &wav).unwrap();
            native_audio_mix::merge_audio(&sources, &mka).unwrap();
        } else {
            native_audio_mix::mix_audio(&sources, &wav, &MixAudioOptions::default()).unwrap();
            native_audio_mix::mix_audio(&sources, &mka, &MixAudioOptions::default()).unwrap();
        }
        let stats = fvid::native_export::export_audio_pcm_selected(
            &mka, &raw, None, 1.0, None, None, None, None, None,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 9003);
        assert_eq!(stats.sample_rate, 44100);
        assert_eq!(stats.channels, if merge { 4 } else { 2 });
        assert_eq!(pcm(&wav), std::fs::read(raw).unwrap());
        if merge {
            let unknown=d.0.join("unknown-layout.wav");
            fvid::native_export::export_audio_pcm_selected(&mka,&unknown,None,1.0,None,None,None,None,None).unwrap();
            let info=fvid::native_pcm::inspect(&mut std::fs::File::open(&unknown).unwrap(),None).unwrap();
            assert_eq!(info.channels,4);assert_eq!(info.channel_mask,0);assert_eq!(pcm(&unknown),pcm(&wav));
            let remixed=d.0.join("unsafe-remix.wav");
            assert!(fvid::native_export::export_audio_pcm_selected(&mka,&remixed,None,1.0,Some(2),None,None,None,None).is_err());assert!(!remixed.exists());
        }

        let bytes = std::fs::read(&mka).unwrap();
        let cli = d.0.join(format!("cli-{name}.mka"));
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_fvid"))
            .args(["media", if merge { "merge-audio" } else { "mix-audio" }])
            .arg(&cli)
            .args(&sources)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let stats: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(stats["backend"], "fvid");
        assert_eq!(std::fs::read(cli).unwrap(), bytes);
        if merge {
            assert!(native_audio_mix::merge_audio(&sources, &mka).is_err());
        } else {
            assert!(
                native_audio_mix::mix_audio(&sources, &mka, &MixAudioOptions::default()).is_err()
            );
        }
        assert_eq!(std::fs::read(&mka).unwrap(), bytes);
        #[cfg(feature = "media")]
        {
            let public = d.0.join(format!("public-{name}.mkv"));
            if merge {
                assert_eq!(
                    fvid::media::merge_audio(&sources, &public).unwrap().backend,
                    "fvid"
                );
            } else {
                assert_eq!(
                    fvid::media::mix_audio(&sources, &public, &MixAudioOptions::default())
                        .unwrap()
                        .backend,
                    "fvid"
                );
            }
            assert_eq!(std::fs::read(public).unwrap(), bytes);
        }
    }
    assert_eq!(
        std::fs::read_dir(&d.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".fvid"))
            .count(),
        0
    );
}
