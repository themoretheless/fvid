#![cfg(feature = "player")]
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fvid::color::ColourDescription;
use fvid::container::webm::{Limits, WebmReader};
use fvid::media;
use fvid::media_control::CopyOptions;
use fvid::media_info::{LosslessTransform, SubtitleCodec, SubtitleConvertOptions};
use fvid::native_probe;
use fvid::playback_webm_subtitles::WebmSubtitleReader;
use fvid::subtitles::{self, Cue};
use fvid_media::owned_codec_metadata;
use fvid_media::owned_subtitle;
use fvid_media::owned_webm_probe;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-subtitles-and-mp4-vui-fallback")
}

fn temp_mkv(stem: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "fvid-batch569-{stem}-{}.mkv",
        std::process::id()
    ))
}

#[test]
fn matroska_probe_normalizes_dolby_and_subtitle_codec_names_across_both_probe_apis() {
    let sub_mkv = fixture_dir().join("mkv-subtitle-codecs.mkv");
    let media_info = media::probe(&sub_mkv).expect("fvid::media::probe subtitle codecs");
    let native_info =
        native_probe::probe(&sub_mkv).expect("fvid::native_probe::probe subtitle codecs");
    let owned_info =
        owned_webm_probe::probe_webm(&sub_mkv).expect("owned_webm_probe subtitle codecs");

    let expected_sub_codecs = ["webvtt", "subrip", "subrip", "ass", "ssa"];
    let media_codecs: Vec<&str> = media_info
        .streams
        .iter()
        .map(|s| s.codec.as_str())
        .collect();
    let native_codecs: Vec<&str> = native_info
        .streams
        .iter()
        .map(|s| s.codec.as_str())
        .collect();
    let owned_codecs: Vec<&str> = owned_info
        .streams
        .iter()
        .map(|s| s.codec.as_str())
        .collect();
    assert_eq!(media_codecs, expected_sub_codecs);
    assert_eq!(native_codecs, expected_sub_codecs);
    assert_eq!(owned_codecs, expected_sub_codecs);
    for codec in expected_sub_codecs {
        assert_eq!(
            owned_codec_metadata::descriptor(codec).map(|d| d.name),
            Some(codec)
        );
    }

    let prev_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/inband-open-signal-and-ac3-duration");
    let ac3_path = prev_dir.join("ac3-no-segment-duration-default.mkv");
    let ac3_info =
        owned_webm_probe::probe_webm(&ac3_path).expect("probe Matroska AC-3");
    assert_eq!(ac3_info.streams[1].codec, "ac3");
    let eac3_path = prev_dir.join("eac3-no-segment-duration.mkv");
    let eac3_info =
        owned_webm_probe::probe_webm(&eac3_path).expect("probe Matroska E-AC-3");
    assert_eq!(eac3_info.streams[1].codec, "eac3");
}

