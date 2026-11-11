use std::path::{Path, PathBuf};

use super::common::{CliResult, make_progress_hook, parse_native_audio_limit};

pub(super) fn try_plan_remux(args: &[String]) -> CliResult<bool> {
    if !(args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("remux"))
    {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut output_format: Option<String> = None;
    let mut quiet = false;
    let mut items = args[2..].iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--quiet" | "-q" => quiet = true,
            "--progress" => {}
            "--output-format" => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate option: --output-format".into());
                }
                output_format = Some(items.next().ok_or("missing output format")?.clone());
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate remux limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    items.next().ok_or("missing remux limit")?,
                    &mut options,
                )?;
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                options.streams = items
                    .next()
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--metadata" => {
                let (key, value) = items
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                options
                    .metadata_delete
                    .push(items.next().ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                let raw = items.next().ok_or("missing stream metadata")?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let (key, value) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                options.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    value.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                let raw = items.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--" => {
                paths.extend(items.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported remux plan option: {arg}").into());
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.is_empty() || paths.len() > 2 {
        return Err("plan remux requires INPUT [OUTPUT]".into());
    }
    let source = paths[0].as_path();
    let resolved_dest_ext = paths
        .get(1)
        .and_then(|d| d.extension().and_then(|s| s.to_str()))
        .or_else(|| {
            output_format
                .as_deref()
                .map(|fmt| match fmt.to_ascii_lowercase().as_str() {
                    "mp4" => "mp4",
                    "m4a" => "m4a",
                    "mka" => "mka",
                    "mkv" | "matroska" => "mkv",
                    "wav" => "wav",
                    _ => "invalid",
                })
        });
    if let Some(dest_ext) = resolved_dest_ext {
        if !matches!(dest_ext, "mp4" | "m4a" | "mka" | "mkv" | "wav") {
            return Err("plan remux output must be .mp4, .m4a, .mka, .mkv, or .wav".into());
        }
        if dest_ext == "wav" {
            if !options.stream_metadata_set.is_empty()
                || !options.stream_metadata_delete.is_empty()
            {
                return Err("WAVE remux does not support stream metadata edits".into());
            }
            fvid_media::owned_wave_metadata::edit_info_chunks(
                &[],
                &options.metadata_delete,
                &options.metadata_set,
            )?;
        } else if matches!(dest_ext, "mp4" | "m4a")
            && (!options.metadata_set.is_empty()
                || !options.metadata_delete.is_empty()
                || !options.stream_metadata_set.is_empty()
                || !options.stream_metadata_delete.is_empty())
        {
            return Err("remux to .mp4/.m4a does not support metadata mutations".into());
        }
    }
    if paths.len() == 1
        && output_format.is_none()
        && limit_flags.is_empty()
        && options.streams.is_empty()
        && options.metadata_set.is_empty()
        && options.metadata_delete.is_empty()
        && options.stream_metadata_set.is_empty()
        && options.stream_metadata_delete.is_empty()
        && let Some(plan) = fvid::native_plan::remux(source)?
    {
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(true);
    }
    let plan = fvid::media::plan_remux(source, &options)?;
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    }
    Ok(true)
}

