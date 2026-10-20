use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};
fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors")
        .join(name)
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn pcm(path: &Path) -> Vec<u8> {
    let mut f = std::fs::File::open(path).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut f, None).unwrap();
    assert!(info.float);
    f.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut out = vec![0; info.data_bytes as usize];
    f.read_exact(&mut out).unwrap();
    out
}
#[test]
fn native_g711_wave_export_preserves_samples_and_bounds_input_work() {
    let dir =
        Directory(std::env::temp_dir().join(format!("fvid-g711-export-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    for law in ["alaw", "mulaw"] {
        for channels in [1, 2] {
            let name = format!("g711-{law}-{channels}ch");
            let source = fixture(&format!("{name}.wav"));
            let expected: Vec<u8> = std::fs::read(fixture(&format!("{name}.s16le")))
                .unwrap()
                .chunks_exact(2)
                .flat_map(|v| {
                    (i16::from_le_bytes(v.try_into().unwrap()) as f32 / 32768.).to_le_bytes()
                })
                .collect();
            let out = dir.0.join(format!("{name}.wav"));
            let stats = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
            assert_eq!(stats.sample_frames, 1024);
            assert_eq!(stats.sample_rate, 8000);
            assert_eq!(pcm(&out), expected);
            assert!(fvid_media::decode_audio(&source, &out, &Default::default()).is_err());
            assert_eq!(pcm(&out), expected);
            let transform = fvid_media_info::AudioDecodeTransform {
                interval: Some((2375, 99375)),
                sample_rate: Some(11025),
                channels: Some(1),
                volume: Some(0.5),
            };
            let transformed = dir.0.join(format!("{name}-transformed.wav"));
            let reference = dir.0.join(format!("{name}-reference.wav"));
            let actual_stats = fvid_media::decode_audio_transformed(
                &source,
                &transformed,
                transform,
                &Default::default(),
            )
            .unwrap();
            let reference_stats = fvid_media::decode_audio_transformed(
                &out,
                &reference,
                transform,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(actual_stats.sample_frames, reference_stats.sample_frames);
            assert_eq!(pcm(&transformed), pcm(&reference));
            let cancelled = dir.0.join(format!("{name}-cancelled.wav"));
            let flag = fvid_control::CancelFlag::new();
            let hook_flag = flag.clone();
            let err = fvid_media::decode_audio(
                &source,
                &cancelled,
                &fvid_control::CopyOptions {
                    cancel: Some(flag),
                    progress: Some(fvid_control::ProgressHook::new(move |event| {
                        assert!(!event.done);
                        hook_flag.cancel();
                    })),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(err.to_lowercase().contains("cancel"), "{err}");
            assert!(!cancelled.exists());
            let out = dir.0.join(format!("{name}-prefix.wav"));
            let stats = fvid_media::decode_audio(
                &source,
                &out,
                &fvid_control::CopyOptions {
                    max_packet_bytes: 128 * channels,
                    max_packets: Some(2),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(stats.sample_frames, 256);
            assert_eq!(pcm(&out), expected[..256 * channels * 4]);
            let out = dir.0.join(format!("{name}-budget.wav"));
            let err = fvid_media::decode_audio(
                &source,
                &out,
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(1024),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(err.contains("controlled memory budget exceeded"), "{err}");
            assert!(!out.exists());
        }
    }
}
