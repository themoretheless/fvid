#![cfg(feature = "player")]
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/xm-pattern-delay")
}
fn render(name: &str, rate: u32, channels: u16) -> (Vec<u8>, fvid::codec::xm_decoder::Ignored) {
    let bytes = std::fs::read(root().join(name)).unwrap();
    let mut r =
        fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut d = fvid::codec::xm_decoder::XmDecoder::new(r.extra_data(), rate, channels).unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = r.next_packet().unwrap() {
        let a = d
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        assert_eq!(a.packet.timebase_den, rate);
        pcm.extend(a.packet.data);
    }
    (pcm, d.ignored())
}
#[test]
fn pattern_delays_match_literal_tick_scores() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 20);
    for c in m["cases"].as_array().unwrap() {
        for rate in [11025, 44100] {
            for channels in [1, 2] {
                let (actual, ignored) = render(c["file"].as_str().unwrap(), rate, channels);
                let (expected, other) = render(c["control"].as_str().unwrap(), rate, channels);
                assert_eq!(actual.len(), expected.len());
                assert!(
                    actual == expected,
                    "{}: PCM differs at rate {rate}, channels {channels}",
                    c["name"]
                );
                assert_eq!(ignored, fvid::codec::xm_decoder::Ignored::default());
                assert_eq!(other, fvid::codec::xm_decoder::Ignored::default());
            }
        }
    }
}

#[path = "support/audio_capture.rs"]
mod capture;
#[test]
fn pattern_delay_worker_matches_independent_ticks_after_rewind_seek_and_eof() {
    use std::time::Duration;
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root().join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let (pcm, ignored) = render(c["control"].as_str().unwrap(), 44100, 2);
        assert_eq!(ignored, fvid::codec::xm_decoder::Ignored::default());
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mut actual =
            fvid::playback_xm::XmAudioReader::open(Cursor::new(bytes.clone()), Default::default())
                .unwrap();
        let total = actual.duration().unwrap();
        let mut expected = Vec::new();
        while let Some(p) = actual.next_packet().unwrap() {
            let first = p.pts as usize * 44100 / 1000;
            let end = (p.pts + p.duration) as usize * 44100 / 1000;
            expected.push((
                Duration::from_nanos(first as u64 * 1_000_000_000 / 44100),
                pcm[first * 8..end * 8].to_vec(),
            ));
        }
        for target in [expected[1].0, total] {
            let actual = fvid::playback_xm::XmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            let suffix: Vec<_> = expected
                .iter()
                .filter(|(at, _)| *at >= target)
                .cloned()
                .collect();
            capture::worker_check_with_seek_expected(Box::new(actual), &expected, target, &suffix);
        }
    }
}

#[test]
fn held_cycle_flags_flow_traces_and_row_limits_are_exact() {
    use fvid::container::xm::{Limits, Module, ROW_CONTINUATION_FLAG};
    let controls: Vec<(&str, Vec<(usize, usize, bool)>)> = vec![
        ("hold-one", vec![(0, 0, false), (0, 0, true), (0, 1, false)]),
        ("last-zero-wins", vec![(0, 0, false), (0, 1, false)]),
        (
            "maximum",
            std::iter::once((0, 0, false))
                .chain(std::iter::repeat_n((0, 0, true), 15))
                .chain(std::iter::once((0, 1, false)))
                .collect(),
        ),
        (
            "loop-held",
            vec![
                (0, 0, false),
                (0, 1, false),
                (0, 1, true),
                (0, 1, false),
                (0, 1, true),
                (0, 2, false),
            ],
        ),
        (
            "jump-held",
            vec![(0, 0, false), (0, 0, true), (0, 1, false), (0, 2, false)],
        ),
        (
            "jump-next-held",
            vec![(0, 0, false), (0, 0, true), (1, 1, false), (1, 2, false)],
        ),
        (
            "speed-one-A",
            vec![
                (0, 0, false),
                (0, 1, false),
                (0, 1, true),
                (0, 1, true),
                (0, 2, false),
            ],
        ),
    ];
    for (name, expected) in controls {
        let bytes = std::fs::read(root().join(format!("{name}.xm"))).unwrap();
        let limits = Limits {
            rows: expected.len(),
            ..Default::default()
        };
        let module = Module::parse(&bytes, &limits).unwrap();
        let trace: Vec<_> = module
            .timeline()
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let packet = module.packet(i);
                assert!(packet[7] == 0 || packet[7] == ROW_CONTINUATION_FLAG);
                (row.order, row.row, packet[7] == ROW_CONTINUATION_FLAG)
            })
            .collect();
        assert_eq!(trace, expected, "{name}");
        assert!(
            Module::parse(
                &bytes,
                &Limits {
                    rows: expected.len() - 1,
                    ..limits
                }
            )
            .is_err(),
            "{name}: holds bypassed row budget"
        );
        let mut invalid = module.packet(0);
        invalid[7] = 2;
        assert!(
            fvid::container::xm::read_row(&invalid)
                .unwrap_err()
                .to_string()
                .contains("continuation flags")
        );
    }
}
