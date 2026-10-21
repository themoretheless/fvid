use std::path::PathBuf;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/aifc-compression-type")
            .join(n),
    )
    .unwrap()
}
fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn faults_isolate_the_missing_compression_type_with_complete_sound_data() {
    for c in cases("cases") {
        let b = bytes(c["file"].as_str().unwrap());
        assert_eq!(&b[..4], b"FORM");
        assert_eq!(&b[8..12], b"AIFC");
        assert_eq!(
            u32::from_be_bytes(b[4..8].try_into().unwrap()) as usize + 8,
            b.len()
        );
        assert_eq!(&b[12..24], b"FVER\0\0\0\x04\xa2\x80\x51\x40");
        let at = 24;
        assert_eq!(&b[at..at + 4], b"COMM");
        let n = u32::from_be_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        assert_eq!(n, c["comm_bytes"].as_u64().unwrap() as usize);
        assert!((18..22).contains(&n));
        let next = at + 8 + n + (n & 1);
        assert_eq!(&b[next..next + 4], b"SSND");
        let size = u32::from_be_bytes(b[next + 4..next + 8].try_into().unwrap()) as usize;
        assert_eq!(
            size,
            8 + 9 * c["channels"].as_u64().unwrap() as usize * c["bits"].as_u64().unwrap() as usize
                / 8
        );
        assert_eq!(b[next + 8..next + 16], [0; 8]);
        let control = bytes(c["control"].as_str().unwrap());
        let reference = control.windows(4).position(|p| p == b"SSND").unwrap();
        assert_eq!(
            &b[next + 16..next + 8 + size],
            &control[reference + 16..reference + 8 + size]
        );
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
    fn time(n: u64) -> Duration {
        Duration::from_nanos(n * 125000)
    }
    fn source(c: &serde_json::Value) -> fvid::playback_aiff::AiffAudioReader {
        fvid::playback_aiff::AiffAudioReader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .unwrap()
    }
    fn consume(mut s: fvid::playback_aiff::AiffAudioReader, start: u64) -> Vec<u8> {
        let mut d = s.make_decoder().unwrap();
        let mut cursor = start;
        let mut out = Vec::new();
        assert_eq!(s.duration(), Some(time(9)));
        while let Some(p) = s.next_packet().unwrap() {
            assert_eq!(p.pts, cursor as i64);
            assert!(p.duration > 0);
            let pcm = d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap()
                .packet;
            assert_eq!(pcm.presentation_time(), time(cursor));
            cursor += p.duration as u64;
            out.extend(pcm.data);
        }
        assert_eq!(cursor, 9);
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
    #[test]
    fn complete_aiff_and_aifc_controls_keep_numeric_pcm_and_actual_navigation() {
        video();
        for c in cases("controls") {
            let gold = bytes(c["expected"].as_str().unwrap());
            let stride = c["channels"].as_u64().unwrap() as usize * 4;
            assert_eq!(consume(source(&c), 0), gold);
            for requested in [-1, 0, 1, 3, 8, 9, 10, i64::MAX] {
                let mut s = source(&c);
                let target = requested.clamp(0, 9);
                assert_eq!(s.seek_to(requested), target);
                assert_eq!(consume(s, target as u64), gold[target as usize * stride..]);
            }
            let mut s = source(&c);
            s.seek_to(3);
            s.rewind();
            assert_eq!(consume(s, 0), gold);
            let all = vec![(Duration::ZERO, gold.clone())];
            capture::worker_check_with_seek_expected(
                Box::new(source(&c)),
                &all,
                time(3),
                &[(time(3), gold[3 * stride..].to_vec())],
            );
            capture::worker_check_with_seek(Box::new(source(&c)), &all, Some(time(9)));
        }
    }
    #[test]
    fn incomplete_aifc_type_refuses_at_header_parsing() {
        for c in cases("cases") {
            let b = bytes(c["file"].as_str().unwrap());
            let e = fvid::playback_aiff::Aiff::parse(&b, &Default::default())
                .expect_err("incomplete AIFC type guessed as NONE");
            assert!(matches!(e, fvid::Error::Invalid(_)));
            assert!(
                e.to_string()
                    .contains("AIFC COMM chunk has no complete compression type"),
                "{e}"
            );
        }
    }
    #[test]
    fn incomplete_aifc_type_never_reaches_decoder_or_worker_admission() {
        for c in cases("cases") {
            let e = match fvid::playback_aiff::AiffAudioReader::open(
                Cursor::new(bytes(c["file"].as_str().unwrap())),
                Default::default(),
            ) {
                Ok(_) => panic!("incomplete AIFC type admitted as PCM"),
                Err(e) => e,
            };
            assert!(matches!(e, fvid::Error::Invalid(_)));
            assert!(
                e.to_string()
                    .contains("AIFC COMM chunk has no complete compression type"),
                "{e}"
            );
        }
    }
    #[test]
    fn complete_controls_keep_file_data_and_channel_budgets() {
        for c in cases("controls") {
            let b = bytes(c["file"].as_str().unwrap());
            let data_bytes =
                9 * c["channels"].as_u64().unwrap() as usize * c["bits"].as_u64().unwrap() as usize
                    / 8;
            let exact = fvid::playback_aiff::Limits {
                file_bytes: b.len(),
                data_bytes,
                channels: c["channels"].as_u64().unwrap() as usize,
            };
            assert!(fvid::playback_aiff::Aiff::parse(&b, &exact).is_ok());
            for limits in [
                fvid::playback_aiff::Limits {
                    file_bytes: b.len() - 1,
                    ..exact
                },
                fvid::playback_aiff::Limits {
                    data_bytes: data_bytes - 1,
                    ..exact
                },
                fvid::playback_aiff::Limits {
                    channels: exact.channels - 1,
                    ..exact
                },
            ] {
                assert!(fvid::playback_aiff::Aiff::parse(&b, &limits).is_err());
            }
        }
    }
}
