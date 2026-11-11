#[cfg(feature = "player")]
use std::io::Cursor;
use std::path::Path;
#[cfg(feature = "player")]
use std::time::Duration;

fn fixture_path(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-wavpack-ogg-seek-and-id3-tags")
        .join(name)
}

#[cfg(feature = "player")]
#[test]
fn ac3_audio_reader_seek_time_ceils_sub_sample_duration() {
    use fvid::audio::{AudioPacket, AudioStream};

    let bytes = std::fs::read(fixture_path("tiny_48k.ac3")).unwrap();
    let mut reader = fvid::playback_ac3::Ac3AudioReader::open(
        Cursor::new(&bytes),
        fvid::playback_ac3::Limits::default(),
    )
    .unwrap();

    // 1 ns is strictly after sample 0 (0 ns), so seek_time must ceil to sample 1.
    let landed = reader.seek_time(Duration::from_nanos(1));
    assert_eq!(landed, 1);

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

#[cfg(feature = "player")]
#[test]
fn wavpack_audio_reader_seek_time_ceils_sub_sample_duration() {
    use fvid::audio::{AudioPacket, AudioStream};

    let bytes = std::fs::read(fixture_path("tiny_48k.wv")).unwrap();
    let mut reader = fvid::playback_wavpack::WavpackAudioReader::open(
        Cursor::new(bytes),
        fvid::playback_wavpack::Limits::default(),
    )
    .unwrap();

    let landed = reader.seek_time(Duration::from_nanos(1));
    assert_eq!(landed, 1);

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

#[cfg(feature = "player")]
#[test]
fn ogg_opus_and_catalog_readers_seek_time_ceils_sub_sample_duration() {
    use fvid::audio::{AudioPacket, AudioStream};

    let path = fixture_path("tiny_48k.opus");
    let bytes = std::fs::read(&path).unwrap();
    let mut opus_reader = fvid::playback_ogg_opus::OggOpusAudioReader::open(
        Cursor::new(&bytes),
        fvid::container::ogg::Limits::default(),
    )
    .unwrap();

    let landed = opus_reader.seek_time(Duration::from_nanos(1));
    assert_eq!(landed, 1);
    assert_eq!(opus_reader.preroll_target(), Some(1));

    let pkt = opus_reader.next_packet().unwrap().unwrap();
    assert_eq!(pkt.pts, 0);
    assert_eq!(pkt.duration, 960);

    let raw_packet = AudioPacket {
        pts: 0,
        timebase_num: 1,
        timebase_den: 48_000,
        data: vec![0u8; 960 * 4],
    };
    let presented = opus_reader
        .present_decoded(raw_packet, 0)
        .unwrap()
        .expect("959 samples should remain after trimming sample 0");
    assert_eq!(presented.pts, 1);
    assert_eq!(presented.data.len(), 959 * 4);

    let mut catalog_reader = fvid::playback_ogg_catalog::Reader::open(&path, 0).unwrap();
    let catalog_landed = catalog_reader.seek_time(Duration::from_nanos(1));
    assert_eq!(catalog_landed, 1);
    assert_eq!(catalog_reader.preroll_target(), Some(1));
}

#[test]
fn mpeg_audio_parses_id3v24_footer_and_normalizes_numeric_tcon_genre() {
    let path = fixture_path("id3v24_footer_and_tcon.mp2");
    let bytes = std::fs::read(&path).unwrap();
    let mp3 = fvid_media::owned_mpeg_audio::Mp3::parse(
        &bytes,
        &fvid_media::owned_mpeg_audio::Limits::default(),
    )
    .unwrap();

    // Header (10) + TIT2 (22) + TCON (15) + Footer (10) = 57 bytes total.
    assert_eq!(mp3.tag_bytes, 57);
    assert_eq!(mp3.frames[0].start, 57);

    let (container_meta, stream_meta) = mp3.probe_metadata();
    assert_eq!(
        container_meta.get("title").map(String::as_str),
        Some("Footer Song")
    );
    assert_eq!(
        stream_meta.get("title").map(String::as_str),
        Some("Footer Song")
    );
    assert_eq!(
        container_meta.get("genre").map(String::as_str),
        Some("Rock")
    );

    let probed = fvid::media::probe(&path).unwrap();
    assert_eq!(
        probed.metadata.get("title").map(String::as_str),
        Some("Footer Song")
    );
    assert_eq!(
        probed.metadata.get("genre").map(String::as_str),
        Some("Rock")
    );
}
