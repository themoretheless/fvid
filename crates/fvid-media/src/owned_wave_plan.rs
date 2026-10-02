//! Metadata-only plans for owned WAVE copy operations.
use fvid_control::CopyOptions;
use fvid_media_info::{MediaPlan, PlanStep, PlanStream};
use std::path::{Path, PathBuf};
type Result<T> = std::result::Result<T, String>;
pub(crate) fn supports(source: &Path, options: &CopyOptions) -> bool {
    crate::owned_wave_remux::supports(source, Path::new("planned.wav"), options)
}
fn plan(
    sources: &[&Path],
    command: &str,
    interval: Option<(i64, i64)>,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    let (stats, frames, info) = crate::owned_wave_remux::inspect_copy(sources, interval, options)?;
    let codec = info.codec();
    let mut steps = Vec::new();
    if let Some((from, to)) = interval {
        steps.push(PlanStep {
            action: "interval".into(),
            detail: format!(
                "sample-exact [{from},{to}) µs, clipped at EOF; {frames} retained sample frames"
            ),
        });
    }
    steps.push(PlanStep{action:if sources.len()>1 {"concat"} else {"copy"}.into(),detail:format!("copy {} unchanged PCM bytes in {} frame-aligned blocks across {} input(s); {} Hz, {} channels, codec {}",stats.payload_bytes,stats.packets,stats.segments,info.sample_rate,info.channels,codec)});
    steps.push(PlanStep{action:"metadata".into(),detail:"preserve first input fmt/valid bits/channel mask and INFO metadata; apply container tag edits; rewrite RIFF/data/fact lengths".into()});
    steps.push(PlanStep{action:"publish".into(),detail:"publish completed .wav without overwriting; failure or cancellation removes temporary output".into()});
    Ok(MediaPlan{
        command:command.into(),input:sources[0].to_owned(),inputs:sources.iter().map(|p|p.to_path_buf()).collect(),
        streams:vec![PlanStream{index:0,media_type:"audio".into(),codec,disposition:if interval.is_some() {"trim_pcm"} else {"copy"}.into()}],
        steps,graph:None,notes:vec!["backend: owned WAVE; no external demuxer, decoder or muxer".into(),
        "owned output requires .wav; packet limits apply globally to frame-aligned input blocks of at most 4096 sample frames".into(),
        "metadata-only preflight shares execution validation; PCM reads, output permissions and publication are checked during execution".into()],
    })
}
pub fn plan_remux(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    plan(&[source], "remux", None, options)
}
pub fn plan_trim(source: &Path, from: i64, to: i64, options: &CopyOptions) -> Result<MediaPlan> {
    plan(&[source], "trim", Some((from, to)), options)
}
pub fn plan_trim_pcm(
    source: &Path,
    from: i64,
    to: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    plan(&[source], "trim-pcm", Some((from, to)), options)
}
pub fn plan_concat(sources: &[PathBuf], options: &CopyOptions) -> Result<MediaPlan> {
    if !(2..=256).contains(&sources.len()) {
        return Err("plan concat requires 2..=256 inputs".into());
    }
    let paths: Vec<_> = sources.iter().map(|p| p.as_path()).collect();
    plan(&paths, "concat", None, options)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_plans_share_execution_preflight_without_outputs_or_progress() {
        let dir = std::env::temp_dir().join(format!("fvid-wave-plans-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.wav");
        let second = dir.join("second.wav");
        crate::owned_wav_file::write_wav_integer_le(&first, 48000, 1, 24, &vec![17; 400 * 3], 4)
            .unwrap();
        crate::owned_wav_file::write_wav_integer_le(&second, 48000, 1, 24, &vec![73; 200 * 3], 4)
            .unwrap();
        let options = CopyOptions {
            max_packet_bytes: 48 * 3,
            progress: Some(fvid_control::ProgressHook::new(|_| {
                panic!("dry-run must not emit execution progress")
            })),
            ..Default::default()
        };
        let copy = crate::plan_remux(&first, &options).unwrap();
        assert_eq!(copy.command, "remux");
        assert_eq!(copy.streams[0].codec, "pcm_s24le");
        assert!(copy.graph.is_none());
        assert!(copy
            .steps
            .iter()
            .any(|s| s.detail.contains("1200 unchanged PCM bytes in 9")));
        let trim = crate::plan_trim_pcm(&first, 1000, 2500, &options).unwrap();
        assert_eq!(trim.command, "trim-pcm");
        assert_eq!(trim.streams[0].disposition, "trim_pcm");
        assert!(trim
            .steps
            .iter()
            .any(|s| s.detail.contains("72 retained sample frames")));
        assert_eq!(
            crate::plan_trim(&first, 1000, 2500, &options)
                .unwrap()
                .command,
            "trim"
        );
        let sources = vec![first.clone(), second.clone()];
        let concat = crate::plan_concat(&sources, &options).unwrap();
        assert_eq!(concat.command, "concat");
        assert_eq!(concat.inputs, sources);
        assert!(concat
            .steps
            .iter()
            .any(|s| s.detail.contains("1800 unchanged PCM bytes in 14")));
        let limited = CopyOptions {
            max_packets: Some(10),
            progress: None,
            ..options.clone()
        };
        let concat = crate::plan_concat(&sources, &limited).unwrap();
        // First file uses nine blocks (the final partial block counts), then
        // one 48-frame block is taken from the second file.
        assert!(concat
            .steps
            .iter()
            .any(|s| s.detail.contains("1344 unchanged PCM bytes in 10")));
        let output = dir.join("joined.wav");
        let stats = crate::concat(&sources, &output, &limited).unwrap();
        assert_eq!(stats.payload_bytes, 1344);
        assert_eq!(stats.packets, 10);
        assert!(crate::plan_trim_pcm(&first, 1, 2500, &options)
            .unwrap_err()
            .contains("not exactly representable"));
        assert!(crate::plan_concat(
            &sources,
            &CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            }
        )
        .unwrap_err()
        .contains("controlled memory"));
        let other = dir.join("other.wav");
        crate::owned_wav_file::write_wav_integer_le(&other, 44100, 1, 24, &[1, 2, 3], 4).unwrap();
        assert!(crate::plan_concat(&[first, other], &options)
            .unwrap_err()
            .contains("incompatible PCM"));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
