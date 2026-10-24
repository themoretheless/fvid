use fvid::media::{CopyOptions, MixAudioOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/audio-mix-merge-metadata-preservation")
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
            "fvid-audio-mix-merge-meta-preserve-{tag}-{}-{}",
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
fn mix_and_merge_audio_preserve_and_edit_first_input_metadata_across_wave_and_matroska_outputs() {
    let tmp = TempDir::new("mix-merge-preserve");
    let wave_in1 = fixture("wave-in1-16bit.wav");
    let wave_in2 = fixture("wave-in2-24bit.wav");
    let mp4_in1 = fixture("mp4-in1.m4a");
    let mka_in1 = fixture("matroska-in1.mka");

    // 1. WAVE + WAVE mix-audio and merge-audio to .wav and .mka:
    //    preserves first-input WAVE INFO tags (including ILNG -> stream language on .mka and QWER 4-CC)
    //    and selectively edits/deletes them without dropping untouched tags.
    let wave_pair = vec![wave_in1.clone(), wave_in2.clone()];

    let out_mix_wave_wav = tmp.0.join("mix-wave-unmodified.wav");
    fvid::media::mix_audio(&wave_pair, &out_mix_wave_wav, &MixAudioOptions::default())
        .expect("mix_audio WAVE + WAVE to .wav");
    let info_mix_wave_wav = fvid::media::probe(&out_mix_wave_wav).unwrap();
    assert_eq!(
        info_mix_wave_wav.metadata.get("title").map(String::as_str),
        Some("MixMerge Wave Input 1 Title")
    );
    assert_eq!(
        info_mix_wave_wav.metadata.get("artist").map(String::as_str),
        Some("MixMerge Wave Input 1 Artist")
    );
    assert_eq!(
        info_mix_wave_wav.metadata.get("album").map(String::as_str),
        Some("MixMerge Wave Input 1 Album")
    );
    assert_eq!(
        info_mix_wave_wav.metadata.get("comment").map(String::as_str),
        Some("MixMerge Wave Input 1 Comment")
    );
    assert_eq!(
        info_mix_wave_wav
            .metadata
            .get("copyright")
            .map(String::as_str),
        Some("MixMerge Wave Input 1 Copyright")
    );
    assert_eq!(
        info_mix_wave_wav
            .metadata
            .get("encoder")
            .map(String::as_str),
        Some("FVidMixMergeWaveSynth")
    );
    assert_eq!(
        info_mix_wave_wav.metadata.get("track").map(String::as_str),
        Some("2/8")
    );
    assert_eq!(
        info_mix_wave_wav.metadata.get("ILNG").map(String::as_str),
        Some("jpn")
    );
    assert_eq!(
        info_mix_wave_wav.metadata.get("QWER").map(String::as_str),
        Some("MixMergeWaveFourCC")
    );

    let out_merge_wave_wav = tmp.0.join("merge-wave-edited.wav");
    let edit_wave_wav_opts = CopyOptions {
        metadata_delete: vec!["comment".into(), "QWER".into()],
        metadata_set: vec![("title".into(), "Edited Merge Wave Title".into())],
        ..Default::default()
    };
    fvid::media::merge_audio_with_controls(&wave_pair, &out_merge_wave_wav, &edit_wave_wav_opts)
        .expect("merge_audio_with_controls WAVE + WAVE to .wav with edits");
    let info_merge_wave_wav = fvid::media::probe(&out_merge_wave_wav).unwrap();
    assert_eq!(info_merge_wave_wav.streams[0].channels, 2);
    assert_eq!(
        info_merge_wave_wav
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Edited Merge Wave Title")
    );
    assert_eq!(
        info_merge_wave_wav
            .metadata
            .get("artist")
            .map(String::as_str),
        Some("MixMerge Wave Input 1 Artist")
    );
    assert_eq!(
        info_merge_wave_wav
            .metadata
            .get("album")
            .map(String::as_str),
        Some("MixMerge Wave Input 1 Album")
    );
    assert!(!info_merge_wave_wav.metadata.contains_key("comment"));
    assert!(!info_merge_wave_wav.metadata.contains_key("QWER"));

    let out_mix_wave_mka = tmp.0.join("mix-wave-unmodified.mka");
    fvid::media::mix_audio_with_controls(
        &wave_pair,
        &out_mix_wave_mka,
        &MixAudioOptions::default(),
        &CopyOptions::default(),
    )
    .expect("mix_audio_with_controls WAVE + WAVE to .mka");
    let info_mix_wave_mka = fvid::media::probe(&out_mix_wave_mka).unwrap();
    assert_eq!(
        info_mix_wave_mka.metadata.get("title").map(String::as_str),
        Some("MixMerge Wave Input 1 Title")
    );
    assert_eq!(
        info_mix_wave_mka.metadata.get("artist").map(String::as_str),
        Some("MixMerge Wave Input 1 Artist")
    );
    assert_eq!(
        info_mix_wave_mka
            .metadata
            .get("ENCODER")
            .map(String::as_str),
        Some("FVidMixMergeWaveSynth")
    );
    assert_eq!(
        info_mix_wave_mka.metadata.get("QWER").map(String::as_str),
        Some("MixMergeWaveFourCC")
    );
    assert_eq!(
        info_mix_wave_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("jpn")
    );

    // 2. MP4/M4A + WAVE mix-audio and merge-audio to .wav and .mka:
    //    preserves first-input MP4 container tags and stream 0 track title/language.
    let mp4_pair = vec![mp4_in1.clone(), wave_in2.clone()];

    let out_merge_mp4_wav = tmp.0.join("merge-mp4-unmodified.wav");
    fvid::media::merge_audio(&mp4_pair, &out_merge_mp4_wav).expect("merge_audio MP4 + WAVE to .wav");
    let info_merge_mp4_wav = fvid::media::probe(&out_merge_mp4_wav).unwrap();
    assert_eq!(
        info_merge_mp4_wav.metadata.get("title").map(String::as_str),
        Some("MixMerge MP4 Title")
    );
    assert_eq!(
        info_merge_mp4_wav
            .metadata
            .get("artist")
            .map(String::as_str),
        Some("MixMerge MP4 Artist")
    );
    assert_eq!(
        info_merge_mp4_wav.metadata.get("album").map(String::as_str),
        Some("MixMerge MP4 Album")
    );
    assert_eq!(
        info_merge_mp4_wav
            .metadata
            .get("comment")
            .map(String::as_str),
        Some("MixMerge MP4 Comment")
    );
    assert_eq!(
        info_merge_mp4_wav
            .metadata
            .get("copyright")
            .map(String::as_str),
        Some("MixMerge MP4 Copyright")
    );
    assert_eq!(
        info_merge_mp4_wav.metadata.get("track").map(String::as_str),
        Some("5/9")
    );
    assert_eq!(
        info_merge_mp4_wav.metadata.get("ILNG").map(String::as_str),
        Some("fra")
    );

    let out_mix_mp4_mka = tmp.0.join("mix-mp4-edited.mka");
    let edit_mp4_mka_opts = CopyOptions {
        metadata_delete: vec!["comment".into(), "description".into()],
        metadata_set: vec![("album".into(), "Edited Mix MP4 Album".into())],
        stream_metadata_set: vec![(0, "title".into(), "Edited Mix MP4 Track".into())],
        ..Default::default()
    };
    fvid::media::mix_audio_with_controls(
        &mp4_pair,
        &out_mix_mp4_mka,
        &MixAudioOptions::default(),
        &edit_mp4_mka_opts,
    )
    .expect("mix_audio_with_controls MP4 + WAVE to .mka with edits");
    let info_mix_mp4_mka = fvid::media::probe(&out_mix_mp4_mka).unwrap();
    assert_eq!(
        info_mix_mp4_mka.metadata.get("title").map(String::as_str),
        Some("MixMerge MP4 Title")
    );
    assert_eq!(
        info_mix_mp4_mka.metadata.get("artist").map(String::as_str),
        Some("MixMerge MP4 Artist")
    );
    assert_eq!(
        info_mix_mp4_mka.metadata.get("album").map(String::as_str),
        Some("Edited Mix MP4 Album")
    );
    assert_eq!(
        info_mix_mp4_mka
            .metadata
            .get("album_artist")
            .map(String::as_str),
        Some("MixMerge MP4 Album Artist")
    );
    assert!(!info_mix_mp4_mka.metadata.contains_key("comment"));
    assert!(!info_mix_mp4_mka.metadata.contains_key("description"));
    assert_eq!(
        info_mix_mp4_mka.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("Edited Mix MP4 Track")
    );
    assert_eq!(
        info_mix_mp4_mka.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("fra")
    );

    // 3. Matroska/MKA + WAVE mix-audio and merge-audio to .wav and .mka via API and CLI:
    //    preserves first-input Matroska container tags (including WritingApp ENCODER and custom tags)
    //    and stream 0 track title/language/custom tags, and supports selective deletion/override.
    let mka_pair = vec![mka_in1.clone(), wave_in2.clone()];

    let out_mix_mka_wav = tmp.0.join("mix-mka-unmodified.wav");
    fvid::media::mix_audio(&mka_pair, &out_mix_mka_wav, &MixAudioOptions::default())
        .expect("mix_audio MKA + WAVE to .wav");
    let info_mix_mka_wav = fvid::media::probe(&out_mix_mka_wav).unwrap();
    assert_eq!(
        info_mix_mka_wav.metadata.get("title").map(String::as_str),
        Some("MixMerge MKA Title")
    );
    assert_eq!(
        info_mix_mka_wav.metadata.get("artist").map(String::as_str),
        Some("MixMerge MKA Artist")
    );
    assert_eq!(
        info_mix_mka_wav.metadata.get("album").map(String::as_str),
        Some("MixMerge MKA Album")
    );
    assert_eq!(
        info_mix_mka_wav.metadata.get("encoder").map(String::as_str),
        Some("FVidMixMergeMkaSynth")
    );
    assert_eq!(
        info_mix_mka_wav.metadata.get("ILNG").map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        info_mix_mka_wav.metadata.get("QWER").map(String::as_str),
        Some("MixMergeMkaFourCC")
    );

    let cli_mix_mka_out = tmp.0.join("cli-mix-mka.mka");
    let cli_mix_res = Command::new(bin_path())
        .args([
            "media",
            "mix-audio",
            cli_mix_mka_out.to_str().unwrap(),
            mka_in1.to_str().unwrap(),
            wave_in2.to_str().unwrap(),
            "--metadata-delete",
            "comment",
            "--metadata-delete",
            "custom_mix_source",
            "--metadata",
            "title=CLI Mix MKA Edited Title",
            "--stream-metadata-delete",
            "0:custom_mix_track",
            "--stream-metadata",
            "0:language=spa",
            "-q",
        ])
        .output()
        .expect("cli mix-audio MKA + WAVE to .mka");
    assert!(
        cli_mix_res.status.success(),
        "cli mix-audio failed: {}",
        String::from_utf8_lossy(&cli_mix_res.stderr)
    );
    let info_cli_mix = fvid::media::probe(&cli_mix_mka_out).unwrap();
    assert_eq!(
        info_cli_mix.metadata.get("title").map(String::as_str),
        Some("CLI Mix MKA Edited Title")
    );
    assert_eq!(
        info_cli_mix.metadata.get("artist").map(String::as_str),
        Some("MixMerge MKA Artist")
    );
    assert_eq!(
        info_cli_mix.metadata.get("album").map(String::as_str),
        Some("MixMerge MKA Album")
    );
    assert_eq!(
        info_cli_mix.metadata.get("ENCODER").map(String::as_str),
        Some("FVidMixMergeMkaSynth")
    );
    assert!(!info_cli_mix.metadata.contains_key("comment"));
    assert!(!info_cli_mix.metadata.contains_key("CUSTOM_MIX_SOURCE"));
    assert_eq!(
        info_cli_mix.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("MixMerge MKA Track Title")
    );
    assert_eq!(
        info_cli_mix.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("spa")
    );
    assert_eq!(
        info_cli_mix.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-00004")
    );
    assert!(!info_cli_mix.streams[0]
        .metadata
        .contains_key("CUSTOM_MIX_TRACK"));

    let cli_merge_mka_out = tmp.0.join("cli-merge-mka.mka");
    let cli_merge_res = Command::new(bin_path())
        .args([
            "media",
            "merge-audio",
            cli_merge_mka_out.to_str().unwrap(),
            mka_in1.to_str().unwrap(),
            wave_in2.to_str().unwrap(),
            "--metadata-delete",
            "copyright",
            "--metadata",
            "title=CLI Merge MKA Edited Title",
            "--stream-metadata",
            "0:title=CLI Merge MKA Track Title",
            "-q",
        ])
        .output()
        .expect("cli merge-audio MKA + WAVE to .mka");
    assert!(
        cli_merge_res.status.success(),
        "cli merge-audio failed: {}",
        String::from_utf8_lossy(&cli_merge_res.stderr)
    );
    let info_cli_merge = fvid::media::probe(&cli_merge_mka_out).unwrap();
    assert_eq!(info_cli_merge.streams[0].channels, 2);
    assert_eq!(
        info_cli_merge.metadata.get("title").map(String::as_str),
        Some("CLI Merge MKA Edited Title")
    );
    assert_eq!(
        info_cli_merge.metadata.get("artist").map(String::as_str),
        Some("MixMerge MKA Artist")
    );
    assert!(!info_cli_merge.metadata.contains_key("copyright"));
    assert_eq!(
        info_cli_merge.streams[0]
            .metadata
            .get("title")
            .map(String::as_str),
        Some("CLI Merge MKA Track Title")
    );
    assert_eq!(
        info_cli_merge.streams[0]
            .metadata
            .get("language")
            .map(String::as_str),
        Some("deu")
    );
    assert_eq!(
        info_cli_merge.streams[0]
            .metadata
            .get("ISRC")
            .map(String::as_str),
        Some("US-FVD-26-00004")
    );
}
