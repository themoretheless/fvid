//! The loudnorm command, preferring owned audio decoding and normalization.
use std::path::Path;
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h") {
        println!(
            "fvid media loudnorm INPUT OUTPUT.wav [--loudnorm-args I=-16:TP=-1.5:LRA=11] [--dual-pass] [--streams 0] [--metadata KEY=VALUE] [--metadata-delete KEY] [--quiet] [--progress] [--max-packet-bytes N] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N]"
        );
        return Ok(());
    }
    let mut paths = Vec::<std::path::PathBuf>::new();
    let mut options = fvid_media::CopyOptions::default();
    let mut normalization = None;
    let mut quiet = false;
    let mut progress = false;
    let mut dual_pass = false;
    let mut seen = std::collections::HashSet::new();
    let mut flags = args[1..].iter();
    while let Some(flag) = flags.next() {
        if flag == "--" {
            paths.extend(flags.map(std::path::PathBuf::from));
            break;
        }
        if !flag.starts_with('-') {
            paths.push(std::path::PathBuf::from(flag));
            continue;
        }
        let key_flag = if flag == "-q" { "--quiet" } else { flag.as_str() };
        if !matches!(
            key_flag,
            "--metadata"
                | "--metadata-delete"
                | "--stream-metadata"
                | "--stream-metadata-delete"
        ) && !seen.insert(key_flag)
        {
            return Err(format!("duplicate loudnorm option: {flag}").into());
        }
        match flag.as_str() {
            "--loudnorm-args" => normalization = Some(value(&mut flags, flag)?.as_str()),
            "--streams" => {
                options.streams = value(&mut flags, flag)?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?
            }
            "--metadata" => {
                let raw = value(&mut flags, flag)?;
                let (key, val) = raw
                    .split_once('=')
                    .ok_or("metadata assignment must be KEY=VALUE")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                options
                    .metadata_set
                    .push((key.to_owned(), val.to_owned()));
            }
            "--metadata-delete" => {
                let key = value(&mut flags, flag)?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                options.metadata_delete.push(key.to_owned());
            }
            "--stream-metadata" => {
                let raw = value(&mut flags, flag)?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata assignment must be INDEX:KEY=VALUE")?;
                let (key, val) = rest
                    .split_once('=')
                    .ok_or("stream metadata assignment must be INDEX:KEY=VALUE")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                options.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    val.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                let raw = value(&mut flags, flag)?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--max-packet-bytes" => options.max_packet_bytes = value(&mut flags, flag)?.parse()?,
            "--max-packets" => options.max_packets = Some(value(&mut flags, flag)?.parse()?),
            "--max-memory-mib" => {
                options.max_controlled_bytes =
                    Some(fvid_media::parse_max_memory_mib(value(&mut flags, flag)?)?)
            }
            "--max-rss-mib" => {
                options.max_rss_bytes =
                    Some(fvid_media::parse_max_rss_mib(value(&mut flags, flag)?)?)
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => progress = true,
            "--dual-pass" => {
                dual_pass = true;
            }
            _ => return Err(format!("unsupported loudnorm option: {flag}").into()),
        }
    }
    let source = paths
        .first()
        .map(std::path::PathBuf::as_path)
        .ok_or("loudnorm requires INPUT.wav OUTPUT.wav")?;
    if paths.len() != 2 {
        return Err("loudnorm requires OUTPUT.wav".into());
    }
    let destination = paths[1].as_path();
    validate_loudnorm_destination(destination, &options)?;
    if progress {
        options.progress = Some(fvid_media::ProgressHook::new(|event| {
            eprintln!(
                "{}",
                serde_json::json!({"packets":event.packets,"payload_bytes":event.payload_bytes,"done":event.done})
            )
        }));
    }
    let stats = match fvid::native_loudnorm::try_apply(
        source,
        destination,
        normalization,
        dual_pass,
        &options,
    )? {
        Some(stats) => stats,
        None => {
            #[cfg(feature = "media")]
            {
                if dual_pass {
                    fvid_media::apply_loudnorm_dual(source, destination, normalization, &options)?
                } else {
                    fvid_media::apply_loudnorm(source, destination, normalization, &options)?
                }
            }
            #[cfg(not(feature = "media"))]
            return Err("loudnorm input or compressed-audio policy is not yet supported by the owned bridge".into());
        }
    };
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }
    Ok(())
}

fn validate_loudnorm_destination(
    destination: &Path,
    options: &fvid_media::CopyOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    let dest_ext = destination.extension().and_then(|s| s.to_str());
    if !matches!(dest_ext, Some("wav" | "mka" | "mkv")) {
        return Err("loudnorm requires OUTPUT.wav, OUTPUT.mka or OUTPUT.mkv".into());
    }
    if dest_ext == Some("wav") {
        if !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty()
        {
            return Err("loudnorm to .wav does not support stream metadata mutations".into());
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &options.metadata_delete,
            &options.metadata_set,
        )?;
    } else if matches!(dest_ext, Some("mka" | "mkv")) {
        fvid::native_loudnorm::validate_matroska_destination_metadata(options)?;
    }
    Ok(())
}

