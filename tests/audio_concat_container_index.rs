use fvid::media::{self, CopyOptions};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-concat-container-index")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
struct Dir(PathBuf);
impl Dir {
    fn new(label: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "fvid-concat-container-index-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn compare(a: &[u8], b: &[u8], exact: bool) {
    assert_eq!(a.len(), b.len());
    if exact {
        assert_eq!(a, b);
    } else {
        let mut peak = 0f32;
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(a.is_finite() && b.is_finite());
            peak = peak.max((a - b).abs());
        }
        assert!(peak < 1e-25, "silent numerical floor:{peak}");
    }
}
fn wave_data(data: &[u8]) -> &[u8] {
    assert_eq!(&data[..4], b"RIFF");
    let mut at = 12;
    loop {
        let n = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        if &data[at..at + 4] == b"data" {
            return &data[at + 8..at + 8 + n];
        }
        at += 8 + n + n % 2;
    }
}
#[test]
fn container_decoders_and_video_control_are_healthy_before_work_counting() {
    let d = Dir::new("sources");
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let out = d.0.join(format!("{name}.f32le"));
        let stats = media::decode_audio(&root().join(name), &out, &CopyOptions::default())
            .unwrap_or_else(|e| panic!("{name}:{e}"));
        assert_eq!(stats.channels, c["channels"].as_u64().unwrap() as i32);
        assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap());
        assert_eq!(stats.decoded_frames, c["work"].as_u64().unwrap());
        compare(
            &std::fs::read(out).unwrap(),
            &bytes(c["oracle"].as_str().unwrap()),
            c["exact"].as_bool().unwrap(),
        );
    }
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(bytes("video-control.mkv")),
        32 << 20,
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            video
                .read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            vec![130; 16 * 16 * 3]
        );
    }
    assert!(video.read_frame_raw().unwrap().is_none());
    video.rewind().unwrap();
    assert_eq!(
        video
            .seek_raw(std::time::Duration::from_millis(60))
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        vec![130; 16 * 16 * 3]
    );
    assert_eq!(
        video.frame_interval(),
        Some((40_000_000, 80_000_000, 1_000_000_000))
    );
    let mut a = fvid::container::webm::WebmReader::open(
        Cursor::new(bytes("video-control.mkv")),
        Default::default(),
    )
    .unwrap();
    let mut b =
        fvid::container::webm::WebmReader::open(Cursor::new(bytes("flac.mka")), Default::default())
            .unwrap();
    a.scan_all().unwrap();
    b.scan_all().unwrap();
    let indices: Vec<_> = a
        .packets
        .iter()
        .enumerate()
        .filter_map(|(i, p)| (p.track == 2).then_some(i))
        .collect();
    assert_eq!(indices.len(), 2);
    assert_eq!(b.packets.len(), 2);
    for (i, index) in indices.into_iter().enumerate() {
        assert_eq!(a.read_packet(index).unwrap(), b.read_packet(i).unwrap());
    }
    for c in cases() {
        let source = root().join(c["file"].as_str().unwrap());
        media::plan_concat(&[source.clone(), source], &CopyOptions::default()).unwrap();
    }
}
#[test]
fn admitted_container_codecs_keep_controlled_concat_plans_and_both_segments() {
    let d = Dir::new("publication");
    let mut failures = Vec::new();
    for c in cases() {
        for (index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let name = c["file"].as_str().unwrap();
            let source = root().join(name);
            let inputs = [source.clone(), source];
            let options = CopyOptions {
                max_packets: Some(c["work"].as_u64().unwrap() * 2),
                max_controlled_bytes: memory,
                ..Default::default()
            };
            let plan = match media::plan_concat(&inputs, &options) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(format!("{name}/{memory:?}:{e}"));
                    continue;
                }
            };
            assert_eq!(plan.streams.len(), 1);
            assert_eq!(plan.streams[0].codec, c["codec"].as_str().unwrap());
            let out = d.0.join(format!("{name}-{index}.wav"));
            let stats = media::concat(&inputs, &out, &options).unwrap();
            assert_eq!(stats.packets, c["work"].as_u64().unwrap() * 2);
            assert_eq!(stats.segments, 2);
            let data = std::fs::read(out).unwrap();
            let pcm = wave_data(&data);
            let oracle = bytes(c["oracle"].as_str().unwrap());
            compare(
                pcm,
                &[oracle.as_slice(), oracle.as_slice()].concat(),
                c["exact"].as_bool().unwrap(),
            );
        }
    }
    assert!(
        failures.is_empty(),
        "packet counting rejected an admitted container decoder:{failures:?}"
    );
}
#[test]
fn legacy_codec_refusals_and_public_resource_controls_remain_in_force() {
    let d = Dir::new("refusals");
    for c in cases() {
        let source = root().join(c["file"].as_str().unwrap());
        assert!(
            fvid::native_plan::decode_audio_with_options(
                &source,
                &Default::default(),
                &CopyOptions::default()
            )
            .is_err(),
            "legacy native exporter must not silently gain codec admission"
        );
        let inputs = [source.clone(), source];
        for (index, options) in [
            CopyOptions {
                max_packet_bytes: 0,
                ..Default::default()
            },
            CopyOptions {
                max_packets: Some(0),
                ..Default::default()
            },
            CopyOptions {
                max_controlled_bytes: Some(64 << 10),
                ..Default::default()
            },
        ]
        .into_iter()
        .enumerate()
        {
            let out =
                d.0.join(format!("{}-{index}.wav", c["codec"].as_str().unwrap()));
            assert!(media::plan_concat(&inputs, &options).is_err());
            assert!(media::concat(&inputs, &out, &options).is_err());
            assert!(!out.exists());
        }
    }
}
