//! Regression tests for Batch 570 (`FVID-0580..0584`):
//! - `FVID-0580`: Matroska `owned_video_decode` and `owned_ffv1_export` container `Colour` fallback
//!   when AVC/HEVC/AV1 bitstream VUI/sequence header reports unspecified code `2` or omits colour description.
//! - `FVID-0581`: `ColourDescription::filled_with` fallback when the primary statement reports H.273
//!   unspecified code `2` (`primaries == 2`, `transfer == 2`, `matrix == 2`) alongside explicit fallback codes.
//! - `FVID-0582`: `native_lossless::write_mp4_selected` (`fvid::media::transcode_lossless` / `transcode_mp4_ffv1`)
//!   preservation of bitstream VUI `colour` and HDR10 SEI `hdr` metadata when MP4 lacks `colr`/`mdcv`/`ccll`.
//! - `FVID-0583`: `fvid_media::plain_subtitle` and `fvid_media::parse_subtitle_text` support for `\h`
//!   non-breaking space (`\u{a0}`), HTML tags, general `{...}` override/comment blocks, SSA `Marked=N`
//!   payloads, custom `[Events]` `Format:` column order, case-insensitive `Dialogue:`, and inline WebVTT cue IDs.
//! - `FVID-0584`: `owned_subtitle_burn::supports`, `plan_burn_subtitles`, and `burn_subtitles` support
//!   for external `.vtt` (WebVTT) and `.ass`/`.ssa` (ASS/SSA) subtitle files without FFmpeg.

use std::fs::File;
use std::io::{BufReader, Cursor};
use std::path::{Path, PathBuf};

use fvid::color::ColourDescription;
use fvid::container::webm::{Limits, WebmReader};
use fvid::media::{CopyOptions, LosslessTransform};
use fvid::playback_native::NativeReader;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mkv-vui-fallback-and-subtitle-burn")
}

fn temp_mkv(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("fvid-b570-{tag}-{}.mkv", std::process::id()))
}

#[test]
fn matroska_video_decode_and_ffv1_export_preserve_container_colour_when_vui_codes_are_unspecified() {
    let mkv_path = fixture_dir().join("avc-vui2-container-colour.mkv");
    let dst = temp_mkv("mkv-vui2-out");
    let _ = std::fs::remove_file(&dst);

    fvid_media::transcode_lossless(
        &mkv_path,
        &dst,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .expect("transcode Matroska AVC with unspecified VUI codes and container Colour");

    let out_mkv = WebmReader::open(
        BufReader::new(File::open(&dst).expect("open output mkv")),
        Limits::default(),
    )
    .expect("read output mkv");
    assert_eq!(
        out_mkv.tracks[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: true,
        }
    );
    let _ = std::fs::remove_file(&dst);
}

#[test]
fn colour_description_filled_with_replaces_unspecified_code_two_with_explicit_fallback() {
    let mp4_path = fixture_dir().join("avc-colr-vui-cross-fill.mp4");
    let bytes = std::fs::read(&mp4_path).expect("read cross-fill MP4");
    let reader = NativeReader::without_memory_limit(Cursor::new(bytes))
        .expect("open NativeReader on cross-fill MP4");
    assert_eq!(
        reader.colour(),
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        }
    );
}

