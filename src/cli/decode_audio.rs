use std::path::PathBuf;

use super::common::{
    CliResult, decode_time, make_progress_hook, parse_native_audio_limit, plain_decode,
};

pub(super) fn try_plan_decode_audio(args: &[String]) -> CliResult<bool> {
    if !(args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("decode-audio"))
    {
        return Ok(false);
    }
    let mut transform = fvid::native_plan::AudioDecodeTransform::default();
    let mut paths = Vec::<PathBuf>::new();
    let mut selected = None;
    let mut output_format: Option<String> = None;
    let mut quiet = false;
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let (mut from, mut to) = (None, None);
    let mut options = args[2..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "-q" | "--quiet" => quiet = true,
            "--progress" => {}
            "--output-format" => {
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
                    return Err("duplicate streams option".into());
                }
                selected = Some(
                    options
                        .next()
                        .ok_or("missing stream index")?
                        .parse::<usize>()?,
                );
            }
            "--from" | "--to" => {
                let slot = if option == "--from" {
                    &mut from
                } else {
                    &mut to
                };
                if slot.is_some() {
                    return Err("duplicate interval boundary".into());
                }
                let time = decode_time(options.next().ok_or("missing interval boundary")?)?;
                if time.subsec_nanos() % 1000 != 0 {
                    return Err("audio plan boundary must be representable in microseconds".into());
                }
                *slot = Some(
                    i64::try_from(time.as_micros())
                        .map_err(|_| "interval exceeds microsecond range")?,
                );
            }
            "--rate" | "--sample-rate" => {
                if transform.sample_rate.is_some() {
                    return Err("duplicate sample-rate option".into());
                }
                transform.sample_rate =
                    Some(options.next().ok_or("missing sample rate")?.parse()?);
            }
            "--channels" => {
                if transform.channels.is_some() {
                    return Err("duplicate channels option".into());
                }
                transform.channels = Some(options.next().ok_or("missing channels")?.parse()?);
            }
            "--volume" => {
                if transform.volume.is_some() {
                    return Err("duplicate volume option".into());
                }
                transform.volume = Some(options.next().ok_or("missing volume")?.parse()?);
            }
            "--metadata" => {
                let (key, value) = options
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                limits.metadata_set.push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                limits
                    .metadata_delete
                    .push(options.next().ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                let raw = options.next().ok_or("missing stream metadata")?;
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
                let raw = options.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                limits
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--" => {
                paths.extend(options.map(PathBuf::from));
                break;
            }
            _ if option.starts_with('-') => {
                return Err(format!("unsupported native audio plan option: {option}").into());
            }
            _ => paths.push(PathBuf::from(option)),
        }
    }
    if paths.is_empty() || paths.len() > 2 {
        return Err(
            "plan decode-audio requires INPUT [OUTPUT.wav|OUTPUT.f32le|OUTPUT.mka|OUTPUT.mkv]"
                .into(),
        );
    }
    let resolved_dest_ext = paths
        .get(1)
        .and_then(|d| d.extension().and_then(|s| s.to_str()))
        .or_else(|| {
            output_format
                .as_deref()
                .map(|fmt| match fmt.to_ascii_lowercase().as_str() {
                    "wav" => "wav",
                    "f32le" => "f32le",
                    "mka" => "mka",
                    "mkv" | "matroska" => "mkv",
                    _ => "invalid",
                })
        });
    if let Some(dest_ext) = resolved_dest_ext {
        if !matches!(dest_ext, "wav" | "f32le" | "mka" | "mkv") {
            return Err(
                "decode-audio requires OUTPUT.wav, OUTPUT.f32le, OUTPUT.mka or OUTPUT.mkv".into(),
            );
        }
        if dest_ext == "f32le"
            && (!limits.metadata_set.is_empty()
                || !limits.metadata_delete.is_empty()
                || !limits.stream_metadata_set.is_empty()
                || !limits.stream_metadata_delete.is_empty())
        {
            return Err("decode-audio to .f32le does not support metadata mutations".into());
        }
        if dest_ext == "wav" {
            if !limits.stream_metadata_set.is_empty() || !limits.stream_metadata_delete.is_empty() {
                return Err(
                    "decode-audio to .wav does not support stream metadata mutations".into(),
                );
            }
            fvid_media::owned_wave_metadata::edit_info_chunks(
                &[],
                &limits.metadata_delete,
                &limits.metadata_set,
            )?;
        } else if matches!(dest_ext, "mka" | "mkv") {
            fvid::native_loudnorm::validate_matroska_destination_metadata(&limits)?;
        }
    }
    transform.interval = match (from, to) {
        (None, None) => None,
        (Some(from), Some(to)) => Some((from, to)),
        _ => return Err("plan interval requires both --from and --to".into()),
    };
    limits.streams = selected.into_iter().collect();
    let plan = fvid::media::plan_decode_audio(&paths[0], &transform, &limits)?;
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    }
    Ok(true)
}

