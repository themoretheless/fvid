use std::path::PathBuf;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avi-pcm")
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
fn avi_frames_already_hold_the_declared_linear_pcm_geometry() {
    for c in cases("cases") {
        let n = c["file"].as_str().unwrap();
        let p = fvid::container::avi::Avi::parse(&bytes(n), &Default::default()).unwrap();
        let a = &p.audio()[0];
        assert_eq!(a.number, 1);
        assert_eq!(a.format.tag, c["tag"].as_u64().unwrap() as u16);
        assert_eq!(a.format.bits, c["bits"].as_u64().unwrap() as u16);
        assert_eq!(
            a.format.block_align,
            c["frame_bytes"].as_u64().unwrap() as u16
        );
        assert_eq!(a.declared, c["frames"].as_u64().unwrap() as u32);
        assert_eq!((a.scale, a.rate), (1, 8000));
        let parts = c["parts"].as_array().unwrap();
        assert_eq!(a.records.len(), parts.len());
        for (r, part) in a.records.iter().zip(parts) {
            assert_eq!(
                r.len as u64,
                part.as_u64().unwrap() * c["frame_bytes"].as_u64().unwrap()
            );
        }
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
    fn time(frame: u64) -> Duration {
        Duration::from_nanos(frame * 125_000)
    }
    fn avi(
        c: &serde_json::Value,
        limits: fvid::container::avi::Limits,
    ) -> fvid::playback_avi_audio::AviAudioReader {
        fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            limits,
        )
        .unwrap()
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
    fn expected(c: &serde_json::Value, target: u64) -> Vec<(Duration, Vec<u8>)> {
        let gold = bytes(c["expected"].as_str().unwrap());
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        let mut start = 0;
        let mut out = Vec::new();
        for part in c["parts"].as_array().unwrap() {
            let end = start + part.as_u64().unwrap();
            let mut at = start.max(target);
            while at < end {
                let take = (end - at).min(2048);
                out.push((
                    time(at),
                    gold[at as usize * width..(at + take) as usize * width].to_vec(),
                ));
                at += take;
            }
            start = end;
        }
        out
    }
    fn read_pcm(
        mut s: Box<dyn AudioStream>,
        c: &serde_json::Value,
        target: u64,
        avi_windows: bool,
    ) -> Vec<(Duration, Vec<u8>)> {
        let total = c["frames"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        let encoded = c["frame_bytes"].as_u64().unwrap() as usize;
        let gold = bytes(c["expected"].as_str().unwrap());
        let target = target.min(total);
        assert_eq!(s.duration(), Some(time(total)));
        assert_eq!(s.seek_to(target as i64), target as i64);
        let mut d = s.make_decoder().unwrap();
        let mut cursor = target;
        let mut out = Vec::new();
        while let Some(p) = s.next_packet().unwrap() {
            assert!(out.len() < 64, "fixture packet loop failed to progress");
            assert!(p.duration > 0);
            if avi_windows {
                assert!(
                    p.duration <= 2048,
                    "PCM output packet exceeds the frame window"
                );
            }
            assert_eq!(p.pts, cursor as i64);
            assert_eq!(p.data.len(), p.duration as usize * encoded);
            let pcm = d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap()
                .packet;
            let end = cursor + p.duration as u64;
            assert!(end <= total);
            assert_eq!(pcm.presentation_time(), time(cursor));
            assert_eq!(
                pcm.data,
                gold[cursor as usize * width..end as usize * width],
                "{}:frame{cursor}",
                c["file"]
            );
            out.push((time(cursor), pcm.data));
            cursor = end;
        }
        assert_eq!(cursor, total);
        out
    }
    #[test]
    fn independent_wav_controls_decode_literal_pcm_and_exact_seek() {
        video();
        for c in cases("cases") {
            for target in [
                0,
                3,
                c["frames"].as_u64().unwrap() - 1,
                c["frames"].as_u64().unwrap(),
                c["frames"].as_u64().unwrap() + 1,
            ] {
                let s = fvid::playback_wav::WavAudioReader::open(
                    Cursor::new(bytes(c["control"].as_str().unwrap())),
                    Default::default(),
                )
                .unwrap();
                read_pcm(Box::new(s), &c, target, false);
            }
        }
    }
    #[test]
    fn baseline_avi_pcm_is_an_explicit_framing_gap() {
        video();
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let error = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            ) {
                Ok(_) => panic!("{n}: baseline PCM profile changed"),
                Err(e) => e.to_string(),
            };
            assert!(error.contains("frames no records"), "{n}:{error}");
            eprintln!("{n}: existing unsupported PCM framing: {error}");
        }
    }
    #[test]
    fn malformed_profiles_remain_refused() {
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
    #[ignore = "enable with AVI linear PCM framing"]
    fn avi_pcm_has_literal_samples_and_bounded_packet_clocks() {
        video();
        for c in cases("cases") {
            let actual = read_pcm(Box::new(avi(&c, Default::default())), &c, 0, true);
            assert_eq!(actual, expected(&c, 0));
        }
    }
    #[test]
    #[ignore = "enable with AVI linear PCM framing"]
    fn avi_pcm_seeks_inside_records_and_rewinds_without_losing_frames() {
        for c in cases("cases") {
            let n = c["frames"].as_i64().unwrap();
            for requested in [
                -1,
                0,
                1,
                2,
                3,
                4,
                8,
                9,
                10,
                2047,
                2048,
                2049,
                n - 1,
                n,
                n + 1,
                i64::MAX,
            ] {
                let target = requested.clamp(0, n);
                let mut s = avi(&c, Default::default());
                assert_eq!(s.seek_to(requested), target);
                let actual = read_pcm(Box::new(s), &c, target as u64, true);
                assert_eq!(actual, expected(&c, target as u64));
            }
            let mut s = avi(&c, Default::default());
            s.seek_to(3);
            s.rewind();
            assert_eq!(read_pcm(Box::new(s), &c, 0, true), expected(&c, 0));
        }
    }
    #[test]
    #[ignore = "enable with AVI linear PCM framing"]
    fn avi_pcm_keeps_actual_interior_and_eof_worker_navigation() {
        video();
        for c in cases("cases") {
            let all = expected(&c, 0);
            let target = time(3);
            capture::worker_check_with_seek_expected(
                Box::new(avi(&c, Default::default())),
                &all,
                target,
                &expected(&c, 3),
            );
            capture::worker_check_with_seek(
                Box::new(avi(&c, Default::default())),
                &all,
                Some(time(c["frames"].as_u64().unwrap())),
            );
        }
    }
    #[test]
    #[ignore = "enable with AVI linear PCM framing"]
    fn large_pcm_chunks_stream_in_windows_and_keep_input_budgets() {
        for c in cases("cases") {
            let data = bytes(c["file"].as_str().unwrap());
            let records = c["parts"].as_array().unwrap().len();
            let exact = fvid::container::avi::Limits {
                file_bytes: data.len(),
                records,
                ..Default::default()
            };
            read_pcm(Box::new(avi(&c, exact)), &c, 0, true);
            let error = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(data.clone()),
                fvid::container::avi::Limits {
                    file_bytes: exact.file_bytes - 1,
                    ..exact
                },
            ) {
                Ok(_) => panic!("whole input byte limit bypassed"),
                Err(e) => e.to_string(),
            };
            assert!(error.contains("byte limit"), "{error}");
            let error = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(data),
                fvid::container::avi::Limits {
                    records: records - 1,
                    ..exact
                },
            ) {
                Ok(_) => panic!("raw record budget bypassed"),
                Err(e) => e.to_string(),
            };
            assert!(error.contains("records"), "{error}");
        }
    }
}
