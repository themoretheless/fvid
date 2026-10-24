#![cfg(feature = "player")]

use fvid::playback_spool::{SPOOL_LEAD, Spool};
use fvid::subtitles::{self, Cue};
use std::io::{Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitles")
        .join(name)
}

#[test]
fn webvtt_parser_skips_multiline_note_and_style_blocks_with_arrow_timings() {
    let bytes = std::fs::read(fixture_path("webvtt_note_and_style.vtt")).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    let cues = subtitles::parse(text);
    assert_eq!(
        cues,
        vec![Cue {
            start: Duration::from_millis(1000),
            end: Duration::from_millis(2500),
            text: "Visible WebVTT line".to_owned(),
        }],
        "WebVTT NOTE, STYLE, and REGION blocks must be skipped until the next blank line"
    );
}

#[test]
fn sami_parser_treats_case_insensitive_nbsp_entities_as_caption_clears() {
    let bytes = std::fs::read(fixture_path("sami_uppercase_nbsp.smi")).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    let cues = subtitles::parse(text);
    assert_eq!(
        cues,
        vec![
            Cue {
                start: Duration::from_millis(1000),
                end: Duration::from_millis(2200),
                text: "First SAMI line".to_owned(),
            },
            Cue {
                start: Duration::from_millis(3500),
                end: Duration::from_millis(4800),
                text: "Second SAMI line".to_owned(),
            },
        ],
        "Uppercase &NBSP; and mixed-case &Nbsp; in SAMI Sync paragraphs must clear captions instead of rendering literal entity text"
    );
}

#[test]
fn ass_parser_ignores_dialogue_prefixes_outside_events_section() {
    let bytes = std::fs::read(fixture_path("ass_script_info_dialogue.ass")).unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    let cues = subtitles::parse(text);
    assert_eq!(
        cues,
        vec![Cue {
            start: Duration::from_millis(1500),
            end: Duration::from_millis(3000),
            text: "Real ASS cue".to_owned(),
        }],
        "ASS parser must only read Dialogue lines inside the [Events] section"
    );
}

#[test]
fn spool_seek_rejects_negative_result_positions_per_std_io_seek_contract() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/streaming-block-boundary.y4m");
    let mut spool = Spool::in_memory(&src, SPOOL_LEAD).expect("in-memory spool");
    let err = spool
        .seek(SeekFrom::Current(-10))
        .expect_err("seeking before byte 0 must return InvalidInput");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    let size = std::fs::metadata(&src).unwrap().len() as i64;
    let err_end = spool
        .seek(SeekFrom::End(-(size + 1)))
        .expect_err("seeking before byte 0 from End must return InvalidInput");
    assert_eq!(err_end.kind(), std::io::ErrorKind::InvalidInput);
}
