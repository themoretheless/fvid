use fvid::media::{AudioDecodeTransform, CopyOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-matroska-pcm-metadata-preservation")
        .join(name)
}

fn bin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_fvid"))
}

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "fvid-audio-mka-pcm-metadata-preservation-{tag}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn wave_mp4_and_matroska_audio_exports_to_matroska_preserve_and_edit_source_container_and_stream_metadata() {
    let tmp = TempDir::new("preserve");
    let wav = fixture("wave-mka-preserve.wav");
    let mp4 = fixture("mp4-mka-preserve.m4a");
    let mka = fixture("matroska-mka-preserve.mka");

    // 1. WAVE -> .mka / .mkv across `decode_audio`, `trim`, `loudnorm`, and `normalize-loudness`
    //    must preserve source RIFF INFO tags (including `ISFT` -> `encoder`, `ILNG` -> stream 0 `language` and container `ILNG`, and 4-CC `QWER`)
    //    and allow selective container/stream metadata edits and deletions without losing untouched tags.
    let out_wav_mka = tmp.0.join("wav-decode-default.mka");
    fvid::media::decode_audio(&wav, &out_wav_mka, &CopyOptions::default())
        .expect("decode_audio wave -> mka default");
    let probed_wav_mka = fvid::media::probe(&out_wav_mka).unwrap();
    assert_eq!(
        probed_wav_mka.metadata.get("title").map(String::as_str),
        Some("Wave To MKA Title")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("artist").map(String::as_str),
        Some("Wave To MKA Artist")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("album").map(String::as_str),
        Some("Wave To MKA Album")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("comment").map(String::as_str),
        Some("Wave To MKA Comment")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("copyright").map(String::as_str),
        Some("Wave To MKA Copyright")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("encoder").map(String::as_str),
        Some("FVidWaveSynth")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("track").map(String::as_str),
        Some("2/8")
    );
    assert_eq!(
        probed_wav_mka.metadata.get("QWER").map(String::as_str),
        Some("WaveFourCCValue")
    );
    assert_eq!(
        probed_wav_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("eng")
    );

    // Selective edit on WAVE -> .mka via `decode_audio_transformed`
    let wav_edit_opts = CopyOptions {
        metadata_set: vec![("title".into(), "Wave Edited MKA Title".into())],
        metadata_delete: vec!["comment".into(), "QWER".into()],
        stream_metadata_set: vec![(0, "title".into(), "Wave Track 0 Name".into())],
        ..Default::default()
    };
    let out_wav_edit_mka = tmp.0.join("wav-decode-edited.mka");
    fvid::media::decode_audio_transformed(
        &wav,
        &out_wav_edit_mka,
        AudioDecodeTransform::default(),
        &wav_edit_opts,
    )
    .expect("decode_audio wave -> mka edited");
    let probed_wav_edit = fvid::media::probe(&out_wav_edit_mka).unwrap();
    assert_eq!(
        probed_wav_edit.metadata.get("title").map(String::as_str),
        Some("Wave Edited MKA Title")
    );
    assert_eq!(
        probed_wav_edit.metadata.get("artist").map(String::as_str),
        Some("Wave To MKA Artist"),
        "untouched WAVE artist tag must be preserved on .mka output"
    );
    assert_eq!(
        probed_wav_edit.metadata.get("album").map(String::as_str),
        Some("Wave To MKA Album")
    );
    assert!(!probed_wav_edit.metadata.contains_key("comment"));
    assert!(!probed_wav_edit.metadata.contains_key("QWER"));
    assert_eq!(
        probed_wav_edit.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Wave Track 0 Name")
    );
    assert_eq!(
        probed_wav_edit.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("eng")
    );

    // 2. MP4 -> .mka / .mkv across `decode_audio`, `trim`, `loudnorm`, and `normalize-loudness`
    //    must preserve ALL MP4 container tags (including `album_artist`, `disc`, `publisher`, `description`)
    //    AND distinct MP4 track `name` (`streams[0].metadata["title"]`) and `language` (`streams[0].metadata["language"]`).
    let out_mp4_mka = tmp.0.join("mp4-decode-default.mka");
    fvid::media::decode_audio(&mp4, &out_mp4_mka, &CopyOptions::default())
        .expect("decode_audio mp4 -> mka default");
    let probed_mp4_mka = fvid::media::probe(&out_mp4_mka).unwrap();
    assert_eq!(
        probed_mp4_mka.metadata.get("title").map(String::as_str),
        Some("MP4 To MKA Title")
    );
    assert_eq!(
        probed_mp4_mka.metadata.get("artist").map(String::as_str),
        Some("MP4 To MKA Artist")
    );
    assert_eq!(
        probed_mp4_mka.metadata.get("album").map(String::as_str),
        Some("MP4 To MKA Album")
    );
    assert_eq!(
        probed_mp4_mka
            .metadata
            .get("album_artist")
            .map(String::as_str),
        Some("MP4 To MKA Album Artist")
    );
    assert_eq!(
        probed_mp4_mka
            .metadata
            .get("description")
            .map(String::as_str),
        Some("MP4 To MKA Description")
    );
    assert_eq!(
        probed_mp4_mka.metadata.get("track").map(String::as_str),
        Some("5/12")
    );
    assert_eq!(
        probed_mp4_mka.metadata.get("disc").map(String::as_str),
        Some("1/2")
    );
    assert_eq!(
        probed_mp4_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MP4 Distinct Track Name")
    );
    assert_eq!(
        probed_mp4_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    // Selective edit on MP4 -> .mka via `trim` (routes through `decode_fvid_media_audio_to_destination` when mutations are present)
    let mp4_trim_opts = CopyOptions {
        metadata_set: vec![("publisher".into(), "MP4 Edited Publisher".into())],
        metadata_delete: vec!["comment".into()],
        stream_metadata_set: vec![(0, "language".into(), "spa".into())],
        ..Default::default()
    };
    let out_mp4_trim_mka = tmp.0.join("mp4-trim-edited.mka");
    fvid::media::trim(&mp4, &out_mp4_trim_mka, 0, 250_000, &mp4_trim_opts)
        .expect("trim mp4 -> mka edited");
    let probed_mp4_trim = fvid::media::probe(&out_mp4_trim_mka).unwrap();
    assert_eq!(
        probed_mp4_trim.metadata.get("title").map(String::as_str),
        Some("MP4 To MKA Title")
    );
    assert_eq!(
        probed_mp4_trim
            .metadata
            .get("album_artist")
            .map(String::as_str),
        Some("MP4 To MKA Album Artist"),
        "non-INFO MP4 tag album_artist must survive WAVE spool when trimming to .mka"
    );
    assert_eq!(
        probed_mp4_trim
            .metadata
            .get("publisher")
            .map(String::as_str),
        Some("MP4 Edited Publisher")
    );
    assert!(!probed_mp4_trim.metadata.contains_key("comment"));
    assert_eq!(
        probed_mp4_trim.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MP4 Distinct Track Name"),
        "distinct MP4 track name must survive WAVE spool when trimming to .mka"
    );
    assert_eq!(
        probed_mp4_trim.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("spa")
    );

    // MP4 `loudnorm` to .mka (default and edited)
    let out_mp4_ln_mka = tmp.0.join("mp4-loudnorm-default.mka");
    fvid::native_loudnorm::try_apply(&mp4, &out_mp4_ln_mka, None, false, &CopyOptions::default())
        .expect("loudnorm mp4 -> mka default")
        .expect("loudnorm mp4 -> mka handled");
    let probed_mp4_ln = fvid::media::probe(&out_mp4_ln_mka).unwrap();
    assert_eq!(
        probed_mp4_ln.metadata.get("title").map(String::as_str),
        Some("MP4 To MKA Title")
    );
    assert_eq!(
        probed_mp4_ln
            .metadata
            .get("album_artist")
            .map(String::as_str),
        Some("MP4 To MKA Album Artist")
    );
    assert_eq!(
        probed_mp4_ln.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MP4 Distinct Track Name")
    );
    assert_eq!(
        probed_mp4_ln.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    // 3. Matroska -> .mka / .mkv across `decode_audio`, `trim`, `loudnorm`, and `normalize-loudness` (API and CLI)
    //    must preserve container FileTags, custom container text tags (`CUSTOM_CONTAINER_NOTE`), `encoder` (`WritingApp`),
    //    distinct track `title`, track `language`, and track-scoped tags (`ISRC`, `CUSTOM_TRACK_NOTE`),
    //    and allow selective overrides/deletions (including case-insensitive custom tag deletion).
    let out_mka_mka = tmp.0.join("mka-decode-default.mka");
    fvid::media::decode_audio(&mka, &out_mka_mka, &CopyOptions::default())
        .expect("decode_audio mka -> mka default");
    let probed_mka_mka = fvid::media::probe(&out_mka_mka).unwrap();
    assert_eq!(
        probed_mka_mka.metadata.get("title").map(String::as_str),
        Some("MKA To MKA Title")
    );
    assert_eq!(
        probed_mka_mka.metadata.get("artist").map(String::as_str),
        Some("MKA To MKA Artist")
    );
    assert_eq!(
        probed_mka_mka
            .metadata
            .get("album_artist")
            .map(String::as_str),
        Some("MKA To MKA Album Artist")
    );
    assert_eq!(
        probed_mka_mka.metadata.get("disc").map(String::as_str),
        Some("2/3")
    );
    assert_eq!(
        probed_mka_mka.metadata.get("publisher").map(String::as_str),
        Some("MKA To MKA Publisher")
    );
    assert_eq!(
        probed_mka_mka
            .metadata
            .get("description")
            .map(String::as_str),
        Some("MKA To MKA Description")
    );
    assert_eq!(
        probed_mka_mka.metadata.get("rating").map(String::as_str),
        Some("5.0")
    );
    assert_eq!(
        probed_mka_mka.metadata.get("encoder").map(String::as_str),
        Some("FVidMkaSynthMuxer")
    );
    assert_eq!(
        probed_mka_mka
            .metadata
            .get("CUSTOM_CONTAINER_NOTE")
            .map(String::as_str),
        Some("ContainerNoteValue")
    );
    assert_eq!(
        probed_mka_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MKA Distinct Track Name")
    );
    assert_eq!(
        probed_mka_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        probed_mka_mka.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-00002")
    );
    assert_eq!(
        probed_mka_mka.streams[0]
            .metadata
            .get("CUSTOM_TRACK_NOTE")
            .map(String::as_str),
        Some("TrackNoteValue")
    );

    // CLI `loudnorm` and `normalize-loudness` on Matroska -> .mka with selective container & stream metadata edits/deletes
    let cli_mka_ln = tmp.0.join("cli-mka-loudnorm.mka");
    let cli_ln = Command::new(bin_path())
        .args([
            "media",
            "loudnorm",
            mka.to_str().unwrap(),
            cli_mka_ln.to_str().unwrap(),
            "--metadata",
            "title=CLI Loudnorm MKA Title",
            "--metadata-delete",
            "custom_container_note",
            "--stream-metadata",
            "0:ISRC=US-FVD-26-99999",
            "--stream-metadata-delete",
            "0:custom_track_note",
            "-q",
        ])
        .output()
        .expect("cli loudnorm mka -> mka");
    assert!(
        cli_ln.status.success(),
        "cli loudnorm mka -> mka failed: {}",
        String::from_utf8_lossy(&cli_ln.stderr)
    );
    let probed_cli_ln = fvid::media::probe(&cli_mka_ln).unwrap();
    assert_eq!(
        probed_cli_ln.metadata.get("title").map(String::as_str),
        Some("CLI Loudnorm MKA Title")
    );
    assert_eq!(
        probed_cli_ln.metadata.get("artist").map(String::as_str),
        Some("MKA To MKA Artist"),
        "untouched container artist must be preserved on loudnorm -> .mka"
    );
    assert_eq!(
        probed_cli_ln.metadata.get("rating").map(String::as_str),
        Some("5.0")
    );
    assert!(!probed_cli_ln
        .metadata
        .contains_key("CUSTOM_CONTAINER_NOTE"));
    assert_eq!(
        probed_cli_ln.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MKA Distinct Track Name"),
        "untouched stream title must be preserved on loudnorm -> .mka"
    );
    assert_eq!(
        probed_cli_ln.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        probed_cli_ln.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-99999")
    );
    assert!(!probed_cli_ln.streams[0]
        .metadata
        .contains_key("CUSTOM_TRACK_NOTE"));

    let cli_mka_norm = tmp.0.join("cli-mka-norm.mka");
    let cli_norm = Command::new(bin_path())
        .args([
            "media",
            "normalize-loudness",
            mka.to_str().unwrap(),
            cli_mka_norm.to_str().unwrap(),
            "--metadata",
            "artist=CLI Normalized MKA Artist",
            "--metadata-delete",
            "comment",
            "--stream-metadata",
            "0:title=CLI Normalized Track 0",
            "-q",
        ])
        .output()
        .expect("cli normalize-loudness mka -> mka");
    assert!(
        cli_norm.status.success(),
        "cli normalize-loudness mka -> mka failed: {}",
        String::from_utf8_lossy(&cli_norm.stderr)
    );
    let probed_cli_norm = fvid::media::probe(&cli_mka_norm).unwrap();
    assert_eq!(
        probed_cli_norm.metadata.get("title").map(String::as_str),
        Some("MKA To MKA Title")
    );
    assert_eq!(
        probed_cli_norm.metadata.get("artist").map(String::as_str),
        Some("CLI Normalized MKA Artist")
    );
    assert!(!probed_cli_norm.metadata.contains_key("comment"));
    assert_eq!(
        probed_cli_norm.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Normalized Track 0")
    );
    assert_eq!(
        probed_cli_norm.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-00002")
    );
}
