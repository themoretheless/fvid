use std::path::{Path, PathBuf};

use super::common::{
    CliResult, decode_time, make_progress_hook, parse_native_audio_limit, plain_decode,
};

fn first_decode_input_path(args: &[String]) -> Option<&str> {
    let mut iter = args.get(1..)?.iter();
    let mut positional = false;
    while let Some(arg) = iter.next() {
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        if !positional && arg.starts_with('-') {
            if !matches!(
                arg.as_str(),
                "--quiet" | "-q" | "--progress" | "--hflip" | "--vflip" | "--seek"
            ) {
                let _ = iter.next();
            }
            continue;
        }
        return Some(arg.as_str());
    }
    None
}

pub(super) fn pixel_decode_args(
    args: &[String],
) -> CliResult<(Vec<String>, fvid::native_pixels::PixelFilters)> {
    use fvid::native_pixels::{Gradient, GradientKind, Negate, PixelFilters};
    let mut filters = PixelFilters::default();
    if args.first().map(String::as_str) != Some("decode") {
        return Ok((args.to_vec(), filters));
    }
    let source_hint = first_decode_input_path(args);
    let mut result = vec![args[0].clone()];
    let mut args = args[1..].iter();
    let mut seen = std::collections::BTreeSet::new();
    let mut seen_morphology = std::collections::BTreeSet::new();
    while let Some(arg) = args.next() {
        if arg == "--" {
            result.push(arg.clone());
            result.extend(args.cloned());
            break;
        }
        if arg == "--grayworld" {
            if filters.grayworld.is_some() {
                return Err("duplicate grayworld".into());
            }
            let value = args.next().ok_or("missing grayworld args")?;
            match fvid_media::owned_timeline::Timeline::grayworld(value) {
                Ok(filter) => filters.grayworld = Some(filter),
                Err(_) => {
                    result.push(arg.clone());
                    result.push(value.clone());
                }
            }
            continue;
        }
        if arg == "--cas" {
            if filters.cas.is_some() {
                return Err("duplicate cas".into());
            }
            filters.cas = Some(fvid_media::owned_cas::Cas::parse(
                args.next().ok_or("missing cas args")?,
            )?);
            continue;
        }
        if arg == "--colorcorrect" {
            if filters.colorcorrect.is_some() {
                return Err("duplicate colorcorrect".into());
            }
            filters.colorcorrect = Some(fvid_media::owned_colorcorrect::ColorCorrect::parse(
                args.next().ok_or("missing colorcorrect args")?,
            )?);
            continue;
        }
        if arg == "--colorbalance" {
            if filters.colorbalance.is_some() {
                return Err("duplicate colorbalance".into());
            }
            filters.colorbalance = Some(fvid_media::owned_colorbalance::ColorBalance::parse(
                args.next().ok_or("missing colorbalance args")?,
            )?);
            continue;
        }
        if arg == "--exposure" {
            if filters.exposure.is_some() {
                return Err("duplicate exposure".into());
            }
            filters.exposure = Some(fvid_media::owned_exposure::Exposure::parse(
                args.next().ok_or("missing exposure args")?,
            )?);
            continue;
        }
        if arg == "--colorchannelmixer" {
            if filters.colorchannelmixer.is_some() {
                return Err("duplicate colorchannelmixer".into());
            }
            filters.colorchannelmixer =
                Some(fvid_media::owned_colorchannelmixer::ColorChannelMixer::parse(
                    args.next().ok_or("missing colorchannelmixer args")?,
                )?);
            continue;
        }
        if arg == "--colorlevels" {
            if filters.colorlevels.is_some() {
                return Err("duplicate colorlevels".into());
            }
            filters.colorlevels = Some(fvid_media::owned_colorlevels::ColorLevels::parse(
                args.next().ok_or("missing colorlevels args")?,
            )?);
            continue;
        }
        if arg == "--vibrance" {
            if filters.vibrance.is_some() {
                return Err("duplicate vibrance".into());
            }
            filters.vibrance = Some(fvid_media::owned_vibrance::Vibrance::parse(
                args.next().ok_or("missing vibrance args")?,
            )?);
            continue;
        }
        if arg == "--colorcontrast" {
            if filters.colorcontrast.is_some() {
                return Err("duplicate colorcontrast".into());
            }
            filters.colorcontrast = Some(fvid_media::owned_colorcontrast::ColorContrast::parse(
                args.next().ok_or("missing colorcontrast args")?,
            )?);
            continue;
        }
        if arg == "--fade" {
            if filters.fade.is_some() {
                return Err("duplicate fade".into());
            }
            let value = args.next().ok_or("missing fade args")?;
            match fvid_media::owned_fade::Fade::parse(value) {
                Ok(filter) => filters.fade = Some(filter),
                Err(_) => {
                    result.push(arg.clone());
                    result.push(value.clone());
                }
            }
            continue;
        }
        if arg == "--colorhold" {
            if filters.colorhold.is_some() {
                return Err("duplicate colorhold".into());
            }
            filters.colorhold = Some(fvid_media::owned_colorhold::ColorHold::parse(
                args.next().ok_or("missing colorhold args")?,
            )?);
            continue;
        }
        if arg == "--lutyuv" {
            if filters.lutyuv.is_some() {
                return Err("duplicate lutyuv".into());
            }
            filters.lutyuv = Some(fvid_media::owned_lutyuv::LutYuv::parse(
                args.next().ok_or("missing lutyuv args")?,
            )?);
            continue;
        }
        if arg == "--shuffleplanes" {
            if filters.shuffleplanes.is_some() {
                return Err("duplicate shuffleplanes".into());
            }
            filters.shuffleplanes = Some(fvid::native_shuffleplanes::ShufflePlanes::parse(
                args.next().ok_or("missing shuffleplanes args")?,
            )?);
            continue;
        }
        if arg == "--boxblur" {
            let value = args.next().ok_or("missing boxblur args")?;
            if eligible_source(&result, source_hint)? {
                if filters.boxblur.is_some() {
                    return Err("duplicate boxblur".into());
                }
                if let Ok(filter) = fvid::native_boxblur::BoxBlurProgram::parse(value) {
                    filters.boxblur = Some(filter);
                    continue;
                }
            }
            result.push(arg.clone());
            result.push(value.clone());
            continue;
        }
        if arg == "--pixelize" {
            let value = args.next().ok_or("missing pixelize args")?;
            if eligible_source(&result, source_hint)? {
                if filters.pixelize.is_some() {
                    return Err("duplicate pixelize".into());
                }
                if let Ok(filter) = fvid::native_pixelize::Pixelize::parse(value) {
                    filters.pixelize = Some(filter);
                    continue;
                }
            }
            result.push(arg.clone());
            result.push(value.clone());
            continue;
        }
        if arg == "--bilateral" {
            let value = args.next().ok_or("missing bilateral args")?;
            if eligible_source(&result, source_hint)? {
                if filters.bilateral.is_some() {
                    return Err("duplicate bilateral".into());
                }
                filters.bilateral = Some(fvid_media::owned_bilateral::Bilateral::parse(value)?);
                continue;
            }
            result.push(arg.clone());
            result.push(value.clone());
            continue;
        }
        if arg == "--gblur" {
            let value = args.next().ok_or("missing gblur args")?;
            if eligible_source(&result, source_hint)? {
                if filters.gblur.is_some() {
                    return Err("duplicate gblur".into());
                }
                filters.gblur = Some(fvid_media::owned_gblur::GaussianBlur::parse(value)?);
                continue;
            }
            result.push(arg.clone());
            result.push(value.clone());
            continue;
        }
        if arg == "--avgblur" {
            let value = args.next().ok_or("missing avgblur args")?;
            if eligible_source(&result, source_hint)? {
                if filters.avgblur.is_some() {
                    return Err("duplicate avgblur".into());
                }
                if let Ok(filter) = fvid::native_avgblur::AverageBlur::parse(value) {
                    filters.avgblur = Some(filter);
                    continue;
                }
            }
            result.push(arg.clone());
            result.push(value.clone());
            continue;
        }
        if arg == "--chromashift" {
            let value = args.next().ok_or("missing chromashift args")?;
            // Only admit the already-owned Y4M and AVC/HEVC routes here. Other input
            // formats retain the full parser and adapter until their migration.
            if eligible_source(&result, source_hint)? {
                if filters.chromashift.is_some() {
                    return Err("duplicate chromashift".into());
                }
                if let Ok(filter) = fvid::native_chromashift::ChromaShift::parse(value) {
                    filters.chromashift = Some(filter);
                    continue;
                }
            }
            result.push(arg.clone());
            result.push(value.clone());
            continue;
        }
        if arg == "--unsharp" {
            let value = args.next().ok_or("missing unsharp args")?;
            if filters.unsharp.is_some() {
                return Err("duplicate unsharp".into());
            }
            if let Ok(filter) = fvid_media::owned_unsharp::Unsharp::parse(value) {
                filters.unsharp = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--eq" {
            let value = args.next().ok_or("missing eq args")?;
            if filters.eq.is_some() {
                return Err("duplicate eq".into());
            }
            if let Ok(filter) = fvid_media::owned_eq::EqualizerProgram::parse(value) {
                filters.eq = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--monochrome" {
            let value = args.next().ok_or("missing monochrome args")?;
            if filters.monochrome.is_some() {
                return Err("duplicate monochrome".into());
            }
            if let Ok(filter) = fvid_media::owned_monochrome::Monochrome::parse(value) {
                filters.monochrome = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--colorize" {
            let value = args.next().ok_or("missing colorize args")?;
            if filters.colorize.is_some() {
                return Err("duplicate colorize".into());
            }
            if let Ok(filter) = fvid_media::owned_colorize::Colorize::parse(value) {
                filters.colorize = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--sab" {
            let value = args.next().ok_or("missing sab options")?;
            if filters.sab.is_some() {
                return Err("duplicate sab".into());
            }
            if let Ok(filter) = fvid_media::owned_sab::Sab::parse(value) {
                filters.sab = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--bitplanenoise" {
            let value = args.next().ok_or("missing bitplanenoise options")?;
            if filters.bitplanenoise.is_some() {
                return Err("duplicate bitplanenoise".into());
            }
            if let Ok(filter) = fvid_media::owned_bitplanenoise::BitPlaneNoise::parse(value) {
                filters.bitplanenoise = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--deband" {
            let value = args.next().ok_or("missing deband options")?;
            if filters.deband.is_some() {
                return Err("duplicate deband".into());
            }
            if let Ok(filter) = fvid_media::owned_deband::Deband::parse(value) {
                filters.deband = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--perspective" {
            let value = args.next().ok_or("missing perspective options")?;
            if filters.perspective.is_some() {
                return Err("duplicate perspective".into());
            }
            if let Ok(filter) = fvid_media::owned_perspective::Perspective::parse(value) {
                filters.perspective = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--gradfun" {
            let value = args.next().ok_or("missing gradfun options")?;
            if filters.gradfun.is_some() {
                return Err("duplicate gradfun".into());
            }
            if let Ok(filter) = fvid_media::owned_gradfun::GradFun::parse(value) {
                filters.gradfun = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--lenscorrection" {
            let value = args.next().ok_or("missing lenscorrection options")?;
            if filters.lenscorrection.is_some() {
                return Err("duplicate lenscorrection".into());
            }
            if let Ok(filter) = fvid_media::owned_lenscorrection::LensCorrection::parse(value) {
                filters.lenscorrection = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--drawbox" {
            let value = args.next().ok_or("missing drawbox options")?;
            if filters.drawbox.is_some() {
                return Err("duplicate drawbox".into());
            }
            if let Ok(filter) = fvid_media::owned_draw::Draw::box_filter(value) {
                filters.drawbox = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--drawgrid" {
            let value = args.next().ok_or("missing drawgrid options")?;
            if filters.drawgrid.is_some() {
                return Err("duplicate drawgrid".into());
            }
            if let Ok(filter) = fvid_media::owned_draw::Draw::grid_filter(value) {
                filters.drawgrid = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--removegrain" {
            let value = args.next().ok_or("missing removegrain options")?;
            if filters.removegrain.is_some() {
                return Err("duplicate removegrain".into());
            }
            if let Ok(filter) = fvid_media::owned_removegrain::RemoveGrain::parse(value) {
                filters.removegrain = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--yaepblur" {
            let value = args.next().ok_or("missing yaepblur options")?;
            if filters.yaepblur.is_some() {
                return Err("duplicate yaepblur".into());
            }
            if let Ok(filter) = fvid_media::owned_yaepblur::YaepBlur::parse(value) {
                filters.yaepblur = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--smartblur" {
            let value = args.next().ok_or("missing smartblur options")?;
            if filters.smartblur.is_some() {
                return Err("duplicate smartblur".into());
            }
            if let Ok(filter) = fvid_media::owned_smartblur::SmartBlur::parse(value) {
                filters.smartblur = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--vignette" {
            let value = args.next().ok_or("missing vignette options")?;
            if filters.vignette.is_some() {
                return Err("duplicate vignette".into());
            }
            if let Ok(filter) = fvid_media::owned_vignette::Vignette::parse(value) {
                filters.vignette = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--curves" {
            let value = args.next().ok_or("missing curves options")?;
            if filters.curves.is_some() {
                return Err("duplicate curves".into());
            }
            if let Ok(filter) = fvid_media::owned_curves::Curves::parse(value) {
                filters.curves = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--hqdn3d" {
            let value = args.next().ok_or("missing hqdn3d options")?;
            if filters.hqdn3d.is_some() {
                return Err("duplicate hqdn3d".into());
            }
            if let Ok(filter) = fvid_media::owned_hqdn3d::HqDn3d::parse(value) {
                filters.hqdn3d = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--tmix" {
            let value = args.next().ok_or("missing tmix options")?;
            if filters.tmix.is_some() {
                return Err("duplicate tmix".into());
            }
            if let Ok(filter) = fvid_media::owned_tmix::TemporalMix::parse(value) {
                filters.tmix = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--lagfun" {
            let value = args.next().ok_or("missing lagfun options")?;
            if filters.lagfun.is_some() {
                return Err("duplicate lagfun".into());
            }
            if let Ok(filter) = fvid_media::owned_lagfun::LagFun::parse(value) {
                filters.lagfun = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--hue" {
            let value = args.next().ok_or("missing hue args")?;
            if filters.hue.is_some() {
                return Err("duplicate hue".into());
            }
            if let Ok(filter) = fvid_media::owned_hue::HueProgram::parse(value) {
                filters.hue = Some(filter);
            } else {
                result.push(arg.clone());
                result.push(value.clone());
            }
            continue;
        }
        if arg == "--negate" {
            if filters.negate.is_some() {
                return Err("duplicate negate".into());
            }
            filters.negate = Some(Negate::parse(args.next().ok_or("missing negate args")?)?);
            continue;
        }
        if matches!(arg.as_str(), "--dilation" | "--erosion") {
            use fvid::native_morphology::{Morphology, MorphologyKind};
            let kind = if arg == "--dilation" {
                MorphologyKind::Dilation
            } else {
                MorphologyKind::Erosion
            };
            if !seen_morphology.insert(kind) {
                return Err(format!("duplicate {}", kind.name()).into());
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing {} args", kind.name()))?;
            match Morphology::parse(kind, value) {
                Ok(filter) => filters.morphology.push(filter),
                Err(error) => {
                    #[cfg(feature = "media")]
                    {
                        let _ = error;
                        result.push(arg.clone());
                        result.push(value.clone());
                    }
                    #[cfg(not(feature = "media"))]
                    {
                        return Err(error.into());
                    }
                }
            }
            continue;
        }
        let kind = match arg.as_str() {
            "--sobel" => GradientKind::Sobel,
            "--prewitt" => GradientKind::Prewitt,
            "--roberts" => GradientKind::Roberts,
            "--kirsch" => GradientKind::Kirsch,
            "--scharr" => GradientKind::Scharr,
            _ => {
                result.push(arg.clone());
                continue;
            }
        };
        if !seen.insert(kind) {
            return Err(format!("duplicate {}", kind.name()).into());
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing {} args", kind.name()))?;
        match Gradient::parse(kind, value) {
            Ok(filter) => filters.gradients.push(filter),
            Err(error) => {
                // Preserve the existing adapter's expression syntax until it
                // has an owned evaluator. Original args reach the full parser.
                #[cfg(feature = "media")]
                {
                    let _ = error;
                    result.push(arg.clone());
                    result.push(value.clone());
                }
                #[cfg(not(feature = "media"))]
                {
                    return Err(error.into());
                }
            }
        }
    }
    filters.gradients.sort_by_key(|filter| filter.kind());
    filters.morphology.sort_by_key(|filter| filter.kind());
    Ok((result, filters))
}

fn eligible_source(result: &[String], source_hint: Option<&str>) -> CliResult<bool> {
    match result
        .get(1..)
        .and_then(|s| s.iter().find(|v| !v.starts_with('-')))
        .map(String::as_str)
        .or(source_hint)
    {
        Some(path) => Ok(fvid::native_lossless::eligible(Path::new(path))?),
        None => Ok(false),
    }
}

// Strip only the native geometry options. The remaining parser still rejects
// extra paths and returns unknown filters to the existing full media parser.
pub(super) fn geometry_decode_args(
    args: &[String],
) -> CliResult<(Vec<String>, fvid::native_geometry::VideoGeometry)> {
    let mut geometry = fvid::native_geometry::VideoGeometry::default();
    if args.first().map(String::as_str) != Some("decode") {
        return Ok((args.to_vec(), geometry));
    }
    let mut result = vec![args[0].clone()];
    let mut args = args[1..].iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => {
                result.push(arg.clone());
                result.extend(args.cloned());
                break;
            }
            "--hflip" => {
                if geometry.horizontal_flip {
                    return Err("duplicate hflip".into());
                }
                geometry.horizontal_flip = true;
            }
            "--vflip" => {
                if geometry.vertical_flip {
                    return Err("duplicate vflip".into());
                }
                geometry.vertical_flip = true;
            }
            "--rotate" => {
                if geometry.rotate.is_some() {
                    return Err("duplicate rotate".into());
                }
                geometry.rotate = Some(fvid::media_info::RotateAngle::parse(
                    args.next().ok_or("missing rotate degrees")?,
                )?);
            }
            "--transpose" => {
                if geometry.transpose.is_some() {
                    return Err("duplicate transpose".into());
                }
                geometry.transpose = Some(fvid::native_geometry::Transpose::parse(
                    args.next().ok_or("missing transpose mode")?,
                )?);
            }
            "--crop" | "--scale" | "--pad" => {
                let fields = args
                    .next()
                    .ok_or("missing geometry value")?
                    .split(':')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
                if arg == "--pad" {
                    if geometry.pad.is_some() {
                        return Err("duplicate pad".into());
                    }
                    geometry.pad = Some(
                        fields
                            .try_into()
                            .map_err(|_| "pad requires WIDTH:HEIGHT:X:Y")?,
                    );
                } else if arg == "--crop" {
                    if geometry.crop.is_some() {
                        return Err("duplicate crop".into());
                    }
                    geometry.crop = Some(
                        fields
                            .try_into()
                            .map_err(|_| "crop requires X:Y:WIDTH:HEIGHT")?,
                    );
                } else {
                    if geometry.scale.is_some() {
                        return Err("duplicate scale".into());
                    }
                    geometry.scale = Some(
                        fields
                            .try_into()
                            .map_err(|_| "scale requires WIDTH:HEIGHT")?,
                    );
                }
            }
            _ => result.push(arg.clone()),
        }
    }
    Ok((result, geometry))
}

fn framestep_decode_args(
    args: &[String],
) -> CliResult<(Vec<String>, fvid_media::owned_framestep::FrameStep)> {
    use fvid_media::owned_framestep::FrameStep;
    let mut step = None;
    if args.first().map(String::as_str) != Some("decode") {
        return Ok((args.to_vec(), FrameStep::parse("")?));
    }
    let mut output = vec![args[0].clone()];
    let mut items = args[1..].iter();
    while let Some(option) = items.next() {
        if option == "--" {
            output.push(option.clone());
            output.extend(items.cloned());
            break;
        }
        if option == "--framestep" {
            if step.is_some() {
                return Err("duplicate framestep option".into());
            }
            step = Some(FrameStep::parse(
                items.next().ok_or("missing framestep options")?,
            )?);
        } else {
            output.push(option.clone());
        }
    }
    Ok((output, step.unwrap_or(FrameStep::parse("")?)))
}

pub(super) fn try_export_y4m(args: &[String]) -> CliResult<bool> {
    let is_direct = args.first().map(String::as_str) == Some("export-y4m");
    let is_plan = args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("export-y4m");
    if !is_direct && !is_plan {
        return Ok(false);
    }
    let planning = is_plan;
    let cmd_idx = usize::from(planning);
    let usage = if planning {
        "usage: fvid media plan export-y4m INPUT [OUTPUT.y4m] [--crop X:Y:W:H] [--hflip] [--vflip] [--transpose MODE] [--rotate DEGREES] [--pad W:H:X:Y] [--scale W:H] [--from SECONDS --to SECONDS] [--streams INDEX]"
    } else {
        "usage: fvid media export-y4m INPUT OUTPUT.y4m [--crop X:Y:W:H] [--hflip] [--vflip] [--transpose MODE] [--rotate DEGREES] [--pad W:H:X:Y] [--scale W:H] [--from SECONDS --to SECONDS] [--streams INDEX]"
    };
    if args.len() < 3 {
        return Err(usage.into());
    }
    let mut geometry_args = vec!["decode".to_owned()];
    geometry_args.extend_from_slice(&args[cmd_idx + 1..]);
    let (geometry_args, mut filters) = pixel_decode_args(&geometry_args)?;
    filters.canonicalize_option_order();
    let (geometry_args, geometry) = geometry_decode_args(&geometry_args)?;
    let (mut from, mut to) = (None, None);
    let mut quiet = false;
    let mut report = false;
    let mut output_format: Option<String> = None;
    let mut copy_options = fvid::media::CopyOptions::default();
    let mut limit_flags = std::collections::BTreeSet::new();
    let mut paths = Vec::<PathBuf>::new();
    let mut options = geometry_args[1..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--" => {
                paths.extend(options.map(PathBuf::from));
                break;
            }
            "--output-format" if planning => {
                if !limit_flags.insert("--output-format") {
                    return Err("duplicate option: --output-format".into());
                }
                output_format = Some(options.next().ok_or("missing output format")?.clone());
            }
            "--from" | "--to" => {
                let slot = if option == "--from" {
                    &mut from
                } else {
                    &mut to
                };
                if slot.is_some() {
                    return Err("duplicate export boundary".into());
                }
                *slot = Some(decode_time(
                    options.next().ok_or("missing export boundary")?,
                )?);
            }
            "--streams" => {
                if !limit_flags.insert("--streams") {
                    return Err("duplicate streams option".into());
                }
                copy_options.streams = options
                    .next()
                    .ok_or("missing stream")?
                    .split(',')
                    .map(str::parse::<usize>)
                    .collect::<Result<Vec<_>, _>>()?;
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
            }
            "--metadata-delete" => {
                copy_options
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
                copy_options.stream_metadata_set.push((
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
                copy_options
                    .stream_metadata_delete
                    .push((index.parse()?, key.to_owned()));
            }
            "--quiet" | "-q" => quiet = true,
            "--progress" => {
                if !planning {
                    report = true;
                }
            }
            "--max-packets" | "--max-packet-bytes" | "--max-rss-mib" | "--max-memory-mib" => {
                if !limit_flags.insert(option.as_str()) {
                    return Err("duplicate export limit".into());
                }
                parse_native_audio_limit(
                    option,
                    options.next().ok_or("missing export limit")?,
                    &mut copy_options,
                )?;
            }
            _ if option.starts_with('-') => {
                return Err(format!("unknown export option: {option}").into());
            }
            _ => paths.push(PathBuf::from(option)),
        }
    }
    let interval = match (from, to) {
        (None, None) => None,
        (Some(from), Some(to)) if from < to => Some((from, to)),
        _ => return Err("export interval requires both --from and --to with from < to".into()),
    };
    if planning {
        if paths.is_empty() || paths.len() > 2 {
            return Err(usage.into());
        }
        if let Some(dest) = paths.get(1)
            && dest.extension().and_then(|s| s.to_str()) != Some("y4m")
        {
            return Err("native video export output requires .y4m".into());
        }
        if let Some(fmt) = output_format.as_deref()
            && !fmt.eq_ignore_ascii_case("y4m")
        {
            return Err("native video export output requires .y4m".into());
        }
        let plan = fvid::native_plan::export_y4m_with_options(
            &paths[0],
            interval,
            &geometry,
            &filters,
            &copy_options,
        )?;
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&plan)?);
        }
        return Ok(true);
    }
    if paths.len() != 2 {
        return Err(usage.into());
    }
    copy_options.progress = make_progress_hook(report);
    let event = fvid::native_export::export_y4m_pipeline_with_options(
        &paths[0],
        &paths[1],
        interval,
        &geometry,
        &filters,
        &copy_options,
    )?;
    if !quiet {
        println!("{{\"backend\":\"fvid\",\"video_frames\":{}}}", event.packets);
    }
    Ok(true)
}

pub(super) fn try_plain_video_decode(args: &[String]) -> CliResult<bool> {
    let (step_args, step) = framestep_decode_args(args)?;
    let (pixel_args, filters) = pixel_decode_args(&step_args)?;
    let (decode_args, geometry) = geometry_decode_args(&pixel_args)?;
    let Some((path, quiet, interval)) = plain_decode(&decode_args)? else {
        return Ok(false);
    };
    let stats = fvid::native_media::decode_video_pipeline_overlay_step(
        Path::new(path),
        interval,
        &geometry,
        &filters,
        None,
        step,
    )?;
    if !quiet {
        // These strings are internal backend/pixel-format names; paths and
        // other user input are never interpolated into this JSON document.
        println!(
            "{{\"backend\":\"{}\",\"video_frames\":{},\"width\":{},\"height\":{},\"pixel_format\":\"{}\",\"decode_errors\":{}}}",
            stats.backend,
            stats.video_frames,
            stats.width,
            stats.height,
            stats.pixel_format,
            stats.decode_errors
        );
    }
    Ok(true)
}

pub(super) fn try_owned_decode_command(args: &[String]) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("decode") {
        return Ok(false);
    }
    let mut quiet = false;
    let mut positional = false;
    let mut source = None;
    let (mut from, mut to) = (None, None);
    let mut transform = fvid::media_info::DecodeTransform::default();
    let mut overlay_path = None;
    let (mut overlay_x, mut overlay_y) = (0i32, 0i32);
    let mut seen = std::collections::BTreeSet::new();
    let mut options = args[1..].iter();
    while let Some(option) = options.next() {
        if !positional && option == "--" {
            positional = true;
            continue;
        }
        if positional || !option.starts_with('-') {
            if source.replace(PathBuf::from(option)).is_some() {
                #[cfg(not(feature = "media"))]
                return Err("decode requires a single INPUT".into());
                #[cfg(feature = "media")]
                return Ok(false);
            }
            continue;
        }
        let key_option = if option == "-q" { "--quiet" } else { option.as_str() };
        if !seen.insert(key_option) {
            return Err(format!("duplicate option: {option}").into());
        }
        match option.as_str() {
            "--quiet" | "-q" => {
                quiet = true;
                continue;
            }
            "--input-format" => {
                transform.input_format =
                    Some(options.next().ok_or("missing input format name")?.clone());
                continue;
            }
            "--overlay" => {
                overlay_path = Some(PathBuf::from(
                    options.next().ok_or("missing overlay path")?,
                ));
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
                    options.next().ok_or("missing decode interval boundary")?,
                )?;
                if option == "--from" {
                    from = Some(value);
                } else {
                    to = Some(value);
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
                transform.rotate = Some(fvid::media_info::RotateAngle::parse(
                    options.next().ok_or("missing rotate degrees")?,
                )?);
                continue;
            }
            "--transpose" => {
                transform.transpose = Some(fvid::media_info::TransposeMode::parse(
                    options.next().ok_or("missing transpose mode")?,
                )?);
                continue;
            }
            "--crop" | "--scale" | "--pad" => {
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
                return Err(format!("unsupported decode option: {option}").into());
                #[cfg(feature = "media")]
                return Ok(false);
            }
        };
        *slot = Some(options.next().ok_or("missing filter parameters")?.clone());
    }
    let Some(source) = source else {
        #[cfg(not(feature = "media"))]
        return Err("decode requires a single INPUT".into());
        #[cfg(feature = "media")]
        return Ok(false);
    };
    transform.overlay = overlay_path.map(|path| fvid::media_info::OverlaySpec {
        path,
        x: overlay_x,
        y: overlay_y,
    });
    transform.interval = match (from, to) {
        (None, None) => None,
        (Some(from), Some(to)) if from >= 0 && from < to => Some((from, to)),
        _ => return Err("decode interval requires both --from and --to with from < to".into()),
    };
    let stats = fvid::media::decode_video_transformed(&source, transform)?;
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }
    Ok(true)
}