pub(super) fn try_decode_audio(args: &[String]) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("decode-audio") {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut parse_args = vec!["decode".to_owned()];
    let mut volume = None;
    let mut channels = None;
    let mut sample_rate = None;
    let mut report_progress = false;
    let mut selected = None;
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        if matches!(
            option.as_str(),
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib"
        ) {
            if !limit_flags.insert(option.as_str()) {
                return Err("duplicate audio limit".into());
            }
            parse_native_audio_limit(
                option,
                options.next().ok_or("missing audio limit")?,
                &mut limits,
            )?;
        } else if option == "--streams" {
            if selected.is_some() {
                return Err("duplicate streams option".into());
            }
            selected = Some(
                options
                    .next()
                    .ok_or("missing stream index")?
                    .parse::<usize>()?,
            );
        } else if option == "--progress" {
            report_progress = true;
        } else if option == "--volume" {
            if volume.is_some() {
                return Err("duplicate volume option".into());
            }
            volume = Some(options.next().ok_or("missing volume")?.parse::<f64>()?);
        } else if option == "--channels" {
            if channels.is_some() {
                return Err("duplicate channels option".into());
            }
            channels = Some(options.next().ok_or("missing channels")?.parse::<u16>()?);
        } else if option == "--sample-rate" || option == "--rate" {
            if sample_rate.is_some() {
                return Err("duplicate sample-rate option".into());
            }
            sample_rate = Some(
                options
                    .next()
                    .ok_or("missing sample-rate")?
                    .parse::<u32>()?,
            );
        } else if option == "--metadata" {
            let (key, value) = options
                .next()
                .ok_or("missing metadata")?
                .split_once('=')
                .ok_or("metadata must be KEY=VALUE")?;
            limits.metadata_set.push((key.to_owned(), value.to_owned()));
        } else if option == "--metadata-delete" {
            limits
                .metadata_delete
                .push(options.next().ok_or("missing metadata key")?.clone());
        } else if option == "--stream-metadata" {
            let raw = options.next().ok_or("missing stream metadata")?;
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
        } else if option == "--stream-metadata-delete" {
            let raw = options.next().ok_or("missing stream metadata key")?;
            let (index, key) = raw
                .split_once(':')
                .ok_or("stream metadata delete must be INDEX:KEY")?;
            limits
                .stream_metadata_delete
                .push((index.parse()?, key.to_owned()));
        } else if matches!(option.as_str(), "--from" | "--to") {
            parse_args.push(option.clone());
            parse_args.push(
                options
                    .next()
                    .ok_or("missing decode interval boundary")?
                    .clone(),
            );
        } else if matches!(option.as_str(), "--quiet" | "-q") {
            parse_args.push(option.clone());
        } else if option == "--" {
            paths.extend(options.map(PathBuf::from));
            break;
        } else if option.starts_with('-') {
            return Err(format!("unsupported native audio option: {option}").into());
        } else {
            paths.push(PathBuf::from(option));
        }
    }
    if paths.len() != 2 {
        return Err(
            "decode-audio requires INPUT OUTPUT.wav|OUTPUT.f32le|OUTPUT.mka|OUTPUT.mkv".into(),
        );
    }
    let source_path = paths[0].as_path();
    let dest_path = paths[1].as_path();
    let dest_ext = dest_path.extension().and_then(|s| s.to_str());
    if !matches!(dest_ext, Some("f32le" | "wav" | "mka" | "mkv")) {
        return Err(
            "decode-audio requires OUTPUT.wav, OUTPUT.f32le, OUTPUT.mka or OUTPUT.mkv".into(),
        );
    }
    if dest_ext == Some("f32le")
        && (!limits.metadata_set.is_empty()
            || !limits.metadata_delete.is_empty()
            || !limits.stream_metadata_set.is_empty()
            || !limits.stream_metadata_delete.is_empty())
    {
        return Err("decode-audio to .f32le does not support metadata mutations".into());
    }
    if dest_ext == Some("wav") {
        if !limits.stream_metadata_set.is_empty() || !limits.stream_metadata_delete.is_empty() {
            return Err(
                "decode-audio to .wav does not support stream metadata mutations".into(),
            );
        }
        fvid_media::owned_wave_metadata::edit_info_chunks(
            &[],
            &limits.metadata_delete,
            &limits.metadata_set,
        )?;
    } else if matches!(dest_ext, Some("mka" | "mkv")) {
        fvid::native_loudnorm::validate_matroska_destination_metadata(&limits)?;
    }
    parse_args.insert(1, source_path.to_string_lossy().into_owned());
    let (_, quiet, interval) =
        plain_decode(&parse_args)?.ok_or("invalid native audio arguments")?;
    limits.streams = selected.into_iter().collect();
    limits.progress = make_progress_hook(report_progress);
    let transform = fvid::media::AudioDecodeTransform {
        interval: interval.map(|(from, to)| {
            (
                i64::try_from(from.as_micros()).unwrap_or(i64::MAX),
                i64::try_from(to.as_micros()).unwrap_or(i64::MAX),
            )
        }),
        volume,
        channels: channels.map(i32::from),
        sample_rate: sample_rate.map(|r| r as i32),
    };
    let stats =
        fvid::media::decode_audio_transformed(source_path, dest_path, transform, &limits)?;
    if !quiet {
        println!(
            "{{\"backend\":\"fvid\",\"sample_frames\":{},\"decoded_frames\":{},\"sample_rate\":{},\"channels\":{},\"sample_format\":\"f32le\",\"decode_errors\":{}}}",
            stats.sample_frames,
            stats.decoded_frames,
            stats.sample_rate,
            stats.channels,
            stats.decode_errors
        );
    }
    Ok(true)
}
