#![cfg(feature = "player")]
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
struct Clean(PathBuf);
impl Drop for Clean {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn root(group: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(group)
}
fn cases(group: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(root(group).join("manifest.json")).unwrap(),
    )
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn explicit(c: &serde_json::Value) -> bool {
    c["codec"] != "ssa" && c["packet_durations_ns"][0].is_u64()
}
fn check(group: &str, selected: impl Fn(&serde_json::Value) -> bool, label: &str) {
    let tmp = std::env::temp_dir().join(format!(
        "fvid-subtitle-presentation-{label}-{}",
        std::process::id()
    ));
    std::fs::create_dir(&tmp).unwrap();
    let _clean = Clean(tmp.clone());
    for (i, c) in cases(group)
        .into_iter()
        .filter(|c| explicit(c) && selected(c))
        .enumerate()
    {
        let source = root(group).join(c["file"].as_str().unwrap());
        let wanted: Vec<_> = c["cues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| {
                (
                    v["text"].as_str().unwrap().to_owned(),
                    u128::from(v["start_ns"].as_u64().unwrap()),
                    u128::from(v["end_ns"].as_u64().unwrap()),
                )
            })
            .collect();
        let options = Default::default();
        let controls = Default::default();
        let mut previous = None;
        for api in 0..2 {
            let output = tmp.join(format!("{i}-{api}.mkv"));
            let plan = if api == 0 {
                fvid::media::plan_convert_subtitles_with_controls(&source, &options, &controls)
            } else {
                fvid_media::plan_convert_subtitles_with_controls(&source, &options, &controls)
            };
            if wanted.is_empty() {
                assert!(
                    plan.unwrap_err()
                        .contains("selected subtitle stream has no cues"),
                    "{}",
                    source.display()
                );
                let result = if api == 0 {
                    fvid::media::convert_subtitles_with_controls(
                        &source, &output, &options, &controls,
                    )
                } else {
                    fvid_media::convert_subtitles_with_controls(
                        &source, &output, &options, &controls,
                    )
                };
                assert!(
                    result
                        .unwrap_err()
                        .contains("selected subtitle stream has no cues")
                );
                assert!(!output.exists());
                continue;
            }
            plan.unwrap();
            let stats = if api == 0 {
                fvid::media::convert_subtitles_with_controls(&source, &output, &options, &controls)
            } else {
                fvid_media::convert_subtitles_with_controls(&source, &output, &options, &controls)
            }
            .unwrap();

            let bytes = std::fs::read(&output).unwrap();
            if let Some(ref p) = previous {
                assert_eq!(&bytes, p);
            } else {
                previous = Some(bytes.clone());
            }
            let mut reader = fvid::playback_webm_subtitles::WebmSubtitleReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            let actual: Vec<_> = reader
                .cues(0)
                .unwrap()
                .into_iter()
                .map(|v| (v.text, v.start.as_nanos(), v.end.as_nanos()))
                .collect();
            assert_eq!(actual, wanted, "{} api{api}", source.display());
            assert_eq!(stats.cues, wanted.len() as u64);
        }
    }
}
#[test]
fn ordinary_conversion_controls_keep_every_cue_and_clock() {
    check(
        "matroska-subtitle-delay",
        |c| c["delay_ns"] == 0,
        "delay-control",
    );
    check(
        "matroska-invisible-subtitles",
        |c| {
            c["invisible"]
                .as_array()
                .unwrap()
                .iter()
                .all(|v| v == false)
        },
        "flag-control",
    );
}
#[test]
fn conversion_materializes_codec_delay_without_reintroducing_hidden_negative_text() {
    check("matroska-subtitle-delay", |c| c["delay_ns"] != 0, "delay");
}
#[test]
fn conversion_does_not_turn_invisible_events_into_visible_captions() {
    check(
        "matroska-invisible-subtitles",
        |c| c["invisible"].as_array().unwrap().iter().any(|v| v == true),
        "flag",
    );
}
#[test]
fn hidden_packets_still_consume_selected_input_work_and_resource_budgets() {
    let tmp = std::env::temp_dir().join(format!(
        "fvid-subtitle-presentation-budget-{}",
        std::process::id()
    ));
    std::fs::create_dir(&tmp).unwrap();
    let _clean = Clean(tmp.clone());
    for source in [
        root("matroska-subtitle-delay").join("utf8-scale1000000-delay20000000.mkv"),
        root("matroska-invisible-subtitles").join("ass-explicit-mask6.mkv"),
    ] {
        for kind in 0..3 {
            let mut controls = fvid::media::CopyOptions::default();
            match kind {
                0 => controls.max_packets = Some(2),
                1 => controls.max_packet_bytes = 8,
                _ => controls.max_controlled_bytes = Some(1024),
            }
            let options = Default::default();
            let output = tmp.join("refused.mkv");
            let plan =
                fvid::media::plan_convert_subtitles_with_controls(&source, &options, &controls)
                    .unwrap_err();
            let execution =
                fvid::media::convert_subtitles_with_controls(&source, &output, &options, &controls)
                    .unwrap_err();
            let expected = match kind {
                0 => "packet work limit exceeded",
                1 => "exceeds",
                _ => "controlled memory budget",
            };
            assert!(plan.contains(expected), "{plan}");
            assert!(execution.contains(expected), "{execution}");
            assert!(!output.exists());
        }
    }
}
