use std::path::{Path, PathBuf};

use super::common::{CliResult, make_progress_hook, parse_native_audio_limit};

pub(super) fn try_plan_loudness(args: &[String]) -> CliResult<bool> {
    if !(args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("loudness"))
    {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let (mut selected, mut weights) = (None, None);
    let mut quiet = false;
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut options = args[2..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "-q" | "--quiet" => quiet = true,
            "--progress" => {}
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(option.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    option,
                    options.next().ok_or("missing audio limit")?,
                    &mut limits,
                )?;
            }
            "--streams" => {
                if selected.is_some() {
                    return Err("duplicate stream index".into());
                }
                selected = Some(
                    options
                        .next()
                        .ok_or("missing stream index")?
                        .parse::<usize>()?,
                );
            }
            "--channel-weights" => {
                if weights.is_some() {
                    return Err("duplicate channel weights".into());
                }
                weights = Some(
                    options
                        .next()
                        .ok_or("missing channel weights")?
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?,
                );
            }
            "--metadata" => {
                let raw = options.next().ok_or("missing metadata assignment")?;
                let (key, val) = raw
                    .split_once('=')
                    .ok_or("metadata assignment must be KEY=VALUE")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                limits.metadata_set.push((key.to_owned(), val.to_owned()));
            }
            "--metadata-delete" => {
                let key = options.next().ok_or("missing metadata key")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                limits.metadata_delete.push(key.to_owned());
            }
            "--stream-metadata" => {
                let raw = options.next().ok_or("missing stream metadata assignment")?;
                let (stream_str, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let index = stream_str.parse::<usize>()?;
                let (key, val) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                limits
                    .stream_metadata_set
                    .push((index, key.to_owned(), val.to_owned()));
            }
            "--stream-metadata-delete" => {
                let raw = options.next().ok_or("missing stream metadata key")?;
                let (stream_str, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                let index = stream_str.parse::<usize>()?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                limits.stream_metadata_delete.push((index, key.to_owned()));
            }
            "--" => {
                paths.extend(options.map(PathBuf::from));
                break;
            }
            _ if option.starts_with('-') => {
                return Err(format!("unsupported owned loudness plan option: {option}").into());
            }
            _ => paths.push(PathBuf::from(option)),
        }
    }
    if paths.len() != 1 {
        return Err("plan loudness requires INPUT".into());
    }
    let source = paths[0].as_path();
    limits.streams = selected.into_iter().collect();
    let plan = fvid::native_plan::loudness_with_options(source, &limits, weights.as_deref())?;
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    }
    Ok(true)
}

