#![cfg(feature = "player")]

use std::io::Cursor;
use std::path::Path;

use fvid::audio::AudioStream;
use fvid::container::smf::{self, SmfFile};
use fvid::container::xm::{self, Module};
use fvid::playback::audio::flac::FlacAudioReader;
use fvid::playback::audio::mp3::Mp3AudioReader;
use fvid::playback::audio::wav::{self, Wav};

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wav-flac-mp3-smf-xm-playback")
}

#[test]
fn wav_rejects_extensible_g711_with_non_8bit_storage_width() {
    let bytes = std::fs::read(fixture_dir().join("wav-extensible-alaw-16bit-storage.wav"))
        .expect("read wav fixture");
    let err = Wav::parse(&bytes, &wav::Limits::default())
        .expect_err("extensible G.711 with 16-bit container storage must be rejected");
    assert!(
        err.to_string()
            .contains("extensible G.711 requires eight-bit storage and precision"),
        "unexpected error: {err}"
    );
}

#[test]
fn flac_audio_reader_exposes_title_and_language_from_vorbis_comments() {
    let bytes = std::fs::read(fixture_dir().join("flac-title-and-language.flac"))
        .expect("read flac fixture");
    let reader = FlacAudioReader::open(
        Cursor::new(bytes),
        fvid::playback::audio::flac::Limits::default(),
    )
    .expect("open flac reader");
    let tracks = reader.audio_tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].name, "FLAC Symphony");
    assert_eq!(tracks[0].language, "deu");
}

#[test]
fn mp3_audio_reader_exposes_title_and_language_from_id3_tags() {
    let bytes =
        std::fs::read(fixture_dir().join("mp3-title-and-language.mp3")).expect("read mp3 fixture");
    let reader = Mp3AudioReader::open(
        Cursor::new(bytes),
        fvid::playback::audio::mp3::Limits::default(),
    )
    .expect("open mp3 reader");
    let tracks = reader.audio_tracks();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].name, "MP3 Concerto");
    assert_eq!(tracks[0].language, "fra");
}

#[test]
fn smf_trims_trailing_nul_from_track_names_and_skips_nul_only_conductor_name() {
    let bytes = std::fs::read(fixture_dir().join("smf-nul-padded-conductor-name.mid"))
        .expect("read smf fixture");
    let smf = SmfFile::parse(&bytes, &smf::Limits::default()).expect("parse smf fixture");
    assert_eq!(smf.name, "Lead Synth");
}

#[test]
fn xm_tail_release_packet_uses_final_timeline_row_bpm_and_speed() {
    let bytes = std::fs::read(fixture_dir().join("xm-fxx-tempo-change-tail.xm"))
        .expect("read xm fixture");
    let module = Module::parse(&bytes, &xm::Limits::default()).expect("parse xm fixture");
    assert_eq!(module.bpm, 125);
    assert_eq!(module.speed, 6);
    assert_eq!(module.timeline().len(), 1);
    assert_eq!(module.timeline()[0].bpm, 150);
    assert_eq!(module.timeline()[0].speed, 3);
    assert_eq!(module.packets(), 2);

    let tail_packet = module.packet(1);
    let tail_row = xm::read_row(&tail_packet).expect("decode tail row packet");
    assert_eq!(tail_row.bpm, 150);
    assert_eq!(tail_row.speed, 3);
    assert_eq!(
        u64::from(tail_row.duration_microseconds),
        module.timeline()[0].duration_microseconds
    );
}
