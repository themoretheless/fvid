use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aiff-avi-ffv1-and-mkv-probe")
        .join(name)
}

#[test]
fn aiff_rejects_block_aligned_ssnd_when_declared_frames_is_zero() {
    let bytes = std::fs::read(fixture_path("aiff_block_aligned_zero_frames.aiff")).unwrap();
    let err = fvid::playback_aiff::Aiff::parse(&bytes, &fvid::playback_aiff::Limits::default())
        .expect_err("zero declared_frames with block-aligned SSND padding must be rejected");
    assert!(
        err.to_string().contains("no samples"),
        "unexpected error: {err}"
    );
}

#[test]
fn avi_audio_reader_supports_sub_sample_seek_time_and_present_decoded_trimming() {
    use fvid::audio::{AudioPacket, AudioStream};

    let bytes = std::fs::read(fixture_path("avi_pcm_4frames.avi")).unwrap();
    let mut reader = fvid::playback_avi_audio::AviAudioReader::open(
        Cursor::new(&bytes),
        fvid::container::avi::Limits::default(),
    )
    .unwrap();

    // At 4 Hz, 1 ns is strictly after sample 0 (0 ns), so seek_time must ceil to sample 1.
    let landed = reader.seek_time(Duration::from_nanos(1));
    assert_eq!(landed, 1);

    let encoded = reader.next_packet().unwrap().unwrap();
    assert_eq!(encoded.pts, 1);
    assert_eq!(encoded.duration, 3);

    // Also verify present_decoded trims any frames preceding presentation_floor (e.g. for ADPCM block landings).
    let raw_packet = AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 1,
        data: vec![0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0],
    };
    let trimmed = reader
        .present_decoded(raw_packet, 0)
        .unwrap()
        .expect("frames 1..4 should remain");
    assert_eq!(trimmed.pts, 1);
    assert_eq!((trimmed.timebase_num, trimmed.timebase_den), (1, 4));
    assert_eq!(trimmed.data, vec![1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0]);
}

#[test]
fn native_lossless_y4m_accepts_omitted_f_token_with_411_and_410_subsampling() {
    for (fixture, expected_pix_fmt) in [
        ("tiny_411.y4m", "yuv411p"),
        ("tiny_410.y4m", "yuv410p"),
    ] {
        let path = fixture_path(fixture);
        assert!(
            fvid::native_lossless_y4m::eligible(&path).unwrap(),
            "{fixture} without explicit F token must be eligible for owned lossless FFV1 export"
        );
        let mut out = Cursor::new(Vec::new());
        let (stats, event) = fvid::native_lossless_y4m::write(
            &path,
            &mut out,
            &fvid::native_geometry::VideoGeometry::default(),
            &fvid::native_pixels::PixelFilters::default(),
            None,
            None,
        )
        .unwrap();
        assert_eq!(stats.video_frames, 1);
        assert_eq!(stats.pixel_format, expected_pix_fmt);
        assert_eq!(event.packets, 1);
    }
}

#[test]
fn matroska_probe_maps_vobsub_and_hdmv_pgs_bitmap_subtitle_codecs() {
    let path = fixture_path("bitmap_subs_and_data_track.mkv");
    let top_info = fvid::media::probe(&path).unwrap();
    assert_eq!(top_info.streams.len(), 3);
    assert_eq!(top_info.streams[0].media_type, "subtitle");
    assert_eq!(top_info.streams[0].codec, "dvd_subtitle");
    assert_eq!(top_info.streams[1].media_type, "subtitle");
    assert_eq!(top_info.streams[1].codec, "hdmv_pgs_subtitle");

    let owned_info = fvid_media::owned_webm_probe::probe_webm(&path).unwrap();
    assert_eq!(owned_info.streams.len(), 3);
    assert_eq!(owned_info.streams[0].media_type, "subtitle");
    assert_eq!(owned_info.streams[0].codec, "dvd_subtitle");
    assert_eq!(owned_info.streams[1].media_type, "subtitle");
    assert_eq!(owned_info.streams[1].codec, "hdmv_pgs_subtitle");
}

#[test]
fn owned_webm_probe_reports_data_media_type_for_non_avs_matroska_tracks() {
    let path = fixture_path("bitmap_subs_and_data_track.mkv");
    let top_info = fvid::media::probe(&path).unwrap();
    let owned_info = fvid_media::owned_webm_probe::probe_webm(&path).unwrap();
    assert_eq!(top_info.streams[2].media_type, "data");
    assert_eq!(owned_info.streams[2].media_type, "data");
}