pub(super) fn try_normalize_loudness(
    args: &[String],
    normalization_plan: bool,
) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("normalize-loudness") {
        return Ok(false);
    }
    if args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h") {
        println!(
            "fvid media normalize-loudness INPUT OUTPUT.wav [--target-lufs -16] [--sample-peak-dbfs -1.5] [--streams INDEX] [--channel-weights W,...] [--quiet] [--progress]\nConstant gain preserves dynamics; the ceiling is sample peak, not true peak."
        );
        return Ok(true);
    }
    let mut paths = Vec::<PathBuf>::new();
    let (mut selected, mut weights, mut target_lufs, mut ceiling) = (None, None, None, None);
    let mut output_format: Option<String> = None;
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut quiet = false;
    let mut progress = false;
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--output-format" if normalization_plan => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate option: --output-format".into());
                }
                output_format = Some(options.next().ok_or("missing output format")?.clone());
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(option.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    option,
                    options.next().ok_or("missing audio limit")?,
                    &mut limits,
                )?;
            }
            "--streams" => {
                if selected.is_some() {
                    return Err("duplicate stream index".into());
                }
                selected = Some(
                    options
                        .next()
                        .ok_or("missing stream index")?
                        .parse::<usize>()?,
                );
            }
            "--channel-weights" => {
                if weights.is_some() {
                    return Err("duplicate channel weights".into());
                }
                weights = Some(
                    options
                        .next()
                        .ok_or("missing channel weights")?
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?,
                );
            }
            "--target-lufs" | "--sample-peak-dbfs" => {
                let slot = if option == "--target-lufs" {
                    &mut target_lufs
                } else {
                    &mut ceiling
                };
                if slot.is_some() {
                    return Err(format!("duplicate option: {option}").into());
                }
                *slot = Some(
                    options
                        .next()
                        .ok_or("missing normalization target")?
                        .parse::<f64>()?,
                );
            }
            "-q" | "--quiet" => quiet = true,
            "--progress" if !normalization_plan => progress = true,
            "--progress" => {}
            "--metadata" => {
                let raw = options.next().ok_or("missing metadata assignment")?;
                let (key, val) = raw
                    .split_once('=')
                    .ok_or("metadata assignment must be KEY=VALUE")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                limits.metadata_set.push((key.to_owned(), val.to_owned()));
            }
            "--metadata-delete" => {
                let key = options.next().ok_or("missing metadata key")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                limits.metadata_delete.push(key.to_owned());
            }
            "--stream-metadata" => {
                let raw = options.next().ok_or("missing stream metadata assignment")?;
                let (stream_str, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let index = stream_str.parse::<usize>()?;
                let (key, val) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                limits
                    .stream_metadata_set
                    .push((index, key.to_owned(), val.to_owned()));
            }
            "--stream-metadata-delete" => {
                let raw = options.next().ok_or("missing stream metadata key")?;
                let (stream_str, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                let index = stream_str.parse::<usize>()?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                limits.stream_metadata_delete.push((index, key.to_owned()));
            }
            "--" => {
                paths.extend(options.map(PathBuf::from));
                break;
            }
            _ if option.starts_with('-') => {
                return Err(format!("unsupported normalization option: {option}").into());
            }
            _ => paths.push(PathBuf::from(option)),
        }
    }
    let source = paths
        .first()
        .map(PathBuf::as_path)
        .ok_or(if normalization_plan {
            "plan normalize-loudness requires INPUT [OUTPUT.wav]"
        } else {
            "normalize-loudness requires INPUT OUTPUT.wav"
        })?;
    let destination = if normalization_plan {
        if paths.len() > 2 {
            return Err(
                "plan normalize-loudness requires INPUT [OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv]".into(),
            );
        }
        paths.get(1).map(PathBuf::as_path)
    } else {
        if paths.len() != 2 {
            return Err("normalize-loudness requires OUTPUT.wav".into());
        }
        Some(paths[1].as_path())
    };
    let resolved_dest_ext = destination
        .and_then(|d| d.extension().and_then(|s| s.to_str()))
        .or_else(|| {
            output_format
                .as_deref()
                .map(|fmt| match fmt.to_ascii_lowercase().as_str() {
                    "wav" => "wav",
                    "mka" => "mka",
                    "mkv" | "matroska" => "mkv",
                    _ => "invalid",
                })
        });
    if let Some(dest_ext) = resolved_dest_ext {
        if !matches!(dest_ext, "wav" | "mka" | "mkv") {
            return Err("normalization requires OUTPUT.wav, OUTPUT.mka or OUTPUT.mkv".into());
        }
        if dest_ext == "wav" {
            if !limits.stream_metadata_set.is_empty() || !limits.stream_metadata_delete.is_empty() {
                return Err(
                    "normalize-loudness to .wav does not support stream metadata mutations".into(),
                );
            }
            fvid_media::owned_wave_metadata::edit_info_chunks(
                &[],
                &limits.metadata_delete,
                &limits.metadata_set,
            )?;
        }
    }
    limits.streams = selected.into_iter().collect();
    let mut weight_limits = limits.clone();
    weight_limits.metadata_set.clear();
    weight_limits.metadata_delete.clear();
    weight_limits.stream_metadata_set.clear();
    weight_limits.stream_metadata_delete.clear();
    let weights = owned_loudness_weights(source, &weight_limits, weights)?;
    let defaults = fvid::native_pcm::NormalizeTarget::default();
    let target = fvid::native_pcm::NormalizeTarget {
        integrated_lufs: target_lufs.unwrap_or(defaults.integrated_lufs),
        sample_peak_dbfs: ceiling.unwrap_or(defaults.sample_peak_dbfs),
    };
    if normalization_plan {
        let plan = fvid::native_plan::normalize_loudness_with_options(
            source,
            &limits,
            Some(&weights),
            target,
        )?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(true);
    }
    let destination = destination.expect("checked destination path for normalize-loudness");
    let hook = progress.then(|| {
        fvid::native_pcm::NormalizationProgressHook::new(|event| {
            eprintln!(
                "{}",
                serde_json::json!({
                    "phase": event.phase.as_str(),
                    "packets": event.packets,
                    "payload_bytes": event.payload_bytes,
                    "phase_complete": event.phase_complete,
                    "done": event.done
                })
            );
        })
    });
    let report = fvid::native_pcm::normalize_loudness_file_with_options(
        source,
        destination,
        &weights,
        target,
        &limits,
        hook.as_ref(),
    )?;
    if !quiet {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "backend": "fvid",
                "sample_frames": report.sample_frames,
                "gain_db": report.gain_db,
                "peak_limited": report.peak_limited,
                "source_integrated_lufs": report.source.integrated_lufs,
                "target_lufs": target.integrated_lufs,
                "sample_peak_ceiling_dbfs": target.sample_peak_dbfs
            }))?
        );
    }
    Ok(true)
}

