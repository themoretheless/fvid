use std::path::PathBuf;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avi-adpcm-grouped")
            .join(name),
    )
    .unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[key]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn grouped_chunks_keep_all_fixed_size_samples_and_their_raw_index() {
    for c in cases("controls").into_iter().chain(cases("cases")) {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let parsed = fvid::container::avi::Avi::parse(&data, &Default::default()).unwrap();
        assert_eq!(parsed.streams, 2);
        let audio = &parsed.audio()[0];
        assert_eq!(audio.number, 1);
        assert_eq!(audio.declared, 3);
        assert_eq!(audio.format.block_align, 8);
        assert_eq!(
            audio.records.len(),
            c["raw_records"].as_u64().unwrap() as usize
        );
        let flat = (0..audio.records.len())
            .flat_map(|n| audio.record(&data, n).unwrap().iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(flat.len(), 24);
        let control = c["control"].as_str().unwrap_or(name);
        let reference = bytes(control);
        let ref_parsed = fvid::container::avi::Avi::parse(&reference, &Default::default()).unwrap();
        let a = &ref_parsed.audio()[0];
        let raw = (0..a.records.len())
            .flat_map(|n| a.record(&reference, n).unwrap().iter().copied())
            .collect::<Vec<_>>();
        assert_eq!(flat, raw, "{name}: grouped bytes changed");
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
mod playback {
    use super::*;
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn open(name: &str, records: usize) -> fvid::playback_avi_audio::AviAudioReader {
        fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes(name)),
            fvid::container::avi::Limits {
                records,
                ..Default::default()
            },
        )
        .unwrap()
    }
    fn packets(name: &str) -> Vec<(Duration, Vec<u8>)> {
        let mut s = open(name, 1 << 20);
        let mut d = s.make_decoder().unwrap();
        let mut out = Vec::new();
        while let Some(p) = s.next_packet().unwrap() {
            let decoded = d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap();
            out.push((decoded.packet.presentation_time(), decoded.packet.data));
        }
        out
    }
    fn video() {
        let mut v = fvid::playback_native::NativeReader::software(
            Cursor::new(bytes("video.mkv")),
            32 << 20,
        )
        .unwrap();
        assert!(v.read_frame_raw().unwrap().is_some());
        assert!(v.read_frame_raw().unwrap().is_none());
    }
    fn verify(c: &serde_json::Value) -> Vec<(Duration, Vec<u8>)> {
        let n = c["file"].as_str().unwrap();
        let p = packets(n);
        let b = c["block_frames"].as_u64().unwrap();
        assert_eq!(p.len(), 3);
        let s = open(n, 1 << 20);
        assert_eq!(s.duration(), Some(s.time_of((3 * b) as i64)));
        for (i, (at, data)) in p.iter().enumerate() {
            assert_eq!(*at, s.time_of(i as i64 * b as i64));
            assert_eq!(data.len(), b as usize * 4);
        }
        assert_eq!(
            p.iter()
                .flat_map(|(_, p)| p.iter().copied())
                .collect::<Vec<_>>(),
            bytes(c["expected"].as_str().unwrap())
        );
        p
    }
    fn worker(c: &serde_json::Value, expected: &[(Duration, Vec<u8>)]) {
        let n = c["file"].as_str().unwrap();
        let b = c["block_frames"].as_i64().unwrap();
        let s = open(n, 1 << 20);
        let target = s.time_of(b + 1);
        let clipped = vec![(target, expected[1].1[4..].to_vec()), expected[2].clone()];
        capture::worker_check_with_seek_expected(
            Box::new(open(n, 1 << 20)),
            expected,
            target,
            &clipped,
        );
        capture::worker_check_with_seek(
            Box::new(open(n, 1 << 20)),
            expected,
            Some(s.time_of(3 * b)),
        );
    }
    #[test]
    fn single_block_controls_keep_literal_pcm_and_actual_navigation() {
        video();
        for c in cases("controls") {
            let p = verify(&c);
            worker(&c, &p);
        }
    }
    #[test]
    fn malformed_partial_blocks_and_wrong_block_counts_remain_refused() {
        for c in cases("malformed") {
            let n = c["file"].as_str().unwrap();
            assert!(
                fvid::playback_avi_audio::AviAudioReader::open(
                    Cursor::new(bytes(n)),
                    Default::default()
                )
                .is_err(),
                "{n}"
            );
        }
    }
    #[test]
    fn baseline_grouped_fixed_samples_are_explicitly_unsupported() {
        video();
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let e = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            ) {
                Ok(_) => panic!("{n}: baseline profile changed"),
                Err(e) => e.to_string(),
            };
            assert!(e.contains("states 3 records and the run holds"), "{n}:{e}");
            eprintln!("{n}: existing unsupported grouped framing: {e}");
        }
    }
    #[test]
    #[ignore = "enable with grouped fixed-block packet framing"]
    fn grouped_chunks_decode_exact_literal_pcm_on_individual_block_clocks() {
        video();
        for c in cases("cases") {
            let p = verify(&c);
            assert_eq!(p, packets(c["control"].as_str().unwrap()));
        }
    }
    #[test]
    #[ignore = "enable with grouped fixed-block packet framing"]
    fn grouped_chunks_keep_actual_mid_block_and_eof_worker_navigation() {
        video();
        for c in cases("cases") {
            let expected = packets(c["control"].as_str().unwrap());
            worker(&c, &expected);
        }
    }
    #[test]
    #[ignore = "enable with grouped fixed-block packet framing"]
    fn grouped_chunks_seek_and_rewind_at_virtual_block_boundaries() {
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let b = c["block_frames"].as_i64().unwrap();
            let mut s = open(n, 1 << 20);
            for target in [
                -1,
                0,
                b - 1,
                b,
                b + 1,
                3 * b - 1,
                3 * b,
                3 * b + 1,
                i64::MAX,
            ] {
                let expected = if target >= 3 * b {
                    3 * b
                } else {
                    target.max(0) / b * b
                };
                assert_eq!(s.seek_to(target), expected, "{n}:target{target}");
                let p = s.next_packet().unwrap();
                if expected == 3 * b {
                    assert!(p.is_none());
                } else {
                    let p = p.unwrap();
                    assert_eq!((p.pts, p.duration, p.data.len()), (expected, b, 8));
                }
            }
            s.rewind();
            let mut count = 0;
            while s.next_packet().unwrap().is_some() {
                count += 1;
            }
            assert_eq!(count, 3);
        }
    }
    #[test]
    #[ignore = "enable with grouped fixed-block packet framing"]
    fn expansion_respects_the_logical_packet_budget_before_publishing() {
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let mut exact = open(n, 3);
            let mut count = 0;
            while exact.next_packet().unwrap().is_some() {
                count += 1;
            }
            assert_eq!(count, 3);
            let error = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(bytes(n)),
                fvid::container::avi::Limits {
                    records: 2,
                    ..Default::default()
                },
            ) {
                Ok(_) => panic!("{n}: expanded blocks bypassed limit"),
                Err(e) => e.to_string(),
            };
            assert!(error.contains("ADPCM blocks"), "{n}:{error}");
        }
    }
}
