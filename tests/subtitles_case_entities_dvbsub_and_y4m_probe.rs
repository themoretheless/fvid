#![cfg(feature = "player")]
use std::path::{Path, PathBuf};
use std::time::Duration;

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/subtitles-case-entities-dvbsub-and-y4m-probe")
        .join(name)
}

#[test]
fn subtitles_parse_sniffs_case_insensitive_ass_events_and_webvtt_header() {
    let ass_path = fixture_path("lowercase_events.ass");
    let ass_cues = fvid::subtitles::load(&ass_path)
        .expect("ASS sidecar with lowercase [events] header must load");
    assert_eq!(ass_cues.len(), 1);
    assert_eq!(ass_cues[0].start, Duration::from_secs(1));
    assert_eq!(ass_cues[0].end, Duration::from_secs(3));
    assert_eq!(ass_cues[0].text, "Rock & Roll's <best>");

    let vtt_path = fixture_path("mixed_case_blocks.vtt");
    let vtt_cues = fvid::subtitles::load(&vtt_path)
        .expect("WebVTT sidecar with lowercase webvtt header must load");
    assert_eq!(vtt_cues.len(), 1);
    assert_eq!(vtt_cues[0].start, Duration::from_secs(1));
    assert_eq!(vtt_cues[0].end, Duration::from_millis(2500));
    assert_eq!(vtt_cues[0].text, "Tom & Jerry: \"Hi!\"\u{a0}forever");
}

#[test]
fn subtitles_plain_decodes_named_and_numeric_html_entities_after_tag_stripping() {
    let srt_path = fixture_path("entities.srt");
    let cues = fvid::subtitles::load(&srt_path).expect("SRT with HTML entities must load");
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "AT&T <5G> \"Pro\" '26'");

    let (_, ass_text) = fvid::subtitles::ass_block(
        "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\i1}A &amp; B &lt;C&gt; &quot;D&quot; &apos;E&apos; &#65;&#x42;",
    );
    assert_eq!(ass_text, "A & B <C> \"D\" 'E' AB");
}

#[test]
fn webvtt_parser_and_owned_subtitle_converter_accept_case_insensitive_headers_and_blocks() {
    let vtt_path = fixture_path("mixed_case_blocks.vtt");
    let dest = std::env::temp_dir().join(format!(
        "fvid-batch577-vtt-case-{}.mkv",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&dest);

    let plan = fvid_media::owned_subtitle::try_plan_with_controls(
        &vtt_path,
        Some(&dest),
        &fvid_media_info::SubtitleConvertOptions {
            streams: vec![],
            codec: fvid_media_info::SubtitleCodec::Ass,
        },
        &fvid_control::CopyOptions::default(),
    )
    .expect("plan mixed-case WebVTT")
    .expect("mixed-case WebVTT must be accepted by owned subtitle planner");
    assert_eq!(plan.streams.len(), 1);
    assert_eq!(plan.streams[0].codec, "webvtt");

    let stats = fvid_media::owned_subtitle::try_convert(&vtt_path, &dest, &[])
        .expect("convert mixed-case WebVTT")
        .expect("mixed-case WebVTT must convert via owned subtitle converter");
    assert_eq!(stats.cues, 1);
    assert_eq!(stats.packets_out, 1);

    let mut reader = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
        std::io::BufReader::new(std::fs::File::open(&dest).unwrap()),
        fvid::container::webm::Limits::default(),
    )
    .unwrap();
    let cues = reader.cues(0).unwrap();
    assert_eq!(cues.len(), 1);
    assert_eq!(cues[0].text, "Tom & Jerry: \"Hi!\"\u{a0}forever");

    let _ = std::fs::remove_file(&dest);
}

#[test]
fn matroska_probe_and_codec_metadata_normalize_s_dvbsub_to_dvb_subtitle() {
    let path = fixture_path("dvbsub_track.mkv");
    let top_info = fvid::media::probe(&path).unwrap();
    assert_eq!(top_info.streams.len(), 1);
    assert_eq!(top_info.streams[0].media_type, "subtitle");
    assert_eq!(top_info.streams[0].codec, "dvb_subtitle");

    let native_info = fvid::native_probe::probe(&path).unwrap();
    assert_eq!(native_info.streams[0].codec, "dvb_subtitle");

    let owned_info = fvid_media::owned_webm_probe::probe_webm(&path).unwrap();
    assert_eq!(owned_info.streams.len(), 1);
    assert_eq!(owned_info.streams[0].media_type, "subtitle");
    assert_eq!(owned_info.streams[0].codec, "dvb_subtitle");

    let desc = fvid_media::owned_codec_metadata::descriptor("dvb_subtitle")
        .expect("dvb_subtitle must be registered in owned_codec_metadata");
    assert_eq!(desc.name, "dvb_subtitle");
}

#[test]
fn y4m_probe_rejects_malformed_pixel_aspect_and_color_range_tokens() {
    let bad_aspect = fixture_path("invalid_aspect.y4m");
    let aspect_err = fvid_media::owned_y4m_probe::probe_y4m(&bad_aspect).unwrap_err();
    assert!(
        aspect_err.contains("pixel aspect"),
        "expected pixel aspect error, got: {aspect_err}"
    );
    assert_eq!(
        fvid::media::probe(&bad_aspect).unwrap_err(),
        aspect_err
    );

    let bad_range = fixture_path("invalid_colorrange.y4m");
    let range_err = fvid_media::owned_y4m_probe::probe_y4m(&bad_range).unwrap_err();
    assert!(
        range_err.contains("colour range"),
        "expected colour range error, got: {range_err}"
    );
    assert_eq!(
        fvid::media::probe(&bad_range).unwrap_err(),
        range_err
    );
}
