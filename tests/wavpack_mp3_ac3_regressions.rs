#![cfg(feature = "player")]
use fvid::audio::{AudioPacket, AudioStream};
use fvid::playback_ac3::{Ac3AudioReader, Limits as Ac3Limits};
use fvid::playback_mp3::{Limits as Mp3Limits, Mp3AudioReader};
use fvid::playback_wavpack::{Limits as WvLimits, WavpackAudioReader};
use std::io::Cursor;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("wavpack-mp3-ac3")
}

fn packet_bytes(len: usize) -> AudioPacket {
    AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 48_000,
        data: vec![0u8; len],
    }
}

#[test]
fn wavpack_rejects_malformed_ape_tags_and_trailing_subblock_bytes() {
    let dir = fixture_dir();
    let valid_bytes = std::fs::read(dir.join("valid.wv")).expect("valid.wv");
    let mut valid_reader =
        WavpackAudioReader::open(Cursor::new(valid_bytes), WvLimits::default()).expect("opens");
    let pkt = valid_reader
        .next_packet()
        .expect("first packet")
        .expect("some packet");
    assert_eq!(pkt.pts, 0);
    assert!(valid_reader.next_packet().expect("eof").is_none());
    valid_reader.validate_eof().expect("clean eof");

    for (file, expected) in [
        ("ape-header-no-flag.wv", "APE header/footer mismatch"),
        ("ape-footer-no-footer-flag.wv", "invalid WavPack APE footer"),
        ("trailing-subblock-byte.wv", "metadata"),
    ] {
        let bytes = std::fs::read(dir.join(file)).expect(file);
        let err = match WavpackAudioReader::open(Cursor::new(bytes), WvLimits::default()) {
            Err(e) => e.to_string(),
            Ok(mut reader) => loop {
                match reader.next_packet() {
                    Err(e) => break e.to_string(),
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("{file} must be rejected"),
                }
            },
        };
        assert!(
            err.contains(expected),
            "{file}: unexpected error message: {err}"
        );
    }
}

#[test]
fn mp3_and_ac3_open_and_present_decoded_guards() {
    let mp3_bytes = include_bytes!("../tests/fixtures/mp3/layer-ii.mp2");
    let mp3_reader =
        Mp3AudioReader::open(Cursor::new(&mp3_bytes[..]), Mp3Limits::default()).expect("opens MP2");

    // Even when presentation_floor is None and gapless is None, present_decoded must reject
    // negative source_pts and unaligned PCM frames, and stamp pts/timebase on valid packets.
    assert!(mp3_reader.present_decoded(packet_bytes(4), -1).is_err());
    assert!(mp3_reader.present_decoded(packet_bytes(6), 0).is_err());
    let presented = mp3_reader
        .present_decoded(
            AudioPacket {
                pts: 999,
                timebase_num: 9,
                timebase_den: 9,
                data: vec![0u8; 4],
            },
            1_152,
        )
        .expect("valid present_decoded")
        .expect("some packet");
    assert_eq!(presented.pts, 1_152);
    assert_eq!((presented.timebase_num, presented.timebase_den), (1, 48_000));

    let ac3_bytes = include_bytes!("../tests/fixtures/audio/ac3-stereo.ac3");
    let tight_ac3 = Ac3Limits {
        file_bytes: ac3_bytes.len() - 1,
        ..Ac3Limits::default()
    };
    assert!(Ac3AudioReader::open(Cursor::new(&ac3_bytes[..]), tight_ac3).is_err());
    let ac3_reader =
        Ac3AudioReader::open(Cursor::new(&ac3_bytes[..]), Ac3Limits::default()).expect("opens AC3");
    assert!(ac3_reader.present_decoded(packet_bytes(8), -1).is_err());
    assert!(ac3_reader.present_decoded(packet_bytes(12), 0).is_err());
}
