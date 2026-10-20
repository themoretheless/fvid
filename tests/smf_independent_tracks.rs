#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn decoded(bytes: Vec<u8>) -> Vec<(Duration, Vec<u8>)> {
    let mut source =
        fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default()).unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut pcm = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        let f = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        pcm.push((f.packet.presentation_time(), f.packet.data));
    }
    pcm
}
#[test]
fn independent_midi_tracks_do_not_silently_play_as_one_simultaneous_score() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/smf-independent-tracks");
    let mut video = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(base.join("video.mkv")).unwrap()),
        32 << 20,
    )
    .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    let single = decoded(std::fs::read(base.join("single0.mid")).unwrap());
    let mixed = decoded(std::fs::read(base.join("simultaneous.mid")).unwrap());
    assert_ne!(single, mixed);
    for name in [
        "single0.mid",
        "single1.mid",
        "single2.mid",
        "simultaneous.mid",
    ] {
        let bytes = std::fs::read(base.join(name)).unwrap();
        let expected = if name == "simultaneous.mid" {
            &mixed
        } else {
            &single
        };
        assert_eq!(&decoded(bytes.clone()), expected);
        let source =
            fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap();
        let end = source.duration().unwrap();
        capture::worker_check_with_seek(Box::new(source), expected, Some(end));
    }
    let bytes = std::fs::read(base.join("independent.mid")).unwrap();
    match fvid::container::smf::SmfFile::parse(&bytes, &Default::default()) {
        Err(e) => {
            assert!(
                e.to_string()
                    .contains("independent tracks are not supported"),
                "{e}"
            );
            assert!(fvid::playback_smf::SmfAudioReader::open(
                Cursor::new(bytes),
                Default::default()
            )
            .is_err());
        }
        Ok(file) => {
            assert_eq!((file.format, file.track_count), (2, 2));
            assert_eq!(
                decoded(bytes.clone()),
                mixed,
                "specific defect is simultaneous mixing"
            );
            let source =
                fvid::playback_smf::SmfAudioReader::open(Cursor::new(bytes), Default::default())
                    .unwrap();
            assert_eq!(source.audio_tracks().len(), 1);
            let end = source.duration().unwrap();
            capture::worker_check_with_seek(Box::new(source), &mixed, Some(end));
            panic!("format2 independent tracks silently merged into one simultaneous native400ms audio track");
        }
    }
}
