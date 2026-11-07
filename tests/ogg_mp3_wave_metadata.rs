use std::collections::BTreeMap;
use std::path::Path;

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ogg-mp3-wave-metadata")
}

#[test]
fn ogg_opus_skips_empty_comment_values_in_track_metadata_and_wave_export() {
    let opus = fixture_dir().join("opus-empty-first-tags.ogg");
    let bytes = std::fs::read(&opus).expect("read opus fixture");
    let program = fvid_media::owned_ogg_opus::Program::open(
        bytes.as_slice(),
        fvid_media::owned_ogg::Limits::default(),
    )
    .expect("open opus program");
    let tracks = program.audio_tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].name, "Opus Real Title");
    assert_eq!(tracks[0].language, "eng");

    let out_wav = std::env::temp_dir().join(format!(
        "fvid-opus-empty-first-tags-{}.wav",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&out_wav);
    fvid_media::decode_audio(&opus, &out_wav, &fvid_media::CopyOptions::default())
        .expect("decode opus to wave");
    let info = fvid_media::probe(&out_wav).expect("probe exported wave");
    let _ = std::fs::remove_file(&out_wav);
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Opus Real Title")
    );
    assert_eq!(info.metadata.get("date").map(String::as_str), Some("2026"));
    assert_eq!(
        info.metadata.get("language").map(String::as_str),
        Some("eng")
    );
}

#[test]
fn ogg_vorbis_program_populates_track_name_and_language_from_vorbis_comments() {
    let vorbis = fixture_dir().join("vorbis-track-tags.ogg");
    let bytes = std::fs::read(&vorbis).expect("read vorbis fixture");
    let program = fvid_media::owned_ogg_vorbis::Program::open(
        bytes.as_slice(),
        fvid_media::owned_ogg::Limits::default(),
    )
    .expect("open vorbis program");
    let tracks = program.audio_tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].name, "Vorbis Direct Title");
    assert_eq!(tracks[0].language, "jpn");
}

#[test]
fn mp3_and_file_tags_wave_info_cap_at_64_entries_preserving_canonical_tags() {
    let mp3_path = fixture_dir().join("mp3-many-txxx-and-empty-tit2.mp3");
    let bytes = std::fs::read(&mp3_path).expect("read mp3 fixture");
    let mp3 = fvid_media::owned_mpeg_audio::Mp3::parse(
        &bytes,
        &fvid_media::owned_mpeg_audio::Limits::default(),
    )
    .expect("parse mp3");
    let wave_info = mp3.wave_info().expect("mp3 wave_info with >64 tags succeeds");
    assert!(
        wave_info
            .windows(15)
            .any(|w| w == b"Real MP3 Title\0"),
        "canonical title is preserved when draining excess TXXX tags"
    );
    assert!(
        wave_info.windows(5).any(|w| w == b"2026\0"),
        "canonical date is preserved when draining excess TXXX tags"
    );

    let mut extra = BTreeMap::new();
    for i in 0..66 {
        extra.insert(format!("X{i:03}"), format!("v{i}"));
    }
    let mut file_tags = fvid_media::owned_file_tags::FileTags::default();
    file_tags.title = "Container Title".into();
    let chunk = fvid_media::owned_wave_metadata::info_for_file_tags(
        &file_tags,
        "Track Fallback",
        "eng",
        Some("fvid-mux"),
        &extra,
    )
    .expect("info_for_file_tags caps at 64 entries");
    assert!(
        chunk.windows(16).any(|w| w == b"Container Title\0"),
        "canonical container title is preserved after 64-entry cap"
    );
}

#[test]
fn wave_probe_skips_empty_riff_info_entries() {
    let wav = fixture_dir().join("wave-empty-inam.wav");
    let info = fvid_media::probe(&wav).expect("probe wave with empty INAM");
    assert!(
        !info.metadata.contains_key("title"),
        "empty INAM entry must be omitted from probe metadata: {:?}",
        info.metadata
    );
    assert_eq!(
        info.metadata.get("artist").map(String::as_str),
        Some("Synth Artist")
    );
}

#[test]
fn mp3_id3v2_skips_empty_text_frames_so_later_nonempty_frames_survive() {
    let mp3_path = fixture_dir().join("mp3-many-txxx-and-empty-tit2.mp3");
    let info = fvid_media::probe(&mp3_path).expect("probe mp3 with empty first TIT2/TYER");
    assert_eq!(
        info.metadata.get("title").map(String::as_str),
        Some("Real MP3 Title")
    );
    assert_eq!(info.metadata.get("date").map(String::as_str), Some("2026"));
}
