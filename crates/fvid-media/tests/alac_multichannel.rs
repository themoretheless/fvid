use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
struct Cleanup(PathBuf);
impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn owned_alac_multi_element_pcm_intervals_and_wave_preserve_named_and_discrete_channels() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/alac-multichannel");
    let info: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let dir = Cleanup(std::env::temp_dir().join(format!("fvid-alac-multi-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for c in info["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let depth = c["depth"].as_u64().unwrap();
        let total = c["samples"].as_u64().unwrap_or(32) as usize;
        let mut expected = Vec::new();
        for n in 0..total {
            for ch in c["order"].as_array().unwrap() {
                let ch = ch.as_u64().unwrap() as usize;
                let v = ((ch + 1) * 101 + n * 7) as i32 * if ch % 2 == 0 { 1 } else { -1 };
                let v = (v + if c["jump_sample"]
                    .as_u64()
                    .is_some_and(|start| n as u64 >= start)
                {
                    c["jump_high"].as_i64().unwrap() as i32
                } else {
                    0
                }) * c["scale"].as_i64().unwrap_or(1) as i32
                    + if c["low_bias"].as_bool().unwrap_or(false) {
                        ch as i32 + 1
                    } else {
                        0
                    };
                expected.extend((v as f32 / (1u64 << (depth - 1)) as f32).to_le_bytes());
            }
        }
        let source = root.join(name);
        let bytes = std::fs::read(&source).unwrap();
        for interval in [
            None,
            Some((Duration::from_nanos(208334), Duration::from_micros(500))),
        ] {
            let wanted = if interval.is_some() {
                &expected[11 * channels * 4..24 * channels * 4]
            } else {
                &expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_alac::decode_matroska_alac_pcm(
                Cursor::new(&bytes),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(pcm, wanted, "{name}");
            assert_eq!(
                stats.sample_frames,
                wanted.len() as u64 / channels as u64 / 4
            );
        }
        for extension in ["mkv", "m4a"] {
            let source = source.with_extension(extension);
            let out = dir.0.join(format!("{name}-{extension}.wav"));
            fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
            let mut file = std::fs::File::open(out).unwrap();
            let wave = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
            assert_eq!(
                wave.channel_mask,
                c["mask"].as_u64().unwrap() as u32,
                "{name} layout"
            );
            assert_eq!(usize::from(wave.channels), channels);
            assert!(wave.float);
            assert_eq!(wave.bits_per_sample, 32);
            file.seek(SeekFrom::Start(wave.data_offset)).unwrap();
            let mut pcm = vec![0; wave.data_bytes as usize];
            file.read_exact(&mut pcm).unwrap();
            assert_eq!(pcm, expected, "{name} WAVE");
        }
    }
}

#[test]
fn discrete_alac_downmix_averages_all_unlabelled_channels_without_silencing_them() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/alac-multichannel");
    let dir = Cleanup(
        std::env::temp_dir().join(format!("fvid-alac-discrete-fold-{}", std::process::id())),
    );
    std::fs::create_dir(&dir.0).unwrap();
    for depth in [16, 32] {
        for extension in ["mkv", "m4a"] {
            for channels in [1, 2] {
                let source = root.join(format!("3-{depth}-discrete.{extension}"));
                let out = dir.0.join(format!("{depth}-{extension}-{channels}.wav"));
                let transform = fvid_media_info::AudioDecodeTransform {
                    channels: Some(channels),
                    ..Default::default()
                };
                fvid_media::decode_audio_transformed(&source, &out, transform, &Default::default())
                    .unwrap();
                let mut f = std::fs::File::open(out).unwrap();
                let wave = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
                assert_eq!(wave.channel_mask, if channels == 1 { 4 } else { 3 });
                f.seek(SeekFrom::Start(wave.data_offset)).unwrap();
                let mut actual = vec![0; wave.data_bytes as usize];
                f.read_exact(&mut actual).unwrap();
                let mut expected = Vec::new();
                for n in 0..32 {
                    let mut average = 0.;
                    for ch in 0..3 {
                        let v = ((ch + 1) * 101 + n * 7) as i32 * if ch % 2 == 0 { 1 } else { -1 };
                        average += (v as f32 / (1u64 << (depth - 1)) as f32) * (1. / 3.);
                    }
                    for _ in 0..channels {
                        expected.extend(average.to_le_bytes());
                    }
                }
                assert_eq!(actual, expected);
            }
        }
    }
}
