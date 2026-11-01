use fvid::media::{self, CopyOptions};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-metadata-selection-window")
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
            "fvid-metadata-selection-{label}-{}",
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
        at += 8 + len + (len % 2);
    }
}
#[test]
fn authored_opus_controls_decode_and_seek_with_bounded_silence() {
    for (n, frames) in [("one.ogg", 960), ("tail.ogg", 1920)] {
        let mut p =
            fvid_media::owned_ogg_opus::Program::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!((p.channels(), p.end_samples()), (1, frames));
        let mut pcm = Vec::new();
        let stats = p.decode_pcm(&mut pcm).unwrap();
        assert_eq!(stats.sample_frames, frames as u64);
        assert_silence(&pcm, frames as usize);
        assert!(p.next_packet().unwrap().is_none());
        p.seek_to(480);
        pcm.clear();
        let stats = p.decode_pcm(&mut pcm).unwrap();
        assert_eq!(stats.sample_frames, (frames - 480) as u64);
        assert_silence(&pcm, frames as usize - 480);
    }
    for c in cases() {
        let info =
            fvid_media::owned_probe::probe_ogg(&root().join(c["file"].as_str().unwrap())).unwrap();
        assert_eq!(info.streams.len(), 1);
        assert_eq!(info.streams[0].channels, 1);
        assert_eq!(info.streams[0].sample_rate, 48000);
    }
}
#[test]
fn automatic_trim_matches_explicit_selection_under_active_window_byte_limits() {
    let d = Dir::new("trim");
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let source = root().join(name);
        let to = c["to_us"].as_i64().unwrap();
        for streams in [vec![0], vec![]] {
            let options = CopyOptions {
                streams: streams.clone(),
                max_packet_bytes: c["cap"].as_u64().unwrap() as usize,
                ..Default::default()
            };
            for ext in ["wav", "mka"] {
                let output = d.0.join(format!("{name}-{}.{ext}", streams.len()));
                if let Err(e) = media::plan_trim(&source, 0, to, &options) {
                    failures.push(format!("plan {name}/{streams:?}:{e}"));
                    continue;
                }
                if let Err(e) = media::trim(&source, &output, 0, to, &options) {
                    failures.push(format!("trim {name}/{streams:?}/{ext}:{e}"));
                    continue;
                }
                let pcm = d.0.join(format!("{name}-{}-{ext}.f32le", streams.len()));
                media::decode_audio(&output, &pcm, &CopyOptions::default()).unwrap();
                let data = std::fs::read(pcm).unwrap();
                assert_eq!(data.len(), to as usize * 48000 / 1_000_000 * 4);
                assert_silence(&data, data.len() / 4);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "metadata selection spent out-of-window packet budgets:{failures:?}"
    );
}
#[test]
fn exhausted_concat_prefix_still_reads_unreached_ogg_stream_headers() {
    let d = Dir::new("concat");
    let inputs = [root().join("front.wav"), root().join("tail.ogg")];
    let options = CopyOptions {
        max_packets: Some(1),
        ..Default::default()
    };
    assert!(media::plan_concat(&inputs, &options).is_ok());
    let out = d.0.join("prefix.wav");
    let stats = media::concat(&inputs, &out, &options).unwrap();
    assert_eq!(stats.packets, 1);
    assert_eq!(
        wave_data(&std::fs::read(out).unwrap()),
        bytes("front.f32le")
    );
}
#[test]
fn active_oversized_packet_zero_limits_memory_and_cancellation_still_refuse() {
    let d = Dir::new("refusal");
    let source = root().join("tail.ogg");
    for streams in [vec![], vec![0]] {
        let options = CopyOptions {
            streams: streams.clone(),
            max_packet_bytes: 6,
            ..Default::default()
        };
        assert!(media::plan_trim(&source, 20000, 40000, &options).is_err());
        let output = d.0.join(format!("active-{}.mka", streams.len()));
        assert!(media::trim(&source, &output, 20000, 40000, &options).is_err());
        assert!(!output.exists());
    }
    let cancel = fvid::media_control::CancelFlag::default();
    cancel.cancel();
    let controls = [
        CopyOptions {
            max_packets: Some(0),
            ..Default::default()
        },
        CopyOptions {
            max_packet_bytes: 0,
            ..Default::default()
        },
        CopyOptions {
            max_controlled_bytes: Some(64 << 10),
            ..Default::default()
        },
        CopyOptions {
            cancel: Some(cancel),
            ..Default::default()
        },
    ];
    for (index, options) in controls.into_iter().enumerate() {
        let output = d.0.join(format!("{index}.mka"));
        assert!(media::plan_trim(&source, 0, 10000, &options).is_err());
        assert!(media::trim(&source, &output, 0, 10000, &options).is_err());
        assert!(!output.exists());
    }
}

fn assert_silence(pcm: &[u8], frames: usize) {
    assert_eq!(pcm.len(), frames * 4);
    let peak = pcm
        .chunks_exact(4)
        .map(|b| {
            let x = f32::from_le_bytes(b.try_into().unwrap());
            assert!(x.is_finite());
            x.abs()
        })
        .fold(0f32, f32::max);
    assert!(
        peak <= 1e-25,
        "silent codec numerical floor exceeds bound:{peak}"
    );
}
