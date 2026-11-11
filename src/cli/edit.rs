use std::path::{Path, PathBuf};

use super::common::{CliResult, decode_time, make_progress_hook, parse_native_audio_limit};
use super::decode_video::{geometry_decode_args, pixel_decode_args};
use super::transcode::owned_lossless_command_with_name;

pub(super) fn owned_xfade_command(args: &[String]) -> CliResult<bool> {
    let planning = args.first().map(String::as_str) == Some("plan");
    let args = if planning { &args[1..] } else { args };
    if args.first().map(String::as_str) != Some("xfade") {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let mut options = fvid::media_control::CopyOptions::default();
    let mut duration = None;
    let mut offset = 0;
    let mut transition = "fade";
    let mut quiet = false;
    let mut seen = std::collections::BTreeSet::new();
    let mut values = args[1..].iter();
    while let Some(flag) = values.next() {
        if flag == "--" {
            paths.extend(values.map(PathBuf::from));
            break;
        }
        if !flag.starts_with('-') {
            paths.push(PathBuf::from(flag));
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
            "-q" | "--quiet" => quiet = true,
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
            "--transition" => transition = values.next().ok_or("missing transition")?,
            "--xfade-duration" => {
                duration = Some(fvid::media_info::parse_time(
                    values.next().ok_or("missing xfade duration")?,
                )?)
            }
            "--xfade-offset" => {
                offset = fvid::media_info::parse_time(values.next().ok_or("missing xfade offset")?)?
            }
            "--max-packets" => {
                options.max_packets = Some(values.next().ok_or("missing max packets")?.parse()?)
            }
            "--max-packet-bytes" => {
                let limit = values.next().ok_or("missing packet byte limit")?.parse()?;
                if limit == 0 {
                    return Err("packet byte limit must be positive".into());
                }
                options.max_packet_bytes = limit;
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
            _ => return Err(format!("unsupported xfade option: {flag}").into()),
        }
    }
    if planning {
        if !(2..=3).contains(&paths.len()) {
            return Err(
                "plan xfade requires MAIN OTHER --xfade-duration SECONDS [--transition NAME] [--xfade-offset SECONDS]"
                    .into(),
            );
        }
        if let Some(dest) = paths.get(2)
            && dest
                .extension()
                .and_then(|e| e.to_str())
                .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
        {
            return Err("owned FFV1 export requires .mkv".into());
        }
    } else if paths.len() != 3 {
        return Err("xfade requires MAIN OTHER OUTPUT.mkv --xfade-duration SECONDS".into());
    }
    let duration = duration.ok_or("xfade requires --xfade-duration")?;
    let main = paths[0].as_path();
    let other = paths[1].as_path();
    let mut default_options = fvid::media_control::CopyOptions::default();
    default_options.cancel = options.cancel.clone();
    if planning {
        if let Some(plan) = fvid_media::owned_xfade::try_plan_xfade(
            main, other, transition, duration, offset, &options,
        )? {
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
            return Ok(true);
        }
        if fvid_media::owned_xfade::try_plan_xfade(
            main,
            other,
            transition,
            duration,
            offset,
            &default_options,
        )?
        .is_some()
        {
            let plan = fvid_media::owned_xfade::plan_xfade(
                main, other, transition, duration, offset, &options,
            )?;
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
            return Ok(true);
        }
        return Ok(false);
    }
    let destination = paths[2].as_path();
    if destination
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err("owned FFV1 export requires .mkv".into());
    }
    let stats = fvid_media::owned_xfade::try_xfade_video(
        main,
        other,
        destination,
        transition,
        duration,
        offset,
        &options,
    )?;
    let Some(stats) = stats else {
        if fvid_media::owned_xfade::try_plan_xfade(
            main,
            other,
            transition,
            duration,
            offset,
            &default_options,
        )?
        .is_some()
        {
            let stats = fvid_media::owned_xfade::xfade_video(
                main,
                other,
                destination,
                transition,
                duration,
                offset,
                &options,
            )?;
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
            return Ok(true);
        }
        return Ok(false);
    };
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }
    Ok(true)
}

