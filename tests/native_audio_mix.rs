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
    let p = d.0.join("eight.wav");
    wave(&p, &[0.0; 80], 48000, 8);
    assert!(!native_audio_mix::eligible(&[p.clone(), p]).unwrap());
}
