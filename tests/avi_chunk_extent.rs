use std::path::PathBuf;
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avi-chunk-extent")
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
fn geometry(c: &serde_json::Value) {
    let b = bytes(c["file"].as_str().unwrap());
    let at = c["record_offset"].as_u64().unwrap() as usize;
    assert_eq!(&b[at..at + 4], b"01wb");
    assert_eq!(
        u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as u64,
        c["claimed"].as_u64().unwrap()
    );
    assert_eq!(b.len() - at - 8, c["available"].as_u64().unwrap() as usize);
    assert!(c["claimed"].as_u64().unwrap() > c["available"].as_u64().unwrap());
    let control = bytes(c["control"].as_str().unwrap());
    assert_eq!(b.len(), control.len());
    assert_eq!(&b[..at + 4], &control[..at + 4]);
    assert_eq!(&b[at + 8..], &control[at + 8..]);
}
#[test]
fn complete_audio_records_and_outer_sentinels_keep_their_geometry() {
    for c in cases("controls") {
        let b = bytes(c["file"].as_str().unwrap());
        let p = fvid::container::avi::Avi::parse(&b, &Default::default()).unwrap();
        assert_eq!(p.audio()[0].records.len(), 3);
        assert_eq!(
            p.audio()[0].declared,
            if c["file"].as_str().unwrap().starts_with("ima")
                || c["file"].as_str().unwrap().starts_with("ms")
            {
                3
            } else {
                9
            }
        );
    }
}
#[test]
fn known_audio_body_cannot_escape_its_enclosing_run() {
    for c in cases("cases") {
        geometry(&c);
        let b = bytes(c["file"].as_str().unwrap());
        let e = fvid::container::avi::Avi::parse(&b, &Default::default())
            .expect_err("truncated audio body was silently clipped");
        assert!(matches!(e, fvid::Error::Invalid(_)));
        assert!(
            e.to_string()
                .contains("audio record extends past its enclosing run"),
            "{e}"
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
    use std::io::Cursor;
    fn source(n: &str) -> fvid::playback_avi_audio::AviAudioReader {
        fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap()
    }
    fn packets(n: &str) -> Vec<(std::time::Duration, Vec<u8>)> {
        let mut s = source(n);
        let mut d = s.make_decoder().unwrap();
        let mut out = Vec::new();
        let mut cursor = 0;
        while let Some(p) = s.next_packet().unwrap() {
            assert_eq!(p.pts, cursor);
            assert!(p.duration > 0);
            let at = s.time_of(p.pts);
            let pcm = d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .unwrap()
                .packet;
            assert_eq!(pcm.presentation_time(), at);
            cursor += p.duration;
            out.push((at, pcm.data));
        }
        assert_eq!(s.duration(), Some(s.time_of(cursor)));
        out
    }
    fn literal(c: &serde_json::Value) -> Vec<(std::time::Duration, Vec<u8>)> {
        let out = packets(c["file"].as_str().unwrap());
        assert_eq!(
            out.iter()
                .flat_map(|(_, p)| p.iter().copied())
                .collect::<Vec<_>>(),
            bytes(c["expected"].as_str().unwrap())
        );
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
    fn complete_controls_keep_literal_pcm_actual_seek_rewind_and_eof() {
        video();
        for c in cases("controls") {
            let n = c["file"].as_str().unwrap();
            let expected = literal(&c);
            let s = source(n);
            assert_eq!(s.duration(), Some(s.time_of(c["frames"].as_i64().unwrap())));
            capture::worker_check_with_seek(
                Box::new(source(n)),
                &expected,
                Some(s.time_of(c["seek"].as_i64().unwrap())),
            );
            capture::worker_check_with_seek(
                Box::new(source(n)),
                &expected,
                Some(s.time_of(c["frames"].as_i64().unwrap())),
            );
        }
    }
    #[test]
    fn truncated_chunks_are_refused_before_decoder_or_worker_admission() {
        for c in cases("cases") {
            let n = c["file"].as_str().unwrap();
            let e = match fvid::playback_avi_audio::AviAudioReader::open(
                Cursor::new(bytes(n)),
                Default::default(),
            ) {
                Ok(_) => panic!("{n}:clipped corruption admitted as complete audio"),
                Err(e) => e,
            };
            assert!(matches!(e, fvid::Error::Invalid(_)));
            assert!(
                e.to_string()
                    .contains("audio record extends past its enclosing run"),
                "{n}:{e}"
            );
        }
    }
}
