use std::io::Cursor;
use std::path::Path;
use std::time::Duration;

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-g711-probe-id3-vorbis-and-smf-xm-seek")
        .join(name)
}

#[test]
fn mp4_probe_normalizes_alaw_and_ulaw_to_pcm_alaw_and_pcm_mulaw() {
    for (file, expected_codec) in [
        ("tiny_alaw.mov", "pcm_alaw"),
        ("tiny_ulaw.mov", "pcm_mulaw"),
    ] {
        let path = fixture_path(file);
        let top = fvid::media::probe(&path).unwrap();
        assert_eq!(top.streams.len(), 1);
        assert_eq!(top.streams[0].media_type, "audio");
        assert_eq!(top.streams[0].codec, expected_codec);

        let owned = fvid_media::owned_mp4_probe::probe_mp4(&path).unwrap();
        assert_eq!(owned.streams.len(), 1);
        assert_eq!(owned.streams[0].media_type, "audio");
        assert_eq!(owned.streams[0].codec, expected_codec);
    }
}

#[test]
fn id3v2_and_id3v1_resolve_extended_standard_genres_and_txxx_aliases() {
    let path = fixture_path("id3_punk_and_aliases.mp2");
    let info = fvid_media::owned_mpeg_probe::probe_mpeg_with_options(
        &path,
        &fvid_control::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(info.metadata.get("genre").map(String::as_str), Some("Punk"));
    assert_eq!(info.metadata.get("date").map(String::as_str), Some("1999"));
    assert_eq!(
        info.metadata.get("publisher").map(String::as_str),
        Some("Indie Label")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-id3-test")
    );

    // Also verify ID3v1 trailer with genre code 20 ("Alternative") and 79 ("Hard Rock").
    let mut raw = std::fs::read(&path).unwrap();
    for (code, expected) in [(19u8, "Industrial"), (20u8, "Alternative"), (79u8, "Hard Rock")] {
        // Strip ID3v2 header so ID3v1 genre is used.
        let mp2_only = &raw[raw.len() - 96..];
        let mut with_v1 = mp2_only.to_vec();
        let mut tag = [0u8; 128];
        tag[..3].copy_from_slice(b"TAG");
        tag[3..7].copy_from_slice(b"Test");
        tag[127] = code;
        with_v1.extend_from_slice(&tag);
        let parsed = fvid_media::owned_mpeg_audio::Mp3::parse(
            &with_v1,
            &fvid_media::owned_mpeg_audio::Limits::default(),
        )
        .unwrap();
        let (meta, _) = parsed.probe_metadata();
        assert_eq!(meta.get("genre").map(String::as_str), Some(expected));
    }
    raw.clear();
}

#[test]
fn vorbis_comments_and_file_tags_normalize_year_organization_and_encoded_by() {
    let path = fixture_path("vorbis_aliases.opus");
    let info = fvid_media::owned_probe::probe_ogg_with_options(
        &path,
        &fvid_control::CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(info.metadata.get("date").map(String::as_str), Some("2001"));
    assert_eq!(
        info.metadata.get("publisher").map(String::as_str),
        Some("TestOrg")
    );
    assert_eq!(
        info.metadata.get("encoder").map(String::as_str),
        Some("fvid-opus-test")
    );
    assert_eq!(info.metadata.get("track").map(String::as_str), Some("7"));

    let mut file_tags = fvid_media::owned_file_tags::FileTags::default();
    assert!(file_tags.insert("YEAR", "2020"));
    assert!(file_tags.insert("ORGANIZATION", "Acme Music"));
    assert_eq!(file_tags.date, "2020");
    assert_eq!(file_tags.publisher, "Acme Music");

    // Verify FLAC Vorbis comment -> WAVE INFO chunk mapping for YEAR, ENCODED-BY, and PART_NUMBER.
    let mut body = vec![0u8; 4];
    let fields = ["YEAR=1998", "ENCODED-BY=flac-enc", "PART_NUMBER=5"];
    body.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    for f in fields {
        body.extend_from_slice(&(f.len() as u32).to_le_bytes());
        body.extend_from_slice(f.as_bytes());
    }
    let mut flac_extra = vec![0x84];
    flac_extra.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
    flac_extra.extend(body);
    let wave_info = fvid_media::owned_flac_layout::wave_info_from_extra(&flac_extra).unwrap();
    assert!(wave_info.windows(4).any(|w| w == b"ICRD"));
    assert!(wave_info.windows(4).any(|w| w == b"1998"));
    assert!(wave_info.windows(4).any(|w| w == b"ISFT"));
    assert!(wave_info.windows(8).any(|w| w == b"flac-enc"));
    assert!(wave_info.windows(4).any(|w| w == b"ITRK"));
}

#[test]
fn smf_and_xm_audio_readers_seek_time_ceil_sub_millisecond_targets() {
    use fvid::audio::AudioStream;

    // 1. SmfAudioReader: windows at 0 ms, 100 ms, 200 ms, ...
    // 99_000_001 ns (99.000001 ms) is strictly after 99 ms and must ceil to 100 ms (window 1).
    let smf_bytes = std::fs::read(fixture_path("tiny.mid")).unwrap();
    let mut smf_reader = fvid::playback_smf::SmfAudioReader::open(
        Cursor::new(&smf_bytes),
        fvid::container::smf::Limits::default(),
    )
    .unwrap();
    let landed_smf = smf_reader.seek_time(Duration::from_nanos(99_000_001));
    assert_eq!(landed_smf, 100);
    let next_smf = smf_reader.next_packet().unwrap().unwrap();
    assert_eq!(next_smf.pts, 100);

    // 2. XmAudioReader: rows at 0 ms, 120 ms, 240 ms, 360 ms, 480 ms (speed=6, bpm=125).
    // 119_000_001 ns (119.000001 ms) is strictly after 119 ms and must ceil to 120 ms (row 1).
    let xm_bytes = std::fs::read(fixture_path("tiny.xm")).unwrap();
    let mut xm_reader = fvid::playback_xm::XmAudioReader::open(
        Cursor::new(&xm_bytes),
        fvid::container::xm::Limits::default(),
    )
    .unwrap();
    let landed_xm = xm_reader.seek_time(Duration::from_nanos(119_000_001));
    assert_eq!(landed_xm, 120);
    let next_xm = xm_reader.next_packet().unwrap().unwrap();
    assert_eq!(next_xm.pts, 120);
}
