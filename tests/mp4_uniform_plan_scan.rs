use fvid_media::{AudioDecodeTransform, CopyOptions};
use std::{
    io::{BufReader, Cursor},
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-uniform-plan-scan")
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(root().join("manifest.json")).unwrap(),
    )
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("fvid-uniform-plan-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn check_pcm(data: &[u8], frames: usize) {
    assert_eq!(&data[..4], b"RIFF");
    let mut at = 12;
    let mut bits = None;
    let mut samples = None;
    while at + 8 <= data.len() {
        let n = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        let p = &data[at + 8..at + 8 + n];
        match &data[at..at + 4] {
            b"fmt " => {
                assert_eq!(u16::from_le_bytes(p[2..4].try_into().unwrap()), 1);
                assert_eq!(u32::from_le_bytes(p[4..8].try_into().unwrap()), 48000);
                bits = Some(u16::from_le_bytes(p[14..16].try_into().unwrap()));
            }
            b"data" => samples = Some(p),
            _ => {}
        }
        at += 8 + n + n % 2;
    }
    let values = samples.unwrap();
    match bits.unwrap() {
        32 => {
            assert_eq!(values.len(), frames * 4);
            assert!(
                values
                    .chunks_exact(4)
                    .all(|x| f32::from_le_bytes(x.try_into().unwrap()) == 0.125)
            );
        }
        64 => {
            assert_eq!(values.len(), frames * 8);
            assert!(
                values
                    .chunks_exact(8)
                    .all(|x| f64::from_le_bytes(x.try_into().unwrap()) == 0.125)
            );
        }
        b => panic!("unexpected decoded PCM precision{b}"),
    }
}
#[test]
fn compact_sources_and_full_prefix_pcm_match_analytic_waveform() {
    let d = Dir::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let source = root().join(name);
        let r = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            BufReader::new(std::fs::File::open(&source).unwrap()),
            Default::default(),
        )
        .unwrap();
        let track = r.tracks().iter().find(|t| t.handler == *b"soun").unwrap();
        let fvid_media::owned_mp4::SampleIndex::Uniform(index) = &track.samples else {
            panic!("{name}: expanded")
        };
        assert_eq!(index.count, c["frames"].as_u64().unwrap() as usize);
        assert!(index.runs.len() <= 2);
        drop(r);
        for (i, limit) in [None, Some(3)].into_iter().enumerate() {
            let options = CopyOptions {
                max_controlled_bytes: Some(64 << 20),
                max_packets: limit,
                ..Default::default()
            };
            fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &options)
                .unwrap();
            let out = d.0.join(format!("{name}-{i}.wav"));
            let stats = fvid_media::decode_audio(&source, &out, &options).unwrap();
            let frames = limit.unwrap_or(c["frames"].as_u64().unwrap());
            assert_eq!(stats.sample_frames, frames, "{name}");
            assert_eq!(stats.decoded_frames, frames, "{name}");
            check_pcm(&std::fs::read(out).unwrap(), frames as usize);
        }
    }
}
#[test]
fn unused_duplicate_and_reached_changed_headers_keep_memory_admission() {
    for name in [
        "frames-64.mp4",
        "control-alias.mp4",
        "control-unused.mp4",
        "control-endian.mp4",
    ] {
        let source = root().join(name);
        let opts = CopyOptions {
            max_controlled_bytes: Some(64 << 10),
            ..Default::default()
        };
        if name == "control-endian.mp4" {
            let e = fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts)
                .unwrap_err();
            assert!(e.contains("controlled memory"), "{e}");
        } else {
            fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts)
                .unwrap();
        }
        let prefix = CopyOptions {
            max_packets: Some(32),
            ..opts
        };
        fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &prefix).unwrap();
        let tight = CopyOptions {
            max_packet_bytes: 1,
            ..prefix
        };
        let e = fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &tight)
            .unwrap_err();
        assert!(
            e.contains("packet") && (e.contains("budget") || e.contains("limit")),
            "{e}"
        );
    }
}
#[cfg(feature = "player")]
#[test]
fn neutral_control_keeps_native_video_seek_eof_and_rewind() {
    use std::time::Duration;
    let mut r = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root().join("frames-262144.mp4")).unwrap()),
        32 << 20,
    )
    .unwrap();
    for _ in 0..3 {
        assert_eq!(
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            vec![130; 16 * 16 * 3]
        );
    }
    assert!(r.read_frame_raw().unwrap().is_none());
    r.rewind().unwrap();
    assert!(r.read_frame_raw().unwrap().is_some());
    assert_eq!(
        r.seek_raw(Duration::from_millis(25))
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        vec![130; 16 * 16 * 3]
    );
    assert_eq!(r.frame_interval(), Some((1, 2, 50)));
}