pub(super) fn try_owned_overlay(args: &[String]) -> CliResult<bool> {
    let planning = args.first().map(String::as_str) == Some("plan");
    let command = usize::from(planning);
    let operation = args.get(command).map(String::as_str);
    if !matches!(
        operation,
        Some("overlay" | "transcode-lossless" | "transcode" | "decode")
    ) {
        return Ok(false);
    }
    let (mut from, mut to) = (None, None);
    let mut paths = Vec::<PathBuf>::new();
    let mut foreground = None;
    let (mut x, mut y) = (0i64, 0i64);
    let (mut quiet, mut report) = (false, false);
    let mut processing = Vec::<String>::new();
    let mut copy_options = fvid::media::CopyOptions::default();
    let mut custom_policy = false;
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut items = args[command + 1..].iter();
    while let Some(item) = items.next() {
        match item.as_str() {
            "--overlay-x" => {
                if !limit_flags.insert("--overlay-x") {
                    return Err("duplicate overlay-x option".into());
                }
                x = items.next().ok_or("missing overlay-x")?.parse()?;
            }
            "--overlay-y" => {
                if !limit_flags.insert("--overlay-y") {
                    return Err("duplicate overlay-y option".into());
                }
                y = items.next().ok_or("missing overlay-y")?.parse()?;
            }
            "--overlay" => {
                if !limit_flags.insert("--overlay") {
                    return Err("duplicate overlay option".into());
                }
                foreground = Some(PathBuf::from(items.next().ok_or("missing overlay path")?));
            }
            "--encoder" if operation == Some("transcode") => {
                if items.next().map(String::as_str) != Some("ffv1") {
                    return Ok(false);
                }
            }
            "--hflip" | "--vflip" => processing.push(item.clone()),
            "--crop" | "--scale" | "--pad" | "--transpose" | "--rotate" | "--unsharp" | "--eq"
            | "--hue" | "--colorize" | "--monochrome" | "--negate" | "--avgblur" | "--gblur"
            | "--bilateral" | "--boxblur" | "--pixelize" | "--chromashift" | "--sobel"
            | "--prewitt" | "--roberts" | "--kirsch" | "--scharr" | "--dilation" | "--erosion"
            | "--shuffleplanes" | "--lutyuv" | "--colorhold" | "--colorcontrast" | "--vibrance"
            | "--colorlevels" | "--colorchannelmixer" | "--fade" | "--exposure"
            | "--colorbalance" | "--colorcorrect" | "--cas" | "--grayworld" | "--curves"
            | "--vignette" | "--smartblur" | "--sab" | "--bitplanenoise" | "--deband"
            | "--perspective" | "--gradfun" | "--lenscorrection" | "--drawbox" | "--drawgrid"
            | "--removegrain" | "--yaepblur" | "--hqdn3d" | "--tmix" | "--lagfun"
            | "--framestep" | "--reverse" | "--shuffleframes" => {
                processing.push(item.clone());
                processing.push(
                    items
                        .next()
                        .ok_or("missing overlay processing value")?
                        .clone(),
                );
            }
            "--from" | "--to" => {
                if operation == Some("decode") {
                    let slot = if item == "--from" {
                        &mut from
                    } else {
                        &mut to
                    };
                    if slot.is_some() {
                        return Err("duplicate decode boundary".into());
                    }
                    *slot = Some(decode_time(items.next().ok_or("missing decode boundary")?)?);
                } else {
                    if !limit_flags.insert(item.as_str()) {
                        return Err("duplicate interval boundary".into());
                    }
                    processing.push(item.clone());
                    processing.push(items.next().ok_or("missing interval boundary")?.clone());
                }
            }
            "--max-packets" | "--max-packet-bytes" | "--max-rss-mib" | "--max-memory-mib"
                if operation != Some("decode") =>
            {
                if !limit_flags.insert(item.as_str()) {
                    return Err("duplicate overlay limit".into());
                }
                parse_native_audio_limit(
                    item,
                    items.next().ok_or("missing overlay limit")?,
                    &mut copy_options,
                )?;
                custom_policy = true;
            }
            "--streams" if operation != Some("decode") => {
                if !limit_flags.insert(item.as_str()) {
                    return Err("duplicate streams option".into());
                }
                copy_options.streams = items
                    .next()
                    .ok_or("missing stream selection")?
                    .split(',')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
                custom_policy = true;
            }
            "--metadata" if operation != Some("decode") => {
                let (key, value) = items
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata requires KEY=VALUE")?;
                copy_options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
                custom_policy = true;
            }
            "--metadata-delete" if operation != Some("decode") => {
                copy_options
                    .metadata_delete
                    .push(items.next().ok_or("missing metadata key")?.clone());
                custom_policy = true;
            }
            "--stream-metadata" | "--stream-metadata-delete" if operation != Some("decode") => {
                let (index, text) = items
                    .next()
                    .ok_or("missing stream metadata")?
                    .split_once(':')
                    .ok_or("stream metadata requires INDEX:KEY")?;
                let index = index.parse()?;
                if item == "--stream-metadata" {
                    let (key, value) = text
                        .split_once('=')
                        .ok_or("stream metadata requires INDEX:KEY=VALUE")?;
                    copy_options
                        .stream_metadata_set
                        .push((index, key.to_owned(), value.to_owned()));
                } else {
                    copy_options
                        .stream_metadata_delete
                        .push((index, text.to_owned()));
                }
                custom_policy = true;
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => {
                if !planning {
                    report = true;
                }
            }
            "--output-format" if planning && operation != Some("decode") => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate option: --output-format".into());
                }
                let fmt = items.next().ok_or("missing output format")?;
                if !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska") {
                    return Err("owned FFV1 export requires .mkv".into());
                }
            }
            "--" => {
                paths.extend(items.map(PathBuf::from));
                break;
            }
            _ if item.starts_with('-') => {
                if operation == Some("overlay") {
                    return Err(format!("unsupported overlay option: {item}").into());
                }
                return Ok(false);
            }
            _ => paths.push(item.into()),
        }
    }
    if operation != Some("overlay") && foreground.is_none() {
        return Ok(false);
    }
    if operation == Some("decode") {
        if report {
            return Ok(false);
        }
        let ([source], Some(overlay)) = (paths.as_slice(), foreground.as_deref()) else {
            return Err("decode overlay requires MAIN --overlay FOREGROUND".into());
        };
        let mut filter_args = vec!["decode".to_owned(), source.to_string_lossy().into_owned()];
        filter_args.extend(processing);
        let (filter_args, geometry) = geometry_decode_args(&filter_args)?;
        let (remaining, mut filters) = pixel_decode_args(&filter_args)?;
        filters.canonicalize_option_order();
        if remaining.len() != 2 {
            return Ok(false);
        }
        let interval = match (from, to) {
            (None, None) => None,
            (Some(from), Some(to)) if from < to => Some((from, to)),
            _ => return Err("decode interval requires both --from and --to with from < to".into()),
        };
        let spec = fvid::media_info::OverlaySpec {
            path: overlay.to_owned(),
            x: i32::try_from(x)?,
            y: i32::try_from(y)?,
        };
        if planning {
            let mut plan = fvid::native_plan::decode_overlay_with_geometry_and_filters(
                source, overlay, x, y, &geometry, &filters,
            )?;
            let mut index = 1;
            if !geometry.is_identity() {
                let detail = if let Some(rotate) = geometry.rotate {
                    format!(
                        "main crop {:?}, hflip {}, vflip {}, transpose {:?}, rotate {:?}, pad {:?}, scale {:?}",
                        geometry.crop,
                        geometry.horizontal_flip,
                        geometry.vertical_flip,
                        geometry.transpose,
                        rotate,
                        geometry.pad,
                        geometry.scale
                    )
                } else {
                    format!(
                        "main crop {:?}, hflip {}, vflip {}, transpose {:?}, pad {:?}, scale {:?}",
                        geometry.crop,
                        geometry.horizontal_flip,
                        geometry.vertical_flip,
                        geometry.transpose,
                        geometry.pad,
                        geometry.scale
                    )
                };
                plan.steps.insert(
                    index,
                    fvid::media_info::PlanStep {
                        action: "geometry".into(),
                        detail,
                    },
                );
                index += 1;
            }
            if !filters.is_empty() {
                plan.steps.insert(
                    index + 1,
                    fvid::media_info::PlanStep {
                        action: "filter".into(),
                        detail: "owned pixel filters in media option order after compositing"
                            .into(),
                    },
                );
            }
            if let Some((from, to)) = interval {
                plan.notes.push(format!(
                    "count presentation starts in [{from:?}, {to:?}); origins remain file-relative"
                ));
            }
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        } else {
            let stats = fvid::native_media::decode_video_pipeline_overlay(
                source,
                interval,
                &geometry,
                &filters,
                Some(&spec),
            )?;
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
        }
        return Ok(true);
    }
    let (source, overlay, destination) = if planning {
        match (paths.as_slice(), foreground.as_deref()) {
            ([source, overlay], None) => (source.as_path(), overlay.as_path(), None),
            ([source, overlay, destination], None) => {
                (source.as_path(), overlay.as_path(), Some(destination.as_path()))
            }
            ([source], Some(overlay)) => (source.as_path(), overlay, None),
            ([source, destination], Some(overlay)) => {
                (source.as_path(), overlay, Some(destination.as_path()))
            }
            _ => {
                return Err(
                    "plan overlay requires MAIN FOREGROUND or MAIN --overlay FOREGROUND".into(),
                );
            }
        }
    } else {
        match (paths.as_slice(), foreground.as_deref()) {
            ([source, overlay, destination], None) => {
                (source.as_path(), overlay.as_path(), Some(destination.as_path()))
            }
            ([source, destination], Some(overlay)) => {
                (source.as_path(), overlay, Some(destination.as_path()))
            }
            _ => return Err("overlay requires MAIN FOREGROUND OUTPUT.mkv".into()),
        }
    };
    if let Some(dest) = destination
        && dest
            .extension()
            .and_then(|e| e.to_str())
            .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err("owned FFV1 export requires .mkv".into());
    }
    let needs_owned_lossless = custom_policy
        || (!fvid::native_export::overlay_eligible(source)?
            && (fvid_media::owned_lossless::supports(
                source,
                &fvid::media_info::LosslessTransform {
                    overlay: Some(fvid::media_info::OverlaySpec {
                        path: overlay.to_path_buf(),
                        x: i32::try_from(x).unwrap_or(0),
                        y: i32::try_from(y).unwrap_or(0),
                    }),
                    ..Default::default()
                },
                &copy_options,
            ) || fvid_media::owned_lossless::supports(
                source,
                &fvid::media_info::LosslessTransform {
                    overlay: Some(fvid::media_info::OverlaySpec {
                        path: overlay.to_path_buf(),
                        x: i32::try_from(x).unwrap_or(0),
                        y: i32::try_from(y).unwrap_or(0),
                    }),
                    ..Default::default()
                },
                &fvid::media::CopyOptions::default(),
            )))
        || processing.iter().any(|p| {
            matches!(
                p.as_str(),
                "--from" | "--to" | "--framestep" | "--reverse" | "--shuffleframes"
            )
        });
    if needs_owned_lossless {
        copy_options.progress = make_progress_hook(report);
        let x32 = i32::try_from(x)?;
        let y32 = i32::try_from(y)?;
        if !processing.is_empty() {
            let mut lossless_args = if planning {
                vec![
                    "plan".to_owned(),
                    "transcode-lossless".to_owned(),
                    source.to_string_lossy().into_owned(),
                ]
            } else {
                vec![
                    "transcode-lossless".to_owned(),
                    source.to_string_lossy().into_owned(),
                    destination.unwrap().to_string_lossy().into_owned(),
                ]
            };
            lossless_args.push("--overlay".to_owned());
            lossless_args.push(overlay.to_string_lossy().into_owned());
            lossless_args.push("--overlay-x".to_owned());
            lossless_args.push(x32.to_string());
            lossless_args.push("--overlay-y".to_owned());
            lossless_args.push(y32.to_string());
            lossless_args.extend(processing);
            if !copy_options.streams.is_empty() {
                lossless_args.push("--streams".to_owned());
                lossless_args.push(
                    copy_options
                        .streams
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(","),
                );
            }
            if let Some(limit) = copy_options.max_packets {
                lossless_args.push("--max-packets".to_owned());
                lossless_args.push(limit.to_string());
            }
            if copy_options.max_packet_bytes != fvid::media::CopyOptions::default().max_packet_bytes
            {
                lossless_args.push("--max-packet-bytes".to_owned());
                lossless_args.push(copy_options.max_packet_bytes.to_string());
            }
            if let Some(rss) = copy_options.max_rss_bytes {
                lossless_args.push("--max-rss-mib".to_owned());
                lossless_args.push((rss >> 20).max(1).to_string());
            }
            if let Some(mem) = copy_options.max_controlled_bytes {
                lossless_args.push("--max-memory-mib".to_owned());
                lossless_args.push((mem >> 20).max(1).to_string());
            }
            for (k, v) in &copy_options.metadata_set {
                lossless_args.push("--metadata".to_owned());
                lossless_args.push(format!("{k}={v}"));
            }
            for k in &copy_options.metadata_delete {
                lossless_args.push("--metadata-delete".to_owned());
                lossless_args.push(k.clone());
            }
            for (idx, k, v) in &copy_options.stream_metadata_set {
                lossless_args.push("--stream-metadata".to_owned());
                lossless_args.push(format!("{idx}:{k}={v}"));
            }
            for (idx, k) in &copy_options.stream_metadata_delete {
                lossless_args.push("--stream-metadata-delete".to_owned());
                lossless_args.push(format!("{idx}:{k}"));
            }
            if quiet {
                lossless_args.push("--quiet".to_owned());
            }
            if report && !planning {
                lossless_args.push("--progress".to_owned());
            }
            return owned_lossless_command_with_name(&lossless_args, operation == Some("overlay"));
        }
        if planning {
            let mut plan =
                fvid_media::owned_lossless::plan_overlay(source, overlay, x32, y32, &copy_options)?;
            if operation != Some("overlay") {
                plan.command = "transcode-lossless".into();
            }
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        } else {
            let stats = fvid_media::owned_lossless::overlay_video(
                source,
                overlay,
                destination.unwrap(),
                x32,
                y32,
                &copy_options,
            )?;
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
        }
        return Ok(true);
    }
    if !fvid::native_export::overlay_eligible(source)? {
        return Ok(false);
    }
    let mut filter_args = vec!["decode".to_owned(), source.to_string_lossy().into_owned()];
    filter_args.extend(processing.clone());
    let (filter_args, geometry) = geometry_decode_args(&filter_args)?;
    let (remaining, mut filters) = pixel_decode_args(&filter_args)?;
    filters.canonicalize_option_order();
    if remaining.len() != 2 {
        return Ok(false);
    }
    if planning {
        let mut plan = fvid::native_plan::overlay_with_geometry_and_filters(
            source, overlay, x, y, &geometry, &filters,
        )?;
        let mut index = 1;
        if !geometry.is_identity() {
            let detail = if let Some(rotate) = geometry.rotate {
                format!(
                    "main crop {:?}, hflip {}, vflip {}, transpose {:?}, rotate {:?}, pad {:?}, scale {:?}",
                    geometry.crop,
                    geometry.horizontal_flip,
                    geometry.vertical_flip,
                    geometry.transpose,
                    rotate,
                    geometry.pad,
                    geometry.scale
                )
            } else {
                format!(
                    "main crop {:?}, hflip {}, vflip {}, transpose {:?}, pad {:?}, scale {:?}",
                    geometry.crop,
                    geometry.horizontal_flip,
                    geometry.vertical_flip,
                    geometry.transpose,
                    geometry.pad,
                    geometry.scale
                )
            };
            plan.steps.insert(
                index,
                fvid::media_info::PlanStep {
                    action: "geometry".into(),
                    detail,
                },
            );
            index += 1;
        }
        if !filters.is_empty() {
            plan.steps.insert(
                index + 1,
                fvid::media_info::PlanStep {
                    action: "filter".into(),
                    detail: format!(
                        "owned pixel filters in media option order after compositing; request {}",
                        processing.join(" ")
                    ),
                },
            );
        }
        if operation != Some("overlay") {
            plan.command = "transcode-lossless".into();
        }
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        let hook = make_progress_hook(report);
        let stats = fvid::native_export::overlay_video_transformed(
            source,
            overlay,
            destination.unwrap(),
            x,
            y,
            None,
            hook.as_ref(),
            &geometry,
            &filters,
        )?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&stats)?);
        }
    }
    Ok(true)
}

