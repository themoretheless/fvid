//! Explicit synthetic PCM amix and amerge reference comparisons.
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


fn main() {
    std::env::var_os("FVID_REFERENCE_FFMPEG").expect("Set FVID_REFERENCE_FFMPEG for this explicit reference benchmark");
    reference_amix_matches_float_pcm();
    reference_amerge_overlapping_layouts_matches_channel_concatenation();
    println!("PCM amix and amerge reference comparisons passed");
}
