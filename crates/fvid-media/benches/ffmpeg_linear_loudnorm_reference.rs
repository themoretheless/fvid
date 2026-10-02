//! Explicit external comparison: never invoked by ordinary tests.
use fvid_media::{CopyOptions, apply_loudnorm, owned_wav_file::*};
use std::{process::Command, time::Instant};
fn main() {
    let dir = std::env::temp_dir().join(format!("fvid-linear-bench-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let ffmpeg = std::env::var_os("FVID_REFERENCE_FFMPEG").unwrap_or_else(|| "ffmpeg".into());
    for rate in [8000, 44100, 48000, 96000] {
        for bits in [8, 16, 24, 32, 64, 0] {
            let source = dir.join(format!("source-{rate}-{bits}.wav"));
            let dest = dir.join(format!("owned-{rate}-{bits}.wav"));
            let pcm: Vec<f64> = (0..rate * 2)
                .flat_map(|i| {
                    let value = 0.05 * (i as f64 * 0.17).sin();
                    [value, -value]
                })
                .collect();
            if bits == 0 {
                let values: Vec<f32> = pcm.iter().map(|&value| value as f32).collect();
                write_wav_f32le(&source, rate, 2, &values).unwrap();
            } else if bits == 64 {
                write_wav_f64le(&source, rate, 2, &pcm).unwrap();
            } else {
                let raw: Vec<u8> = pcm
                    .iter()
                    .flat_map(|&value| match bits {
                        8 => vec![(value * 128. + 128.).round_ties_even() as u8],
                        16 => ((value * 32768.).round_ties_even() as i16)
                            .to_le_bytes()
                            .to_vec(),
                        24 => ((value * 8388608.).round_ties_even() as i32).to_le_bytes()[..3]
                            .to_vec(),
                        _ => ((value * 2147483648.).round_ties_even() as i32)
                            .to_le_bytes()
                            .to_vec(),
                    })
                    .collect();
                write_wav_integer_le(&source, rate, 2, bits, &raw, 3).unwrap();
            }
            // Offset is intentionally nonzero: linear mode must ignore it.
            let args = "I=-16:TP=-1.5:LRA=11:measured_I=-22:measured_TP=-12:measured_LRA=2:measured_thresh=-32:linear=true:offset=12";
            let begin = Instant::now();
            let stats =
                apply_loudnorm(&source, &dest, Some(args), &CopyOptions::default()).unwrap();
            let owned_time = begin.elapsed();
            let begin = Instant::now();
            let reference = Command::new(&ffmpeg)
                .args(["-nostdin", "-v", "error", "-i"])
                .arg(&source)
                .args([
                    "-af",
                    &format!("loudnorm={args},aformat=sample_fmts=flt"),
                    "-f",
                    "f32le",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(
                reference.status.success(),
                "{}",
                String::from_utf8_lossy(&reference.stderr)
            );
            let reference_time = begin.elapsed();
            let wave = std::fs::read(&dest).unwrap();
            let mut file = std::fs::File::open(&dest).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            let owned = &wave[info.data_offset as usize
                ..(info.data_offset + u64::from(info.data_bytes)) as usize];
            assert_eq!(owned.len(), reference.stdout.len());
            let error = owned
                .chunks_exact(4)
                .zip(reference.stdout.chunks_exact(4))
                .map(|(a, b)| {
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(error <= 2e-8, "{rate}/{bits}: {error}");
            assert_eq!(stats.sample_rate, rate);
            println!(
                "{rate}/{bits}: owned={owned_time:?}, reference with process startup={reference_time:?}, max PCM error={error}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}