pub(super) fn try_loudness(args: &[String], normalization_plan: bool) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("loudness") {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut selected = None;
    let mut weights = None;
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut quiet = false;
    let mut progress = false;
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(option.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    option,
                    options.next().ok_or("missing audio limit")?,
                    &mut limits,
                )?;
            }
            "--streams" => {
                if selected.is_some() {
                    return Err("duplicate streams option".into());
                }
                selected = Some(
                    options
                        .next()
                        .ok_or("missing audio stream index")?
                        .parse::<usize>()?,
                );
            }
            "--channel-weights" => {
                if weights.is_some() {
                    return Err("duplicate channel weights".into());
                }
                weights = Some(
                    options
                        .next()
                        .ok_or("missing channel weights")?
                        .split(',')
                        .map(str::parse)
                        .collect::<Result<Vec<f64>, _>>()?,
                );
            }
            "-q" | "--quiet" => quiet = true,
            "--progress" if !normalization_plan => progress = true,
            "--progress" => {}
            "--metadata" => {
                let raw = options.next().ok_or("missing metadata assignment")?;
                let (key, val) = raw
                    .split_once('=')
                    .ok_or("metadata assignment must be KEY=VALUE")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                limits.metadata_set.push((key.to_owned(), val.to_owned()));
            }
            "--metadata-delete" => {
                let key = options.next().ok_or("missing metadata key")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                limits.metadata_delete.push(key.to_owned());
            }
            "--stream-metadata" => {
                let raw = options.next().ok_or("missing stream metadata assignment")?;
                let (stream_str, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let index = stream_str.parse::<usize>()?;
                let (key, val) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                limits
                    .stream_metadata_set
                    .push((index, key.to_owned(), val.to_owned()));
            }
            "--stream-metadata-delete" => {
                let raw = options.next().ok_or("missing stream metadata key")?;
                let (stream_str, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                let index = stream_str.parse::<usize>()?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                limits.stream_metadata_delete.push((index, key.to_owned()));
            }
            "--" => {
                paths.extend(options.map(PathBuf::from));
                break;
            }
            _ if option.starts_with('-') => {
                return Err(format!("unsupported owned loudness option: {option}").into());
            }
            _ => paths.push(PathBuf::from(option)),
        }
    }
    if paths.len() != 1 {
        return Err("loudness requires INPUT".into());
    }
    let source = paths[0].as_path();
    limits.streams = selected.into_iter().collect();
    limits.progress = make_progress_hook(progress);
    if limits.streams.len() > 1
        || !limits.metadata_set.is_empty()
        || !limits.metadata_delete.is_empty()
        || !limits.stream_metadata_set.is_empty()
        || !limits.stream_metadata_delete.is_empty()
    {
        return Err("owned loudness requires at most one audio stream and no metadata edits".into());
    }
    let weights = owned_loudness_weights(source, &limits, weights)?;
    let stats = fvid::native_pcm::measure_loudness_file_with_options(source, &weights, &limits)?;
    if !quiet {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "backend": "fvid",
                "sample_frames": stats.sample_frames,
                "measured_blocks": stats.measured_blocks,
                "integrated_lufs": stats.integrated_lufs,
                "channel_weights": weights,
                "sample_peak_dbfs": stats.sample_peak_dbfs,
                "range_lu": stats.range_lu
            }))?
        );
    }
    Ok(true)
}

