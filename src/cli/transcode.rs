use std::path::{Path, PathBuf};

#[cfg(feature = "media")]
use super::audio_ops::resolve_plan_mix_or_merge_destination;
#[cfg(feature = "media")]
use super::common::emit_json;
use super::common::{CliResult, make_progress_hook};
use super::decode_video::{geometry_decode_args, pixel_decode_args};
#[cfg(feature = "media")]
use super::edit::resolve_plan_concat_destination;

pub(super) fn try_transcode_rewrite(args: &[String]) -> CliResult<Option<Vec<String>>> {
    if args.first().map(String::as_str) != Some("transcode") {
        return Ok(None);
    }
    let mut owned = vec!["transcode-lossless".to_owned()];
    let mut encoder = None;
    let mut encoder_options = Vec::new();
    let mut positional = false;
    let mut path_count = 0usize;
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        if !positional && option == "--" {
            positional = true;
            owned.push(option.clone());
            continue;
        }
        if !positional && option == "--encoder" {
            if encoder.is_some() {
                return Err("duplicate encoder".into());
            }
            encoder = Some(options.next().ok_or("missing encoder name")?.as_str());
        } else if !positional && option == "--encoder-option" {
            let (key, value) = options
                .next()
                .ok_or("missing encoder option")?
                .split_once('=')
                .ok_or("encoder option requires KEY=VALUE")?;
            encoder_options.push((key.to_owned(), value.to_owned()));
        } else if !positional && option.starts_with('-') {
            owned.push(option.clone());
            // Keep parameter values together, including strings resembling flags.
            if !matches!(
                option.as_str(),
                "--hflip" | "--vflip" | "--quiet" | "-q" | "--progress" | "--seek"
            ) && let Some(value) = options.next()
            {
                owned.push(value.clone());
            }
        } else {
            path_count += 1;
            owned.push(option.clone());
        }
    }
    if path_count != 2 {
        return Err("transcode requires INPUT OUTPUT --encoder NAME".into());
    }
    let name =
        encoder.ok_or("transcode requires --encoder; quality is defined by its options")?;
    let settings = fvid::media_info::EncoderSettings {
        name: name.to_owned(),
        options: encoder_options,
    };
    settings.validate()?;
    if fvid_media::owned_lossless::supports_encoder(&settings) {
        return Ok(Some(owned));
    }
    #[cfg(not(feature = "media"))]
    return Err("owned transcode does not yet implement requested encoder/settings".into());
    #[cfg(feature = "media")]
    Ok(None)
}

pub(super) fn try_native_ffv1_lossless(args: &[String]) -> CliResult<bool> {
    if !(matches!(
        args.first().map(String::as_str),
        Some("transcode-lossless" | "crop-lossless")
    ) && args.len() >= 3)
    {
        return Ok(false);
    }
    let mut owned = args.to_vec();
    owned[0] = "decode".into();
    let (pixel_args, mut filters) = pixel_decode_args(&owned)?;
    filters.canonicalize_option_order();
    let (remaining, geometry) = geometry_decode_args(&pixel_args)?;
    if args[0] == "crop-lossless" {
        if geometry.crop.is_none() {
            return Err("--crop required".into());
        }
        for (present, option) in [
            (geometry.scale.is_some(), "--scale"),
            (geometry.transpose.is_some(), "--transpose"),
            (geometry.rotate.is_some(), "--rotate"),
            (geometry.pad.is_some(), "--pad"),
        ] {
            if present {
                return Err(
                    format!("crop-lossless does not take {option}; use transcode-lossless").into(),
                );
            }
        }
        if !filters.is_empty() {
            return Err("crop-lossless does not take pixel filters; use transcode-lossless".into());
        }
    }
    let mut paths = Vec::<String>::new();
    let mut quiet = false;
    let mut report = false;
    #[cfg_attr(not(feature = "media"), allow(unused_mut))]
    let mut unknown = false;
    let mut iter = remaining[1..].iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--" => {
                paths.extend(iter.cloned());
                break;
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => report = true,
            _ if arg.starts_with('-') => {
                #[cfg(not(feature = "media"))]
                return Err(format!("unsupported {} option: {arg}", args[0]).into());
                #[cfg(feature = "media")]
                {
                    unknown = true;
                    break;
                }
            }
            _ => paths.push(arg.clone()),
        }
    }
    if !unknown
        && paths.len() == 2
        && fvid::native_lossless::eligible(Path::new(&paths[0]))?
    {
        let hook = make_progress_hook(report);
        let stats = fvid::native_export::transcode_ffv1_transformed(
            Path::new(&paths[0]),
            Path::new(&paths[1]),
            &geometry,
            &filters,
            None,
            hook.as_ref(),
        )?;
        if !quiet {
            println!("{}", serde_json::to_string(&stats)?);
        }
        return Ok(true);
    }
    Ok(false)
}

pub(super) fn owned_lossless_command(args: &[String]) -> CliResult<bool> {
    owned_lossless_command_with_name(args, false)
}