#[test]
fn webm_subtitle_reader_decodes_webvtt_ascii_and_legacy_matroska_subtitle_tracks() {
    let path = fixture_dir().join("mkv-subtitle-codecs.mkv");
    let mut reader = WebmSubtitleReader::open(
        BufReader::new(File::open(&path).expect("open mkv-subtitle-codecs.mkv")),
        Limits::default(),
    )
    .expect("open WebmSubtitleReader");

    let tracks = reader.tracks();
    let labels: Vec<String> = tracks.iter().map(|t| t.label()).collect();
    assert_eq!(labels, vec!["WebVTT", "ASCII", "UTF-8", "ASS", "SSA"]);

    assert_eq!(
        reader.cues(0).expect("WebVTT cues"),
        vec![
            Cue {
                start: Duration::from_millis(100),
                end: Duration::from_millis(900),
                text: "WebVTT\u{a0}first".to_owned(),
            },
            Cue {
                start: Duration::from_millis(1000),
                end: Duration::from_millis(1700),
                text: "WebVTT second".to_owned(),
            },
        ]
    );
    assert_eq!(
        reader.cues(1).expect("S_TEXT/ASCII cues"),
        vec![Cue {
            start: Duration::from_millis(200),
            end: Duration::from_millis(800),
            text: "ASCII line".to_owned(),
        }]
    );
    assert_eq!(
        reader.cues(2).expect("S_UTF8 cues"),
        vec![Cue {
            start: Duration::from_millis(300),
            end: Duration::from_millis(900),
            text: "Legacy UTF8 line".to_owned(),
        }]
    );
    assert_eq!(
        reader.cues(3).expect("S_ASS cues"),
        vec![Cue {
            start: Duration::from_millis(400),
            end: Duration::from_millis(1000),
            text: "Legacy\u{a0}ASS".to_owned(),
        }]
    );
    assert_eq!(
        reader.cues(4).expect("S_SSA cues"),
        vec![Cue {
            start: Duration::from_millis(500),
            end: Duration::from_millis(1100),
            text: "Legacy\u{a0}SSA".to_owned(),
        }]
    );
}

#[test]
fn matroska_subtitle_conversion_and_planning_support_default_duration_and_ssa() {
    let utf8_src = fixture_dir().join("utf8-default-duration.mkv");
    let utf8_dst = temp_mkv("utf8-default-out");
    let _ = std::fs::remove_file(&utf8_dst);

    let utf8_plan = owned_subtitle::try_plan_with_controls(
        &utf8_src,
        Some(&utf8_dst),
        &SubtitleConvertOptions {
            streams: vec![],
            codec: SubtitleCodec::Ass,
        },
        &CopyOptions::default(),
    )
    .expect("plan utf8 default duration")
    .expect("utf8 plan supported");
    assert_eq!(utf8_plan.streams[0].codec, "subrip");
    assert_eq!(utf8_plan.steps[0].action, "decode");

    let utf8_stats = owned_subtitle::try_convert(&utf8_src, &utf8_dst, &[])
        .expect("convert utf8 default duration")
        .expect("utf8 conversion supported");
    assert_eq!(utf8_stats.cues, 2);

    let mut utf8_reader = WebmSubtitleReader::open(
        BufReader::new(File::open(&utf8_dst).expect("open converted utf8 mkv")),
        Limits::default(),
    )
    .expect("open converted utf8 subtitle reader");
    assert_eq!(
        utf8_reader.cues(0).expect("converted utf8 cues"),
        vec![
            Cue {
                start: Duration::from_millis(200),
                end: Duration::from_millis(950),
                text: "First default cue".to_owned(),
            },
            Cue {
                start: Duration::from_millis(1200),
                end: Duration::from_millis(1950),
                text: "Second default cue".to_owned(),
            },
        ]
    );
    let _ = std::fs::remove_file(&utf8_dst);

    let ssa_src = fixture_dir().join("ssa-default-duration.mkv");
    let ssa_dst = temp_mkv("ssa-default-out");
    let _ = std::fs::remove_file(&ssa_dst);

    let ssa_plan = owned_subtitle::try_plan_with_controls(
        &ssa_src,
        Some(&ssa_dst),
        &SubtitleConvertOptions {
            streams: vec![],
            codec: SubtitleCodec::Ass,
        },
        &CopyOptions::default(),
    )
    .expect("plan ssa default duration")
    .expect("ssa plan supported");
    assert_eq!(ssa_plan.streams[0].codec, "ssa");
    assert_eq!(ssa_plan.steps[0].action, "copy");

    let ssa_stats = owned_subtitle::try_convert(&ssa_src, &ssa_dst, &[])
        .expect("convert ssa default duration")
        .expect("ssa conversion supported");
    assert_eq!(ssa_stats.cues, 2);

    let mut raw_mkv = WebmReader::open(
        BufReader::new(File::open(&ssa_dst).expect("open converted ssa mkv")),
        Limits::default(),
    )
    .expect("open WebmReader on converted ssa");
    raw_mkv.scan_all().expect("scan converted ssa mkv");
    assert_eq!(raw_mkv.tracks[0].codec, "S_TEXT/ASS");
    let first_event = String::from_utf8(raw_mkv.read_packet(0).expect("read packet 0")).unwrap();
    let second_event = String::from_utf8(raw_mkv.read_packet(1).expect("read packet 1")).unwrap();
    assert_eq!(
        first_event,
        "0,0,Default,Alice,0,0,0,,SSA\\hconverted\\Nline"
    );
    assert_eq!(second_event, "1,1,Default,Bob,0,0,0,,Second SSA cue");

    let mut ssa_reader = WebmSubtitleReader::open(
        BufReader::new(File::open(&ssa_dst).expect("reopen converted ssa mkv")),
        Limits::default(),
    )
    .expect("open converted ssa subtitle reader");
    assert_eq!(
        ssa_reader.cues(0).expect("converted ssa cues"),
        vec![
            Cue {
                start: Duration::from_millis(300),
                end: Duration::from_millis(1200),
                text: "SSA\u{a0}converted\nline".to_owned(),
            },
            Cue {
                start: Duration::from_millis(1500),
                end: Duration::from_millis(2400),
                text: "Second SSA cue".to_owned(),
            },
        ]
    );
    let _ = std::fs::remove_file(&ssa_dst);
}