pub(super) fn resolve_plan_mix_or_merge_destination<'a>(
    command: &str,
    paths: &'a [PathBuf],
    output_format: Option<&str>,
    limits: &fvid::media::CopyOptions,
) -> CliResult<(Option<bool>, &'a [PathBuf])> {
    let mut format_matroska = match output_format {
        None => None,
        Some("wav") => Some(false),
        Some("mka" | "mkv" | "matroska") => Some(true),
        Some(_) => {
            return Err(format!("{command} requires .wav, .mka or .mkv output").into());
        }
    };
    let sources = if command == "merge-audio" {
        if paths.len() == 3 {
            let ext = paths[0].extension().and_then(|s| s.to_str());
            let dest_matroska = match ext {
                Some("wav") => false,
                Some("mka" | "mkv") => true,
                _ => {
                    return Err(format!("{command} requires .wav, .mka or .mkv output").into());
                }
            };
            format_matroska = Some(dest_matroska);
            &paths[1..]
        } else {
            paths
        }
    } else if paths.len() >= 3 && !paths[0].exists() {
        let ext = paths[0].extension().and_then(|s| s.to_str());
        let dest_matroska = match ext {
            Some("wav") => false,
            Some("mka" | "mkv") => true,
            _ => {
                return Err(format!("{command} requires .wav, .mka or .mkv output").into());
            }
        };
        format_matroska = Some(dest_matroska);
        &paths[1..]
    } else {
        paths
    };
    if format_matroska == Some(false) {
        if !limits.stream_metadata_set.is_empty() || !limits.stream_metadata_delete.is_empty() {
            return Err("WAVE output does not support stream metadata mutations".into());
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &limits.metadata_delete,
            &limits.metadata_set,
        )?;
    }
    Ok((format_matroska, sources))
}