pub(super) fn owned_lossless_command_with_name(
    args: &[String],
    force_overlay_command: bool,
) -> CliResult<bool> {
    let planning = args.first().map(String::as_str) == Some("plan");
    let command = usize::from(planning);
    let op = args.get(command).map(String::as_str);
    let valid_op = matches!(op, Some("transcode-lossless" | "crop-lossless"));
    if !valid_op {
        return Ok(false);
    }
    let is_crop_lossless = op == Some("crop-lossless");
    let mut paths = Vec::<PathBuf>::new();
    let mut quiet = false;
    let mut report = false;
    let mut policy = fvid::media_control::CopyOptions::default();
    let mut custom_policy = false;
    let (mut from, mut to) = (None, None);
    let mut transform = fvid::media_info::LosslessTransform::default();
    let mut overlay_path = None;
    let (mut overlay_x, mut overlay_y) = (0i32, 0i32);
    let mut options = args[command + 1..].iter();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(option) = options.next() {
        if option == "--" {
            paths.extend(options.map(PathBuf::from));
            break;
        }
        if !option.starts_with('-') {
            paths.push(PathBuf::from(option));
            continue;
        }
        let key_option = if option == "-q" { "--quiet" } else { option.as_str() };
        if !matches!(
            key_option,
            "--metadata" | "--metadata-delete" | "--stream-metadata" | "--stream-metadata-delete"
        ) && !seen.insert(key_option)
        {
            return Err(format!("duplicate option: {option}").into());
        }
        match option.as_str() {
            "--stream-metadata" | "--stream-metadata-delete" => {
                let (index, text) = options
                    .next()
                    .ok_or("missing stream metadata")?
                    .split_once(':')
                    .ok_or("stream metadata requires INDEX:KEY")?;
                let index = index.parse()?;
                if option == "--stream-metadata" {
                    let (key, value) = text
                        .split_once('=')
                        .ok_or("stream metadata requires INDEX:KEY=VALUE")?;
                    policy
                        .stream_metadata_set
                        .push((index, key.to_owned(), value.to_owned()));
                } else {
                    policy.stream_metadata_delete.push((index, text.to_owned()));
                }
                custom_policy = true;
                continue;
            }
            "--metadata" => {
                let (key, value) = options
                    .next()
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata requires KEY=VALUE")?;
                policy.metadata_set.push((key.to_owned(), value.to_owned()));
                custom_policy = true;
                continue;
            }
            "--metadata-delete" => {
                policy
                    .metadata_delete
                    .push(options.next().ok_or("missing metadata key")?.clone());
                custom_policy = true;
                continue;
            }
            "--streams" => {
                policy.streams = options
                    .next()
                    .ok_or("missing stream selection")?
                    .split(',')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
                custom_policy = true;
                continue;
            }
            "--max-packets" => {
                policy.max_packets = Some(options.next().ok_or("missing packet limit")?.parse()?);
                custom_policy = true;
                continue;
            }
            "--max-packet-bytes" => {
                let limit = options.next().ok_or("missing packet byte limit")?.parse()?;
                if limit == 0 {
                    return Err("packet byte limit must be positive".into());
                }
                policy.max_packet_bytes = limit;
                custom_policy = true;
                continue;
            }
            "--max-rss-mib" => {
                policy.max_rss_bytes = Some(fvid_media::parse_max_rss_mib(
                    options.next().ok_or("missing RSS limit")?,
                )?);
                custom_policy = true;
                continue;
            }
            "--max-memory-mib" => {
                policy.max_controlled_bytes = Some(fvid_media::parse_max_memory_mib(
                    options.next().ok_or("missing memory limit")?,
                )?);
                custom_policy = true;
                continue;
            }
            "--overlay" => {
                if is_crop_lossless {
                    return Err(
                        "crop-lossless does not take --overlay; use overlay or transcode-lossless"
                            .into(),
                    );
                }
                overlay_path = Some(PathBuf::from(options.next().ok_or("missing overlay path")?));
                continue;
            }
            "--overlay-x" => {
                overlay_x = options.next().ok_or("missing overlay-x")?.parse()?;
                continue;
            }
            "--overlay-y" => {
                overlay_y = options.next().ok_or("missing overlay-y")?.parse()?;
                continue;
            }
            "--from" | "--to" => {
                let value = fvid::media_info::parse_time(
                    options.next().ok_or("missing interval boundary")?,
                )?;
                if option == "--from" {
                    from = Some(value);
                } else {
                    to = Some(value);
                }
                continue;
            }
            "--quiet" | "-q" => {
                quiet = true;
                continue;
            }
            "--progress" => {
                if !planning {
                    report = true;
                }
                continue;
            }
            "--output-format" if planning => {
                let fmt = options.next().ok_or("missing output format")?;
                if !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska") {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                continue;
            }
            "--hflip" => {
                transform.horizontal_flip = true;
                continue;
            }
            "--vflip" => {
                transform.vertical_flip = true;
                continue;
            }
            "--rotate" => {
                if is_crop_lossless {
                    return Err(
                        "crop-lossless does not take --rotate; use transcode-lossless".into(),
                    );
                }
                if transform.rotate.is_some() {
                    return Err("duplicate rotate".into());
                }
                transform.rotate = Some(fvid::media_info::RotateAngle::parse(
                    options.next().ok_or("missing rotate degrees")?,
                )?);
                continue;
            }
            "--transpose" => {
                if is_crop_lossless {
                    return Err(
                        "crop-lossless does not take --transpose; use transcode-lossless".into(),
                    );
                }
                transform.transpose = Some(fvid::media_info::TransposeMode::parse(
                    options.next().ok_or("missing transpose mode")?,
                )?);
                continue;
            }
            "--crop" | "--scale" | "--pad" => {
                if is_crop_lossless && option != "--crop" {
                    return Err(
                        format!("crop-lossless does not take {option}; use transcode-lossless")
                            .into(),
                    );
                }
                let fields = options
                    .next()
                    .ok_or("missing geometry value")?
                    .split(':')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
                match option.as_str() {
                    "--crop" => {
                        let [x, y, width, height]: [usize; 4] = fields
                            .try_into()
                            .map_err(|_| "crop requires X:Y:WIDTH:HEIGHT")?;
                        transform.crop = Some(fvid::media_info::CropRect {
                            x,
                            y,
                            width,
                            height,
                        });
                    }
                    "--scale" => {
                        let [width, height]: [usize; 2] = fields
                            .try_into()
                            .map_err(|_| "scale requires WIDTH:HEIGHT")?;
                        transform.scale = Some(fvid::media_info::ScaleSize {
                            width: width.try_into()?,
                            height: height.try_into()?,
                        });
                    }
                    _ => {
                        let [width, height, x, y]: [usize; 4] = fields
                            .try_into()
                            .map_err(|_| "pad requires WIDTH:HEIGHT:X:Y")?;
                        transform.pad = Some(fvid::media_info::PadRect {
                            width: width.try_into()?,
                            height: height.try_into()?,
                            x: x.try_into()?,
                            y: y.try_into()?,
                        });
                    }
                }
                continue;
            }
            _ => {}
        }
        if is_crop_lossless {
            return Err("crop-lossless does not take pixel filters; use transcode-lossless".into());
        }
        let slot = match option.as_str() {
            "--framestep" => &mut transform.framestep,
            "--reverse" => &mut transform.reverse,
            "--shuffleframes" => &mut transform.shuffleframes,
            "--eq" => &mut transform.eq,
            "--unsharp" => &mut transform.unsharp,
            "--hue" => &mut transform.hue,
            "--colorize" => &mut transform.colorize,
            "--monochrome" => &mut transform.monochrome,
            "--lutyuv" => &mut transform.lutyuv,
            "--colorhold" => &mut transform.colorhold,
            "--colorcontrast" => &mut transform.colorcontrast,
            "--vibrance" => &mut transform.vibrance,
            "--colorlevels" => &mut transform.colorlevels,
            "--colorchannelmixer" => &mut transform.colorchannelmixer,
            "--exposure" => &mut transform.exposure,
            "--sab" => &mut transform.sab,
            "--bitplanenoise" => &mut transform.bitplanenoise,
            "--deband" => &mut transform.deband,
            "--perspective" => &mut transform.perspective,
            "--gradfun" => &mut transform.gradfun,
            "--lenscorrection" => &mut transform.lenscorrection,
            "--drawbox" => &mut transform.drawbox,
            "--drawgrid" => &mut transform.drawgrid,
            "--removegrain" => &mut transform.removegrain,
            "--yaepblur" => &mut transform.yaepblur,
            "--smartblur" => &mut transform.smartblur,
            "--vignette" => &mut transform.vignette,
            "--colorbalance" => &mut transform.colorbalance,
            "--colorcorrect" => &mut transform.colorcorrect,
            "--cas" => &mut transform.cas,
            "--grayworld" => &mut transform.grayworld,
            "--curves" => &mut transform.curves,
            "--fade" => &mut transform.fade,
            "--hqdn3d" => &mut transform.hqdn3d,
            "--tmix" => &mut transform.tmix,
            "--lagfun" => &mut transform.lagfun,
            "--gblur" => &mut transform.gblur,
            "--bilateral" => &mut transform.bilateral,
            "--avgblur" => &mut transform.avgblur,
            "--boxblur" => &mut transform.boxblur,
            "--negate" => &mut transform.negate,
            "--sobel" => &mut transform.sobel,
            "--prewitt" => &mut transform.prewitt,
            "--roberts" => &mut transform.roberts,
            "--kirsch" => &mut transform.kirsch,
            "--scharr" => &mut transform.scharr,
            "--pixelize" => &mut transform.pixelize,
            "--dilation" => &mut transform.dilation,
            "--erosion" => &mut transform.erosion,
            "--chromashift" => &mut transform.chromashift,
            "--shuffleplanes" => &mut transform.shuffleplanes,
            _ => {
                #[cfg(not(feature = "media"))]
                return Err(format!(
                    "unsupported {} option: {option}",
                    op.unwrap_or("transcode-lossless")
                )
                .into());
                #[cfg(feature = "media")]
                return Ok(false);
            }
        };
        *slot = Some(options.next().ok_or("missing filter parameters")?.clone());
    }
    if planning {
        if !(1..=2).contains(&paths.len()) {
            return Err(format!(
                "plan {} requires a single INPUT",
                op.unwrap_or("transcode-lossless")
            )
            .into());
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
        return Err(
            format!("{} requires INPUT OUTPUT.mkv", op.unwrap_or("transcode-lossless")).into(),
        );
    } else if paths[1]
        .extension()
        .and_then(|e| e.to_str())
        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
    {
        return Err("owned FFV1 export requires .mkv".into());
    }
    if is_crop_lossless && transform.crop.is_none() {
        return Err("--crop required".into());
    }
    transform.overlay = overlay_path.map(|path| fvid::media_info::OverlaySpec {
        path,
        x: overlay_x,
        y: overlay_y,
    });
    transform.interval = match (from, to) {
        (None, None) => None,
        (Some(from), Some(to)) if from >= 0 && from < to => Some((from, to)),
        _ => {
            return Err(
                "lossless interval requires both --from and --to with 0 <= from < to".into(),
            );
        }
    };
    let source = paths[0].as_path();
    let destination = paths.get(1).map(PathBuf::as_path);
    if !custom_policy
        && fvid::native_lossless::supports(&transform)
        && fvid::native_lossless::eligible(source)?
    {
        if !planning {
            return Ok(false);
        } // The controlled native exporter owns this path.
        let mut plan = fvid::native_plan::transcode_lossless(source, &transform)?;
        if is_crop_lossless {
            plan.command = "crop-lossless".into();
        } else if force_overlay_command {
            plan.command = "overlay".into();
        }
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(true);
    }
    let mut options = policy;
    options.progress = make_progress_hook(report);
    if !custom_policy
        && fvid::native_lossless::supports_framestep(&transform)
        && fvid::native_lossless::eligible(source)?
    {
        if planning {
            let mut plan =
                fvid::media::plan_transcode_lossless(source, &transform, &options, None)?;
            if is_crop_lossless {
                plan.command = "crop-lossless".into();
            } else if force_overlay_command {
                plan.command = "overlay".into();
            }
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        } else {
            let stats =
                fvid::media::transcode_lossless(source, destination.unwrap(), transform, &options)?;
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
        }
        return Ok(true);
    }
    if !custom_policy
        && fvid::native_lossless::supports_overlay(&transform)
        && fvid::native_export::overlay_eligible(source)?
    {
        if planning {
            let mut plan =
                fvid::media::plan_transcode_lossless(source, &transform, &options, None)?;
            if is_crop_lossless {
                plan.command = "crop-lossless".into();
            } else if force_overlay_command {
                plan.command = "overlay".into();
            }
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
        } else {
            let stats =
                fvid::media::transcode_lossless(source, destination.unwrap(), transform, &options)?;
            if !quiet {
                println!("{}", serde_json::to_string_pretty(&stats)?);
            }
        }
        return Ok(true);
    }
    if !fvid_media::owned_lossless::supports(source, &transform, &options)
        && !fvid_media::owned_lossless::supports(
            source,
            &transform,
            &fvid::media::CopyOptions::default(),
        )
    {
        return Ok(false);
    }
    if planning {
        let mut plan =
            fvid_media::owned_lossless::plan_transcode_lossless(source, &transform, &options, None)?;
        if is_crop_lossless {
            plan.command = "crop-lossless".into();
        } else if force_overlay_command {
            plan.command = "overlay".into();
        }
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
    } else {
        let stats = fvid_media::owned_lossless::transcode_lossless(
            source,
            destination.unwrap(),
            transform,
            &options,
        )?;
        if !quiet {
            println!("{}", serde_json::to_string(&stats)?);
        }
    }
    Ok(true)
}

#[cfg(all(feature = "media", feature = "player"))]
fn play_command(args: &[String]) -> CliResult {
    fvid::player::run_media(args.to_vec())
}

#[cfg(feature = "media")]
fn string_filter_slot<'a>(
    spec: &'a mut fvid::media_info::VideoFilterSpec,
    flag: &str,
) -> Option<(&'a mut Option<String>, &'static str)> {
    Some(match flag {
        "--pix-fmt" => (&mut spec.pix_fmt, "missing pixel format"),
        "--colorspace" => (&mut spec.colorspace, "missing colorspace args"),
        "--zscale" => (&mut spec.zscale, "missing zscale args"),
        "--tonemap" => (&mut spec.tonemap, "missing tonemap args"),
        "--yadif" => (&mut spec.yadif, "missing yadif args"),
        "--bwdif" => (&mut spec.bwdif, "missing bwdif args"),
        "--w3fdif" => (&mut spec.w3fdif, "missing w3fdif args"),
        "--tblend" => (&mut spec.tblend, "missing tblend args"),
        "--tmix" => (&mut spec.tmix, "missing tmix args"),
        "--hqdn3d" => (&mut spec.hqdn3d, "missing hqdn3d args"),
        "--gblur" => (&mut spec.gblur, "missing gblur args"),
        "--eq" => (&mut spec.eq, "missing eq args"),
        "--unsharp" => (&mut spec.unsharp, "missing unsharp args"),
        "--hue" => (&mut spec.hue, "missing hue args"),
        "--avgblur" => (&mut spec.avgblur, "missing avgblur args"),
        "--boxblur" => (&mut spec.boxblur, "missing boxblur args"),
        "--negate" => (&mut spec.negate, "missing negate args (0|1)"),
        "--edgedetect" => (&mut spec.edgedetect, "missing edgedetect args"),
        "--sobel" => (&mut spec.sobel, "missing sobel args"),
        "--prewitt" => (&mut spec.prewitt, "missing prewitt args"),
        "--roberts" => (&mut spec.roberts, "missing roberts args"),
        "--kirsch" => (&mut spec.kirsch, "missing kirsch args"),
        "--scharr" => (&mut spec.scharr, "missing scharr args"),
        "--atadenoise" => (&mut spec.atadenoise, "missing atadenoise args"),
        "--owdenoise" => (&mut spec.owdenoise, "missing owdenoise args"),
        "--vaguedenoiser" => (&mut spec.vaguedenoiser, "missing vaguedenoiser args"),
        "--nlmeans" => (&mut spec.nlmeans, "missing nlmeans args"),
        "--bm3d" => (&mut spec.bm3d, "missing bm3d args"),
        "--dctdnoiz" => (&mut spec.dctdnoiz, "missing dctdnoiz args"),
        "--fftdnoiz" => (&mut spec.fftdnoiz, "missing fftdnoiz args"),
        "--smartblur" => (&mut spec.smartblur, "missing smartblur args"),
        "--sab" => (&mut spec.sab, "missing sab args"),
        "--bilateral" => (&mut spec.bilateral, "missing bilateral args"),
        "--cas" => (&mut spec.cas, "missing cas args"),
        "--epx" => (&mut spec.epx, "missing epx args"),
        "--vignette" => (&mut spec.vignette, "missing vignette args"),
        "--curves" => (&mut spec.curves, "missing curves args"),
        "--colorbalance" => (&mut spec.colorbalance, "missing colorbalance args"),
        "--colorlevels" => (&mut spec.colorlevels, "missing colorlevels args"),
        "--colorchannelmixer" => (&mut spec.colorchannelmixer, "missing colorchannelmixer args"),
        "--deflicker" => (&mut spec.deflicker, "missing deflicker args"),
        "--photosensitivity" => (&mut spec.photosensitivity, "missing photosensitivity args"),
        "--monochrome" => (&mut spec.monochrome, "missing monochrome args"),
        "--grayworld" => (&mut spec.grayworld, "missing grayworld args (0 for defaults)"),
        "--drawbox" => (&mut spec.drawbox, "missing drawbox args"),
        "--drawgrid" => (&mut spec.drawgrid, "missing drawgrid args"),
        "--lagfun" => (&mut spec.lagfun, "missing lagfun args"),
        "--amplify" => (&mut spec.amplify, "missing amplify args"),
        "--bitplanenoise" => (&mut spec.bitplanenoise, "missing bitplanenoise args"),
        "--deband" => (&mut spec.deband, "missing deband args"),
        "--gradfun" => (&mut spec.gradfun, "missing gradfun args"),
        "--lenscorrection" => (&mut spec.lenscorrection, "missing lenscorrection args"),
        "--pixelize" => (&mut spec.pixelize, "missing pixelize args"),
        "--removegrain" => (&mut spec.removegrain, "missing removegrain args"),
        "--yaepblur" => (&mut spec.yaepblur, "missing yaepblur args"),
        "--vibrance" => (&mut spec.vibrance, "missing vibrance args"),
        "--dilation" => (&mut spec.dilation, "missing dilation args"),
        "--erosion" => (&mut spec.erosion, "missing erosion args"),
        "--colorize" => (&mut spec.colorize, "missing colorize args"),
        "--exposure" => (&mut spec.exposure, "missing exposure args"),
        "--chromashift" => (&mut spec.chromashift, "missing chromashift args"),
        "--colorcontrast" => (&mut spec.colorcontrast, "missing colorcontrast args"),
        "--colorcorrect" => (&mut spec.colorcorrect, "missing colorcorrect args"),
        "--histeq" => (&mut spec.histeq, "missing histeq args"),
        "--shuffleplanes" => (&mut spec.shuffleplanes, "missing shuffleplanes args"),
        "--lutyuv" => (&mut spec.lutyuv, "missing lutyuv args"),
        "--colorhold" => (&mut spec.colorhold, "missing colorhold args"),
        "--fade" => (&mut spec.fade, "missing fade args"),
        "--perspective" => (&mut spec.perspective, "missing perspective args"),
        "--lumakey" => (&mut spec.lumakey, "missing lumakey args"),
        "--chromakey" => (&mut spec.chromakey, "missing chromakey args"),
        "--colorkey" => (&mut spec.colorkey, "missing colorkey args"),
        "--despill" => (&mut spec.despill, "missing despill args"),
        "--selectivecolor" => (&mut spec.selectivecolor, "missing selectivecolor args"),
        "--stereo3d" => (&mut spec.stereo3d, "missing stereo3d args"),
        "--field" => (&mut spec.field, "missing field args"),
        "--hqx" => (&mut spec.hqx, "missing hqx args"),
        "--xbr" => (&mut spec.xbr, "missing xbr args"),
        "--il" => (&mut spec.il, "missing il args"),
        "--super2xsai" => (&mut spec.super2xsai, "missing super2xsai args"),
        "--kerndeint" => (&mut spec.kerndeint, "missing kerndeint args"),
        "--phase" => (&mut spec.phase, "missing phase args"),
        "--estdif" => (&mut spec.estdif, "missing estdif args"),
        "--tinterlace" => (&mut spec.tinterlace, "missing tinterlace args"),
        "--separatefields" => (&mut spec.separatefields, "missing separatefields args"),
        "--weave" => (&mut spec.weave, "missing weave args"),
        "--doubleweave" => (&mut spec.doubleweave, "missing doubleweave args"),
        "--framepack" => (&mut spec.framepack, "missing framepack args"),
        "--telecine" => (&mut spec.telecine, "missing telecine args"),
        "--pullup" => (&mut spec.pullup, "missing pullup args"),
        "--decimate" => (&mut spec.decimate, "missing decimate args"),
        "--mpdecimate" => (&mut spec.mpdecimate, "missing mpdecimate args"),
        "--framestep" => (&mut spec.framestep, "missing framestep args"),
        "--tile" => (&mut spec.tile, "missing tile args"),
        "--untile" => (&mut spec.untile, "missing untile args"),
        "--shuffleframes" => (&mut spec.shuffleframes, "missing shuffleframes args"),
        "--reverse" => (&mut spec.reverse, "missing reverse args"),
        "--loop" => (&mut spec.r#loop, "missing loop args"),
        "--thumbnail" => (&mut spec.thumbnail, "missing thumbnail args"),
        "--freezedetect" => (&mut spec.freezedetect, "missing freezedetect args"),
        "--pseudocolor" => (&mut spec.pseudocolor, "missing pseudocolor args"),
        "--minterpolate" => (&mut spec.minterpolate, "missing minterpolate args"),
        "--fps" => (&mut spec.fps, "missing fps rate"),
        _ => return None,
    })
}

#[cfg(feature = "media")]
fn check_crop_lossless_forbidden(
    spec: &fvid::media_info::VideoFilterSpec,
    setpts: Option<&str>,
    select: Option<&str>,
) -> CliResult {
    let forbidden: &[(bool, &str)] = &[
        (spec.scale.is_some(), "--scale"),
        (spec.transpose.is_some(), "--transpose"),
        (spec.rotate.is_some(), "--rotate"),
        (spec.pad.is_some(), "--pad"),
        (spec.pix_fmt.is_some(), "--pix-fmt"),
        (spec.colorspace.is_some(), "--colorspace"),
        (spec.zscale.is_some(), "--zscale"),
        (spec.tonemap.is_some(), "--tonemap"),
        (spec.yadif.is_some(), "--yadif"),
        (spec.bwdif.is_some(), "--bwdif"),
        (spec.w3fdif.is_some(), "--w3fdif"),
        (spec.tblend.is_some(), "--tblend"),
        (spec.tmix.is_some(), "--tmix"),
        (spec.hqdn3d.is_some(), "--hqdn3d"),
        (spec.fps.is_some(), "--fps"),
        (spec.gblur.is_some(), "--gblur"),
        (spec.eq.is_some(), "--eq"),
        (spec.unsharp.is_some(), "--unsharp"),
        (spec.hue.is_some(), "--hue"),
        (spec.avgblur.is_some(), "--avgblur"),
        (spec.boxblur.is_some(), "--boxblur"),
        (spec.negate.is_some(), "--negate"),
        (spec.edgedetect.is_some(), "--edgedetect"),
        (spec.sobel.is_some(), "--sobel"),
        (spec.prewitt.is_some(), "--prewitt"),
        (spec.roberts.is_some(), "--roberts"),
        (spec.kirsch.is_some(), "--kirsch"),
        (spec.scharr.is_some(), "--scharr"),
        (spec.atadenoise.is_some(), "--atadenoise"),
        (spec.owdenoise.is_some(), "--owdenoise"),
        (spec.vaguedenoiser.is_some(), "--vaguedenoiser"),
        (spec.nlmeans.is_some(), "--nlmeans"),
        (spec.bm3d.is_some(), "--bm3d"),
        (spec.dctdnoiz.is_some(), "--dctdnoiz"),
        (spec.fftdnoiz.is_some(), "--fftdnoiz"),
        (spec.smartblur.is_some(), "--smartblur"),
        (spec.sab.is_some(), "--sab"),
        (spec.bilateral.is_some(), "--bilateral"),
        (spec.cas.is_some(), "--cas"),
        (spec.epx.is_some(), "--epx"),
        (spec.vignette.is_some(), "--vignette"),
        (spec.curves.is_some(), "--curves"),
        (spec.colorbalance.is_some(), "--colorbalance"),
        (spec.colorlevels.is_some(), "--colorlevels"),
        (spec.colorchannelmixer.is_some(), "--colorchannelmixer"),
        (spec.deflicker.is_some(), "--deflicker"),
        (spec.photosensitivity.is_some(), "--photosensitivity"),
        (spec.monochrome.is_some(), "--monochrome"),
        (spec.grayworld.is_some(), "--grayworld"),
        (spec.drawbox.is_some(), "--drawbox"),
        (spec.drawgrid.is_some(), "--drawgrid"),
        (spec.lagfun.is_some(), "--lagfun"),
        (spec.amplify.is_some(), "--amplify"),
        (spec.bitplanenoise.is_some(), "--bitplanenoise"),
        (spec.deband.is_some(), "--deband"),
        (spec.gradfun.is_some(), "--gradfun"),
        (spec.lenscorrection.is_some(), "--lenscorrection"),
        (spec.pixelize.is_some(), "--pixelize"),
        (spec.removegrain.is_some(), "--removegrain"),
        (spec.yaepblur.is_some(), "--yaepblur"),
        (spec.vibrance.is_some(), "--vibrance"),
        (spec.dilation.is_some(), "--dilation"),
        (spec.erosion.is_some(), "--erosion"),
        (spec.colorize.is_some(), "--colorize"),
        (spec.exposure.is_some(), "--exposure"),
        (spec.chromashift.is_some(), "--chromashift"),
        (spec.colorcontrast.is_some(), "--colorcontrast"),
        (spec.colorcorrect.is_some(), "--colorcorrect"),
        (spec.histeq.is_some(), "--histeq"),
        (spec.shuffleplanes.is_some(), "--shuffleplanes"),
        (spec.lutyuv.is_some(), "--lutyuv"),
        (spec.colorhold.is_some(), "--colorhold"),
        (spec.fade.is_some(), "--fade"),
        (spec.perspective.is_some(), "--perspective"),
        (spec.lumakey.is_some(), "--lumakey"),
        (spec.chromakey.is_some(), "--chromakey"),
        (spec.colorkey.is_some(), "--colorkey"),
        (spec.despill.is_some(), "--despill"),
        (spec.selectivecolor.is_some(), "--selectivecolor"),
        (spec.stereo3d.is_some(), "--stereo3d"),
        (spec.field.is_some(), "--field"),
        (spec.hqx.is_some(), "--hqx"),
        (spec.xbr.is_some(), "--xbr"),
        (spec.il.is_some(), "--il"),
        (spec.super2xsai.is_some(), "--super2xsai"),
        (spec.kerndeint.is_some(), "--kerndeint"),
        (spec.phase.is_some(), "--phase"),
        (spec.estdif.is_some(), "--estdif"),
        (spec.tinterlace.is_some(), "--tinterlace"),
        (spec.separatefields.is_some(), "--separatefields"),
        (spec.weave.is_some(), "--weave"),
        (spec.doubleweave.is_some(), "--doubleweave"),
        (spec.framepack.is_some(), "--framepack"),
        (spec.telecine.is_some(), "--telecine"),
        (spec.pullup.is_some(), "--pullup"),
        (spec.decimate.is_some(), "--decimate"),
        (spec.mpdecimate.is_some(), "--mpdecimate"),
        (spec.framestep.is_some(), "--framestep"),
        (spec.tile.is_some(), "--tile"),
        (spec.untile.is_some(), "--untile"),
        (spec.shuffleframes.is_some(), "--shuffleframes"),
        (spec.reverse.is_some(), "--reverse"),
        (spec.r#loop.is_some(), "--loop"),
        (spec.thumbnail.is_some(), "--thumbnail"),
        (spec.freezedetect.is_some(), "--freezedetect"),
        (setpts.is_some(), "--setpts"),
        (select.is_some(), "--select"),
        (spec.pseudocolor.is_some(), "--pseudocolor"),
        (spec.minterpolate.is_some(), "--minterpolate"),
    ];
    for &(present, flag) in forbidden {
        if present {
            return Err(
                format!("crop-lossless does not take {flag}; use transcode-lossless").into(),
            );
        }
    }
    if spec.overlay.is_some() {
        return Err(
            "crop-lossless does not take --overlay; use overlay or transcode-lossless".into(),
        );
    }
    Ok(())
}

#[cfg(feature = "media")]
pub(super) fn run_native(args: &[String]) -> CliResult {
    let help = "fvid media play INPUT... [--no-audio] [--mute] [--fullscreen] [--rate N] [--audio-track N] [--subtitle-track N] [--no-subtitles] [--subtitles FILE] [--audio-device NAME] [--list-audio-devices] (local file or http/https/rtsp/rtmp/udp URL) | probe INPUT [--input-format NAME] | capabilities | plan [remux|transcode-lossless|trim|trim-pcm|concat|overlay|xfade|burn-subtitles|loudness|loudnorm|mix-audio|merge-audio|decode-audio] INPUT... [flags] | remux INPUT OUTPUT [--streams 0,1] [--metadata KEY=VALUE] [--metadata-delete KEY] [--stream-metadata INDEX:KEY=VALUE] [--stream-metadata-delete INDEX:KEY] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | convert-subtitles INPUT OUTPUT.mkv [--codec ass] [--streams INDEX] | burn-subtitles INPUT OUTPUT.mkv --subs FILE.srt | overlay MAIN OVERLAY OUTPUT.mkv [--overlay-x N] [--overlay-y N] | xfade MAIN OTHER OUTPUT.mkv --xfade-duration SECONDS [--transition NAME] [--xfade-offset SECONDS] | decode INPUT [--input-format NAME] [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--transpose MODE] [--rotate DEGREES] [--pad WIDTH:HEIGHT:X:Y] [--scale WIDTH:HEIGHT] [--epx ARGS] [--pix-fmt NAME] [--colorspace ARGS] [--zscale ARGS] [--tonemap ARGS] [--yadif ARGS] [--bwdif ARGS] [--w3fdif ARGS] [--tblend ARGS] [--tmix ARGS] [--hqdn3d ARGS] [--gblur ARGS] [--eq ARGS] [--unsharp ARGS] [--hue ARGS] [--avgblur ARGS] [--boxblur ARGS] [--negate 0|1] [--edgedetect ARGS] [--sobel ARGS] [--prewitt ARGS] [--roberts ARGS] [--kirsch ARGS] [--scharr ARGS] [--atadenoise ARGS] [--owdenoise ARGS] [--vaguedenoiser ARGS] [--nlmeans ARGS] [--bm3d ARGS] [--dctdnoiz ARGS] [--fftdnoiz ARGS] [--smartblur ARGS] [--sab ARGS] [--bilateral ARGS] [--cas ARGS] [--vignette ARGS] [--curves ARGS] [--colorbalance ARGS] [--colorlevels ARGS] [--colorchannelmixer ARGS] [--deflicker ARGS] [--photosensitivity ARGS] [--monochrome ARGS] [--grayworld 0|ARGS] [--drawbox ARGS] [--drawgrid ARGS] [--lagfun ARGS] [--amplify ARGS] [--bitplanenoise ARGS] [--deband ARGS] [--gradfun ARGS] [--lenscorrection ARGS] [--pixelize ARGS] [--removegrain ARGS] [--yaepblur ARGS] [--vibrance ARGS] [--dilation ARGS] [--erosion ARGS] [--colorize ARGS] [--exposure ARGS] [--chromashift ARGS] [--colorcontrast ARGS] [--colorcorrect ARGS] [--histeq ARGS] [--shuffleplanes ARGS] [--lutyuv ARGS] [--colorhold ARGS] [--fade ARGS] [--perspective ARGS] [--lumakey ARGS] [--chromakey ARGS] [--colorkey ARGS] [--despill ARGS] [--selectivecolor ARGS] [--stereo3d ARGS] [--field ARGS] [--hqx ARGS] [--xbr ARGS] [--il ARGS] [--super2xsai ARGS] [--kerndeint ARGS] [--phase ARGS] [--estdif ARGS] [--tinterlace ARGS] [--separatefields ARGS] [--weave ARGS] [--doubleweave ARGS] [--framepack ARGS] [--telecine ARGS] [--pullup ARGS] [--decimate ARGS] [--mpdecimate ARGS] [--framestep ARGS] [--tile ARGS] [--untile ARGS] [--shuffleframes ARGS] [--reverse ARGS] [--loop ARGS] [--thumbnail ARGS] [--freezedetect ARGS] [--setpts ARGS] [--pseudocolor ARGS] [--minterpolate ARGS] [--fps RATE] [--subs FILE.srt] [--from SECONDS --to SECONDS] [--device N] | loudness INPUT [--streams INDEX] | loudnorm INPUT OUTPUT.wav [--loudnorm-args ARGS] [--dual-pass] [--streams INDEX] | decode-audio INPUT OUTPUT.wav [--streams INDEX] [--rate HZ] [--channels N] [--volume GAIN] [--from SECONDS --to SECONDS] [--max-packet-bytes N] [--max-packets N] [--max-rss-mib N] | trim-pcm INPUT OUTPUT --from SECONDS --to SECONDS [--streams 0] | trim INPUT OUTPUT --from SECONDS --to SECONDS [--streams 0] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | mix-audio OUTPUT a.m4a b.m4a [c.m4a...] [--weights W,...] [--duration shortest] [--normalize|--no-normalize] | merge-audio OUTPUT a.m4a b.m4a | concat OUTPUT INPUT INPUT... [--streams 0] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | transcode INPUT OUTPUT --encoder NAME [--encoder-option KEY=VALUE] [--from SECONDS --to SECONDS [--seek]] | transcode-lossless INPUT OUTPUT.mkv [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--transpose MODE] [--rotate DEGREES] [--pad WIDTH:HEIGHT:X:Y] [--scale WIDTH:HEIGHT] [--epx ARGS] [--pix-fmt NAME] [--colorspace ARGS] [--zscale ARGS] [--tonemap ARGS] [--yadif ARGS] [--bwdif ARGS] [--w3fdif ARGS] [--tblend ARGS] [--tmix ARGS] [--hqdn3d ARGS] [--gblur ARGS] [--eq ARGS] [--unsharp ARGS] [--hue ARGS] [--avgblur ARGS] [--boxblur ARGS] [--negate 0|1] [--edgedetect ARGS] [--sobel ARGS] [--prewitt ARGS] [--roberts ARGS] [--kirsch ARGS] [--scharr ARGS] [--atadenoise ARGS] [--owdenoise ARGS] [--vaguedenoiser ARGS] [--nlmeans ARGS] [--bm3d ARGS] [--dctdnoiz ARGS] [--fftdnoiz ARGS] [--smartblur ARGS] [--sab ARGS] [--bilateral ARGS] [--cas ARGS] [--vignette ARGS] [--curves ARGS] [--colorbalance ARGS] [--colorlevels ARGS] [--colorchannelmixer ARGS] [--deflicker ARGS] [--photosensitivity ARGS] [--monochrome ARGS] [--grayworld 0|ARGS] [--drawbox ARGS] [--drawgrid ARGS] [--lagfun ARGS] [--amplify ARGS] [--bitplanenoise ARGS] [--deband ARGS] [--gradfun ARGS] [--lenscorrection ARGS] [--pixelize ARGS] [--removegrain ARGS] [--yaepblur ARGS] [--vibrance ARGS] [--dilation ARGS] [--erosion ARGS] [--colorize ARGS] [--exposure ARGS] [--chromashift ARGS] [--colorcontrast ARGS] [--colorcorrect ARGS] [--histeq ARGS] [--shuffleplanes ARGS] [--lutyuv ARGS] [--colorhold ARGS] [--fade ARGS] [--perspective ARGS] [--lumakey ARGS] [--chromakey ARGS] [--colorkey ARGS] [--despill ARGS] [--selectivecolor ARGS] [--stereo3d ARGS] [--field ARGS] [--hqx ARGS] [--xbr ARGS] [--il ARGS] [--super2xsai ARGS] [--kerndeint ARGS] [--phase ARGS] [--estdif ARGS] [--tinterlace ARGS] [--separatefields ARGS] [--weave ARGS] [--doubleweave ARGS] [--framepack ARGS] [--telecine ARGS] [--pullup ARGS] [--decimate ARGS] [--mpdecimate ARGS] [--framestep ARGS] [--tile ARGS] [--untile ARGS] [--shuffleframes ARGS] [--reverse ARGS] [--loop ARGS] [--thumbnail ARGS] [--freezedetect ARGS] [--setpts ARGS] [--pseudocolor ARGS] [--minterpolate ARGS] [--fps RATE] [--from SECONDS --to SECONDS [--seek]] [--streams 0,1] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N] | crop-lossless INPUT OUTPUT.mkv --crop X:Y:WIDTH:HEIGHT [--streams 0,1] | hw-filter INPUT OUTPUT.mp4 [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--from SECONDS --to SECONDS] [--device N]";
    let Some(command) = args.first() else {
        return Err(help.into());
    };
    if command == "--help" || command == "-h" {
        println!("{help}");
        return Ok(());
    }
    if command == "play" {
        #[cfg(feature = "player")]
        {
            return play_command(&args[1..]);
        }
        #[cfg(not(feature = "player"))]
        {
            return Err("play requires cargo build --features media,player".into());
        }
    }
    if command == "probe" {
        let mut paths = Vec::new();
        let mut hint: Option<&str> = None;
        let mut quiet = false;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--input-format" => {
                    if hint.is_some() {
                        return Err(help.into());
                    }
                    let Some(value) = args.get(i + 1) else {
                        return Err(help.into());
                    };
                    hint = Some(value.as_str());
                    i += 2;
                }
                "--quiet" | "-q" => {
                    quiet = true;
                    i += 1;
                }
                arg if arg.starts_with('-') => return Err(help.into()),
                _ => {
                    paths.push(args[i].as_str());
                    i += 1;
                }
            }
        }
        if paths.len() != 1 {
            return Err(help.into());
        }
        emit_json(
            quiet,
            serde_json::to_string_pretty(&fvid::media::probe_as(&PathBuf::from(paths[0]), hint)?)?,
        );
        return Ok(());
    }

    let mut paths = Vec::new();
    let mut output_format: Option<String> = None;
    let mut input_format: Option<String> = None;
    let mut from = None;
    let mut to = None;
    let mut filter_spec = fvid::media_info::VideoFilterSpec::default();
    let mut overlay_path: Option<PathBuf> = None;
    let mut overlay_x = 0i32;
    let mut overlay_y = 0i32;
    let mut setpts: Option<String> = None;
    let mut select: Option<String> = None;
    let mut xfade_transition = None;
    let mut xfade_duration = None;
    let mut xfade_offset = None;
    let mut loudnorm_args = None;
    let mut loudnorm_dual = false;
    let mut target_lufs = None;
    let mut sample_peak_dbfs = None;
    let mut channel_weights = None;
    let mut mix_duration = fvid_media::MixDuration::Shortest;
    let mut mix_normalize = true;
    let mut mix_weights = Vec::new();
    let mut sample_rate = None;
    let mut channels = None;
    let mut volume = None;
    let mut seek = false;
    let mut encoder = None;
    let mut encoder_options = Vec::new();
    let mut subtitle_codec = fvid_media::SubtitleCodec::Ass;
    let mut device = 0usize;
    let mut cuda_shader = None;
    #[cfg(feature = "media-cuda")]
    let mut cuda_shader_sampling = false;
    let mut device_set = false;
    let mut quiet = false;
    let mut progress = false;
    let mut options = fvid_media::CopyOptions::default();
    let mut i = 1;
    while i < args.len() {
        if let Some((slot, missing_msg)) = string_filter_slot(&mut filter_spec, args[i].as_str()) {
            i += 1;
            *slot = Some(args.get(i).ok_or(missing_msg)?.clone());
            i += 1;
            continue;
        }
        match args[i].as_str() {
            "--quiet" | "-q" => quiet = true,
            "--progress" => progress = true,
            "--input-format" => {
                i += 1;
                input_format = Some(args.get(i).ok_or("missing input format name")?.clone());
            }
            "--output-format" => {
                if output_format.is_some() {
                    return Err("duplicate output-format option".into());
                }
                i += 1;
                output_format = Some(args.get(i).ok_or("missing output format")?.clone());
            }
            "--encoder" => {
                i += 1;
                encoder = Some(args.get(i).ok_or("missing encoder name")?.clone());
            }
            "--codec" => {
                i += 1;
                subtitle_codec =
                    fvid_media::SubtitleCodec::parse(args.get(i).ok_or("missing subtitle codec")?)?;
            }
            "--encoder-option" => {
                i += 1;
                let (key, value) = args
                    .get(i)
                    .ok_or("missing encoder option")?
                    .split_once('=')
                    .ok_or("encoder option must be KEY=VALUE")?;
                encoder_options.push((key.to_owned(), value.to_owned()));
            }
            "--device" => {
                i += 1;
                device = args.get(i).ok_or("missing device")?.parse()?;
                device_set = true;
            }
            "--seek" => seek = true,
            "--cuda-shader" | "--cuda-sampling-shader" => {
                #[cfg(feature = "media-cuda")]
                {
                    cuda_shader_sampling = args[i] == "--cuda-sampling-shader";
                }
                if command != "hw-filter" {
                    return Err("--cuda-shader requires hw-filter".into());
                }
                if cuda_shader.is_some() {
                    return Err("one CUDA shader per hw-filter invocation".into());
                }
                i += 1;
                let path = args.get(i).ok_or("missing CUDA shader path")?;
                use std::io::Read;
                let mut source = String::new();
                std::fs::File::open(path)?
                    .take(65537)
                    .read_to_string(&mut source)?;
                if source.trim().is_empty() || source.len() > 65536 || source.contains('\0') {
                    return Err("CUDA shader must contain 1..=65536 bytes without NUL".into());
                }
                cuda_shader = Some(std::sync::Arc::<str>::from(source));
            }
            "--hflip" => filter_spec.horizontal_flip = true,
            "--vflip" => filter_spec.vertical_flip = true,
            "--crop" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing crop")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<usize>, _>>()?;
                if values.len() != 4 {
                    return Err("crop must be X:Y:WIDTH:HEIGHT".into());
                }
                filter_spec.crop = Some(fvid_media::CropRect {
                    x: values[0],
                    y: values[1],
                    width: values[2],
                    height: values[3],
                });
            }
            "--scale" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing scale")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<u32>, _>>()?;
                if values.len() != 2 {
                    return Err("scale must be WIDTH:HEIGHT".into());
                }
                filter_spec.scale = Some(fvid_media::ScaleSize {
                    width: values[0],
                    height: values[1],
                });
            }
            "--transpose" => {
                i += 1;
                filter_spec.transpose = Some(fvid_media::TransposeMode::parse(
                    args.get(i).ok_or("missing transpose mode")?,
                )?);
            }
            "--rotate" => {
                i += 1;
                filter_spec.rotate = Some(fvid_media::RotateAngle::parse(
                    args.get(i).ok_or("missing rotate degrees")?,
                )?);
            }
            "--pad" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing pad")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<u32>, _>>()?;
                if values.len() != 4 {
                    return Err("pad must be WIDTH:HEIGHT:X:Y".into());
                }
                filter_spec.pad = Some(fvid_media::PadRect {
                    width: values[0],
                    height: values[1],
                    x: values[2],
                    y: values[3],
                });
            }
            "--subs" => {
                i += 1;
                filter_spec.burn_subs = Some(PathBuf::from(
                    args.get(i).ok_or("missing subtitle file")?,
                ));
            }
            "--overlay" => {
                i += 1;
                overlay_path = Some(PathBuf::from(args.get(i).ok_or("missing overlay video")?));
            }
            "--overlay-x" => {
                i += 1;
                overlay_x = args
                    .get(i)
                    .ok_or("missing overlay-x")?
                    .parse()
                    .map_err(|_| "overlay-x must be an integer")?;
            }
            "--overlay-y" => {
                i += 1;
                overlay_y = args
                    .get(i)
                    .ok_or("missing overlay-y")?
                    .parse()
                    .map_err(|_| "overlay-y must be an integer")?;
            }
            "--setpts" => {
                i += 1;
                setpts = Some(args.get(i).ok_or("missing setpts args")?.clone());
            }
            "--select" => {
                i += 1;
                select = Some(args.get(i).ok_or("missing select args")?.clone());
            }
            "--transition" => {
                i += 1;
                xfade_transition = Some(args.get(i).ok_or("missing xfade transition")?.clone());
            }
            "--xfade-duration" => {
                i += 1;
                xfade_duration = Some(fvid::media_info::parse_time(
                    args.get(i).ok_or("missing xfade duration")?,
                )?);
            }
            "--xfade-offset" => {
                i += 1;
                xfade_offset = Some(fvid::media_info::parse_time(
                    args.get(i).ok_or("missing xfade offset")?,
                )?);
            }
            "--loudnorm-args" => {
                i += 1;
                loudnorm_args = Some(args.get(i).ok_or("missing loudnorm args")?.clone());
            }
            "--dual-pass" => loudnorm_dual = true,
            "--target-lufs" => {
                i += 1;
                target_lufs = Some(
                    args.get(i)
                        .ok_or("missing normalization target")?
                        .parse::<f64>()?,
                );
            }
            "--sample-peak-dbfs" => {
                i += 1;
                sample_peak_dbfs = Some(
                    args.get(i)
                        .ok_or("missing normalization target")?
                        .parse::<f64>()?,
                );
            }
            "--channel-weights" => {
                i += 1;
                channel_weights = Some(
                    args.get(i)
                        .ok_or("missing channel weights")?
                        .split(',')
                        .map(str::parse::<f64>)
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
            "--duration" => {
                i += 1;
                mix_duration =
                    fvid_media::MixDuration::parse(args.get(i).ok_or("missing mix duration")?)?;
            }
            "--normalize" => mix_normalize = true,
            "--no-normalize" => mix_normalize = false,
            "--weights" => {
                i += 1;
                mix_weights = args
                    .get(i)
                    .ok_or("missing mix weights")?
                    .split(|c| c == ',' || c == ' ')
                    .filter(|part| !part.is_empty())
                    .map(|part| {
                        part.parse::<f32>()
                            .map_err(|_| "mix weight must be a number")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                if mix_weights.is_empty() {
                    return Err("mix weights must not be empty".into());
                }
            }
            "--rate" => {
                i += 1;
                sample_rate = Some(
                    args.get(i)
                        .ok_or("missing sample rate")?
                        .parse::<i32>()
                        .map_err(|_| "sample rate must be an integer")?,
                );
            }
            "--channels" => {
                i += 1;
                channels = Some(
                    args.get(i)
                        .ok_or("missing channel count")?
                        .parse::<i32>()
                        .map_err(|_| "channels must be an integer")?,
                );
            }
            "--volume" => {
                i += 1;
                volume = Some(
                    args.get(i)
                        .ok_or("missing volume")?
                        .parse::<f64>()
                        .map_err(|_| "volume must be a linear gain number")?,
                );
            }
            "--streams" => {
                i += 1;
                options.streams = args
                    .get(i)
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--max-packets" => {
                i += 1;
                options.max_packets = Some(
                    args.get(i)
                        .ok_or("missing max-packets")?
                        .parse()
                        .map_err(|_| "invalid max-packets")?,
                );
            }
            "--max-packet-bytes" => {
                i += 1;
                let limit = args
                    .get(i)
                    .ok_or("missing max-packet-bytes")?
                    .parse()
                    .map_err(|_| "invalid max-packet-bytes")?;
                if limit == 0 {
                    return Err("packet byte limit must be positive".into());
                }
                options.max_packet_bytes = limit;
            }
            "--max-memory-mib" => {
                i += 1;
                options.max_controlled_bytes = Some(fvid_media::parse_max_memory_mib(
                    args.get(i).ok_or("missing max-memory-mib")?,
                )?);
            }
            "--max-rss-mib" => {
                i += 1;
                options.max_rss_bytes = Some(fvid_media::parse_max_rss_mib(
                    args.get(i).ok_or("missing max-rss-mib")?,
                )?);
            }
            "--metadata" => {
                i += 1;
                let (key, value) = args
                    .get(i)
                    .ok_or("missing metadata")?
                    .split_once('=')
                    .ok_or("metadata must be KEY=VALUE")?;
                options
                    .metadata_set
                    .push((key.to_owned(), value.to_owned()));
            }
            "--metadata-delete" => {
                i += 1;
                options
                    .metadata_delete
                    .push(args.get(i).ok_or("missing metadata key")?.clone());
            }
            "--stream-metadata" => {
                i += 1;
                let raw = args.get(i).ok_or("missing stream metadata")?;
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
                i += 1;
                let raw = args.get(i).ok_or("missing stream metadata key")?;
                let (index, key) = raw
                    .split_once(':')
                    .ok_or("stream metadata delete must be INDEX:KEY")?;
                options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--from" => {
                i += 1;
                from = Some(fvid::media_info::parse_time(
                    args.get(i).ok_or("missing start")?,
                )?);
            }
            "--to" => {
                i += 1;
                to = Some(fvid::media_info::parse_time(
                    args.get(i).ok_or("missing end")?,
                )?);
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown media option: {value}").into());
            }
            value => paths.push(PathBuf::from(value)),
        }
        i += 1;
    }

    filter_spec.overlay = overlay_path.map(|path| fvid_media::OverlaySpec {
        path,
        x: overlay_x,
        y: overlay_y,
    });
    options.progress = make_progress_hook(progress);

    let metadata_mutation = !options.metadata_set.is_empty()
        || !options.metadata_delete.is_empty()
        || !options.stream_metadata_set.is_empty()
        || !options.stream_metadata_delete.is_empty();
    // A demuxer named by the caller is only ever read from, so the commands that write a file
    // refuse the option rather than accept and ignore it.
    if input_format.is_some() && command != "decode" {
        return Err(format!("{command} does not take --input-format").into());
    }
    if output_format.is_some() && command != "plan" {
        return Err("unknown media option: --output-format".into());
    }
    if command == "plan" {
        if encoder.is_some() || !encoder_options.is_empty() {
            return Err("plan does not take metadata mutation or encoder selection".into());
        }
        let mut plan_paths = paths;
        let sub = if plan_paths
            .first()
            .and_then(|p| p.to_str())
            .is_some_and(|s| {
                matches!(
                    s,
                    "trim"
                        | "trim-pcm"
                        | "concat"
                        | "remux"
                        | "transcode-lossless"
                        | "crop-lossless"
                        | "overlay"
                        | "xfade"
                        | "burn-subtitles"
                        | "convert-subtitles"
                        | "loudness"
                        | "normalize-loudness"
                        | "loudnorm"
                        | "mix-audio"
                        | "merge-audio"
                        | "decode-audio"
                )
            }) {
            Some(plan_paths.remove(0))
        } else {
            None
        };
        if output_format.is_some()
            && !matches!(
                sub.as_deref().and_then(|s| s.to_str()),
                None
                    | Some(
                        "mix-audio"
                            | "merge-audio"
                            | "concat"
                            | "remux"
                            | "trim"
                            | "trim-pcm"
                            | "decode-audio"
                            | "normalize-loudness"
                            | "loudnorm"
                            | "convert-subtitles"
                            | "transcode-lossless"
                            | "crop-lossless"
                            | "overlay"
                            | "xfade"
                            | "burn-subtitles"
                    )
            )
        {
            return Err("unknown media option: --output-format".into());
        }
        if metadata_mutation
            && !matches!(
                sub.as_deref().and_then(|s| s.to_str()),
                None
                    | Some(
                        "mix-audio"
                            | "merge-audio"
                            | "concat"
                            | "remux"
                            | "transcode-lossless"
                            | "crop-lossless"
                            | "overlay"
                            | "xfade"
                            | "burn-subtitles"
                            | "convert-subtitles"
                            | "normalize-loudness"
                            | "loudnorm"
                            | "decode-audio"
                            | "trim"
                            | "trim-pcm"
                    )
            )
        {
            return Err("plan does not take metadata mutation or encoder selection".into());
        }
        let plan_interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("plan interval requires both --from and --to".into()),
        };
        let geometry =
            filter_spec.has_any_filter() || setpts.is_some() || select.is_some() || seek;
        let transform = filter_spec.into_lossless_transform(None, plan_interval, seek);
        let plan = match sub.as_ref().and_then(|p| p.to_str()) {
            Some("trim") => {
                if plan_paths.is_empty() || plan_paths.len() > 2 || geometry {
                    return Err(
                        "plan trim requires INPUT [OUTPUT] --from SECONDS --to SECONDS without geometry flags"
                            .into(),
                    );
                }
                let resolved_dest_ext = plan_paths
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
                                "mp4" => "mp4",
                                "m4a" => "m4a",
                                "y4m" => "y4m",
                                _ => "invalid",
                            })
                    });
                if let Some(dest_ext) = resolved_dest_ext {
                    if dest_ext == "invalid" {
                        return Err("unsupported plan trim output format".into());
                    }
                    if dest_ext == "y4m" && metadata_mutation {
                        return Err("Y4M output does not support metadata mutations".into());
                    }
                    if dest_ext == "f32le" && metadata_mutation {
                        return Err(
                            "trim to .f32le does not support metadata mutations".into(),
                        );
                    }
                    if dest_ext == "wav" {
                        if !options.stream_metadata_set.is_empty()
                            || !options.stream_metadata_delete.is_empty()
                        {
                            return Err(
                                "trim to .wav does not support stream metadata mutations".into(),
                            );
                        }
                        fvid_media::owned_wave_metadata::edit_info_chunks(
                            &[],
                            &options.metadata_delete,
                            &options.metadata_set,
                        )?;
                    }
                }
                let (start, end) = match (from, to) {
                    (Some(start), Some(end)) => (start, end),
                    _ => return Err("plan trim requires both --from and --to".into()),
                };
                fvid::media::plan_trim(&plan_paths[0], start, end, &options)?
            }
            Some("trim-pcm") => {
                if plan_paths.is_empty() || plan_paths.len() > 2 || geometry {
                    return Err(
                        "plan trim-pcm requires INPUT [OUTPUT.wav] --from SECONDS --to SECONDS [--streams INDEX] without geometry flags"
                            .into(),
                    );
                }
                let resolved_dest_ext = plan_paths
                    .get(1)
                    .and_then(|d| d.extension().and_then(|s| s.to_str()))
                    .or_else(|| {
                        output_format
                            .as_deref()
                            .map(|fmt| match fmt.to_ascii_lowercase().as_str() {
                                "wav" => "wav",
                                _ => "invalid",
                            })
                    });
                if let Some(dest_ext) = resolved_dest_ext {
                    if dest_ext != "wav" {
                        return Err("trim-pcm requires OUTPUT.wav".into());
                    }
                    if !options.stream_metadata_set.is_empty()
                        || !options.stream_metadata_delete.is_empty()
                    {
                        return Err(
                            "trim-pcm to .wav does not support stream metadata mutations".into(),
                        );
                    }
                    fvid_media::owned_wave_metadata::edit_info_chunks(
                        &[],
                        &options.metadata_delete,
                        &options.metadata_set,
                    )?;
                }
                let (start, end) = match (from, to) {
                    (Some(start), Some(end)) => (start, end),
                    _ => return Err("plan trim-pcm requires both --from and --to".into()),
                };
                fvid::media::plan_trim_pcm(&plan_paths[0], start, end, &options)?
            }
            Some("concat") => {
                if plan_paths.len() < 2 || from.is_some() || to.is_some() || geometry {
                    return Err(
                        "plan concat requires INPUT INPUT... without --from/--to/geometry flags"
                            .into(),
                    );
                }
                let (_, sources) = resolve_plan_concat_destination(
                    &plan_paths,
                    output_format.as_deref(),
                    &options,
                )?;
                fvid::media::plan_concat(sources, &options)?
            }
            Some("remux") => {
                if plan_paths.is_empty()
                    || plan_paths.len() > 2
                    || from.is_some()
                    || to.is_some()
                    || geometry
                {
                    return Err(
                        "plan remux requires INPUT [OUTPUT] without transform flags".into(),
                    );
                }
                let resolved_dest_ext = plan_paths
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
                        return Err(
                            "plan remux output must be .mp4, .m4a, .mka, .mkv, or .wav".into(),
                        );
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
                    } else if matches!(dest_ext, "mp4" | "m4a") && metadata_mutation {
                        return Err(
                            "remux to .mp4/.m4a does not support metadata mutations".into(),
                        );
                    }
                }
                fvid::media::plan_remux(&plan_paths[0], &options)?
            }
            Some("transcode-lossless") => {
                if !(1..=2).contains(&plan_paths.len()) {
                    return Err("plan transcode-lossless requires a single INPUT".into());
                }
                if let Some(dest) = plan_paths.get(1)
                    && dest
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                if let Some(fmt) = output_format.as_deref()
                    && !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska")
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                fvid::media::plan_transcode_lossless(&plan_paths[0], &transform, &options, None)?
            }
            Some("crop-lossless") => {
                if !(1..=2).contains(&plan_paths.len()) {
                    return Err("plan crop-lossless requires a single INPUT".into());
                }
                if let Some(dest) = plan_paths.get(1)
                    && dest
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                if let Some(fmt) = output_format.as_deref()
                    && !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska")
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                if transform.crop.is_none() {
                    return Err("--crop required".into());
                }
                let mut plan = fvid::media::plan_transcode_lossless(
                    &plan_paths[0],
                    &transform,
                    &options,
                    None,
                )?;
                plan.command = "crop-lossless".into();
                plan
            }
            Some("overlay") => {
                let (source, overlay_file, dest, x, y) =
                    match (plan_paths.as_slice(), transform.overlay.as_ref()) {
                        ([source, overlay], None) => (
                            source.as_path(),
                            overlay.as_path(),
                            None,
                            overlay_x,
                            overlay_y,
                        ),
                        ([source, overlay, dest], None) => (
                            source.as_path(),
                            overlay.as_path(),
                            Some(dest.as_path()),
                            overlay_x,
                            overlay_y,
                        ),
                        ([source], Some(spec)) => {
                            (source.as_path(), spec.path.as_path(), None, spec.x, spec.y)
                        }
                        ([source, dest], Some(spec)) => (
                            source.as_path(),
                            spec.path.as_path(),
                            Some(dest.as_path()),
                            spec.x,
                            spec.y,
                        ),
                        _ => {
                            return Err(
                                "plan overlay requires MAIN FOREGROUND or MAIN --overlay FOREGROUND"
                                    .into(),
                            );
                        }
                    };
                if let Some(dest) = dest
                    && dest
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                if let Some(fmt) = output_format.as_deref()
                    && !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska")
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                fvid::media::plan_overlay(source, overlay_file, x, y, &options)?
            }
            Some("xfade") => {
                if !(2..=3).contains(&plan_paths.len()) {
                    return Err(
                        "plan xfade requires MAIN OTHER --xfade-duration SECONDS [--transition NAME] [--xfade-offset SECONDS]"
                            .into(),
                    );
                }
                if let Some(dest) = plan_paths.get(2)
                    && dest
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                if let Some(fmt) = output_format.as_deref()
                    && !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska")
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                let duration = xfade_duration.ok_or("plan xfade requires --xfade-duration")?;
                let transition = xfade_transition.as_deref().unwrap_or("fade");
                let offset = xfade_offset.unwrap_or(0);
                fvid_media::plan_xfade(
                    &plan_paths[0],
                    &plan_paths[1],
                    transition,
                    duration,
                    offset,
                    &options,
                )?
            }
            Some("burn-subtitles") => {
                if !(1..=2).contains(&plan_paths.len()) || transform.burn_subs.is_none() {
                    return Err("plan burn-subtitles requires INPUT --subs FILE.srt".into());
                }
                if let Some(dest) = plan_paths.get(1)
                    && dest
                        .extension()
                        .and_then(|e| e.to_str())
                        .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                if let Some(fmt) = output_format.as_deref()
                    && !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska")
                {
                    return Err("owned FFV1 export requires .mkv".into());
                }
                fvid_media::plan_burn_subtitles(
                    &plan_paths[0],
                    transform.burn_subs.as_ref().unwrap(),
                    &options,
                )?
            }
            Some("convert-subtitles") => {
                if plan_paths.is_empty() || plan_paths.len() > 2 || geometry {
                    return Err(
                        "plan convert-subtitles requires INPUT [OUTPUT.mkv] [--codec ass] [--streams INDEX] without video transform flags"
                            .into(),
                    );
                }
                let fmt_dest = match output_format.as_deref().map(|s| s.to_ascii_lowercase()) {
                    None => None,
                    Some(ref s) if s == "mkv" || s == "matroska" => Some(Path::new("output.mkv")),
                    Some(_) => {
                        return Err("convert-subtitles requires Matroska (.mkv) output".into());
                    }
                };
                fvid::media::plan_convert_subtitles_for_destination(
                    &plan_paths[0],
                    plan_paths.get(1).map(PathBuf::as_path).or(fmt_dest),
                    &fvid_media::SubtitleConvertOptions {
                        streams: options.streams.clone(),
                        codec: subtitle_codec,
                    },
                    &options,
                )?
            }
            Some("loudness") => {
                if plan_paths.len() != 1 || geometry {
                    return Err(
                        "plan loudness requires INPUT without video transform flags".into(),
                    );
                }
                if let Some(weights) = channel_weights.as_deref() {
                    fvid::native_plan::loudness_with_options(
                        &plan_paths[0],
                        &options,
                        Some(weights),
                    )?
                } else {
                    fvid::media::plan_loudness(&plan_paths[0], &options)?
                }
            }
            Some("normalize-loudness") => {
                if plan_paths.is_empty() || plan_paths.len() > 2 || geometry {
                    return Err(
                        "plan normalize-loudness requires INPUT [OUTPUT.wav] without video transform flags"
                            .into(),
                    );
                }
                let resolved_dest_ext = plan_paths
                    .get(1)
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
                        return Err(
                            "normalization requires OUTPUT.wav, OUTPUT.mka or OUTPUT.mkv".into(),
                        );
                    }
                    if dest_ext == "wav" {
                        if !options.stream_metadata_set.is_empty()
                            || !options.stream_metadata_delete.is_empty()
                        {
                            return Err(
                                "normalize-loudness to .wav does not support stream metadata mutations"
                                    .into(),
                            );
                        }
                        fvid_media::owned_wave_metadata::edit_info_chunks(
                            &[],
                            &options.metadata_delete,
                            &options.metadata_set,
                        )?;
                    }
                }
                let defaults = fvid::native_pcm::NormalizeTarget::default();
                let target = fvid::native_pcm::NormalizeTarget {
                    integrated_lufs: target_lufs.unwrap_or(defaults.integrated_lufs),
                    sample_peak_dbfs: sample_peak_dbfs.unwrap_or(defaults.sample_peak_dbfs),
                };
                fvid::media::plan_normalize_loudness(
                    &plan_paths[0],
                    &options,
                    channel_weights.as_deref(),
                    target,
                )?
            }
            Some("loudnorm") => {
                if plan_paths.is_empty() || plan_paths.len() > 2 || geometry {
                    return Err(
                        "plan loudnorm requires INPUT [OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv] [--loudnorm-args ARGS] [--dual-pass] without video transform flags"
                            .into(),
                    );
                }
                let resolved_dest_ext = plan_paths
                    .get(1)
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
                        return Err(
                            "loudnorm requires OUTPUT.wav, OUTPUT.mka or OUTPUT.mkv".into(),
                        );
                    }
                    if dest_ext == "wav" {
                        if !options.stream_metadata_set.is_empty()
                            || !options.stream_metadata_delete.is_empty()
                        {
                            return Err(
                                "loudnorm to .wav does not support stream metadata mutations"
                                    .into(),
                            );
                        }
                        fvid_media::owned_wave_metadata::edit_info_chunks(
                            &[],
                            &options.metadata_delete,
                            &options.metadata_set,
                        )?;
                    }
                }
                fvid::media::plan_loudnorm(
                    &plan_paths[0],
                    loudnorm_args.as_deref(),
                    loudnorm_dual,
                    &options,
                )?
            }
            Some("mix-audio") => {
                if plan_paths.len() < 2 || geometry {
                    return Err(
                        "plan mix-audio requires INPUT INPUT... [--weights W,...] [--duration shortest] [--normalize|--no-normalize]"
                            .into(),
                    );
                }
                let (_, sources) = resolve_plan_mix_or_merge_destination(
                    "mix-audio",
                    &plan_paths,
                    output_format.as_deref(),
                    &options,
                )?;
                fvid::media::plan_mix_audio_with_controls(
                    sources,
                    &fvid_media::MixAudioOptions {
                        duration: mix_duration,
                        normalize: mix_normalize,
                        weights: mix_weights,
                    },
                    &options,
                )?
            }
            Some("merge-audio") => {
                if !(2..=3).contains(&plan_paths.len()) || geometry {
                    return Err(
                        "plan merge-audio requires [OUTPUT.wav|OUTPUT.mka|OUTPUT.mkv] INPUT INPUT"
                            .into(),
                    );
                }
                let (_, sources) = resolve_plan_mix_or_merge_destination(
                    "merge-audio",
                    &plan_paths,
                    output_format.as_deref(),
                    &options,
                )?;
                if sources.len() != 2 {
                    return Err("plan merge-audio requires INPUT INPUT".into());
                }
                fvid::media::plan_merge_audio_with_controls(sources, &options)?
            }
            Some("decode-audio") => {
                if plan_paths.is_empty() || plan_paths.len() > 2 || geometry {
                    return Err(
                        "plan decode-audio requires INPUT [OUTPUT.wav|OUTPUT.f32le|OUTPUT.mka|OUTPUT.mkv] [--rate HZ] [--channels N] [--volume GAIN] [--from SECONDS --to SECONDS] [--streams INDEX] without video transform flags"
                            .into(),
                    );
                }
                let resolved_dest_ext = plan_paths
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
                            "decode-audio requires OUTPUT.wav, OUTPUT.f32le, OUTPUT.mka or OUTPUT.mkv"
                                .into(),
                        );
                    }
                    if dest_ext == "f32le" && metadata_mutation {
                        return Err(
                            "decode-audio to .f32le does not support metadata mutations".into(),
                        );
                    }
                    if dest_ext == "wav" {
                        if !options.stream_metadata_set.is_empty()
                            || !options.stream_metadata_delete.is_empty()
                        {
                            return Err(
                                "decode-audio to .wav does not support stream metadata mutations"
                                    .into(),
                            );
                        }
                        fvid_media::owned_wave_metadata::edit_info_chunks(
                            &[],
                            &options.metadata_delete,
                            &options.metadata_set,
                        )?;
                    }
                }
                let interval = match (from, to) {
                    (None, None) => None,
                    (Some(start), Some(end)) => Some((start, end)),
                    _ => {
                        return Err(
                            "plan decode-audio interval requires both --from and --to".into(),
                        );
                    }
                };
                fvid::media::plan_decode_audio(
                    &plan_paths[0],
                    &fvid_media::AudioDecodeTransform {
                        interval,
                        sample_rate,
                        channels,
                        volume,
                    },
                    &options,
                )?
            }
            None => {
                if !(1..=2).contains(&plan_paths.len()) {
                    return Err(
                        "plan requires INPUT, or: plan trim|trim-pcm|concat|remux|transcode-lossless|overlay|xfade|burn-subtitles|loudness|loudnorm|mix-audio|merge-audio|decode-audio ..."
                            .into(),
                    );
                }
                let idle = !geometry && transform.interval.is_none();
                if idle {
                    let resolved_dest_ext = plan_paths
                        .get(1)
                        .and_then(|d| d.extension().and_then(|s| s.to_str()))
                        .or_else(|| {
                            output_format.as_deref().map(|fmt| {
                                match fmt.to_ascii_lowercase().as_str() {
                                    "mp4" => "mp4",
                                    "m4a" => "m4a",
                                    "mka" => "mka",
                                    "mkv" | "matroska" => "mkv",
                                    "wav" => "wav",
                                    _ => "invalid",
                                }
                            })
                        });
                    if let Some(dest_ext) = resolved_dest_ext {
                        if !matches!(dest_ext, "mp4" | "m4a" | "mka" | "mkv" | "wav") {
                            return Err(
                                "plan remux output must be .mp4, .m4a, .mka, .mkv, or .wav".into(),
                            );
                        }
                        if dest_ext == "wav" {
                            if !options.stream_metadata_set.is_empty()
                                || !options.stream_metadata_delete.is_empty()
                            {
                                return Err(
                                    "WAVE remux does not support stream metadata edits".into(),
                                );
                            }
                            fvid_media::owned_wave_metadata::edit_info_chunks(
                                &[],
                                &options.metadata_delete,
                                &options.metadata_set,
                            )?;
                        } else if matches!(dest_ext, "mp4" | "m4a") && metadata_mutation {
                            return Err(
                                "remux to .mp4/.m4a does not support metadata mutations".into(),
                            );
                        }
                    }
                    fvid::media::plan_remux(&plan_paths[0], &options)?
                } else {
                    if let Some(dest) = plan_paths.get(1)
                        && dest
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_none_or(|e| !e.eq_ignore_ascii_case("mkv"))
                    {
                        return Err("owned FFV1 export requires .mkv".into());
                    }
                    if let Some(fmt) = output_format.as_deref()
                        && !matches!(fmt.to_ascii_lowercase().as_str(), "mkv" | "matroska")
                    {
                        return Err("owned FFV1 export requires .mkv".into());
                    }
                    fvid::media::plan_transcode_lossless(
                        &plan_paths[0],
                        &transform,
                        &options,
                        None,
                    )?
                }
            }
            Some(_) => unreachable!(),
        };
        emit_json(quiet, serde_json::to_string_pretty(&plan)?);
        return Ok(());
    }

    if metadata_mutation
        && !matches!(
            command.as_str(),
            "remux"
                | "mix-audio"
                | "merge-audio"
                | "concat"
                | "transcode-lossless"
                | "crop-lossless"
                | "transcode"
                | "overlay"
                | "xfade"
                | "burn-subtitles"
                | "convert-subtitles"
                | "loudnorm"
                | "normalize-loudness"
                | "decode-audio"
                | "trim"
                | "trim-pcm"
        )
    {
        return Err("metadata mutation requires remux".into());
    }
    if command != "transcode" && (encoder.is_some() || !encoder_options.is_empty()) {
        return Err("encoder selection requires explicit transcode command".into());
    }
    if command == "hw-filter" && paths.len() == 2 {
        #[cfg(feature = "media-cuda")]
        {
            let interval = match (from, to) {
                (None, None) => None,
                (Some(start), Some(end)) => Some((start, end)),
                _ => return Err("hw-filter interval requires both --from and --to".into()),
            };
            let stats = fvid_media::hw_filter(
                &paths[0],
                &paths[1],
                &fvid_media::HwFilterOptions {
                    shader: cuda_shader,
                    shader_sampling: cuda_shader_sampling,
                    crop: filter_spec.crop,
                    horizontal_flip: filter_spec.horizontal_flip,
                    vertical_flip: filter_spec.vertical_flip,
                    device,
                    host_bounce: false,
                    interval,
                    ..Default::default()
                },
            )?;
            emit_json(quiet, serde_json::to_string_pretty(&stats)?);
            return Ok(());
        }
        #[cfg(not(feature = "media-cuda"))]
        {
            let _ = (device, cuda_shader);
            return Err(
                "hw-filter requires cargo build --release --features media-cuda (CUDA NVDEC/NVENC)"
                    .into(),
            );
        }
    }
    if command == "decode" && paths.len() == 1 {
        if device_set
            && (filter_spec.crop.is_some()
                || filter_spec.scale.is_some()
                || filter_spec.transpose.is_some()
                || filter_spec.rotate.is_some()
                || filter_spec.pad.is_some()
                || filter_spec.burn_subs.is_some()
                || filter_spec.pix_fmt.is_some()
                || filter_spec.horizontal_flip
                || filter_spec.vertical_flip
                || from.is_some())
        {
            return Err(
                "decode --device does not take --crop/--hflip/--vflip/--transpose/--rotate/--pad/--scale/--subs/--pix-fmt/--from yet"
                    .into(),
            );
        }
        if device_set && input_format.is_some() {
            return Err("decode --device does not take --input-format yet".into());
        }
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("decode interval requires both --from and --to".into()),
        };
        let stats = if device_set {
            #[cfg(feature = "media-cuda")]
            {
                fvid_media::decode_video_cuda(&paths[0], device)?
            }
            #[cfg(not(feature = "media-cuda"))]
            {
                let _ = device;
                return Err("decode --device requires cargo build --features media-cuda".into());
            }
        } else {
            fvid::media::decode_video_transformed(
                &paths[0],
                filter_spec.into_decode_transform(interval, input_format),
            )?
        };
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if (command == "crop-lossless" || command == "transcode-lossless" || command == "transcode")
        && paths.len() == 2
    {
        if command == "crop-lossless" {
            if filter_spec.crop.is_none() {
                return Err("--crop required".into());
            }
            check_crop_lossless_forbidden(&filter_spec, setpts.as_deref(), select.as_deref())?;
        }
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("lossless interval requires both --from and --to".into()),
        };
        let mut lossless_spec = filter_spec;
        lossless_spec.burn_subs = None;
        let transform = lossless_spec.into_lossless_transform(None, interval, seek);
        let stats = if command == "transcode" {
            fvid::media::transcode(
                &paths[0],
                &paths[1],
                transform,
                &options,
                &fvid_media::EncoderSettings {
                    name: encoder
                        .ok_or("transcode requires --encoder; quality is defined by its options")?,
                    options: encoder_options,
                },
            )?
        } else {
            fvid::media::transcode_lossless(&paths[0], &paths[1], transform, &options)?
        };
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }

    let mut non_compositing = filter_spec.clone();
    non_compositing.burn_subs = None;
    non_compositing.overlay = None;
    if non_compositing.has_any_filter() || setpts.is_some() || select.is_some() || seek {
        return Err(
            "--crop/--hflip/--vflip/--transpose/--rotate/--pad/--scale/--epx/--pix-fmt/--colorspace/--zscale/--tonemap/--yadif/--bwdif/--w3fdif/--tblend/--tmix/--hqdn3d/--gblur/--eq/--unsharp/--hue/--avgblur/--boxblur/--negate/--edgedetect/--sobel/--prewitt/--roberts/--kirsch/--scharr/--atadenoise/--owdenoise/--vaguedenoiser/--nlmeans/--bm3d/--dctdnoiz/--fftdnoiz/--smartblur/--sab/--bilateral/--cas/--vignette/--curves/--colorbalance/--colorlevels/--colorchannelmixer/--deflicker/--photosensitivity/--monochrome/--grayworld/--drawbox/--drawgrid/--lagfun/--amplify/--bitplanenoise/--deband/--gradfun/--lenscorrection/--pixelize/--removegrain/--yaepblur/--vibrance/--dilation/--erosion/--colorize/--exposure/--chromashift/--colorcontrast/--colorcorrect/--histeq/--shuffleplanes/--lutyuv/--colorhold/--fade/--perspective/--lumakey/--chromakey/--colorkey/--despill/--selectivecolor/--stereo3d/--field/--hqx/--xbr/--il/--super2xsai/--kerndeint/--phase/--estdif/--tinterlace/--separatefields/--weave/--doubleweave/--framepack/--telecine/--pullup/--decimate/--mpdecimate/--framestep/--shuffleframes/--reverse/--loop/--thumbnail/--freezedetect/--setpts/--pseudocolor/--minterpolate/--fps/--seek require decode, crop-lossless, transcode-lossless, transcode, or hw-filter"
                .into(),
        );
    }
    if command == "decode-audio" && paths.len() == 2 {
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("decode-audio interval requires both --from and --to".into()),
        };
        let stats = fvid::media::decode_audio_transformed(
            &paths[0],
            &paths[1],
            fvid_media::AudioDecodeTransform {
                interval,
                sample_rate,
                channels,
                volume,
            },
            &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if sample_rate.is_some() || channels.is_some() || volume.is_some() {
        return Err("--rate/--channels/--volume require decode-audio".into());
    }
    if command == "loudness" && paths.len() == 1 {
        let stats = fvid::media::measure_loudness(&paths[0], &options)?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "loudnorm" && paths.len() == 2 {
        if filter_spec.has_any_filter()
            || from.is_some()
            || to.is_some()
            || volume.is_some()
            || sample_rate.is_some()
            || channels.is_some()
        {
            return Err(
                "loudnorm takes INPUT OUTPUT [--loudnorm-args ARGS] [--dual-pass] [--streams INDEX] only"
                    .into(),
            );
        }
        let stats = if loudnorm_dual {
            fvid::media::apply_loudnorm_dual(
                &paths[0],
                &paths[1],
                loudnorm_args.as_deref(),
                &options,
            )?
        } else {
            fvid::media::apply_loudnorm(&paths[0], &paths[1], loudnorm_args.as_deref(), &options)?
        };
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if loudnorm_args.is_some() || loudnorm_dual {
        return Err("--loudnorm-args/--dual-pass require loudnorm".into());
    }
    if command == "convert-subtitles" && paths.len() == 2 {
        let stats = fvid::media::convert_subtitles_with_controls(
            &paths[0],
            &paths[1],
            &fvid_media::SubtitleConvertOptions {
                streams: options.streams.clone(),
                codec: subtitle_codec,
            },
            &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "burn-subtitles" && paths.len() == 2 {
        let subs = filter_spec
            .burn_subs
            .as_deref()
            .ok_or("burn-subtitles requires --subs FILE")?;
        let stats = fvid_media::burn_subtitles(&paths[0], &paths[1], subs, &options)?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "xfade" && paths.len() == 3 {
        let transition = xfade_transition.as_deref().unwrap_or("fade");
        let duration = xfade_duration.ok_or("--xfade-duration required")?;
        let offset = xfade_offset.unwrap_or(0);
        let stats = fvid_media::xfade_video(
            &paths[0], &paths[1], &paths[2], transition, duration, offset, &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "xfade" {
        return Err("xfade requires MAIN OTHER OUTPUT.mkv --xfade-duration SECONDS".into());
    }
    if xfade_transition.is_some() || xfade_duration.is_some() || xfade_offset.is_some() {
        return Err("--transition/--xfade-duration/--xfade-offset require xfade".into());
    }
    if command == "overlay" && paths.len() == 3 {
        let stats = fvid::media::overlay_video(
            &paths[0], &paths[1], &paths[2], overlay_x, overlay_y, &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if filter_spec.burn_subs.is_some() {
        return Err("--subs requires decode or burn-subtitles".into());
    }
    if filter_spec.overlay.is_some() {
        return Err("--overlay requires decode, overlay, transcode-lossless, or transcode".into());
    }
    if command == "trim-pcm" && paths.len() == 2 {
        let stats = fvid::media::trim_pcm(
            &paths[0],
            &paths[1],
            from.ok_or("--from required")?,
            to.ok_or("--to required")?,
            &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "mix-audio" && paths.len() >= 3 {
        let stats = fvid::media::mix_audio_with_controls(
            &paths[1..],
            &paths[0],
            &fvid_media::MixAudioOptions {
                duration: mix_duration,
                normalize: mix_normalize,
                weights: mix_weights,
            },
            &options,
        )?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "merge-audio" && paths.len() == 3 {
        let stats = fvid::media::merge_audio_with_controls(&paths[1..], &paths[0], &options)?;
        emit_json(quiet, serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    let stats = match command.as_str() {
        "remux" if paths.len() == 2 && from.is_none() && to.is_none() => {
            fvid::media::remux(&paths[0], &paths[1], &options)?
        }
        "trim" if paths.len() == 2 => fvid::media::trim(
            &paths[0],
            &paths[1],
            from.ok_or("--from required")?,
            to.ok_or("--to required")?,
            &options,
        )?,
        "concat" if paths.len() >= 3 && from.is_none() && to.is_none() => {
            fvid::media::concat(&paths[1..], &paths[0], &options)?
        }
        _ => return Err(help.into()),
    };
    emit_json(quiet, serde_json::to_string_pretty(&stats)?);
    Ok(())
}