#[test]
fn sidecar_parse_ass_supports_custom_format_order_case_insensitive_dialogue_and_hard_space() {
    let ass_path = fixture_dir().join("custom-format-and-hardspace.ass");
    let cues = subtitles::load(&ass_path).expect("load custom ASS sidecar");
    assert_eq!(
        cues,
        vec![
            Cue {
                start: Duration::from_millis(250),
                end: Duration::from_millis(1500),
                text: "Hard\u{a0}space\nanditalics".to_owned(),
            },
            Cue {
                start: Duration::from_millis(2000),
                end: Duration::from_millis(3250),
                text: "Second\u{a0}cue, with comma".to_owned(),
            },
        ]
    );
}

#[test]
fn mp4_video_decode_preserves_container_nclx_colour_when_avc_or_hevc_vui_codes_are_unspecified() {
    let mut opts = CopyOptions::default();
    opts.metadata_set.push(("TITLE".into(), "VUI2Fallback".into()));

    let avc_path = fixture_dir().join("avc-vui-unspecified-colr.mp4");
    let avc_dst = temp_mkv("avc-vui2-out");
    let _ = std::fs::remove_file(&avc_dst);
    fvid_media::transcode_lossless(&avc_path, &avc_dst, LosslessTransform::default(), &opts)
        .expect("transcode AVC MP4 with unspecified VUI colour codes");
    let avc_mkv = WebmReader::open(
        BufReader::new(File::open(&avc_dst).expect("open AVC output mkv")),
        Limits::default(),
    )
    .expect("read AVC output mkv");
    assert_eq!(
        avc_mkv.tracks[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: true,
        }
    );
    let _ = std::fs::remove_file(&avc_dst);

    let hevc_path = fixture_dir().join("hevc-vui-unspecified-colr.mp4");
    let hevc_dst = temp_mkv("hevc-vui2-out");
    let _ = std::fs::remove_file(&hevc_dst);
    fvid_media::transcode_lossless(&hevc_path, &hevc_dst, LosslessTransform::default(), &opts)
        .expect("transcode HEVC MP4 with unspecified VUI colour codes");
    let hevc_mkv = WebmReader::open(
        BufReader::new(File::open(&hevc_dst).expect("open HEVC output mkv")),
        Limits::default(),
    )
    .expect("read HEVC output mkv");
    assert_eq!(
        hevc_mkv.tracks[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: true,
        }
    );
    let _ = std::fs::remove_file(&hevc_dst);
}