pub(super) fn try_remux(args: &[String]) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("remux") {
        return Ok(false);
    }
    if (args.len() == 3 || (args.len() == 4 && args[3] == "--progress"))
        && !args[1].starts_with('-')
        && !args[2].starts_with('-')
        && matches!(
            Path::new(&args[2]).extension().and_then(|s| s.to_str()),
            Some("mp4" | "m4a" | "mka" | "mkv")
        )
    {
        use std::io::Read;
        let mut prefix = [0; 8];
        let mut source = std::fs::File::open(&args[1])?;
        source.read_exact(&mut prefix)?;
        let progress = make_progress_hook(args.len() == 4);
        let dest_ext = Path::new(&args[2]).extension().and_then(|s| s.to_str());
        if prefix[..4] == [0x1a, 0x45, 0xdf, 0xa3] && matches!(dest_ext, Some("mkv" | "mka")) {
            let stats = fvid::native_export::remux_matroska(
                Path::new(&args[1]),
                Path::new(&args[2]),
                None,
                progress.as_ref(),
            )?;
            println!(
                "{}",
                serde_json::json!({"backend":"fvid","packets":stats.packets,"payload_bytes":stats.payload_bytes,"remux":"matroska-preserve"})
            );
            return Ok(true);
        }
        if fvid::container::adts::header(&prefix).is_some() {
            let packets = fvid::native_export::remux_adts_aac_controlled(
                Path::new(&args[1]),
                Path::new(&args[2]),
                None,
                progress.as_ref(),
            )?;
            println!("{{\"backend\":\"fvid\",\"audio_packets\":{packets}}}");
            return Ok(true);
        }
        if fvid::container::mp4::recognizes_prefix(&prefix)
            && matches!(dest_ext, Some("mka" | "mkv"))
            && fvid::native_export::is_single_track_mp4_aac(Path::new(&args[1]))?
        {
            let stats = fvid::native_export::remux_mp4_aac_matroska(
                Path::new(&args[1]),
                Path::new(&args[2]),
                None,
                progress.as_ref(),
            )?;
            println!("{{\"backend\":\"fvid\",\"audio_packets\":{}}}", stats.packets);
            return Ok(true);
        }
        if fvid::container::mp4::recognizes_prefix(&prefix)
            && matches!(dest_ext, Some("mka" | "mkv"))
            && fvid::native_export::is_native_mp4_matroska(Path::new(&args[1]))?
        {
            let stats = fvid::native_export::remux_mp4_matroska(
                Path::new(&args[1]),
                Path::new(&args[2]),
                None,
                progress.as_ref(),
            )?;
            println!("{{\"backend\":\"fvid\",\"packets\":{}}}", stats.packets);
            return Ok(true);
        }
        if fvid::container::mp4::recognizes_prefix(&prefix)
            && matches!(dest_ext, Some("mp4" | "m4a"))
        {
            fvid::native_export::remux_mp4_controlled(
                Path::new(&args[1]),
                Path::new(&args[2]),
                None,
                progress.as_ref(),
            )?;
            println!("{{\"backend\":\"fvid\",\"remux\":\"mp4-faststart\"}}");
            return Ok(true);
        }
    }

    let mut paths = Vec::<PathBuf>::new();
    let mut options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut quiet = false;
    let mut report_progress = false;
    let mut items = args[1..].iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--quiet" | "-q" => quiet = true,
            "--progress" => report_progress = true,
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate remux limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    items.next().ok_or("missing remux limit")?,
                    &mut options,
                )?;
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                options.streams = items
                    .next()
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--metadata" => {
                let (key, value) = items
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                options
                    .metadata_delete
                    .push(items.next().ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                let raw = items.next().ok_or("missing stream metadata")?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let (key, value) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                options.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    value.to_owned(),
                ));
            }
            "--stream-metadata-delete" => {
                let raw = items.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--" => {
                paths.extend(items.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported remux option: {arg}").into());
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if paths.len() != 2 {
        return Err("remux requires INPUT OUTPUT".into());
    }
    let dest_ext = paths[1].extension().and_then(|s| s.to_str());
    if !matches!(dest_ext, Some("mp4" | "m4a" | "mka" | "mkv" | "wav")) {
        return Err("remux output must be .mp4, .m4a, .mka, .mkv, or .wav".into());
    }
    if dest_ext == Some("wav") {
        if !options.stream_metadata_set.is_empty() || !options.stream_metadata_delete.is_empty() {
            return Err("WAVE remux does not support stream metadata edits".into());
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &options.metadata_delete,
            &options.metadata_set,
        )?;
    } else if matches!(dest_ext, Some("mp4" | "m4a"))
        && (!options.metadata_set.is_empty()
            || !options.metadata_delete.is_empty()
            || !options.stream_metadata_set.is_empty()
            || !options.stream_metadata_delete.is_empty())
    {
        return Err("remux to .mp4/.m4a does not support metadata mutations".into());
    }
    options.progress = make_progress_hook(report_progress);
    let stats = fvid::media::remux(&paths[0], &paths[1], &options)?;
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }
    Ok(true)
}
