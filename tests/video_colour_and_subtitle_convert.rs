//! Regression tests for Batch 571 (`FVID-0585..0589`):
//! - `FVID-0585`: `WebmVideoReader` and `Mp4VideoReader` preserve container `Colour` (`colr`) `matrix`
//!   and `full_range` fallback and per-frame `presented_signal` (`bitstream_colour()`) when single-layer
//!   AVC or HEVC VUI omits `colour_description_present_flag` or reports H.273 code `2` (`Unspecified`).
//! - `FVID-0586`: Matroska and MP4 VP9 (`color_space == 0`, `CS_UNKNOWN`) and AV1 (`matrix == 2`) fall
//!   back to container `Colour` (`colr`) `matrix` and `full_range` across `WebmVideoReader`,
//!   `Mp4VideoReader`, `owned_video_decode`, and `owned_ffv1_export`.
//! - `FVID-0587`: Standalone `.ssa` (`SubStation Alpha v4.00` with `Format: Marked, ...` and
//!   `Dialogue: Marked=N, ...`) conversion and planning in `fvid::native_subtitle` and
//!   `fvid_media::owned_subtitle`, normalizing `CodecPrivate` to canonical `Format: Layer, ...`.
//! - `FVID-0588`: Standalone `.vtt` (`WebVTT` with `WEBVTT` header, `NOTE`/`STYLE`/`REGION` blocks,
//!   cue identifiers, and short `MM:SS.mmm` timestamps) and Matroska `S_TEXT/WEBVTT` track subtitle
//!   conversion and planning in `fvid::native_subtitle` and `fvid_media::owned_subtitle`.
//! - `FVID-0589`: `fvid::subtitles::ass_block` case-insensitive `Dialogue:`, `Comment:`, `Format:`,
//!   and `Style:` prefix handling for Matroska `S_TEXT/ASS` and `S_TEXT/SSA` subtitle blocks.

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};
use std::time::Duration;

use fvid::color::ColourDescription;
use fvid::container::webm::{Limits, WebmReader};
use fvid::media::{CopyOptions, LosslessTransform};
use fvid::media_info::{SubtitleCodec, SubtitleConvertOptions};
use fvid::playback_native::NativeReader;
use fvid::subtitles::ass_block;
use fvid_media::owned_subtitle;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/video-colour-and-subtitle-convert")
}

fn temp_mkv(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fvid-b571-{tag}-{}.mkv", std::process::id()))
}

#[test]
fn webm_and_mp4_video_readers_preserve_container_colour_fallback_for_avc_vui_code_two() {
    for file_name in ["avc-vui2-bt2020-full.mkv", "avc-vui2-bt2020-full.mp4"] {
        let path = fixture_dir().join(file_name);
        let bytes = std::fs::read(&path).expect("read AVC VUI=2 fixture");
        let mut reader = NativeReader::without_memory_limit(Cursor::new(bytes))
            .unwrap_or_else(|e| panic!("open NativeReader on {file_name}: {e}"));
        assert_eq!(
            reader.colour(),
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            },
            "{file_name} open colour must combine VUI transfer=16 with container primaries=9, matrix=9, full_range=true"
        );
        let raw = reader
            .read_frame_raw()
            .unwrap_or_else(|e| panic!("decode first frame of {file_name}: {e}"))
            .expect("first frame present");
        let [w, h] = reader.dimensions();
        assert_eq!((w, h), (32, 16));
        drop(raw);
        assert_eq!(
            reader.colour(),
            ColourDescription {
                primaries: 9,
                transfer: 16,
                matrix: 9,
                full_range: true,
            },
            "{file_name} active colour after decoding frame must preserve presented_signal and container fallback"
        );
    }
}

