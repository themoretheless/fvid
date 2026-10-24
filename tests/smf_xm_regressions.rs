#![cfg(feature = "player")]
use fvid::audio::{AudioPacket, AudioStream};
use fvid::container::smf::{Limits as SmfLimits, SmfFile};
use fvid::container::xm::{Limits as XmLimits, Module};
use fvid::playback_smf::SmfAudioReader;
use fvid::playback_xm::XmAudioReader;
use std::io::Cursor;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("smf-xm")
}

fn packet_bytes(len: usize) -> AudioPacket {
    AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 44_100,
        data: vec![0u8; len],
    }
}

#[test]
fn valid_smf_and_xm_fixtures_open_and_decode_packets() {
    let dir = fixture_dir();
    let valid_mid = std::fs::read(dir.join("valid.mid")).expect("valid.mid");
    let mut smf_reader =
        SmfAudioReader::open(Cursor::new(&valid_mid), SmfLimits::default()).expect("opens SMF");
    let first_smf = smf_reader
        .next_packet()
        .expect("read packet")
        .expect("packet 0");
    assert_eq!(first_smf.pts, 0);

    let valid_xm = std::fs::read(dir.join("valid-16bit.xm")).expect("valid-16bit.xm");
    let mut xm_reader =
        XmAudioReader::open(Cursor::new(&valid_xm), XmLimits::default()).expect("opens XM");
    assert_eq!(xm_reader.module().instruments[0].samples[0].frames.len(), 2);
    let first_xm = xm_reader
        .next_packet()
        .expect("read packet")
        .expect("packet 0");
    assert_eq!(first_xm.pts, 0);
}

#[test]
fn rejects_smf_missing_end_of_track() {
    let dir = fixture_dir();
    let bytes = std::fs::read(dir.join("missing-eot.mid")).expect("missing-eot.mid");
    let err = SmfFile::parse(&bytes, &SmfLimits::default())
        .expect_err("missing-eot.mid must be rejected");
    assert!(
        err.to_string().contains("end-of-track"),
        "missing-eot.mid: unexpected error: {err}"
    );
    assert!(SmfAudioReader::open(Cursor::new(bytes), SmfLimits::default()).is_err());
}

#[test]
fn xm_unpacked_cell_clamps_out_of_range_note_and_rejects_odd_16bit_sample_and_pattern_overrun() {
    let dir = fixture_dir();
    let packed_bytes = std::fs::read(dir.join("packed-note-120.xm")).expect("packed-note-120.xm");
    let unpacked_bytes =
        std::fs::read(dir.join("unpacked-note-120.xm")).expect("unpacked-note-120.xm");

    let packed = Module::parse(&packed_bytes, &XmLimits::default()).expect("packed module");
    let unpacked = Module::parse(&unpacked_bytes, &XmLimits::default()).expect("unpacked module");

    assert_eq!(
        unpacked.row_cells(&unpacked.timeline()[0]),
        packed.row_cells(&packed.timeline()[0]),
        "unpacked cell with out-of-range note > 97 must clamp to 0 just like packed cell"
    );
    assert_eq!(unpacked.row_cells(&unpacked.timeline()[0])[0].note, 0);

    let odd_bytes = std::fs::read(dir.join("odd-16bit-sample.xm")).expect("odd-16bit-sample.xm");
    let err = Module::parse(&odd_bytes, &XmLimits::default())
        .expect_err("odd-byte 16-bit sample length must be rejected");
    assert!(
        err.to_string().contains("16-bit") && err.to_string().contains("odd byte length"),
        "unexpected error: {err}"
    );
    assert!(XmAudioReader::open(Cursor::new(odd_bytes), XmLimits::default()).is_err());

    let overrun_bytes = std::fs::read(dir.join("pattern-overrun-into-instrument.xm"))
        .expect("pattern-overrun-into-instrument.xm");
    let overrun_err = Module::parse(&overrun_bytes, &XmLimits::default())
        .expect_err("pattern rows running past declared packed data must be rejected");
    assert!(
        overrun_err
            .to_string()
            .contains("rows run past the data its header states"),
        "unexpected error: {overrun_err}"
    );
    assert!(XmAudioReader::open(Cursor::new(overrun_bytes), XmLimits::default()).is_err());
}

#[test]
fn smf_and_xm_present_decoded_validate_negative_pts_and_pcm_frame_alignment() {
    let dir = fixture_dir();
    let valid_mid = std::fs::read(dir.join("valid.mid")).expect("valid.mid");
    let smf_reader =
        SmfAudioReader::open(Cursor::new(valid_mid), SmfLimits::default()).expect("opens SMF");

    let valid_xm = std::fs::read(dir.join("valid-16bit.xm")).expect("valid-16bit.xm");
    let xm_reader =
        XmAudioReader::open(Cursor::new(valid_xm), XmLimits::default()).expect("opens XM");

    assert!(smf_reader.present_decoded(packet_bytes(8), -1).is_err());
    assert!(xm_reader.present_decoded(packet_bytes(8), -1).is_err());

    assert!(smf_reader.present_decoded(packet_bytes(12), 0).is_err());
    assert!(xm_reader.present_decoded(packet_bytes(12), 0).is_err());

    let presented_smf = smf_reader
        .present_decoded(packet_bytes(8), 0)
        .expect("valid SMF present_decoded")
        .expect("some packet");
    assert_eq!(presented_smf.data.len(), 8);

    let presented_xm = xm_reader
        .present_decoded(packet_bytes(8), 0)
        .expect("valid XM present_decoded")
        .expect("some packet");
    assert_eq!(presented_xm.data.len(), 8);
}
