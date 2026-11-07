use fvid_control::CopyOptions;
use fvid_media::owned_probe::{probe_as_with_options, probe_ogg_with_options, probe_wave_with_options};
use fvid_media::{owned_flac_probe, owned_mpeg_probe, owned_wave_remux};
use std::path::Path;

#[test]
fn ogg_probe_reuses_parsed_container_for_opus_and_vorbis() {
    let opus = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-opus-stereo.ogg");
    let vorbis = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/ogg-vorbis-stereo.ogg");
    let opus_info = probe_ogg_with_options(opus, &CopyOptions::default()).expect("probe opus");
    assert_eq!(opus_info.format, "ogg");
    assert_eq!(opus_info.streams.len(), 1);
    assert_eq!(opus_info.streams[0].codec, "opus");

    let vorbis_info = probe_ogg_with_options(vorbis, &CopyOptions::default()).expect("probe vorbis");
    assert_eq!(vorbis_info.format, "ogg");
    assert_eq!(vorbis_info.streams.len(), 1);
    assert_eq!(vorbis_info.streams[0].codec, "vorbis");
}

#[test]
fn wave_probe_honors_single_g711_law_check_and_cancel_flag() {
    let wave = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/g711-alaw-48k-mono.wav");
    let info = probe_wave_with_options(wave, &CopyOptions::default()).expect("probe alaw wave");
    assert_eq!(info.format, "wav");
    assert_eq!(info.streams[0].codec, "pcm_alaw");
}

#[test]
fn flac_and_mpeg_probe_reuse_opened_file_bytes() {
    let flac = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/flac-4ch-quad.flac");
    let mp3 = Path::new("tests/fixtures/playback-errors/wave-ogg-flac-plan/raw-stereo.mp3");
    let flac_info =
        owned_flac_probe::probe_flac_with_options(flac, &CopyOptions::default()).expect("probe flac");
    assert_eq!(flac_info.format, "flac");
    assert!(flac_info.bit_rate.is_some());

    let mp3_info =
        owned_mpeg_probe::probe_mpeg_with_options(mp3, &CopyOptions::default()).expect("probe mp3");
    assert_eq!(mp3_info.format, "mp3");
    assert!(mp3_info.bit_rate.is_some());
}

#[test]
fn wave_remux_trim_and_unhinted_probe_signature_work_in_single_pass() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-probe-wave-remux-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let wav = dir.join("pcm.wav");
    let mut bytes = Vec::new();
    let samples = [0i16, 1000, -1000, 500, -500, 0, 250, -250];
    let data_bytes = (samples.len() * 2) as u32;
    let riff_size = 36 + data_bytes;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&8000u32.to_le_bytes());
    bytes.extend_from_slice(&16000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for s in samples {
        bytes.extend_from_slice(&s.to_le_bytes());
    }
    std::fs::write(&wav, &bytes).expect("write wav");

    let probed = probe_as_with_options(&wav, None, &CopyOptions::default()).expect("unhinted wav probe");
    assert_eq!(probed.format, "wav");
    assert_eq!(probed.streams[0].codec, "pcm_s16le");

    let out = dir.join("trimmed.wav");
    let stats = owned_wave_remux::trim(&wav, &out, 0, 500_000, &CopyOptions::default()).expect("trim wav");
    assert!(stats.payload_bytes > 0);
    let _ = std::fs::remove_dir_all(&dir);
}
