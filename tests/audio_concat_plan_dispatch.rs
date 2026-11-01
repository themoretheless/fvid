use fvid::media::{self, CopyOptions};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-metadata-selection-window")
}
struct Dir(PathBuf);
impl Dir {
    fn new(label: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "fvid-audio-concat-plan-{label}-{}",
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
fn wave_data(data: &[u8]) -> &[u8] {
    assert_eq!(&data[..4], b"RIFF");
    assert_eq!(&data[8..12], b"WAVE");
    let mut at = 12;
    loop {
        let len = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        if &data[at..at + 4] == b"data" {
            return &data[at + 8..at + 8 + len];
        }
        at += 8 + len + len % 2;
    }
}
#[test]
fn g711_source_decodes_to_exact_independent_pcm_before_concat() {
    let d = Dir::new("source");
    for (memory_index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
        let output = d.0.join(format!("{memory_index}.f32le"));
        media::decode_audio(
            &root().join("front-alaw.wav"),
            &output,
            &CopyOptions {
                max_controlled_bytes: memory,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read(output).unwrap(),
            std::fs::read(root().join("front-alaw.f32le")).unwrap()
        );
    }
}
#[test]
fn concat_plans_use_the_existing_decoders_for_all_admitted_first_sources() {
    let d = Dir::new("publication");
    let mut failures = Vec::new();
    for (name, codec, frames) in [
        ("front-alaw.wav", "pcm_alaw", 960),
        ("one.ogg", "opus", 960),
        ("vorbis.ogg", "vorbis", 411),
    ] {
        for (index, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let source = root().join(name);
            let inputs = [source.clone(), source];
            let options = CopyOptions {
                max_controlled_bytes: memory,
                max_packets: Some(64),
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
            assert_eq!(plan.streams[0].codec, codec);
            assert_eq!(plan.streams[0].disposition, "decode_to_pcm");
            let output = d.0.join(format!("{name}-{index}.wav"));
            let stats = media::concat(&inputs, &output, &options).unwrap();
            assert_eq!(stats.segments, 2);
            let data = std::fs::read(output).unwrap();
            let pcm = wave_data(&data);
            assert_eq!(pcm.len(), 2 * frames * 4);
            if codec == "pcm_alaw" {
                let oracle = std::fs::read(root().join("front-alaw.f32le")).unwrap();
                assert_eq!(pcm, [oracle.as_slice(), oracle.as_slice()].concat());
            } else {
                let peak = pcm
                    .chunks_exact(4)
                    .map(|b| {
                        let x = f32::from_le_bytes(b.try_into().unwrap());
                        assert!(x.is_finite());
                        x.abs()
                    })
                    .fold(0f32, f32::max);
                assert!(peak <= 1e-25, "{name}:silent floor={peak}");
            }
        }
    }
    assert!(
        failures.is_empty(),
        "admitted audio was replanned as unsupported AAC:{failures:?}"
    );
}
#[test]
fn invalid_selection_work_bytes_and_memory_still_refuse_before_publication() {
    let d = Dir::new("refusal");
    let source = root().join("front-alaw.wav");
    let inputs = [source.clone(), source];
    for (index, options) in [
        CopyOptions {
            streams: vec![1],
            ..Default::default()
        },
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
        assert!(media::plan_concat(&inputs, &options).is_err());
        let output = d.0.join(format!("{index}.wav"));
        assert!(media::concat(&inputs, &output, &options).is_err());
        assert!(!output.exists());
    }
}