#[test]
fn matroska_and_mp4_vp9_cs_unknown_fall_back_to_container_colour_in_reader_and_lossless_export() {
    for file_name in ["vp9-cs-unknown-bt2020.mkv", "vp9-cs-unknown-bt2020.mp4"] {
        let path = fixture_dir().join(file_name);
        let bytes = std::fs::read(&path).expect("read VP9 CS_UNKNOWN fixture");
        let mut reader = NativeReader::without_memory_limit(Cursor::new(bytes))
            .unwrap_or_else(|e| panic!("open NativeReader on {file_name}: {e}"));
        assert_eq!(
            reader.colour(),
            ColourDescription {
                primaries: 9,
                transfer: 13,
                matrix: 9,
                full_range: true,
            },
            "{file_name} reader colour must preserve container BT.2020 full-range metadata"
        );
        let raw = reader
            .read_frame_raw()
            .unwrap_or_else(|e| panic!("decode first VP9 frame of {file_name}: {e}"))
            .expect("first VP9 frame present");
        let [w, h] = reader.dimensions();
        assert_eq!((w, h), (64, 64));
        drop(raw);
        assert_eq!(
            reader.colour(),
            ColourDescription {
                primaries: 9,
                transfer: 13,
                matrix: 9,
                full_range: true,
            },
            "{file_name} reader colour after VP9 decode must not overwrite container matrix=9 with BT.601 (6)"
        );
    }

    let mkv_path = fixture_dir().join("vp9-cs-unknown-bt2020.mkv");
    let dst = temp_mkv("vp9-cs-unknown-lossless");
    let _ = std::fs::remove_file(&dst);
    fvid_media::transcode_lossless(
        &mkv_path,
        &dst,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .expect("transcode Matroska VP9 CS_UNKNOWN with container Colour");

    let out_mkv = WebmReader::open(
        BufReader::new(File::open(&dst).expect("open transcoded VP9 mkv")),
        Limits::default(),
    )
    .expect("read transcoded VP9 mkv");
    assert_eq!(
        out_mkv.tracks[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 13,
            matrix: 9,
            full_range: true,
        },
        "transcode_lossless on Matroska VP9 CS_UNKNOWN must preserve container matrix=9 instead of 6"
    );
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn standalone_ssa_v4_converts_and_plans_to_matroska_ass_with_normalized_layer() {
    let ssa_path = fixture_dir().join("standalone-v4.ssa");
    let dst = temp_mkv("standalone-ssa");
    let _ = std::fs::remove_file(&dst);
    let opts = CopyOptions::default();

    let plan = fvid::native_subtitle::try_plan_with_controls(
        &ssa_path,
        Some(&dst),
        &SubtitleConvertOptions {
            streams: vec![],
            codec: SubtitleCodec::Ass,
        },
        &opts,
    )
    .expect("plan standalone .ssa")
    .expect("standalone .ssa must be supported by native_subtitle");
    assert_eq!(plan.streams[0].codec, "ssa");
    assert_eq!(plan.steps[0].action, "copy");

    let stats = owned_subtitle::try_convert(&ssa_path, &dst, &[])
        .expect("convert standalone .ssa to Matroska S_TEXT/ASS")
        .expect("standalone .ssa conversion supported");
    assert_eq!(stats.cues, 2);

    let mut reader = WebmReader::open(
        BufReader::new(File::open(&dst).expect("open converted ssa mkv")),
        Limits::default(),
    )
    .expect("read converted ssa mkv");
    reader.scan_all().expect("scan converted ssa mkv");
    assert_eq!(reader.tracks[0].codec, "S_TEXT/ASS");
    let header = String::from_utf8(reader.tracks[0].codec_private.clone()).expect("UTF-8 header");
    assert!(
        header.contains("Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n"),
        "CodecPrivate must normalize Marked column to canonical Layer Format: line, got:\n{header}"
    );
    assert_eq!(reader.packets.len(), 2);
    assert_eq!(
        String::from_utf8(reader.read_packet(0).unwrap()).unwrap(),
        "0,0,Default,Alice,0,0,0,,First SSA cue"
    );
    assert_eq!(
        String::from_utf8(reader.read_packet(1).unwrap()).unwrap(),
        "1,1,Default,Bob,0,0,0,,Second, SSA cue"
    );
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn standalone_vtt_and_matroska_webvtt_convert_and_plan_to_matroska_ass() {
    let opts = CopyOptions::default();

    let vtt_path = fixture_dir().join("standalone-cues.vtt");
    let vtt_dst = temp_mkv("standalone-vtt");
    let _ = std::fs::remove_file(&vtt_dst);

    let vtt_plan = fvid::native_subtitle::try_plan_with_controls(
        &vtt_path,
        Some(&vtt_dst),
        &SubtitleConvertOptions {
            streams: vec![],
            codec: SubtitleCodec::Ass,
        },
        &opts,
    )
    .expect("plan standalone .vtt")
    .expect("standalone .vtt must be supported by native_subtitle");
    assert_eq!(vtt_plan.streams[0].codec, "webvtt");
    assert_eq!(vtt_plan.steps[0].action, "decode");

    let vtt_stats = owned_subtitle::try_convert(&vtt_path, &vtt_dst, &[])
        .expect("convert standalone .vtt to Matroska S_TEXT/ASS")
        .expect("standalone .vtt conversion supported");
    assert_eq!(vtt_stats.cues, 2);

    let mut vtt_reader = WebmReader::open(
        BufReader::new(File::open(&vtt_dst).expect("open converted vtt mkv")),
        Limits::default(),
    )
    .expect("read converted vtt mkv");
    vtt_reader.scan_all().expect("scan converted vtt mkv");
    assert_eq!(vtt_reader.packets.len(), 2);
    assert_eq!(vtt_reader.packets[0].pts_ns, 250_000_000);
    assert_eq!(vtt_reader.packets[0].duration_ns, Some(700_000_000));
    assert_eq!(
        String::from_utf8(vtt_reader.read_packet(0).unwrap()).unwrap(),
        "0,0,Default,,0,0,0,,{\\b1}First{\\b0} WebVTT cue"
    );
    assert_eq!(vtt_reader.packets[1].pts_ns, 1_100_000_000);
    assert_eq!(vtt_reader.packets[1].duration_ns, Some(800_000_000));
    assert_eq!(
        String::from_utf8(vtt_reader.read_packet(1).unwrap()).unwrap(),
        "1,0,Default,,0,0,0,,{\\i1}Second{\\i0} WebVTT cue"
    );
    let _ = std::fs::remove_file(&vtt_dst);

    let mkv_vtt_path = fixture_dir().join("matroska-webvtt.mkv");
    let mkv_dst = temp_mkv("matroska-vtt");
    let _ = std::fs::remove_file(&mkv_dst);

    let mkv_plan = fvid::native_subtitle::try_plan_with_controls(
        &mkv_vtt_path,
        Some(&mkv_dst),
        &SubtitleConvertOptions {
            streams: vec![],
            codec: SubtitleCodec::Ass,
        },
        &opts,
    )
    .expect("plan Matroska S_TEXT/WEBVTT")
    .expect("Matroska S_TEXT/WEBVTT must be supported by native_subtitle");
    assert_eq!(mkv_plan.streams[0].codec, "webvtt");
    assert_eq!(mkv_plan.steps[0].action, "decode");

    let mkv_stats = owned_subtitle::try_convert(&mkv_vtt_path, &mkv_dst, &[])
        .expect("convert Matroska S_TEXT/WEBVTT to S_TEXT/ASS")
        .expect("Matroska S_TEXT/WEBVTT conversion supported");
    assert_eq!(mkv_stats.cues, 2);

    let mut mkv_reader = WebmReader::open(
        BufReader::new(File::open(&mkv_dst).expect("open converted Matroska vtt")),
        Limits::default(),
    )
    .expect("read converted Matroska vtt");
    mkv_reader.scan_all().expect("scan converted Matroska vtt");
    assert_eq!(mkv_reader.packets.len(), 2);
    assert_eq!(
        String::from_utf8(mkv_reader.read_packet(0).unwrap()).unwrap(),
        "0,0,Default,,0,0,0,,{\\b1}Muxed{\\b0} WebVTT one"
    );
    assert_eq!(
        String::from_utf8(mkv_reader.read_packet(1).unwrap()).unwrap(),
        "1,0,Default,,0,0,0,,{\\i1}Muxed{\\i0} WebVTT two"
    );
    let _ = std::fs::remove_file(&mkv_dst);
}

#[test]
fn subtitles_ass_block_accepts_case_insensitive_dialogue_and_rejects_comment_format_style() {
    let (span, text) = ass_block("dialogue: 0,0:00:00.50,0:00:01.50,Default,Alice,0,0,0,,Hello\\hASS");
    assert_eq!(
        span,
        Some((Duration::from_millis(500), Duration::from_millis(1500)))
    );
    assert_eq!(text, "Hello\u{a0}ASS");

    let (span_upper, text_upper) =
        ass_block("DIALOGUE: Marked=0,0:00:01.00,0:00:02.00,Default,Bob,0,0,0,,Upper SSA");
    assert_eq!(
        span_upper,
        Some((Duration::from_millis(1000), Duration::from_millis(2000)))
    );
    assert_eq!(text_upper, "Upper SSA");

    for non_dialogue in [
        "comment: 0,0:00:00.50,0:00:01.50,Default,,0,0,0,,Hidden comment",
        "COMMENT: 0,0:00:00.50,0:00:01.50,Default,,0,0,0,,Hidden comment",
        "format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
        "style: Default,Arial,18,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,1,0,2,10,10,10,1",
    ] {
        assert_eq!(
            ass_block(non_dialogue),
            (None, String::new()),
            "non-dialogue event/header line {non_dialogue:?} must produce empty cue"
        );
    }
}
