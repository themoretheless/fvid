#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-composition-gap")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn check(n: &str) {
    let expected = bytes("expected.f32le");
    let video = fvid_media::decode_video(&path(n)).unwrap();
    assert_eq!(video.decode_errors, 0);
    assert!(video.video_frames > 0);
    let mut root =
        fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    let mut owned =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    for i in 0..3 {
        let a = root.tracks()[1].samples.get(i).unwrap();
        let b = owned.tracks()[1].samples.get(i).unwrap();
        assert_eq!((a.pts, a.duration), ([0, 320, 480][i], 160));
        assert_eq!((a.pts, a.duration), (b.pts, b.duration));
        let (mut x, mut y) = (Vec::new(), Vec::new());
        root.read_packet(1, i, &mut x).unwrap();
        owned.read_packet(1, i, &mut y).unwrap();
        assert_eq!(x, bytes(&format!("pcm-{i}.packet")));
        assert_eq!(x, y);
    }
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(n)),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 640);
    assert_eq!(pcm, expected, "owned {n}");
    let mut s = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    let all = transitions::collect(&mut s);
    let pcm: Vec<u8> = all.into_iter().flat_map(|x| x.3).collect();
    assert_eq!(pcm, expected, "native {n}");
    for ms in [0, 10, 20, 25, 35, 40, 60, 79, 80] {
        s.seek_time(Duration::from_millis(ms));
        let got: Vec<u8> = transitions::collect(&mut s)
            .into_iter()
            .flat_map(|x| x.3)
            .collect();
        assert_eq!(
            got,
            expected[(ms as usize * 8 * 4).min(expected.len())..],
            "seek{ms} {n}"
        );
    }
}
#[test]
fn declared_decode_gap_control_has_correct_pcm_and_seek() {
    check("decode-gap.mp4");
}
#[test]
fn composition_gap_has_identical_pcm_and_seek() {
    check("composition-gap.mp4");
}

#[test]
fn composition_gap_trims_and_resource_refusals_keep_exact_boundaries() {
    let expected = bytes("expected.f32le");
    for (a, b) in [(25, 35), (45, 55), (10, 65)] {
        let mut out = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes("composition-gap.mp4")),
            &mut out,
            Some((Duration::from_millis(a), Duration::from_millis(b))),
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, expected[a as usize * 8 * 4..b as usize * 8 * 4]);
    }
    let cancel = fvid::media_control::CancelFlag::new();
    cancel.cancel();
    for (options, reason) in [
        (
            fvid_media::CopyOptions {
                cancel: Some(cancel),
                ..Default::default()
            },
            "media operation cancelled",
        ),
        (
            fvid_media::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            "controlled memory budget exceeded",
        ),
        (
            fvid_media::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
            "MP4 packet exceeds budget",
        ),
        (
            fvid_media::CopyOptions {
                max_packets: Some(0),
                ..Default::default()
            },
            "packet work limit requires at least one packet",
        ),
    ] {
        let mut out = Vec::new();
        let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes("composition-gap.mp4")),
            &mut out,
            None,
            &options,
        )
        .unwrap_err();
        assert!(e.to_string().contains(reason), "{e}");
        assert!(out.is_empty());
    }
}
#[test]
fn actual_audio_worker_preserves_composition_gap_and_seeks_into_silence() {
    use std::sync::{Arc, Mutex};
    let trace = Arc::new(Mutex::new(transitions::Trace::default()));
    let capture = trace.clone();
    let r = Mp4AudioReader::open(
        Cursor::new(bytes("composition-gap.mp4")),
        Default::default(),
    )
    .unwrap();
    let mut audio = fvid::audio_thread::AudioPlayback::start(Box::new(r), move || {
        Box::new(transitions::Capture::new(capture))
    });
    audio.play();
    transitions::ended(&audio, 0);
    let expected = bytes("expected.f32le");
    let out: Vec<u8> = trace
        .lock()
        .unwrap()
        .packets
        .iter()
        .flat_map(|p| p.3.iter().copied())
        .collect();
    assert_eq!(out, expected);
    trace.lock().unwrap().packets.clear();
    audio.seek(Duration::from_millis(25));
    audio.play();
    transitions::ended(&audio, audio.generation());
    let packets = trace.lock().unwrap().packets.clone();
    assert_eq!(packets.first().unwrap().2, Duration::from_millis(25));
    let out: Vec<u8> = packets.into_iter().flat_map(|p| p.3).collect();
    assert_eq!(out, expected[25 * 8 * 4..]);
}
