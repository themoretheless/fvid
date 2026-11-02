#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/pcm-padding-playback")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[key]
        .as_array()
        .unwrap()
        .clone()
}
fn open(c: &serde_json::Value, kind: &str) -> fvid::Result<Box<dyn AudioStream>> {
    let data = Cursor::new(bytes(&format!("{}.{}", c["name"].as_str().unwrap(), kind)));
    if kind == "wav" {
        Ok(Box::new(fvid::playback_wav::WavAudioReader::open(
            data,
            Default::default(),
        )?))
    } else {
        Ok(Box::new(fvid::playback_avi_audio::AviAudioReader::open(
            data,
            Default::default(),
        )?))
    }
}
#[test]
fn zero_padding_and_full_precision_low_bits_have_exact_pcm_clocks_and_video() {
    for c in cases("good") {
        let wanted = bytes(&format!("{}.f32le", c["name"].as_str().unwrap()));
        for kind in ["wav", "avi"] {
            let mut s = open(&c, kind).unwrap();
            let full = rational::collect(&mut *s);
            assert_eq!(
                full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
                wanted,
                "{} {kind}",
                c["name"]
            );
            let last = full.last().unwrap();
            assert_eq!(last.point(last.frames()), (4099, 8000));
            assert!(s.next_packet().unwrap().is_none());
            s.rewind();
            rational::exact(&rational::collect(&mut *s), &full, "padding rewind");
        }
    }
    let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = v.read_frame().unwrap().unwrap();
        let got: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(got, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(v.read_frame().unwrap().is_none());
}
#[test]
fn unused_nonzero_bits_are_rejected_before_wave_or_avi_can_emit_pcm() {
    let mut accepted = Vec::new();
    for c in cases("bad") {
        for kind in ["wav", "avi"] {
            match open(&c, kind) {
                Ok(mut s) => {
                    let decoded = rational::collect(&mut *s);
                    assert_eq!(
                        decoded.iter().map(rational::Packet::frames).sum::<usize>(),
                        4099
                    );
                    accepted.push(format!("{}.{kind}", c["name"]));
                }
                Err(e) => {
                    assert!(matches!(e, fvid::Error::Invalid(_)), "{e}");
                    assert!(e.to_string().contains("nonzero PCM padding bits"), "{e}");
                }
            }
        }
    }
    assert!(
        accepted.is_empty(),
        "nonzero unused bits reached playback: {accepted:?}"
    );
}
#[test]
fn valid_precisions_and_full_width_controls_preserve_actual_worker_seek_and_eof() {
    for c in cases("good") {
        for kind in ["wav", "avi"] {
            let full = rational::collect(&mut *open(&c, kind).unwrap());
            let last = full.last().unwrap();
            let end = last.point(last.frames());
            for at in [
                Duration::ZERO,
                Duration::from_nanos(2049 * 125000),
                Duration::MAX,
            ] {
                let wanted = rational::suffix(&full, at, end);
                let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
                let copy = raw.clone();
                let trace = Arc::new(Mutex::new(transitions::Trace::default()));
                let mut worker =
                    fvid::audio_thread::AudioPlayback::start(open(&c, kind).unwrap(), move || {
                        Box::new(rational::RawCapture {
                            inner: transitions::Capture::new(trace),
                            raw: copy,
                        })
                    });
                worker.seek(at);
                worker.play();
                transitions::ended(&worker, worker.generation());
                let got = raw.lock().unwrap();
                rational::exact(&got.packets, &wanted, "padding worker");
                assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
            }
        }
    }
}
