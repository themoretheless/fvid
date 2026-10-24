#![cfg(feature = "player")]

use fvid::audio::{AudioPacket, AudioStream};
use fvid::playback_aiff::{Aiff, AiffAudioReader, Limits as AiffLimits};
use fvid::playback_au::{AuAudioReader, Limits as AuLimits};
use fvid::playback_wav::{Limits as WavLimits, Wav, WavAudioReader};
use std::path::PathBuf;

fn fixture_bytes(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("wav-aiff-au")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()))
}

#[test]
fn wav_extensible_rejects_channel_mask_popcount_exceeding_channels_and_oversized_cbsize() {
    let mask_overflow = fixture_bytes("wav_extensible_channel_mask_overflow.wav");
    let err = Wav::parse(&mask_overflow, &WavLimits::default())
        .expect_err("channel mask naming 2 speakers for a 1-channel track must be rejected");
    assert!(
        err.to_string().contains("channel mask"),
        "unexpected error for channel mask overflow: {err}"
    );

    let cbsize_overflow = fixture_bytes("wav_extensible_cbsize_overflow.wav");
    let err = Wav::parse(&cbsize_overflow, &WavLimits::default())
        .expect_err("cbSize exceeding fmt chunk body must be rejected");
    assert!(
        err.to_string().contains("extensible fmt chunk"),
        "unexpected error for cbSize overflow: {err}"
    );

    let valid = fixture_bytes("wav_extensible_valid.wav");
    let mut reader = WavAudioReader::open(&valid[..], WavLimits::default())
        .expect("valid extensible WAV should open");
    assert_eq!(reader.channels(), 1);
    assert_eq!(reader.sample_rate(), 48_000);
    assert_eq!(reader.bits_per_sample(), 16);
    let pkt = reader
        .next_packet()
        .expect("read packet")
        .expect("one packet");
    assert_eq!((pkt.pts, pkt.duration), (0, 4));
}

#[test]
fn aifc_g711_rejects_non_8bit_sample_width_and_accepts_8bit() {
    for name in [
        "aifc_alaw_16bit_invalid.aifc",
        "aifc_ulaw_24bit_invalid.aifc",
    ] {
        let bytes = fixture_bytes(name);
        let err = Aiff::parse(&bytes, &AiffLimits::default())
            .expect_err("AIFC G.711 with non-8-bit sample width must be rejected");
        assert!(
            err.to_string().contains("G.711"),
            "{name}: unexpected error: {err}"
        );
    }

    let valid = fixture_bytes("aifc_alaw_8bit_valid.aifc");
    let mut reader = AiffAudioReader::open(&valid[..], AiffLimits::default())
        .expect("valid 8-bit AIFC alaw should open");
    assert_eq!(reader.codec(), "pcm_alaw");
    assert_eq!(reader.bits_per_sample(), 8);
    let pkt = reader
        .next_packet()
        .expect("read packet")
        .expect("one packet");
    assert_eq!((pkt.pts, pkt.duration, pkt.data.len()), (0, 4, 4));
}

#[test]
fn au_audio_reader_present_decoded_validates_geometry_stamps_timeline_and_trims_floor() {
    let valid = fixture_bytes("au_pcm16_valid.au");
    let mut reader =
        AuAudioReader::open(&valid[..], AuLimits::default()).expect("valid AU should open");
    assert_eq!(reader.sample_rate(), 48_000);
    assert_eq!(reader.channels(), 1);

    let make_packet = |samples: &[f32]| AudioPacket {
        pts: 0,
        timebase_num: 0,
        timebase_den: 0,
        data: samples.iter().flat_map(|s| s.to_le_bytes()).collect(),
    };

    // 1. Negative source_pts must be rejected.
    let err = reader
        .present_decoded(make_packet(&[0.25, -0.25]), -1)
        .err()
        .expect("negative source_pts must be rejected");
    assert!(
        err.to_string().contains("precedes its source timeline"),
        "unexpected error for negative source_pts: {err}"
    );

    // 2. Unaligned decoded payload byte length must be rejected.
    let bad_geom = AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 48_000,
        data: vec![0u8; 6],
    };
    let err = reader
        .present_decoded(bad_geom, 0)
        .err()
        .expect("unaligned decoded audio packet must be rejected");
    assert!(
        err.to_string().contains("invalid decoded audio geometry"),
        "unexpected error for unaligned decoded payload: {err}"
    );

    // 3. Seek sets presentation_floor; present_decoded stamps timebase and trims pre-floor frames.
    assert_eq!(reader.seek_to(2), 2);
    let trimmed = reader
        .present_decoded(make_packet(&[0.1, 0.2, 0.3, 0.4]), 0)
        .expect("present_decoded succeeds")
        .expect("two frames remain after floor trim");
    assert_eq!(trimmed.pts, 2);
    assert_eq!((trimmed.timebase_num, trimmed.timebase_den), (1, 48_000));
    let decoded_samples: Vec<f32> = trimmed
        .data
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    assert_eq!(decoded_samples, vec![0.3, 0.4]);

    // 4. Packet entirely before presentation_floor is suppressed (`Ok(None)`).
    let suppressed = reader
        .present_decoded(make_packet(&[0.1, 0.2]), 0)
        .expect("present_decoded succeeds");
    assert!(
        suppressed.is_none(),
        "frames prior to floor must be suppressed"
    );
}