pub(super) fn native_mix(args: &[String]) -> CliResult<bool> {
    let planning = args.first().map(String::as_str) == Some("plan");
    let command = usize::from(planning);
    if args.get(command).map(String::as_str) != Some("mix-audio") {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut options = fvid::media_info::MixAudioOptions::default();
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut output_format: Option<String> = None;
    let mut quiet = false;
    let mut progress = false;
    let mut iter = args[command + 1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-q" | "--quiet" => quiet = true,
            "--progress" if !planning => progress = true,
            "--progress" => {}
            "--output-format" if planning => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate output-format option".into());
                }
                output_format =
                    Some(iter.next().ok_or("missing mix-audio output format")?.clone());
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    iter.next().ok_or("missing audio limit")?,
                    &mut limits,
                )?;
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                limits.streams = iter
                    .next()
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--metadata" => {
                let (key, value) = iter
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                limits.metadata_set.push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                limits
                    .metadata_delete
                    .push(iter.next().ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                let raw = iter.next().ok_or("missing stream metadata")?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let (key, value) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                limits.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    value.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                let raw = iter.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                limits
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--normalize" => options.normalize = true,
            "--no-normalize" => options.normalize = false,
            "--duration" => {
                options.duration = fvid::media_info::MixDuration::parse(
                    iter.next().ok_or("missing mix duration")?,
                )?
            }
            "--weights" => {
                options.weights = iter
                    .next()
                    .ok_or("missing mix weights")?
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .map(str::parse::<f32>)
                    .collect::<Result<Vec<_>, _>>()?;
                if options.weights.is_empty() {
                    return Err("mix weights must not be empty".into());
                }
            }
            "--" => {
                paths.extend(iter.map(PathBuf::from));
                break;
            }
            s if s.starts_with('-') => {
                return Err(format!("unsupported mix-audio option: {s}").into());
            }
            _ => paths.push(arg.into()),
        }
    }
    let start = usize::from(!planning);
    if paths.len() < start + 2 {
        return Err("mix-audio requires at least two inputs".into());
    }
    limits.progress = make_progress_hook(progress);
    if planning {
        let (_, sources) = resolve_plan_mix_or_merge_destination(
            "mix-audio",
            &paths,
            output_format.as_deref(),
            &limits,
        )?;
        let plan = fvid::media::plan_mix_audio_with_controls(sources, &options, &limits)?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        let sources = &paths[start..];
        let stats = fvid::media::mix_audio_with_controls(sources, &paths[0], &options, &limits)?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
    }
    Ok(true)
}

pub(super) fn native_merge(args: &[String]) -> CliResult<bool> {
    let planning = args.first().map(String::as_str) == Some("plan");
    let at = usize::from(planning);
    if args.get(at).map(String::as_str) != Some("merge-audio") {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut output_format: Option<String> = None;
    let mut quiet = false;
    let mut progress = false;
    let mut iter = args[at + 1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-q" | "--quiet" => quiet = true,
            "--progress" if !planning => progress = true,
            "--progress" => {}
            "--output-format" if planning => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate output-format option".into());
                }
                output_format =
                    Some(iter.next().ok_or("missing merge-audio output format")?.clone());
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    iter.next().ok_or("missing audio limit")?,
                    &mut limits,
                )?;
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                limits.streams = iter
                    .next()
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--metadata" => {
                let (key, value) = iter
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                limits.metadata_set.push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                limits
                    .metadata_delete
                    .push(iter.next().ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                let raw = iter.next().ok_or("missing stream metadata")?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let (key, value) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                limits.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    value.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                let raw = iter.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                limits
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--" => {
                paths.extend(iter.map(PathBuf::from));
                break;
            }
            s if s.starts_with('-') => {
                return Err(format!("unsupported merge-audio option: {s}").into());
            }
            _ => paths.push(arg.into()),
        }
    }
    if planning {
        if !(2..=3).contains(&paths.len()) {
            return Err("merge-audio requires exactly two inputs".into());
        }
    } else if paths.len() != 3 {
        return Err("merge-audio requires exactly two inputs".into());
    }
    limits.progress = make_progress_hook(progress);
    if planning {
        let (_, sources) = resolve_plan_mix_or_merge_destination(
            "merge-audio",
            &paths,
            output_format.as_deref(),
            &limits,
        )?;
        let plan = fvid::media::plan_merge_audio_with_controls(sources, &limits)?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        let sources = &paths[1..];
        let stats = fvid::media::merge_audio_with_controls(sources, &paths[0], &limits)?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
    }
    Ok(true)
}

fn owned_loudness_weights(
    source: &Path,
    options: &fvid::media::CopyOptions,
    weights: Option<Vec<f64>>,
) -> CliResult<Vec<f64>> {
    match weights {
        Some(weights) => Ok(weights),
        None => Ok(fvid::native_pcm::loudness_channel_weights_with_options(
            source, options,
        )?),
    }
}