fn value<'a>(flags: &mut std::slice::Iter<'a, String>, flag: &str) -> Result<&'a String, String> {
    flags
        .next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

pub fn run_plan(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h") {
        println!(
            "fvid media plan loudnorm INPUT [--loudnorm-args ARGS] [--dual-pass] [--streams INDEX] [--metadata KEY=VALUE] [--metadata-delete KEY] [--max-packet-bytes N] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N]"
        );
        return Ok(());
    }
    let mut paths = Vec::<std::path::PathBuf>::new();
    let mut normalization = None;
    let mut dual_pass = false;
    let mut quiet = false;
    let mut output_format = None;
    let mut options = fvid_media::CopyOptions::default();
    let mut seen = std::collections::HashSet::new();
    let mut flags = args[1..].iter();
    while let Some(flag) = flags.next() {
        if flag == "--" {
            paths.extend(flags.map(std::path::PathBuf::from));
            break;
        }
        if !flag.starts_with('-') {
            paths.push(std::path::PathBuf::from(flag));
            continue;
        }
        let key_flag = if flag == "-q" { "--quiet" } else { flag.as_str() };
        if !matches!(
            key_flag,
            "--metadata"
                | "--metadata-delete"
                | "--stream-metadata"
                | "--stream-metadata-delete"
        ) && !seen.insert(key_flag)
        {
            return Err(format!("duplicate loudnorm option: {flag}").into());
        }
        match flag.as_str() {
            "--loudnorm-args" => normalization = Some(value(&mut flags, flag)?.as_str()),
            "--dual-pass" => dual_pass = true,
            "--quiet" | "-q" => quiet = true,
            "--progress" => {}
            "--output-format" => output_format = Some(value(&mut flags, flag)?.as_str()),
            "--streams" => {
                options.streams = value(&mut flags, flag)?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?
            }
            "--metadata" => {
                let raw = value(&mut flags, flag)?;
                let (key, val) = raw
                    .split_once('=')
                    .ok_or("metadata assignment must be KEY=VALUE")?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                options
                    .metadata_set
                    .push((key.to_owned(), val.to_owned()));
            }
            "--metadata-delete" => {
                let key = value(&mut flags, flag)?;
                if key.is_empty() {
                    return Err("metadata key must be non-empty".into());
                }
                options.metadata_delete.push(key.to_owned());
            }
            "--stream-metadata" => {
                let raw = value(&mut flags, flag)?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata assignment must be INDEX:KEY=VALUE")?;
                let (key, val) = rest
                    .split_once('=')
                    .ok_or("stream metadata assignment must be INDEX:KEY=VALUE")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                options.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    val.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                let raw = value(&mut flags, flag)?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                if key.is_empty() {
                    return Err("stream metadata key must be non-empty".into());
                }
                options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--max-packet-bytes" => options.max_packet_bytes = value(&mut flags, flag)?.parse()?,
            "--max-packets" => options.max_packets = Some(value(&mut flags, flag)?.parse()?),
            "--max-memory-mib" => {
                options.max_controlled_bytes =
                    Some(fvid_media::parse_max_memory_mib(value(&mut flags, flag)?)?)
            }
            "--max-rss-mib" => {
                options.max_rss_bytes =
                    Some(fvid_media::parse_max_rss_mib(value(&mut flags, flag)?)?)
            }
            _ => return Err(format!("unsupported loudnorm plan option: {flag}").into()),
        }
    }
    let source = paths
        .first()
        .map(std::path::PathBuf::as_path)
        .ok_or("plan loudnorm requires INPUT")?;
    if paths.len() > 2 {
        return Err("plan loudnorm requires INPUT [OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv]".into());
    }
    if let Some(destination) = paths.get(1).map(std::path::PathBuf::as_path) {
        validate_loudnorm_destination(destination, &options)?;
    }
    if let Some(fmt) = output_format {
        let synthetic = match fmt.to_ascii_lowercase().as_str() {
            "wav" => Path::new("output.wav"),
            "mka" => Path::new("output.mka"),
            "mkv" | "matroska" => Path::new("output.mkv"),
            _ => return Err("loudnorm requires OUTPUT.wav, OUTPUT.mka or OUTPUT.mkv".into()),
        };
        validate_loudnorm_destination(synthetic, &options)?;
    }
    let plan = match fvid::native_loudnorm::try_plan(source, normalization, dual_pass, &options)? {
        Some(plan) => plan,
        None => {
            #[cfg(feature = "media")]
            {
                fvid_media::plan_loudnorm(source, normalization, dual_pass, &options)?
            }
            #[cfg(not(feature = "media"))]
            return Err(
                "loudnorm input or policy is not yet supported by the owned planner".into(),
            );
        }
    };
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    }
    Ok(())
}