#[test]
fn native_lossless_write_mp4_preserves_bitstream_vui_colour_and_cross_filled_metadata() {
    let vui_only_path = fixture_dir().join("avc-vui-only-no-colr.mp4");
    let vui_only_dst = temp_mkv("mp4-vui-only-native");
    let _ = std::fs::remove_file(&vui_only_dst);

    let stats = fvid::media::transcode_lossless(
        &vui_only_path,
        &vui_only_dst,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .expect("fvid::media::transcode_lossless on VUI-only MP4");
    assert_eq!(stats.backend, "fvid");

    let out_mkv = WebmReader::open(
        BufReader::new(File::open(&vui_only_dst).expect("open VUI-only output mkv")),
        Limits::default(),
    )
    .expect("read VUI-only output mkv");
    assert_eq!(
        out_mkv.tracks[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: true,
        }
    );
    let _ = std::fs::remove_file(&vui_only_dst);

    let cross_path = fixture_dir().join("avc-colr-vui-cross-fill.mp4");
    let cross_dst = temp_mkv("mp4-cross-fill-native");
    let _ = std::fs::remove_file(&cross_dst);
    fvid::media::transcode_lossless(
        &cross_path,
        &cross_dst,
        LosslessTransform::default(),
        &CopyOptions::default(),
    )
    .expect("fvid::media::transcode_lossless on cross-fill MP4");
    let cross_mkv = WebmReader::open(
        BufReader::new(File::open(&cross_dst).expect("open cross-fill output mkv")),
        Limits::default(),
    )
    .expect("read cross-fill output mkv");
    assert_eq!(
        cross_mkv.tracks[0].colour,
        ColourDescription {
            primaries: 9,
            transfer: 16,
            matrix: 9,
            full_range: false,
        }
    );
    let _ = std::fs::remove_file(&cross_dst);
}

#[test]
fn fvid_media_plain_subtitle_and_parse_subtitle_text_support_hard_space_html_ssa_and_custom_format() {
    assert_eq!(
        fvid_media::owned_subtitle::plain_subtitle("0,Marked=0,Default,Alice,0,0,0,,{comment}<b>Hello</b>\\hworld\\Nnext"),
        "Hello\u{a0}world\nnext"
    );
    assert_eq!(
        fvid_media::owned_subtitle::plain_subtitle("dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,<i>Case</i>\\htest"),
        "Case\u{a0}test"
    );

    let vtt_text = std::fs::read_to_string(fixture_dir().join("burn-cues.vtt")).unwrap();
    let vtt_cues = fvid_media::owned_subtitle::parse_subtitle_text(&vtt_text).expect("parse WebVTT with inline cue id");
    assert_eq!(vtt_cues.len(), 1);
    assert_eq!(
        (vtt_cues[0].start_us, vtt_cues[0].end_us, vtt_cues[0].text.as_str()),
        (500_000, 1_000_000, "VTT\u{a0}Burn")
    );

    let ass_text = std::fs::read_to_string(fixture_dir().join("burn-cues.ass")).unwrap();
    let ass_cues = fvid_media::owned_subtitle::parse_subtitle_text(&ass_text).expect("parse ASS with custom Format:");
    assert_eq!(ass_cues.len(), 1);
    assert_eq!(
        (ass_cues[0].start_us, ass_cues[0].end_us, ass_cues[0].text.as_str()),
        (500_000, 1_000_000, "ASS\u{a0}Burn")
    );

    let ssa_text = std::fs::read_to_string(fixture_dir().join("burn-cues.ssa")).unwrap();
    let ssa_cues = fvid_media::owned_subtitle::parse_subtitle_text(&ssa_text).expect("parse SSA with Marked=0");
    assert_eq!(ssa_cues.len(), 1);
    assert_eq!(
        (ssa_cues[0].start_us, ssa_cues[0].end_us, ssa_cues[0].text.as_str()),
        (500_000, 1_000_000, "SSA\u{a0}Burn")
    );
}

#[test]
fn owned_subtitle_burn_supports_plans_and_executes_vtt_ass_and_ssa_files() {
    let y4m = fixture_dir().join("burn-source.y4m");
    let opts = CopyOptions::default();

    for ext in ["vtt", "ass", "ssa"] {
        let subs = fixture_dir().join(format!("burn-cues.{ext}"));
        assert!(
            fvid_media::owned_subtitle_burn::supports(&y4m, &subs, &opts),
            "owned_subtitle_burn::supports must admit .{ext}"
        );
        let plan = fvid::media::plan_burn_subtitles(&y4m, &subs, &opts)
            .unwrap_or_else(|e| panic!("plan_burn_subtitles on .{ext} failed: {e}"));
        assert!(plan.graph.is_none(), "plan on .{ext} must use owned burn-in");

        let dst = temp_mkv(&format!("burn-{ext}"));
        let _ = std::fs::remove_file(&dst);
        let stats = fvid::media::burn_subtitles(&y4m, &dst, &subs, &opts)
            .unwrap_or_else(|e| panic!("burn_subtitles on .{ext} failed: {e}"));
        assert_eq!(stats.video_frames, 3);

        let mut reader = WebmReader::open(
            BufReader::new(File::open(&dst).expect("open burned mkv")),
            Limits::default(),
        )
        .expect("read burned mkv");
        reader.scan_all().expect("scan burned mkv");
        assert_eq!(reader.packets.len(), 3);

        let mut decoder =
            fvid_media::owned_ffv1_decoder::Decoder::new(96, 64, 1 << 20).expect("FFV1 decoder");
        let mut original = vec![32u8; 96 * 64];
        original.extend(vec![64u8; 2 * 48 * 32]);
        for idx in 0..3 {
            let pkt = reader.read_packet(idx).expect("read packet");
            let pic = decoder.decode(&pkt).expect("decode FFV1 packet");
            if idx == 1 {
                assert_ne!(
                    pic.frame.data, original,
                    ".{ext} cue active on frame 1 (500ms..1000ms) must modify frame pixels"
                );
            } else {
                assert_eq!(
                    pic.frame.data, original,
                    ".{ext} cue inactive on frame {idx} must leave frame pixels unchanged"
                );
            }
        }
        let _ = std::fs::remove_file(&dst);
    }
}
