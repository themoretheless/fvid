use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors/flac-layout")
}
#[test]
fn flac_comment_masks_survive_canonical_index_and_public_wave_export() {
    let root = root();
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-flac-layout-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        for ext in ["flac", "mkv"] {
            let source = root.join(format!("{name}.{ext}"));
            let out = dir.join(format!("{name}-{ext}.wav"));
            fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default())
                .unwrap();
            if ext == "flac" {
                let index = fvid_media::owned_flac::Flac::parse(
                    &std::fs::read(&source).unwrap(),
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(u64::from(index.channel_mask), c["mask"].as_u64().unwrap());
            }
            fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(
                &mut std::fs::File::open(&out).unwrap(),
                None,
            )
            .unwrap();
            let source_name = c["source"].as_str().unwrap();
            let native = root.parent().unwrap().join("flac-native");
            let raw = std::fs::read(native.join(format!("{source_name}.flac"))).unwrap();
            let word = u64::from_be_bytes(raw[18..26].try_into().unwrap());
            let bits = ((word >> 36) & 31) + 1;
            let integer = std::fs::read(native.join(format!("{source_name}.s32le"))).unwrap();
            let golden: Vec<u8> = integer
                .chunks_exact(4)
                .flat_map(|v| {
                    ((i32::from_le_bytes(v.try_into().unwrap()) as f64
                        / (1u64 << (bits - 1)) as f64) as f32)
                        .to_le_bytes()
                })
                .collect();
            let mut wave = std::fs::File::open(&out).unwrap();
            wave.seek(SeekFrom::Start(info.data_offset)).unwrap();
            let mut actual = vec![0; info.data_bytes as usize];
            wave.read_exact(&mut actual).unwrap();
            assert_eq!(actual, golden, "{name}/{ext}");
            assert_eq!(
                info.channel_mask,
                c["mask"].as_u64().unwrap() as u32,
                "{name}/{ext}"
            );
        }
    }
}
#[test]
fn authored_flac_layouts_downmix_to_exact_speaker_policy_oracles() {
    let root = root();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("fvid-flac-downmix-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let k = std::f32::consts::FRAC_1_SQRT_2;
    let table = [
        [1., 0.],
        [0., 1.],
        [k, k],
        [0., 0.],
        [k, 0.],
        [0., k],
        [k, 0.],
        [0., k],
        [k, k],
        [k, 0.],
        [0., k],
        [k, k],
        [k, 0.],
        [k, k],
        [0., k],
        [k, 0.],
        [k, k],
        [0., k],
    ];
    for c in manifest["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let mask = c["mask"].as_u64().unwrap() as u32;
        let native = root.parent().unwrap().join("flac-native");
        let stem = c["source"].as_str().unwrap();
        let raw = std::fs::read(native.join(format!("{stem}.flac"))).unwrap();
        let word = u64::from_be_bytes(raw[18..26].try_into().unwrap());
        let bits = ((word >> 36) & 31) + 1;
        let integer = std::fs::read(native.join(format!("{stem}.s32le"))).unwrap();
        let samples: Vec<f32> = integer
            .chunks_exact(4)
            .map(|v| {
                (i32::from_le_bytes(v.try_into().unwrap()) as f64 / (1u64 << (bits - 1)) as f64)
                    as f32
            })
            .collect();
        for output in [1, 2] {
            if output == channels {
                continue;
            }
            let mut expected = Vec::new();
            for frame in samples.chunks_exact(channels).skip(16).take(32) {
                // A zero mask names no roles. Preserve the committed audible
                // fallback independently of the production routing function.
                let mut stereo = if mask == 0 {
                    match channels {
                        1 => [frame[0]; 2],
                        2 => [frame[0], frame[1]],
                        _ => [frame.iter().sum::<f32>() / channels as f32; 2],
                    }
                } else {
                    [0f32; 2]
                };
                let mut bits = mask;
                for &value in frame {
                    if bits == 0 {
                        break;
                    }
                    let speaker = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let weights = if channels == 1 && speaker == 2 {
                        [1., 1.]
                    } else {
                        table[speaker]
                    };
                    stereo[0] += value * weights[0];
                    stereo[1] += value * weights[1];
                }
                if output == 1 {
                    expected.push((stereo[0] + stereo[1]) * 0.5);
                } else {
                    expected.extend(stereo);
                }
            }
            for ext in ["flac", "mkv"] {
                let source = root.join(format!("{name}.{ext}"));
                let destination = dir.join(format!("{name}-{ext}-{output}.wav"));
                let transform = fvid_media_info::AudioDecodeTransform {
                    channels: Some(output as i32),
                    interval: Some((2000, 6000)),
                    ..Default::default()
                };
                fvid_media::plan_decode_audio(&source, &transform, &Default::default()).unwrap();
                let stats = fvid_media::decode_audio_transformed(
                    &source,
                    &destination,
                    transform,
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(stats.sample_frames, 32);
                let mut file = std::fs::File::open(destination).unwrap();
                let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
                file.seek(SeekFrom::Start(info.data_offset)).unwrap();
                let mut bytes = vec![0; info.data_bytes as usize];
                file.read_exact(&mut bytes).unwrap();
                let actual: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|v| f32::from_le_bytes(v.try_into().unwrap()))
                    .collect();
                assert_eq!(actual.len(), expected.len());
                for (&actual, &wanted) in actual.iter().zip(&expected) {
                    assert!(
                        (actual - wanted).abs()
                            <= 16.
                                * f32::EPSILON
                                * samples.iter().copied().map(f32::abs).fold(0., f32::max)
                                * channels as f32,
                        "{name}/{ext}: {actual} vs {wanted}"
                    );
                }
            }
        }
    }
}
