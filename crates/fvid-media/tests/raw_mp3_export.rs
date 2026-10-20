use fvid_control::CopyOptions;
use std::{
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn same(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "sample {i}: {a} versus {b}");
    }
}
#[test]
fn raw_mp3_gapless_metadata_controls_and_preroll_match_independent_pcm() {
    let directory = root().join("mp3-gapless");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    let options = CopyOptions {
        max_controlled_bytes: Some(32 << 20),
        ..Default::default()
    };
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let source = directory.join(format!("{name}.mp3"));
        let rate = case["rate"].as_u64().unwrap();
        let stride = case["channels"].as_u64().unwrap() as usize * 4;
        let golden = std::fs::read(
            root()
                .join("mp3-decoder")
                .join(format!("{}.f32le", case["source"].as_str().unwrap())),
        )
        .unwrap();
        let a = case["start"].as_u64().unwrap() as usize;
        let b = case["end"].as_u64().unwrap() as usize;
        let expected = &golden[a * stride..b * stride];
        for interval in [
            None,
            Some((Duration::from_millis(40), Duration::from_millis(60))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
                &source, &mut pcm, interval, &options,
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            let expected = if interval.is_some() {
                let first = (rate * 40).div_ceil(1000) as usize;
                let last = (rate * 60).div_ceil(1000) as usize;
                &expected[first * stride..last.min(b - a) * stride]
            } else {
                expected
            };
            assert_eq!(stats.sample_frames, expected.len() as u64 / stride as u64);
            same(&pcm, expected);
        }
    }
}
#[test]
fn pure_free_format_and_coded_info_markers_remain_audio() {
    let dir = root().join("mp3-free");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
            &dir.join(format!("{name}.mp3")),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let original = format!(
            "{}-{}ch",
            case["rate"].as_u64().unwrap(),
            case["channels"].as_u64().unwrap()
        );
        let golden =
            std::fs::read(root().join("mp3-decoder").join(format!("{original}.f32le"))).unwrap();
        assert_eq!(stats.sample_frames, case["samples"].as_u64().unwrap());
        same(&pcm, &golden[..pcm.len()]);
    }
    let source = root().join("mp3-gapless/coded-marker.mp3");
    let expected = std::fs::read(root().join("mp3-gapless/coded-marker.f32le")).unwrap();
    let mut pcm = Vec::new();
    fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(&source, &mut pcm, None, &Default::default())
        .unwrap();
    same(&pcm, &expected);
    assert!(
        fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
            &root().join("mp3-gapless/info-only.mp3"),
            &mut Vec::new(),
            None,
            &Default::default()
        )
        .unwrap_err()
        .contains("nothing but an info frame")
    );
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn pcm(path: &Path) -> Vec<u8> {
    let mut file = std::fs::File::open(path).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None).unwrap();
    assert!(info.float);
    file.seek(SeekFrom::Start(info.data_offset)).unwrap();
    let mut pcm = vec![0; info.data_bytes as usize];
    file.read_exact(&mut pcm).unwrap();
    pcm
}
#[test]
fn public_raw_mp3_plan_export_dsp_limits_and_cancellation() {
    let dir = Directory(std::env::temp_dir().join(format!("fvid-raw-mp3-{}", std::process::id())));
    std::fs::create_dir(&dir.0).unwrap();
    let source = root().join("mp3-gapless/primed-free.mp3");
    let plan =
        fvid_media::plan_decode_audio(&source, &Default::default(), &Default::default()).unwrap();
    assert_eq!(plan.streams[0].codec, "mp3");
    assert!(plan.steps[0].detail.contains("raw MP3"));
    let out = dir.0.join("out.wav");
    let stats = fvid_media::decode_audio(&source, &out, &Default::default()).unwrap();
    assert_eq!(stats.sample_frames, 4001);
    let golden = std::fs::read(root().join("mp3-decoder/48000-1ch.f32le")).unwrap();
    same(&pcm(&out), &golden[536 * 4..4537 * 4]);
    let transform = fvid_media_info::AudioDecodeTransform {
        interval: Some((40000, 60000)),
        sample_rate: Some(44100),
        channels: Some(2),
        volume: Some(0.5),
    };
    let actual = dir.0.join("actual.wav");
    let expected = dir.0.join("expected.wav");
    fvid_media::decode_audio_transformed(&source, &actual, transform, &Default::default()).unwrap();
    fvid_media::decode_audio_transformed(&out, &expected, transform, &Default::default()).unwrap();
    assert_eq!(pcm(&actual), pcm(&expected));
    assert!(fvid_media::decode_audio(&source, &out, &Default::default()).is_err());
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
    same(&pcm(&prefix), &golden[536 * 4..1152 * 4]);
    for (name, options, reason) in [
        (
            "budget",
            CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            "controlled memory budget exceeded",
        ),
        (
            "packet",
            CopyOptions {
                max_packet_bytes: 8,
                ..Default::default()
            },
            "packet bytes exceed limit",
        ),
    ] {
        let destination = dir.0.join(format!("{name}.wav"));
        let error = fvid_media::decode_audio(&source, &destination, &options).unwrap_err();
        assert!(error.contains(reason), "{error}");
        assert!(!destination.exists());
    }
    let indexed = dir.0.join("index-budget.wav");
    let base = std::fs::metadata(&source).unwrap().len() as usize
        + fvid_codecs::mp3::Decoder::decode_admission_bytes();
    let error = fvid_media::decode_audio(
        &source,
        &indexed,
        &CopyOptions {
            max_controlled_bytes: Some(base),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("MPEG file/index/decoder"), "{error}");
    assert!(!indexed.exists());
    let flag = fvid_control::CancelFlag::new();
    let trigger = flag.clone();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = events.clone();
    let cancelled = dir.0.join("cancelled.wav");
    let error = fvid_media::decode_audio(
        &source,
        &cancelled,
        &CopyOptions {
            cancel: Some(flag),
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
    assert!(error.contains("cancelled"));
    assert!(!cancelled.exists());
    assert!(events.lock().unwrap().iter().all(|e| !e.done));
}

#[test]
fn free_format_probes_honor_encoded_packet_bound_including_padding_byte() {
    let source = root().join("mp3-free/48000-1ch-640k.mp3");
    let mut accepted = Vec::new();
    let stats = fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
        &source,
        &mut accepted,
        None,
        &CopyOptions {
            max_packet_bytes: 1921,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 4608);
    let mut refused = Vec::new();
    let error = fvid_media::owned_raw_mp3::decode_raw_mp3_pcm(
        &source,
        &mut refused,
        None,
        &CopyOptions {
            max_packet_bytes: 1920,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("encoded packet byte limit"), "{error}");
    assert!(refused.is_empty());
}
