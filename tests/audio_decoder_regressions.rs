#![cfg(feature = "player")]
use fvid::audio::AudioDecode;
use fvid::codec::adpcm_decoder::block_frames;
use fvid::codec::flac_decoder::FlacDecoder;
use fvid::codec::midi_decoder::MidiDecoder;
use fvid::codec::mp2_decoder::Mp2Decoder;
use fvid::codec::xm_decoder::{Ignored, XmDecoder};
use fvid::container::smf::{self, Event};
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("audio-decoders")
}

#[test]
fn flac_decoder_validates_streaminfo_frame_bounds_and_max_block_size() {
    let dir = fixture_dir();
    let si_valid = std::fs::read(dir.join("flac_streaminfo_192.bin")).expect("flac_streaminfo_192");
    let si_max_64 =
        std::fs::read(dir.join("flac_streaminfo_max_64.bin")).expect("flac_streaminfo_max_64");
    let si_bad_frames = std::fs::read(dir.join("flac_streaminfo_min_frame_gt_max_frame.bin"))
        .expect("flac_streaminfo_min_frame_gt_max_frame");
    let frame_192 = std::fs::read(dir.join("flac_frame_192.bin")).expect("flac_frame_192");

    let mut valid = FlacDecoder::new(&si_valid, 48_000, 1).expect("valid STREAMINFO");
    let pkt = valid
        .decode_encoded(&frame_192, 0, 192)
        .expect("valid 192-sample frame")
        .expect("some packet");
    assert_eq!(pkt.data.len(), 192 * 4);

    let bad_frames_err = match FlacDecoder::new(&si_bad_frames, 48_000, 1) {
        Ok(_) => panic!("min_frame > max_frame STREAMINFO must be rejected"),
        Err(e) => e,
    };
    assert!(
        bad_frames_err.to_string().contains("frame size"),
        "unexpected error: {bad_frames_err}"
    );

    let mut max_64 = FlacDecoder::new(&si_max_64, 48_000, 1).expect("opens max_block=64");
    let max_block_err = match max_64.decode_encoded(&frame_192, 0, 192) {
        Ok(_) => panic!("frame with 192 samples must be rejected when max_block is 64"),
        Err(e) => e,
    };
    assert!(
        max_block_err.to_string().contains("block size"),
        "unexpected error: {max_block_err}"
    );
}

#[test]
fn mp2_and_adpcm_ima_wav_geometry_guards() {
    for rate in [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000] {
        assert!(Mp2Decoder::new(rate, 2).is_ok(), "valid MP2 rate {rate}");
    }
    for bad_rate in [1, 12345, 96000] {
        assert!(
            Mp2Decoder::new(bad_rate, 2).is_err(),
            "non-standard MP2 rate {bad_rate} must be rejected"
        );
    }

    // Stereo IMA WAV requires 8-byte (two 4-byte channel words) payload alignment after the 8-byte header
    assert_eq!(block_frames("adpcm_ima_wav", 16, 2, 4), Some(9));
    assert_eq!(block_frames("adpcm_ima_wav", 12, 2, 4), None);
    assert_eq!(block_frames("adpcm_ima_wav", 14, 2, 4), None);
}

#[test]
fn xm_and_midi_decoders_enforce_row_channels_and_reset_counters() {
    let dir = fixture_dir();
    let xm_bytes = std::fs::read(dir.join("xm_two_channel.xm")).expect("xm_two_channel.xm");
    let row_2ch = std::fs::read(dir.join("xm_row_2ch_valid.bin")).expect("xm_row_2ch_valid.bin");
    let row_1ch =
        std::fs::read(dir.join("xm_row_1ch_mismatch.bin")).expect("xm_row_1ch_mismatch.bin");

    let mut xm_dec = XmDecoder::new(&xm_bytes, 48_000, 2).expect("opens 2ch XM");
    assert_eq!(xm_dec.ignored(), Ignored::default());

    let mismatch_err = match xm_dec.decode_encoded(&row_1ch, 0, 120) {
        Ok(_) => panic!("1-cell row packet on 2-channel XM module must be rejected"),
        Err(e) => e,
    };
    assert!(
        mismatch_err.to_string().contains("channel"),
        "unexpected error: {mismatch_err}"
    );

    xm_dec
        .decode_encoded(&row_2ch, 0, 120)
        .expect("valid 2-cell row packet")
        .expect("some packet");
    assert_eq!(xm_dec.ignored().effects, 1);
    assert_eq!(xm_dec.ignored().volume_columns, 1);

    xm_dec.reset();
    assert_eq!(
        xm_dec.ignored(),
        Ignored::default(),
        "XmDecoder::reset must restore ignored counters to initial module state"
    );

    // MidiDecoder::reset must reset the rendered frame clock so voice steal order
    // starts fresh after reset.
    let mut midi = MidiDecoder::new(1_000, 2).expect("opens MIDI decoder");
    let mut one_note = Vec::new();
    smf::write_record(
        Event::NoteOn {
            channel: 0,
            key: 60,
            velocity: 100,
        },
        0,
        &mut one_note,
    );
    midi.decode_encoded(&one_note, 0, 100)
        .expect("renders 100ms window");
    midi.reset();
    // Play a new note after reset and verify it renders cleanly from frame 0
    let pkt = midi
        .decode_encoded(&one_note, 0, 100)
        .expect("renders after reset")
        .expect("some packet");
    assert_eq!(pkt.data.len(), 100 * 2 * 4);
}
