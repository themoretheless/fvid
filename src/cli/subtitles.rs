use super::common::{CliResult, make_progress_hook, parse_native_audio_limit};

pub(super) fn owned_subtitle_burn_command(args: &[String]) -> CliResult<bool> {
    let planning = args.first().map(String::as_str) == Some("plan");
    let args = if planning { &args[1..] } else { args };
    if args.first().map(String::as_str) != Some("burn-subtitles") {
        return Ok(false);
    }
    let mut paths = Vec::<std::path::PathBuf>::new();
    let mut options = fvid::media_control::CopyOptions::default();
    let mut subs = None;
    let mut quiet = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut values = args[1..].iter();
    while let Some(flag) = values.next() {
        if flag == "--" {
            paths.extend(values.map(std::path::PathBuf::from));
            break;
        }
        if !flag.starts_with('-') {
            paths.push(std::path::PathBuf::from(flag));
            continue;
        }
        let key_flag = if flag == "-q" { "--quiet" } else { flag.as_str() };
        if !matches!(
            key_flag,
            "--metadata" | "--metadata-delete" | "--stream-metadata" | "--stream-metadata-delete"
        ) && !seen.insert(key_flag)
        {
            return Err(format!("duplicate option: {flag}").into());
        }
        match flag.as_str() {
            "--subs" => subs = Some(values.next().ok_or("missing subtitle path")?),
            "-q" | "--quiet" => quiet = true,
            "--max-packets" => {
                options.max_packets = Some(values.next().ok_or("missing packet limit")?.parse()?)
            }
            "--max-packet-bytes" => {
                let limit = values.next().ok_or("missing packet byte limit")?.parse()?;
                if limit == 0 {
                    return Err("packet byte limit must be positive".into());
                }
                options.max_packet_bytes = limit;
            }
            "--max-rss-mib" => {
                options.max_rss_bytes = Some(fvid_media::parse_max_rss_mib(
                    values.next().ok_or("missing RSS limit")?,
                )?)
            }
            "--max-memory-mib" => {
                options.max_controlled_bytes = Some(fvid_media::parse_max_memory_mib(
                    values.next().ok_or("missing memory limit")?,
                )?)
            }
            "--streams" => {
                options.streams = values
                    .next()
                    .ok_or("missing streams")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?
            }
            "--metadata" => {
                let (key, value) = values
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata requires KEY=VALUE")?;
                options.metadata_set.push((key.into(), value.into()));
            }
            "--metadata-delete" => options
                .metadata_delete
                .push(values.next().ok_or("missing metadata key")?.clone()),
            "--stream-metadata" | "--stream-metadata-delete" => {
                let (index, text) = values
                    .next()
                    .ok_or("missing stream metadata")?
                    .split_once(':')
                    .ok_or("stream metadata requires INDEX:KEY")?;
                let index = index.parse()?;
                if flag == "--stream-metadata" {
                    let (key, value) = text
                        .split_once('=')
                        .ok_or("stream metadata requires INDEX:KEY=VALUE")?;
                    options
                        .stream_metadata_set
                        .push((index, key.into(), value.into()));
                } else {
                    options.stream_metadata_delete.push((index, text.into()));
                }
            }
            "--progress" => {
                if !planning {
                    options.progress = make_progress_hook(true);
                }
            }
            "--output-format" if planning => {
                let fmt = values.next().ok_or("missing output format")?;
                if !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska") {
                    return Err("owned FFV1 export requires .mkv".into());
                }
            }
            _ => return Err(format!("unsupported burn-subtitles option: {flag}").into()),
        }
    }
    if planning {
        if !(1..=2).contains(&paths.len()) {
            return Err("plan burn-subtitles requires INPUT --subs FILE.srt".into());
        }
        if let Some(dest) = paths.get(1)
            && dest
                .extension()
                .and_then(|e| e.to_str())
                .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
        {
            return Err("owned FFV1 export requires .mkv".into());
        }
    } else if paths.len() != 2 {
        return Err("burn-subtitles requires INPUT OUTPUT.mkv --subs FILE.srt".into());
    } else if paths[1]
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err("owned FFV1 export requires .mkv".into());
    }
    let subs = std::path::Path::new(subs.ok_or("burn-subtitles requires --subs FILE")?);
    let source = paths[0].as_path();
    let mut default_options = fvid::media_control::CopyOptions::default();
    default_options.cancel = options.cancel.clone();
    if !fvid_media::owned_subtitle_burn::supports(source, subs, &options)
        && !fvid_media::owned_subtitle_burn::supports(source, subs, &default_options)
    {
        return Ok(false);
    }
    let json = if planning {
        serde_json::to_string_pretty(&fvid_media::owned_subtitle_burn::plan_burn_subtitles(
            source, subs, &options,
        )?)?
    } else {
        serde_json::to_string_pretty(&fvid_media::owned_subtitle_burn::burn_subtitles(
            source,
            paths[1].as_path(),
            subs,
            &options,
        )?)?
    };
    if !quiet {
        println!("{json}");
    }
    Ok(true)
}

