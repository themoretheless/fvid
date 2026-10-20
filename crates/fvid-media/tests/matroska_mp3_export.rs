use fvid_control::CopyOptions;
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn assert_pcm(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (i, (a, b)) in actual
        .chunks_exact(4)
        .zip(expected.chunks_exact(4))
        .enumerate()
    {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "sample {i}: {a} versus {b}");
    }
}
#[test]
fn owned_matroska_mp3_all_rates_gapless_free_and_primed_history_match_pcm() {
    let dir = root().join("mp3-gapless");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let options = CopyOptions {
        max_controlled_bytes: Some(32 << 20),
        ..Default::default()
    };
    let mut checked = 0;
    for case in manifest["cases"].as_array().unwrap() {
        if case["no_video"].as_bool() == Some(true) {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let channels = case["channels"].as_u64().unwrap() as usize;
        let stride = channels * 4;
        let source = std::fs::read(dir.join(format!("{name}.mkv"))).unwrap();
        let golden = std::fs::read(
            root()
                .join("mp3-decoder")
                .join(format!("{}.f32le", case["source"].as_str().unwrap())),
        )
        .unwrap();
        let start = case["start"].as_u64().unwrap() as usize;
        let end = case["end"].as_u64().unwrap() as usize;
        let expected = &golden[start * stride..end * stride];
        for interval in [
            None,
            Some((Duration::from_millis(40), Duration::from_millis(60))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_mp3::decode_matroska_mp3_pcm(
                Cursor::new(&source),
                &mut pcm,
                interval,
                &options,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let wanted = if interval.is_some() {
                let a = (rate * 40).div_ceil(1000) as usize;
                let b = (rate * 60).div_ceil(1000) as usize;
                &expected[a * stride..b.min(end - start) * stride]
            } else {
                expected
            };
            assert_eq!(stats.sample_rate, u32::try_from(rate).unwrap());
            assert_eq!(stats.channels, channels as u16);
            assert_eq!(stats.sample_frames, wanted.len() as u64 / stride as u64);
            assert_pcm(&pcm, wanted);
        }
        checked += 1;
    }
    assert_eq!(checked, 110);
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wave_pcm(path: &Path) -> Vec<u8> {
    let mut file = std::fs::File::open(path).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    assert!(info.float);
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut pcm = vec![0; info.data_bytes as usize];
    file.read_exact(&mut pcm).unwrap();
    pcm
}
#[test]
fn public_mp3_wave_export_transform_limits_and_publication_are_owned() {
    let dir =
        Directory(std::env::temp_dir().join(format!("fvid-mp3-export-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let source = root().join("mp3-gapless/primed-free.mkv");
    let out = dir.0.join("out.wav");
    let plan =
        fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default()).unwrap();
    assert_eq!(plan.streams[0].codec, "mp3");
    let stats = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
    assert_eq!(stats.sample_frames, 4001);
    let expected = std::fs::read(root().join("mp3-decoder/48000-1ch.f32le")).unwrap();
    assert_pcm(&wave_pcm(&out), &expected[536 * 4..4537 * 4]);
    assert!(fvid_media::decode_audio(&source, &out, &Default::default()).is_err());
    let transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((40000, 60000)),
        sample_rate: Some(44100),
        channels: Some(2),
        volume: Some(0.5),
    };
    let actual = dir.0.join("transform.wav");
    let reference = dir.0.join("reference.wav");
    fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default()).unwrap();
    fvid_media::decode_audio_transformed(&out, &reference, transform, &Default::default()).unwrap();
    assert_eq!(wave_pcm(&actual), wave_pcm(&reference));
    let prefix = dir.0.join("prefix.wav");
    let stats = fvid_media::decode_audio(
        &source,
        &prefix,
        &CopyOptions {
            max_packets: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 616);
    assert_pcm(&wave_pcm(&prefix), &expected[536 * 4..1152 * 4]);
    let too_small = dir.0.join("packet-limit.wav");
    assert!(
        fvid_media::decode_audio(
            &source,
            &too_small,
            &CopyOptions {
                max_packet_bytes: 8,
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!too_small.exists());
    let mid_cancel = dir.0.join("mid-cancel.wav");
    let cancellation = fvid_control::CancelFlag::new();
    let trigger = cancellation.clone();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let error = fvid_media::decode_audio(
        &source,
        &mid_cancel,
        &CopyOptions {
            cancel: Some(cancellation),
            progress: Some(fvid_control::ProgressHook::new(move |event| {
                captured.lock().unwrap().push(event);
                if event.packets >= 2 {
                    trigger.cancel();
                }
            })),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(!mid_cancel.exists());
    assert!(events.lock().unwrap().iter().all(|event| !event.done));
    let limited = dir.0.join("limited.wav");
    let error = fvid_media::decode_audio(
        &source,
        &limited,
        &CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error.contains("controlled memory budget exceeded"),
        "{error}"
    );
    assert!(!limited.exists());
    let flag = fvid_control::CancelFlag::new();
    flag.cancel();
    let cancelled = dir.0.join("cancelled.wav");
    assert!(
        fvid_media::decode_audio(
            &source,
            &cancelled,
            &CopyOptions {
                cancel: Some(flag),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("cancelled")
    );
    assert!(!cancelled.exists());
}

#[test]
fn matroska_coded_info_marker_is_audio_in_the_library() {
    let source = std::fs::read(root().join("mp3-gapless/coded-marker.mkv")).unwrap();
    let expected = std::fs::read(root().join("mp3-gapless/coded-marker.f32le")).unwrap();
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_matroska_mp3::decode_matroska_mp3_pcm(
        Cursor::new(source),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 4608);
    assert_pcm(&pcm, &expected);
}