pub(super) fn try_video_trim(args: &[String]) -> CliResult<bool> {
    let plan = args.first().map(String::as_str) == Some("plan");
    let command = usize::from(plan);
    if args.get(command).map(String::as_str) != Some("trim") {
        return Ok(false);
    }
    let mut pre_paths = Vec::new();
    let mut pre_items = args[command + 1..].iter();
    while let Some(arg) = pre_items.next() {
        match arg.as_str() {
            "--output-format"
            | "--from"
            | "--to"
            | "--streams"
            | "--max-packets"
            | "--max-packet-bytes"
            | "--max-memory-mib"
            | "--max-rss-mib"
            | "--metadata"
            | "--metadata-delete"
            | "--stream-metadata"
            | "--stream-metadata-delete" => {
                let _ = pre_items.next();
            }
            "--" => {
                pre_paths.extend(pre_items.map(String::as_str));
                break;
            }
            _ if arg.starts_with('-') => {}
            _ => pre_paths.push(arg.as_str()),
        }
    }
    let matches = if plan {
        args.windows(2)
            .any(|s| s[0] == "--output-format" && s[1] == "y4m")
            || pre_paths.get(1).is_some_and(|p| {
                Path::new(p).extension().and_then(|s| s.to_str()) == Some("y4m")
            })
    } else {
        pre_paths.get(1).is_some_and(|p| {
            Path::new(p).extension().and_then(|s| s.to_str()) == Some("y4m")
        })
    };
    if !matches {
        return Ok(false);
    }
    let mut paths = Vec::new();
    let (mut from, mut to, mut selected) = (None, None, None);
    let mut quiet = false;
    let mut report = false;
    let mut copy_options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut items = args[command + 1..].iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--output-format" if plan => {
                if items.next().map(String::as_str) != Some("y4m") {
                    return Err("expected y4m output format".into());
                }
            }
            "--from" | "--to" => {
                let slot = if arg == "--from" {
                    &mut from
                } else {
                    &mut to
                };
                if slot.is_some() {
                    return Err("duplicate trim boundary".into());
                }
                let value = decode_time(items.next().ok_or("missing trim boundary")?)?;
                if value.subsec_nanos() % 1000 != 0 {
                    return Err("trim boundary must be representable in microseconds".into());
                }
                *slot =
                    Some(i64::try_from(value.as_micros()).map_err(|_| "trim boundary overflow")?);
            }
            "--streams" => {
                if selected.is_some() {
                    return Err("duplicate streams option".into());
                }
                selected = Some(items.next().ok_or("missing stream")?.parse::<usize>()?);
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate trim limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    items.next().ok_or("missing trim limit")?,
                    &mut copy_options,
                )?;
            }
            "--metadata" => {
                let (key, value) = items
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                copy_options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                copy_options
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
                copy_options.stream_metadata_set.push((
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
                copy_options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => {
                if !plan {
                    report = true;
                }
            }
            "--" => {
                paths.extend(items.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported native video trim option: {arg}").into());
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    copy_options.streams = selected.into_iter().collect();
    let (from, to) = (from.ok_or("--from required")?, to.ok_or("--to required")?);
    if plan {
        if paths.is_empty() {
            return Err("missing input".into());
        }
        if paths.len() > 2 {
            return Err(format!("unexpected plan trim argument: {}", paths[2].display()).into());
        }
        if let Some(dest) = paths.get(1)
            && dest.extension().and_then(|s| s.to_str()) != Some("y4m")
        {
            return Err("expected y4m output format".into());
        }
        let plan = fvid::native_plan::trim_y4m_with_options(&paths[0], from, to, &copy_options)?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        if paths.len() < 2 {
            return Err("trim requires INPUT and OUTPUT".into());
        }
        if paths.len() > 2 {
            return Err(format!("unexpected trim argument: {}", paths[2].display()).into());
        }
        copy_options.progress = make_progress_hook(report);
        let stats =
            fvid::native_export::trim_y4m_with_options(&paths[0], &paths[1], from, to, &copy_options)?;
        if !quiet {
            println!(
                "{}",
                serde_json::json!({"packets":stats.packets,"payload_bytes":stats.payload_bytes,"segments":1,"backend":"fvid","fvid_payload_copies":0})
            );
        }
    }
    Ok(true)
}

pub(super) fn resolve_plan_concat_destination<'a>(
    paths: &'a [PathBuf],
    output_format: Option<&str>,
    limits: &fvid::media::CopyOptions,
) -> CliResult<(Option<&'static str>, &'a [PathBuf])> {
    let mut format = match output_format {
        None => None,
        Some("wav") => Some("wav"),
        Some("mka") => Some("mka"),
        Some("mkv" | "matroska") => Some("mkv"),
        Some("mp4") => Some("mp4"),
        Some("m4a") => Some("m4a"),
        Some("y4m") => Some("y4m"),
        Some(_) => {
            return Err("concat output must be .mp4, .m4a, .mka, .mkv, .wav, or .y4m".into());
        }
    };
    let sources = if paths.len() >= 3 && !paths[0].exists() {
        let ext = match paths[0].extension().and_then(|s| s.to_str()) {
            Some("wav") => "wav",
            Some("mka") => "mka",
            Some("mkv") => "mkv",
            Some("mp4") => "mp4",
            Some("m4a") => "m4a",
            Some("y4m") => "y4m",
            _ => {
                return Err("concat output must be .mp4, .m4a, .mka, .mkv, .wav, or .y4m".into());
            }
        };
        format = Some(ext);
        &paths[1..]
    } else {
        paths
    };
    match format {
        Some("wav") => {
            if !limits.stream_metadata_set.is_empty() || !limits.stream_metadata_delete.is_empty() {
                return Err("WAVE concat does not support stream metadata edits".into());
            }
            fvid_media::owned_wave_metadata::edit_info_chunks(
                &[],
                &limits.metadata_delete,
                &limits.metadata_set,
            )?;
        }
        Some("y4m") => {
            if !limits.metadata_set.is_empty()
                || !limits.metadata_delete.is_empty()
                || !limits.stream_metadata_set.is_empty()
                || !limits.stream_metadata_delete.is_empty()
            {
                return Err("Y4M output does not support metadata mutations".into());
            }
        }
        Some("mp4" | "m4a") => {
            if !limits.metadata_set.is_empty()
                || !limits.metadata_delete.is_empty()
                || !limits.stream_metadata_set.is_empty()
                || !limits.stream_metadata_delete.is_empty()
            {
                return Err("concat to .mp4/.m4a does not support metadata mutations".into());
            }
        }
        _ => {}
    }
    Ok((format, sources))
}

pub(super) fn try_mp4_matroska_concat(args: &[String]) -> CliResult<bool> {
    let plan = args.first().map(String::as_str) == Some("plan");
    let command = usize::from(plan);
    if args.get(command).map(String::as_str) != Some("concat") {
        return Ok(false);
    }
    let mut paths = Vec::new();
    let mut quiet = false;
    let mut progress = false;
    let mut output_format = "mkv";
    let mut explicit_output_format = false;
    let mut copy_options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut items = args[command + 1..].iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--quiet" | "-q" => quiet = true,
            "--progress" if !plan => progress = true,
            "--progress" => {}
            "--output-format" if plan => {
                explicit_output_format = true;
                output_format = items
                    .next()
                    .map(String::as_str)
                    .ok_or("missing concat output format")?;
                if !matches!(Some(output_format), Some("matroska" | "mkv" | "mka" | "wav")) {
                    return Ok(false);
                }
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate concat limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    items.next().ok_or("missing concat limit")?,
                    &mut copy_options,
                )?;
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                copy_options.streams = items
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
                copy_options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                copy_options
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
                copy_options.stream_metadata_set.push((
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
                copy_options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--" => {
                paths.extend(items.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => return Ok(false),
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if plan {
        if paths.len() < 2 {
            return Ok(false);
        }
    } else {
        if paths.len() < 3 {
            return Ok(false);
        }
        let dest_ext = paths[0].extension().and_then(|s| s.to_str());
        if !matches!(dest_ext, Some("mkv" | "mka" | "wav")) {
            return Ok(false);
        }
    }
    let (dest, sources) = if plan {
        let (resolved_fmt, sources) = resolve_plan_concat_destination(
            &paths,
            explicit_output_format.then_some(output_format),
            &copy_options,
        )?;
        if let Some(fmt) = resolved_fmt {
            if !matches!(fmt, "mkv" | "mka" | "wav") {
                return Ok(false);
            }
            explicit_output_format = true;
            output_format = fmt;
        }
        (None, sources)
    } else {
        (Some(paths[0].as_path()), &paths[1..])
    };
    if plan && !explicit_output_format {
        let mut all_wave = true;
        let mut all_adts = true;
        for source in sources {
            all_wave &= fvid::native_pcm::is_wave(source)?;
            all_adts &= fvid::native_export::is_adts_source(source)?;
        }
        if all_adts {
            return Ok(false);
        }
        if all_wave
            && fvid::native_pcm::concat_info(sources).is_ok()
            && fvid_media::plan_concat(sources, &copy_options).is_ok()
        {
            return Ok(false);
        }
    }
    if plan {
        if output_format == "wav" {
            let mut all_wave = true;
            for source in sources {
                all_wave &= fvid::native_pcm::is_wave(source)?;
            }
            if all_wave && fvid::native_pcm::concat_info(sources).is_ok() {
                if !copy_options.stream_metadata_set.is_empty()
                    || !copy_options.stream_metadata_delete.is_empty()
                {
                    return Err("WAVE concat does not support stream metadata edits".into());
                }
                fvid_media::owned_wave_metadata::edit_info_chunks(
                    &[],
                    &copy_options.metadata_delete,
                    &copy_options.metadata_set,
                )?;
                let result = fvid::media::plan_concat(sources, &copy_options)?;
                if !quiet {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                return Ok(true);
            }
        }
        let planned_dest = match output_format {
            "mka" => "output.mka",
            "wav" => "output.wav",
            _ => "output.mkv",
        };
        let Some(result) = fvid::native_plan::concat_matroska_to_with_options(
            sources,
            Path::new(planned_dest),
            &copy_options,
        )?
        else {
            return Ok(false);
        };
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
    } else {
        let dest = dest.unwrap();
        copy_options.progress = make_progress_hook(progress);
        let Some(stats) = fvid::native_export::try_concat_mp4_matroska_with_options(
            sources,
            dest,
            &copy_options,
        )?
        else {
            return Ok(false);
        };
        if !quiet {
            println!(
                "{}",
                serde_json::json!({"packets":stats.packets,"payload_bytes":stats.payload_bytes,"segments":sources.len(),"backend":"fvid","fvid_payload_copies":0})
            );
        }
    }
    Ok(true)
}

pub(super) fn try_video_concat(args: &[String]) -> CliResult<bool> {
    let plan = args.first().map(String::as_str) == Some("plan");
    let command = usize::from(plan);
    if args.get(command).map(String::as_str) != Some("concat") {
        return Ok(false);
    }
    let mut pre_paths = Vec::new();
    let mut pre_items = args[command + 1..].iter();
    while let Some(arg) = pre_items.next() {
        match arg.as_str() {
            "--output-format"
            | "--streams"
            | "--max-packets"
            | "--max-packet-bytes"
            | "--max-memory-mib"
            | "--max-rss-mib"
            | "--metadata"
            | "--metadata-delete"
            | "--stream-metadata"
            | "--stream-metadata-delete" => {
                let _ = pre_items.next();
            }
            "--" => {
                pre_paths.extend(pre_items.map(String::as_str));
                break;
            }
            _ if arg.starts_with('-') => {}
            _ => pre_paths.push(arg.as_str()),
        }
    }
    let leading_plan_y4m = plan
        && pre_paths.len() >= 3
        && !Path::new(pre_paths[0]).exists()
        && Path::new(pre_paths[0]).extension().and_then(|s| s.to_str()) == Some("y4m");
    let matches = if plan {
        leading_plan_y4m
            || args
                .windows(2)
                .any(|s| s[0] == "--output-format" && s[1] == "y4m")
    } else {
        pre_paths.first().is_some_and(|p| {
            Path::new(p).extension().and_then(|s| s.to_str()) == Some("y4m")
        })
    };
    if !matches {
        return Ok(false);
    }
    let mut paths = Vec::new();
    let mut selected = None;
    let mut quiet = false;
    let mut report = false;
    let mut copy_options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut items = args[command + 1..].iter();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--output-format" if plan => {
                if items.next().map(String::as_str) != Some("y4m") {
                    return Err("expected y4m output format".into());
                }
            }
            "--streams" => {
                if selected.is_some() {
                    return Err("duplicate streams option".into());
                }
                selected = Some(items.next().ok_or("missing stream")?.parse::<usize>()?);
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate concat limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    items.next().ok_or("missing concat limit")?,
                    &mut copy_options,
                )?;
            }
            "--metadata" => {
                let (key, value) = items
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                copy_options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                copy_options
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
                copy_options.stream_metadata_set.push((
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
                copy_options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => {
                if !plan {
                    report = true;
                }
            }
            "--" => {
                paths.extend(items.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported native video concat option: {arg}").into());
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    copy_options.streams = selected.into_iter().collect();
    if plan {
        let sources = if leading_plan_y4m {
            &paths[1..]
        } else {
            paths.as_slice()
        };
        if sources.len() < 2 {
            return Err("plan concat requires INPUT INPUT...".into());
        }
        let plan = fvid::native_plan::concat_y4m_with_options(sources, &copy_options)?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        if paths.len() < 3 {
            return Err("concat requires OUTPUT.y4m INPUT INPUT...".into());
        }
        let dest = &paths[0];
        let sources = &paths[1..];
        copy_options.progress = make_progress_hook(report);
        let stats = fvid::native_export::concat_y4m_with_options(sources, dest, &copy_options)?;
        if !quiet {
            println!(
                "{}",
                serde_json::json!({"packets":stats.packets,"payload_bytes":stats.payload_bytes,"segments":sources.len(),"backend":"fvid","fvid_payload_copies":0})
            );
        }
    }
    Ok(true)
}

pub(super) fn try_native_audio_concat(args: &[String]) -> CliResult<bool> {
    let plan = args.first().map(String::as_str) == Some("plan");
    let start = usize::from(plan);
    if args.get(start).map(String::as_str) != Some("concat") {
        return Ok(false);
    }
    let mut paths = Vec::new();
    let mut quiet = false;
    let mut progress = false;
    let mut output_format = None;
    let mut copy_options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut custom_options = false;
    let mut raw_streams = None;
    let mut options = args[start + 1..].iter();
    while let Some(arg) = options.next() {
        match arg.as_str() {
            "--quiet" | "-q" => quiet = true,
            "--progress" if !plan => progress = true,
            "--progress" => {}
            "--output-format" if plan => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate output-format option".into());
                }
                output_format =
                    Some(options.next().ok_or("missing concat output format")?.clone());
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                let raw = options.next().ok_or("missing stream indices")?;
                let streams: Vec<usize> = raw
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
                raw_streams = Some(streams);
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    options.next().ok_or("missing audio limit")?,
                    &mut copy_options,
                )?;
                custom_options = true;
            }
            "--metadata" => {
                let (key, value) = options
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                copy_options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
                custom_options = true;
            }
            "--metadata-delete" => {
                copy_options
                    .metadata_delete
                    .push(options.next().ok_or("missing metadata key")?.clone());
                custom_options = true;
            }
            "--stream-metadata" => {
                let raw = options.next().ok_or("missing stream metadata")?;
                let (index, rest) = raw
                    .split_once(':')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                let (key, value) = rest
                    .split_once('=')
                    .ok_or("stream metadata must be INDEX:KEY=VALUE")?;
                copy_options.stream_metadata_set.push((
                    index.parse()?,
                    key.to_owned(),
                    value.to_owned(),
                ));
                custom_options = true;
            }
            "--stream-metadata-delete" => {
                let raw = options.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                copy_options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
                custom_options = true;
            }
            "--" => {
                paths.extend(options.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported native audio concat option: {arg}").into());
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    if plan {
        if paths.len() < 2 {
            return Err("plan concat requires INPUT INPUT...".into());
        }
    } else if paths.len() < 3 {
        return Err("concat requires OUTPUT INPUT INPUT...".into());
    }
    let (dest, sources) = if plan {
        let (_, sources) =
            resolve_plan_concat_destination(&paths, output_format.as_deref(), &copy_options)?;
        (None, sources)
    } else {
        (Some(paths[0].as_path()), &paths[1..])
    };
    if let Some(dest) = dest {
        let dest_ext = dest.extension().and_then(|s| s.to_str());
        if !matches!(
            dest_ext,
            Some("mp4" | "m4a" | "mka" | "mkv" | "wav" | "y4m")
        ) {
            return Err("concat output must be .mp4, .m4a, .mka, .mkv, .wav, or .y4m".into());
        }
        if dest_ext == Some("wav") {
            if !copy_options.stream_metadata_set.is_empty()
                || !copy_options.stream_metadata_delete.is_empty()
            {
                return Err("WAVE concat does not support stream metadata edits".into());
            }
            fvid_media::owned_wave_metadata::edit_info_chunks(
                &[],
                &copy_options.metadata_delete,
                &copy_options.metadata_set,
            )?;
        } else if dest_ext == Some("y4m")
            && (!copy_options.metadata_set.is_empty()
                || !copy_options.metadata_delete.is_empty()
                || !copy_options.stream_metadata_set.is_empty()
                || !copy_options.stream_metadata_delete.is_empty())
        {
            return Err("Y4M output does not support metadata mutations".into());
        } else if matches!(dest_ext, Some("mp4" | "m4a"))
            && (!copy_options.metadata_set.is_empty()
                || !copy_options.metadata_delete.is_empty()
                || !copy_options.stream_metadata_set.is_empty()
                || !copy_options.stream_metadata_delete.is_empty())
        {
            return Err("concat to .mp4/.m4a does not support metadata mutations".into());
        }
    }
    let adts = fvid::native_export::is_adts_source(&sources[0])?;
    if !adts && !fvid::native_pcm::is_wave(&sources[0])? {
        return Ok(false);
    }
    if let Some(streams) = raw_streams {
        if streams != [0] {
            return Err(if adts {
                "ADTS has only stream 0".into()
            } else {
                "WAVE has only stream 0".into()
            });
        }
        copy_options.streams = streams;
    }
    if plan {
        let plan = if custom_options {
            fvid::media::plan_concat(sources, &copy_options)?
        } else if adts {
            fvid::native_plan::concat_adts(sources)?
        } else {
            fvid::native_plan::concat_wave(sources)?
        };
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        let dest = dest.unwrap();
        let hook = make_progress_hook(progress);
        copy_options.progress = hook.clone();
        if custom_options {
            let stats = fvid::media::concat(sources, dest, &copy_options)?;
            if !quiet {
                println!(
                    "{{\"packets\":{},\"payload_bytes\":{},\"segments\":{},\"backend\":\"fvid\",\"fvid_payload_copies\":{}}}",
                    stats.packets, stats.payload_bytes, stats.segments, stats.fvid_payload_copies
                );
            }
            return Ok(true);
        }
        if adts {
            let stats = fvid::native_export::concat_adts_aac(sources, dest, None, hook.as_ref())?;
            if !quiet {
                println!(
                    "{{\"packets\":{},\"payload_bytes\":{},\"segments\":{},\"backend\":\"fvid\",\"fvid_payload_copies\":0}}",
                    stats.packets,
                    stats.payload_bytes,
                    sources.len()
                );
            }
            return Ok(true);
        }
        let stats = fvid::native_pcm::concat_wave(sources, dest, None, hook.as_ref())?;
        if !quiet {
            println!(
                "{{\"packets\":{},\"sample_frames\":{},\"payload_bytes\":{},\"segments\":{},\"backend\":\"fvid\",\"fvid_payload_copies\":0}}",
                stats.packets,
                stats.sample_frames,
                stats.payload_bytes,
                sources.len()
            );
        }
    }
    Ok(true)
}

/// Shared native CLI routing for PCM trim and its read-only plan.
pub(super) fn try_native_audio_trim(args: &[String]) -> CliResult<bool> {
    let plan = args.first().map(String::as_str) == Some("plan");
    let command_index = usize::from(plan);
    let Some(command) = args.get(command_index).map(String::as_str) else {
        return Ok(false);
    };
    if !matches!(command, "trim" | "trim-pcm") {
        return Ok(false);
    }
    let mut paths = Vec::<PathBuf>::new();
    let (mut from, mut to, mut selected) = (None, None, None);
    let mut output_format: Option<String> = None;
    let (mut quiet, mut report) = (false, false);
    let mut limits = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut args_iter = args[command_index + 1..].iter();
    while let Some(arg) = args_iter.next() {
        match arg.as_str() {
            "--output-format" if plan => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate option: --output-format".into());
                }
                output_format = Some(args_iter.next().ok_or("missing output format")?.clone());
            }
            "--max-packets" | "--max-packet-bytes" | "--max-memory-mib" | "--max-rss-mib" => {
                if !limit_flags.insert(arg.as_str()) {
                    return Err("duplicate audio limit".into());
                }
                parse_native_audio_limit(
                    arg,
                    args_iter.next().ok_or("missing audio limit")?,
                    &mut limits,
                )?;
            }
            "--from" | "--to" => {
                let slot = if arg == "--from" {
                    &mut from
                } else {
                    &mut to
                };
                if slot.is_some() {
                    return Err("duplicate PCM interval boundary".into());
                }
                let time = decode_time(args_iter.next().ok_or("missing PCM interval boundary")?)?;
                if time.subsec_nanos() % 1000 != 0 {
                    return Err("PCM boundary must be representable in microseconds".into());
                }
                *slot =
                    Some(i64::try_from(time.as_micros()).map_err(|_| "PCM interval overflow")?);
            }
            "--streams" => {
                if selected.is_some() {
                    return Err("duplicate streams option".into());
                }
                selected = Some(
                    args_iter
                        .next()
                        .ok_or("missing stream index")?
                        .parse::<usize>()?,
                );
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => {
                if !plan {
                    report = true;
                }
            }
            "--metadata" => {
                let (key, value) = args_iter
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                limits
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                limits
                    .metadata_delete
                    .push(args_iter.next().ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                let raw = args_iter.next().ok_or("missing stream metadata")?;
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
                let raw = args_iter.next().ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                limits
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--" => {
                paths.extend(args_iter.map(PathBuf::from));
                break;
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unsupported native PCM trim option: {arg}").into());
            }
            _ => paths.push(PathBuf::from(arg)),
        }
    }
    let Some(source_path) = paths.first().map(PathBuf::as_path) else {
        return Err(if plan {
            format!("plan {command} requires INPUT [OUTPUT]").into()
        } else {
            format!("{command} requires INPUT OUTPUT").into()
        });
    };
    let dest_path = paths.get(1).map(PathBuf::as_path);
    if plan {
        if paths.len() > 2 {
            return Err(format!("plan {command} requires INPUT [OUTPUT]").into());
        }
    } else if paths.len() != 2 {
        return Err(format!("{command} requires INPUT OUTPUT").into());
    }
    let container_audio = {
        let mut head = [0u8; 8];
        std::fs::File::open(source_path)
            .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut head))
            .is_ok_and(|()| {
                head.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
                    || fvid::container::mp4::recognizes_prefix(&head)
            })
    };
    let owned_audio = command == "trim"
        && (fvid::native_media::is_owned_audio_trim_source(source_path)? || container_audio);
    let wave = fvid::native_pcm::is_wave(source_path)?;
    let standalone_audio = command == "trim"
        && fvid_media::owned_container_loudness::recognizes_standalone_audio(source_path)?;
    if !owned_audio && !wave && !standalone_audio {
        return Ok(false);
    }
    let resolved_dest_ext = dest_path
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
        if command == "trim-pcm" {
            if dest_ext != "wav" {
                return Err("trim-pcm requires OUTPUT.wav".into());
            }
        } else if !matches!(dest_ext, "wav" | "f32le" | "mka" | "mkv") {
            return Err("trim requires OUTPUT.wav, OUTPUT.f32le, OUTPUT.mka or OUTPUT.mkv".into());
        }
        if dest_ext == "f32le"
            && (!limits.metadata_set.is_empty()
                || !limits.metadata_delete.is_empty()
                || !limits.stream_metadata_set.is_empty()
                || !limits.stream_metadata_delete.is_empty())
        {
            return Err("trim to .f32le does not support metadata mutations".into());
        }
        if dest_ext == "wav" {
            if !limits.stream_metadata_set.is_empty() || !limits.stream_metadata_delete.is_empty() {
                return Err(
                    format!("{command} to .wav does not support stream metadata mutations").into(),
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
    if !owned_audio && !standalone_audio && selected.is_some_and(|n| n != 0) {
        return Err(if wave {
            "WAVE has only stream 0"
        } else {
            "audio input has only stream 0"
        }
        .into());
    }
    let (from, to) = (from.ok_or("--from required")?, to.ok_or("--to required")?);
    let custom_limits =
        limits.max_packet_bytes != fvid::media::CopyOptions::default().max_packet_bytes
            || limits.max_packets.is_some()
            || limits.max_controlled_bytes.is_some()
            || limits.max_rss_bytes.is_some()
            || !limits.metadata_set.is_empty()
            || !limits.metadata_delete.is_empty()
            || !limits.stream_metadata_set.is_empty()
            || !limits.stream_metadata_delete.is_empty();
    limits.streams = selected.into_iter().collect();
    let native_trim_track = owned_audio
        && limits.max_controlled_bytes.is_none()
        && limits.metadata_set.is_empty()
        && limits.metadata_delete.is_empty()
        && limits.stream_metadata_set.is_empty()
        && limits.stream_metadata_delete.is_empty()
        && fvid::native_media::is_owned_audio_trim_source(source_path)?
        && selected.is_none_or(|idx| {
            fvid::native_media::constant_audio_source_info_selected(source_path, Some(idx)).is_ok()
        });
    if plan {
        let mut result = if command == "trim-pcm" {
            fvid::media::plan_trim_pcm(source_path, from, to, &limits)?
        } else if wave && matches!(resolved_dest_ext, Some("f32le" | "mka" | "mkv")) {
            let transform = fvid::media::AudioDecodeTransform {
                interval: Some((from, to)),
                ..Default::default()
            };
            fvid::media::plan_decode_audio(source_path, &transform, &limits)?
        } else {
            fvid::media::plan_trim(source_path, from, to, &limits)?
        };
        result.command = command.to_owned();
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        return Ok(true);
    }
    let dest_path = dest_path.expect("checked destination path");
    let progress = make_progress_hook(report);
    limits.progress = progress.clone();
    if native_trim_track {
        let stats = fvid::native_export::trim_audio_pcm_with_controls(
            source_path,
            dest_path,
            from,
            to,
            &limits,
        )?;
        if !quiet {
            println!(
                "{}",
                serde_json::json!({"packets":stats.decoded_frames,"sample_frames":stats.sample_frames,"payload_bytes":stats.sample_frames*u64::from(stats.channels)*4,"segments":1,"backend":"fvid","fvid_payload_copies":0})
            );
        }
        return Ok(true);
    }
    if !wave {
        let stats = fvid::media::trim(source_path, dest_path, from, to, &limits)?;
        if !quiet {
            println!(
                "{}",
                serde_json::json!({"packets":stats.packets,"payload_bytes":stats.payload_bytes,"segments":stats.segments,"backend":stats.backend,"fvid_payload_copies":stats.fvid_payload_copies})
            );
        }
        return Ok(true);
    }
    if command == "trim"
        && matches!(
            dest_path.extension().and_then(|s| s.to_str()),
            Some("f32le" | "mka" | "mkv")
        )
    {
        let copy = fvid::media::trim(source_path, dest_path, from, to, &limits)?;
        if !quiet {
            println!(
                "{}",
                serde_json::json!({"packets":copy.packets,"payload_bytes":copy.payload_bytes,"segments":1,"backend":"fvid","fvid_payload_copies":copy.fvid_payload_copies})
            );
        }
        return Ok(true);
    }
    let stats = if custom_limits {
        fvid::media::trim_pcm(source_path, dest_path, from, to, &limits)?
    } else {
        fvid::native_pcm::trim_wave(source_path, dest_path, from, to, None, progress.as_ref())?
    };
    if !quiet {
        if command == "trim" {
            println!(
                "{}",
                serde_json::json!({"packets":stats.packets,"payload_bytes":stats.payload_bytes,"segments":1,"backend":"fvid","fvid_payload_copies":stats.fvid_payload_copies})
            );
        } else {
            println!(
                "{}",
                serde_json::json!({"packets":stats.packets,"sample_frames":stats.sample_frames,"payload_bytes":stats.payload_bytes,"fvid_payload_copies":stats.fvid_payload_copies})
            );
        }
    }
    Ok(true)
}
