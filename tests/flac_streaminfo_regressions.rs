use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/flac-streaminfo")
        .join(name)
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixture_path(name)).expect("read FLAC regression fixture")
}

#[test]
fn flac_streaminfo_and_metadata_ordering_validation() {
    let valid = fvid_media::owned_flac::Flac::parse(
        &fixture("valid_baseline.flac"),
        &fvid_media::owned_flac::Limits::default(),
    )
    .expect("valid baseline FLAC must parse");
    assert_eq!(valid.sample_rate, 48_000);
    assert_eq!(valid.channels, 1);
    assert_eq!(valid.bits_per_sample, 16);
    assert_eq!(valid.channel_mask, 0x04);
    assert_eq!(valid.samples(), 192);

    let pad_first = fvid_media::owned_flac::Flac::parse(
        &fixture("padding_before_streaminfo.flac"),
        &fvid_media::owned_flac::Limits::default(),
    )
    .expect_err("non-STREAMINFO first metadata block must be rejected");
    assert!(
        pad_first.to_string().contains("STREAMINFO"),
        "unexpected error for padding_before_streaminfo: {pad_first}"
    );

    let duplicate = fvid_media::owned_flac::Flac::parse(
        &fixture("duplicate_streaminfo.flac"),
        &fvid_media::owned_flac::Limits::default(),
    )
    .expect_err("duplicate STREAMINFO block must be rejected");
    assert!(
        duplicate.to_string().contains("STREAMINFO"),
        "unexpected error for duplicate_streaminfo: {duplicate}"
    );

    let overlong = fvid_media::owned_flac::Flac::parse(
        &fixture("overlong_streaminfo.flac"),
        &fvid_media::owned_flac::Limits::default(),
    )
    .expect_err("STREAMINFO with length != 34 must be rejected");
    assert!(
        overlong.to_string().contains("STREAMINFO holds 36 bytes"),
        "unexpected error for overlong_streaminfo: {overlong}"
    );

    for name in [
        "reserved_metadata_type_7.flac",
        "reserved_metadata_type_127.flac",
    ] {
        let err = fvid_media::owned_flac::Flac::parse(
            &fixture(name),
            &fvid_media::owned_flac::Limits::default(),
        )
        .expect_err("reserved metadata block type must be rejected");
        assert!(
            err.to_string().contains("metadata block type"),
            "unexpected error for {name}: {err}"
        );
    }

    for name in ["min_block_below_16.flac", "min_block_exceeds_max_block.flac"] {
        let err = fvid_media::owned_flac::Flac::parse(
            &fixture(name),
            &fvid_media::owned_flac::Limits::default(),
        )
        .expect_err("invalid STREAMINFO block size bounds must be rejected");
        assert!(
            err.to_string().contains("block size"),
            "unexpected error for {name}: {err}"
        );
    }

    let max_frame_err = fvid_media::owned_flac::Flac::parse(
        &fixture("max_frame_mismatch_when_min_zero.flac"),
        &fvid_media::owned_flac::Limits::default(),
    )
    .expect_err("non-zero max_frame disagreeing with frames when min_frame == 0 must be rejected");
    assert!(
        max_frame_err.to_string().contains("frame sizes"),
        "unexpected error for max_frame_mismatch_when_min_zero: {max_frame_err}"
    );

    let mask_err = fvid_media::owned_flac::Flac::parse(
        &fixture("channel_mask_popcount_overflow.flac"),
        &fvid_media::owned_flac::Limits::default(),
    )
    .expect_err("Vorbis comment channel mask exceeding channel count must be rejected");
    assert!(
        mask_err.to_string().contains("channel mask"),
        "unexpected error for channel_mask_popcount_overflow: {mask_err}"
    );
}

#[test]
fn matroska_flac_channel_mask_popcount_validation() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-flac-streaminfo-test-{}",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&dir);
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());

    let valid_mkv = fixture_path("matroska_flac_valid.mkv");
    let out_wav = dir.join("valid.wav");
    fvid_media::plan_decode_audio(&valid_mkv, &Default::default(), &Default::default())
        .expect("valid Matroska FLAC plan_decode_audio must succeed");
    fvid_media::decode_audio(&valid_mkv, &out_wav, &Default::default())
        .expect("valid Matroska FLAC decode_audio must succeed");

    let bad_mkv = fixture_path("matroska_flac_mask_popcount_overflow.mkv");
    let bad_wav = dir.join("bad.wav");
    let plan_err =
        fvid_media::plan_decode_audio(&bad_mkv, &Default::default(), &Default::default())
            .expect_err("Matroska FLAC with mask popcount > channels must fail plan_decode_audio");
    assert!(
        plan_err.contains("channel mask"),
        "unexpected plan_decode_audio error: {plan_err}"
    );
    let decode_err = fvid_media::decode_audio(&bad_mkv, &bad_wav, &Default::default())
        .expect_err("Matroska FLAC with mask popcount > channels must fail decode_audio");
    assert!(
        decode_err.contains("channel mask"),
        "unexpected decode_audio error: {decode_err}"
    );

    #[cfg(feature = "player")]
    {
        use fvid::audio::AudioStream;
        use std::io::Cursor;

        let valid_reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(fixture("matroska_flac_valid.mkv")),
            Default::default(),
        )
        .expect("open valid Matroska FLAC");
        assert_eq!(valid_reader.speaker_mask().expect("valid mask"), Some(0x04));

        let bad_reader = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(fixture("matroska_flac_mask_popcount_overflow.mkv")),
            Default::default(),
        )
        .expect("open Matroska FLAC");
        let speaker_err = bad_reader
            .speaker_mask()
            .expect_err("WebmAudioReader::speaker_mask must reject mask popcount > channels");
        assert!(
            speaker_err.to_string().contains("channel mask"),
            "unexpected WebmAudioReader::speaker_mask error: {speaker_err}"
        );
    }
}