pub(super) fn try_convert_subtitles(args: &[String]) -> CliResult<bool> {
    let is_direct = args.first().map(String::as_str) == Some("convert-subtitles");
    let is_plan = args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("convert-subtitles");
    if !is_direct && !is_plan {
        return Ok(false);
    }
    let planning = is_plan;
    let cmd_idx = usize::from(planning);
    let mut paths = Vec::new();
    let mut streams = Vec::new();
    let mut output_format: Option<String> = None;
    let mut quiet = false;
    let mut progress = false;
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut options = args[cmd_idx + 1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "-q" | "--quiet" => quiet = true,
            "--progress" => {
                if !planning {
                    progress = true;
                }
            }
            "--output-format" if planning => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate option: --output-format".into());
                }
                output_format = Some(options.next().ok_or("missing output format")?.clone());
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(option.as_str()) {
                    return Err("duplicate subtitle limit".into());
                }
                parse_native_audio_limit(
                    option,
                    options.next().ok_or("missing subtitle limit")?,
                    &mut limits,
                )?;
            }
            "--codec" => {
                if !limit_flags.insert("--codec") {
                    return Err("duplicate codec option".into());
                }
                let codec_val = options.next().ok_or("missing subtitle codec")?;
                if fvid::media_info::SubtitleCodec::parse(codec_val).is_err() {
                    return Err(format!("unsupported subtitle codec: {codec_val}").into());
                }
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                streams = options
                    .next()
                    .ok_or("missing subtitle stream")?
                    .split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<Vec<usize>, _>>()?;
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
                paths.extend(options.map(std::path::PathBuf::from));
                break;
            }
            _ if option.starts_with('-') => {
                return Err(format!("unsupported convert-subtitles option: {option}").into());
            }
            _ => paths.push(std::path::PathBuf::from(option)),
        }
    }
    limits.streams = streams.clone();
    limits.progress = make_progress_hook(progress);
    if planning {
        if paths.is_empty() || paths.len() > 2 {
            return Err(
                "plan convert-subtitles requires INPUT [OUTPUT.mkv] [--codec ass] [--streams INDEX]"
                    .into(),
            );
        }
        let fmt_dest = match output_format.as_deref().map(|s| s.to_ascii_lowercase()) {
            None => None,
            Some(ref s) if s == "mkv" || s == "matroska" => Some(std::path::Path::new("output.mkv")),
            Some(_) => {
                return Err("convert-subtitles requires Matroska (.mkv) output".into());
            }
        };
        let plan = fvid::media::plan_convert_subtitles_for_destination(
            &paths[0],
            paths.get(1).map(std::path::PathBuf::as_path).or(fmt_dest),
            &fvid::media_info::SubtitleConvertOptions {
                streams,
                codec: fvid::media_info::SubtitleCodec::Ass,
            },
            &limits,
        )?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(true);
    }
    if paths.len() != 2 {
        return Err(
            "convert-subtitles requires INPUT OUTPUT.mkv [--codec ass] [--streams INDEX]".into(),
        );
    }
    let stats = fvid::media::convert_subtitles_with_controls(
        &paths[0],
        &paths[1],
        &fvid::media_info::SubtitleConvertOptions {
            streams,
            codec: fvid::media_info::SubtitleCodec::Ass,
        },
        &limits,
    )?;
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }
    Ok(true)
}
