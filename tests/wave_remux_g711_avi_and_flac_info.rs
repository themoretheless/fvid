use std::path::PathBuf;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-remux-g711-avi-and-flac-info")
        .join(name)
}

#[test]
fn wave_concat_rejects_classic_stereo_and_extensible_directout_mismatch() {
    let classic = fixture_path("classic_stereo_pcm16.wav");
    let directout = fixture_path("extensible_directout_stereo_pcm16.wav");
    let err = fvid_media::plan_concat(
        &[classic, directout],
        &fvid_control::CopyOptions::default(),
    )
    .expect_err("classic stereo (implicit 0x3) and extensible DirectOut (explicit 0x0) must not concat");
    assert!(
        err.contains("incompatible PCM formats/layouts"),
        "unexpected error: {err}"
    );
}

#[test]
fn extensible_g711_directout_preserves_zero_channel_mask_in_decode_plan_and_export() {
    let source = fixture_path("extensible_directout_alaw.wav");
    let options = fvid_control::CopyOptions::default();
    let transform = fvid_media_info::AudioDecodeTransform::default();
    let plan = fvid_media::plan_decode_audio(&source, &transform, &options)
        .expect("extensible DirectOut A-law decode plan should succeed");
    assert!(
        plan.steps[0].detail.contains("preserve speaker mask 0x0"),
        "expected speaker mask 0x0 in plan detail, got: {}",
        plan.steps[0].detail
    );

    let dir = std::env::temp_dir().join(format!(
        "fvid-batch578-g711-directout-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let output = dir.join("decoded.wav");
    let stats = fvid_media::decode_audio_transformed(&source, &output, transform, &options)
        .expect("extensible DirectOut A-law decode should succeed");
    assert_eq!(stats.sample_frames, 4);
    assert_eq!(stats.channels, 2);

    let mut file = std::fs::File::open(&output).unwrap();
    let info = fvid_media::owned_wave_inspect::inspect(&mut file, None)
        .expect("decoded WAVE should inspect cleanly");
    assert_eq!(info.channel_mask, 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extensible_g711_rejects_valid_bits_not_equal_to_eight() {
    let invalid_valid_bits = fixture_path("extensible_alaw_invalid_valid_bits.wav");
    let options = fvid_control::CopyOptions::default();
    let transform = fvid_media_info::AudioDecodeTransform::default();
    let err = fvid_media::plan_decode_audio(&invalid_valid_bits, &transform, &options)
        .expect_err("WAVEFORMATEXTENSIBLE G.711 with wValidBitsPerSample != 8 must be rejected");
    assert!(
        err.contains("G.711 WAVE format disagrees with its code width"),
        "unexpected error: {err}"
    );
}

#[test]
fn avi_audio_reader_accepts_wave_format_extensible_g711() {
    use fvid::audio::AudioStream;
    let path = fixture_path("avi_extensible_alaw.avi");
    let bytes = std::fs::read(&path).unwrap();
    let mut reader = fvid::playback_avi_audio::AviAudioReader::open(
        &bytes[..],
        fvid::container::avi::Limits::default(),
    )
    .expect("AVI with WAVEFORMATEXTENSIBLE A-law should open");
    assert_eq!(reader.codec(), "pcm_alaw");
    assert_eq!(reader.sample_rate(), 8000);
    assert_eq!(reader.channels(), 2);
    assert_eq!(reader.bits_per_sample(), 8);
    assert_eq!(reader.speaker_mask().unwrap(), Some(0x3));
    let packet = reader.next_packet().unwrap().expect("expected packet");
    assert_eq!(packet.pts, 0);
    assert_eq!(packet.duration, 4);
    assert_eq!(packet.data.len(), 8);
}

#[test]
fn flac_wave_info_from_extra_skips_empty_vorbis_comments_before_nonempty_tags() {
    let raw = std::fs::read(fixture_path("flac_vorbis_comment_empty_and_valid.bin")).unwrap();
    let info_chunks = fvid_media::owned_flac_layout::wave_info_from_extra(&raw)
        .expect("wave_info_from_extra should succeed");
    assert!(
        info_chunks
            .windows(b"INAM\x11\x00\x00\x00Valid Song Title\0".len())
            .any(|w| w == b"INAM\x11\x00\x00\x00Valid Song Title\0"),
        "expected INAM with 'Valid Song Title', got: {info_chunks:?}"
    );
    assert!(
        info_chunks
            .windows(b"ICRD\x05\x00\x00\x002026\0".len())
            .any(|w| w == b"ICRD\x05\x00\x00\x002026\0"),
        "expected ICRD with '2026', got: {info_chunks:?}"
    );
}
