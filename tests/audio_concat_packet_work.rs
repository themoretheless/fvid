use fvid::media::{self, CopyOptions};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-concat-packet-work")
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
        let p =
            std::env::temp_dir().join(format!("fvid-concat-work-{label}-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn compare(actual: &[u8], wanted: &[u8], exact: bool) {
    assert_eq!(actual.len(), wanted.len());
    if exact {
        assert_eq!(actual, wanted);
    } else {
        for (a, b) in actual.chunks_exact(4).zip(wanted.chunks_exact(4)) {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!(
                a.is_finite() && b.is_finite() && (a - b).abs() < 2e-6,
                "{a}!={b}"
            );
        }
    }
}
fn wave_data(data: &[u8]) -> &[u8] {
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut at = 12;
    loop {
        let n = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        if &data[at..at + 4] == b"data" {
            return &data[at + 8..at + 8 + n];
        }
        at += 8 + n + n % 2;
    }
}
fn options(c: &serde_json::Value, memory: Option<usize>, extra: u64) -> CopyOptions {
    CopyOptions {
        max_packets: Some(c["work"].as_u64().unwrap() + extra),
        max_packet_bytes: c["cap"].as_u64().unwrap() as usize,
        max_controlled_bytes: memory,
        ..Default::default()
    }
}
#[test]
fn authored_source_pcm_and_work_counts_match_actual_decoding() {
    let d = Dir::new("sources");
    for c in cases() {
        for (index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let name = c["file"].as_str().unwrap();
            let out = d.0.join(format!("{name}-{index}.f32le"));
            let stats = media::decode_audio(&root().join(name), &out, &options(&c, memory, 1))
                .unwrap_or_else(|e| panic!("{name}:{e}"));
            assert_eq!(
                stats.decoded_frames,
                c["work"].as_u64().unwrap(),
                "{name} work units"
            );
            assert_eq!(
                stats.sample_frames,
                c["frames"].as_u64().unwrap(),
                "{name} frames"
            );
            compare(
                &std::fs::read(out).unwrap(),
                &bytes(c["oracle"].as_str().unwrap()),
                c["exact"].as_bool().unwrap(),
            );
        }
    }
}
#[test]
fn reached_later_segment_is_not_hidden_by_an_unknown_first_work_count() {
    let d = Dir::new("reached");
    let mut failures = Vec::new();
    for c in cases() {
        for (index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let name = c["file"].as_str().unwrap();
            let inputs = [root().join(name), root().join(c["next"].as_str().unwrap())];
            let opts = options(&c, memory, 1);
            if media::plan_concat(&inputs, &opts).is_ok() {
                failures.push(format!("{name}/{memory:?}:active later segment admitted"));
            }
            let out = d.0.join(format!("{name}-{index}.wav"));
            assert!(
                media::concat(&inputs, &out, &opts).is_err(),
                "{name}:execution unexpectedly accepted incompatible/oversized media"
            );
            assert!(!out.exists());
        }
    }
    assert!(
        failures.is_empty(),
        "first count falsely consumed the remaining budget:{failures:?}"
    );
}
#[test]
fn a_truly_exhausted_boundary_preserves_exact_first_segment_pcm() {
    let d = Dir::new("boundary");
    for c in cases() {
        for (index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let name = c["file"].as_str().unwrap();
            let inputs = [root().join(name), root().join(c["next"].as_str().unwrap())];
            let opts = options(&c, memory, 0);
            media::plan_concat(&inputs, &opts).unwrap_or_else(|e| panic!("{name}:{e}"));
            let out = d.0.join(format!("{name}-{index}.wav"));
            let stats = media::concat(&inputs, &out, &opts).unwrap();
            assert_eq!(stats.packets, c["work"].as_u64().unwrap());
            let data = std::fs::read(out).unwrap();
            compare(
                wave_data(&data),
                &bytes(c["oracle"].as_str().unwrap()),
                c["exact"].as_bool().unwrap(),
            );
        }
    }
}

#[test]
fn every_later_packet_is_valid_under_its_generous_byte_limit() {
    let d = Dir::new("later-source");
    for c in cases() {
        let name = c["next"].as_str().unwrap();
        let out = d.0.join(format!("{name}.f32le"));
        let stats = media::decode_audio(
            &root().join(name),
            &out,
            &CopyOptions {
                max_packet_bytes: c["cap"].as_u64().unwrap() as usize + 8,
                max_packets: Some(1),
                ..Default::default()
            },
        )
        .unwrap_or_else(|e| panic!("{name}:{e}"));
        assert_eq!(stats.decoded_frames, 1);
        assert_eq!(stats.sample_frames, 960);
        let pcm = std::fs::read(out).unwrap();
        assert_eq!(
            pcm.len(),
            960 * c["channels"].as_u64().unwrap() as usize * 4
        );
        let peak = pcm
            .chunks_exact(4)
            .map(|b| {
                let x = f32::from_le_bytes(b.try_into().unwrap());
                assert!(x.is_finite());
                x.abs()
            })
            .fold(0f32, f32::max);
        assert!(peak < 1e-25, "{name}:unexpected non-silent output:{peak}");
    }
}
#[test]
fn a_healthy_reached_successor_preserves_both_complete_waveforms() {
    let d = Dir::new("healthy");
    for c in cases() {
        for (index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let name = c["file"].as_str().unwrap();
            let inputs = [
                root().join(name),
                root().join(format!(
                    "opus-{}-small.ogg",
                    c["channels"].as_u64().unwrap()
                )),
            ];
            let opts = options(&c, memory, 1);
            media::plan_concat(&inputs, &opts).unwrap_or_else(|e| panic!("{name}:{e}"));
            let out = d.0.join(format!("{name}-{index}.wav"));
            let stats = media::concat(&inputs, &out, &opts).unwrap();
            assert_eq!(stats.packets, c["work"].as_u64().unwrap() + 1);
            let data = std::fs::read(out).unwrap();
            let pcm = wave_data(&data);
            let oracle = bytes(c["oracle"].as_str().unwrap());
            assert_eq!(
                pcm.len(),
                oracle.len() + 960 * c["channels"].as_u64().unwrap() as usize * 4
            );
            compare(&pcm[..oracle.len()], &oracle, c["exact"].as_bool().unwrap());
            let peak = pcm[oracle.len()..]
                .chunks_exact(4)
                .map(|b| {
                    let x = f32::from_le_bytes(b.try_into().unwrap());
                    assert!(x.is_finite());
                    x.abs()
                })
                .fold(0f32, f32::max);
            assert!(peak < 1e-25);
        }
    }
}
