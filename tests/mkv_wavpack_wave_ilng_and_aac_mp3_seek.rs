#[cfg(feature = "player")]
use std::io::Cursor;
use std::path::Path;
#[cfg(feature = "player")]
use std::time::Duration;

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mkv-wavpack-wave-ilng-and-aac-mp3-seek")
        .join(name)
}

#[test]
fn matroska_probe_normalizes_a_wavpack4_to_wavpack() {
    let path = fixture_path("wavpack_track.mkv");
    let top_info = fvid::media::probe(&path).unwrap();
    assert_eq!(top_info.streams.len(), 1);
    assert_eq!(top_info.streams[0].media_type, "audio");
    assert_eq!(top_info.streams[0].codec, "wavpack");

    let owned_info = fvid_media::owned_webm_probe::probe_webm(&path).unwrap();
    assert_eq!(owned_info.streams.len(), 1);
    assert_eq!(owned_info.streams[0].media_type, "audio");
    assert_eq!(owned_info.streams[0].codec, "wavpack");
}

#[test]
fn owned_codec_metadata_recognizes_canonical_audio_and_bitmap_subtitle_codecs() {
    for codec in [
        "wavpack",
        "adpcm_ima_qt",
        "adpcm_ima_wav",
        "adpcm_ms",
        "mace3",
        "mace6",
        "dvd_subtitle",
        "hdmv_pgs_subtitle",
    ] {
        let desc = fvid_media::owned_codec_metadata::descriptor(codec)
            .unwrap_or_else(|| panic!("codec {codec} must be registered in owned_codec_metadata"));
        assert_eq!(desc.name, codec);
    }
}

#[test]
fn wave_probe_and_metadata_editor_map_ilng_to_language() {
    let path = fixture_path("wave_with_ilng.wav");
    let info = fvid_media::owned_probe::probe_wave_with_options(
        &path,
        &fvid_control::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(
        info.metadata.get("language").map(String::as_str),
        Some("eng")
    );
    assert!(!info.metadata.contains_key("ILNG"));

    assert_eq!(
        fvid_media::owned_wave_metadata::tag_for_key("language").unwrap(),
        *b"ILNG"
    );
    let edited_info = fvid_media::owned_wave_metadata::edit_info_chunks(
        &[],
        &[],
        &[("language".to_owned(), "fra".to_owned())],
    )
    .unwrap();
    assert!(edited_info.windows(4).any(|w| w == b"ILNG"));
    assert!(edited_info.windows(3).any(|w| w == b"fra"));
}

#[cfg(feature = "player")]
#[test]
fn aac_audio_reader_seek_time_ceils_sub_sample_duration() {
    use fvid::audio::{AudioPacket, AudioStream};

    let bytes = std::fs::read(fixture_path("tiny_48k.aac")).unwrap();
    let mut reader = fvid::playback_aac::AacAudioReader::open(
        Cursor::new(&bytes),
        fvid::playback_aac::Limits::default(),
    )
    .unwrap();

    // 1 ns is strictly after sample 0 (0 ns), so seek_time must ceil to sample 1.
    let landed = reader.seek_time(Duration::from_nanos(1));
    assert_eq!(landed, 1);
    assert_eq!(reader.preroll_target(), Some(1));

    let raw_packet = AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 48_000,
        data: vec![
            0, 0, 0, 0, 0, 0, 0, 0,
            1, 0, 0, 0, 1, 0, 0, 0,
            2, 0, 0, 0, 2, 0, 0, 0,
        ],
    };
    let presented = reader
        .present_decoded(raw_packet, 0)
        .unwrap()
        .expect("samples 1..3 should remain after trimming sample 0");
    assert_eq!(presented.pts, 1);
    assert_eq!(
        presented.data,
        vec![
            1, 0, 0, 0, 1, 0, 0, 0,
            2, 0, 0, 0, 2, 0, 0, 0,
        ]
    );
}

#[cfg(feature = "player")]
#[test]
fn mp3_audio_reader_seek_time_ceils_sub_sample_duration() {
    use fvid::audio::{AudioPacket, AudioStream};

    let bytes = std::fs::read(fixture_path("tiny_48k.mp2")).unwrap();
    let mut reader = fvid::playback_mp3::Mp3AudioReader::open(
        Cursor::new(&bytes),
        fvid::playback_mp3::Limits::default(),
    )
    .unwrap();

    // 1 ns is strictly after sample 0 (0 ns), so seek_time must ceil to sample 1.
    let landed = reader.seek_time(Duration::from_nanos(1));
    assert_eq!(landed, 1);
    assert_eq!(reader.preroll_target(), Some(1));

    let raw_packet = AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 48_000,
        data: vec![0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
    };
    let presented = reader
        .present_decoded(raw_packet, 0)
        .unwrap()
        .expect("samples 1..3 should remain after trimming sample 0");
    assert_eq!(presented.pts, 1);
    assert_eq!(presented.data, vec![1, 0, 0, 0, 2, 0, 0, 0]);
}
