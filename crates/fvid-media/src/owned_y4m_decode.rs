//! Owned raw planar Y4M decode-and-discard, with bounded scratch storage.
use crate::owned_y4m::{Header, PixelFormat, line};
use fvid_media_info::{CropRect, DecodeStats, DecodeTransform, PadRect, ScaleSize, TransposeMode};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
};
type Result<T> = std::result::Result<T, String>;

/// Recognize only the progressive planar grammar owned by FVid. Legacy callers
/// retain their existing route for unsupported interlace/chroma configurations.
pub(crate) fn supports(source: &Path) -> bool {
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    line(&mut reader, &mut bytes).is_ok_and(|present| present) && Header::parse(&bytes).is_ok()
}

/// Consume every raw sample byte rather than treating a metadata-only seek as
/// decoding. No RGB conversion, external codec, or frame-sized allocation.
pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_reader(BufReader::new(
        File::open(source).map_err(|e| e.to_string())?,
    ))
}
pub fn decode_reader(source: impl BufRead) -> Result<DecodeStats> {
    decode_reader_transformed(source, &Default::default())
}
pub(crate) fn supported_request(transform: &DecodeTransform) -> bool {
    if transform.curves.as_deref().is_some_and(|a|crate::owned_curves::Curves::parse(a).is_err()) {return false;}
    if transform.vignette.as_deref().is_some_and(|a|crate::owned_vignette::Vignette::parse(a).is_err()) {return false;}
    if transform.smartblur.as_deref().is_some_and(|a|crate::owned_smartblur::SmartBlur::parse(a).is_err()) {return false;}
    if transform.sab.as_deref().is_some_and(|a|crate::owned_sab::Sab::parse(a).is_err()) {return false;}
    if transform.bitplanenoise.as_deref().is_some_and(|a|crate::owned_bitplanenoise::BitPlaneNoise::parse(a).is_err()) {return false;}
    if transform.gradfun.as_deref().is_some_and(|a|crate::owned_gradfun::GradFun::parse(a).is_err()) {return false;}
    if transform.hqdn3d.as_deref().is_some_and(|a|crate::owned_hqdn3d::HqDn3d::parse(a).is_err()) {return false;}
    if transform.tmix.as_deref().is_some_and(|a|crate::owned_tmix::TemporalMix::parse(a).is_err()) {return false;}
    if transform.lagfun.as_deref().is_some_and(|a|crate::owned_lagfun::LagFun::parse(a).is_err()) {return false;}
    if transform.fade.as_deref().is_some_and(|a|crate::owned_fade::Fade::parse(a).is_err()) {return false;}
    if transform.grayworld.as_deref().is_some_and(|a| crate::owned_timeline::Timeline::grayworld(a).is_err()) { return false; }
    if transform.cas.as_deref().is_some_and(|a| crate::owned_cas::Cas::parse(a).is_err()) { return false; }
    if transform.colorcorrect.as_deref().is_some_and(|a| crate::owned_colorcorrect::ColorCorrect::parse(a).is_err()) { return false; }
    if transform.colorbalance.as_deref().is_some_and(|a| crate::owned_colorbalance::ColorBalance::parse(a).is_err()) { return false; }
    if transform.exposure.as_deref().is_some_and(|a| crate::owned_exposure::Exposure::parse(a).is_err()) { return false; }
    if transform.colorchannelmixer.as_deref().is_some_and(|a| crate::owned_colorchannelmixer::ColorChannelMixer::parse(a).is_err()) { return false; }
    if transform.colorlevels.as_deref().is_some_and(|a| crate::owned_colorlevels::ColorLevels::parse(a).is_err()) { return false; }
    if transform.vibrance.as_deref().is_some_and(|a| crate::owned_vibrance::Vibrance::parse(a).is_err()) { return false; }
    if transform.colorcontrast.as_deref().is_some_and(|a| crate::owned_colorcontrast::ColorContrast::parse(a).is_err()) { return false; }
    if transform.colorhold.as_deref().is_some_and(|a| crate::owned_colorhold::ColorHold::parse(a).is_err()) { return false; }
    if transform.lutyuv.as_deref().is_some_and(|a| crate::owned_lutyuv::LutYuv::parse(a).is_err()) { return false; }
    if transform.monochrome.as_deref().is_some_and(|a| crate::owned_monochrome::Monochrome::parse(a).is_err()) { return false; }
    if transform.colorize.as_deref().is_some_and(|a| crate::owned_colorize::Colorize::parse(a).is_err()) { return false; }
    if transform.bilateral.as_deref().is_some_and(|a| crate::owned_bilateral::Bilateral::parse(a).is_err()) { return false; }
    if transform.rotate.is_some_and(|a| fvid_media_info::RotateAngle::parse(&a.degrees.to_string()).is_err()) { return false; }
    if transform.gblur.as_deref().is_some_and(|a| crate::owned_gblur::GaussianBlur::parse(a).is_err()) { return false; }
    transform
        .unsharp
        .as_deref()
        .is_none_or(|args| crate::owned_unsharp::Unsharp::parse(args).is_ok())
        && transform
            .eq
            .as_deref()
            .is_none_or(|args| crate::owned_eq::EqualizerProgram::parse(args).is_ok())
        && transform
            .hue
            .as_deref()
            .is_none_or(|args| crate::owned_hue::HueProgram::parse(args).is_ok())
        && transform.reverse.as_deref().is_none_or(str::is_empty)
        && transform
            .shuffleframes
            .as_deref()
            .is_none_or(|args| crate::owned_shuffleframes::ShuffleFrames::parse(args).is_ok())
        && transform
            .framestep
            .as_deref()
            .is_none_or(|args| crate::owned_framestep::FrameStep::parse(args).is_ok())
        && morphology(transform).into_iter().all(|(kind, args)| {
            args.as_deref()
                .is_none_or(|a| crate::owned_morphology::Morphology::parse(kind, a).is_ok())
        })
        && gradients(transform).into_iter().all(|(kind, args)| {
            args.as_deref()
                .is_none_or(|a| crate::owned_gradient::Gradient::parse(kind, a).is_ok())
        })
        && transform
            .shuffleplanes
            .as_deref()
            .is_none_or(|a| crate::owned_shuffleplanes::ShufflePlanes::parse(a).is_ok())
        && transform
            .pixelize
            .as_deref()
            .is_none_or(|a| crate::owned_pixelize::Pixelize::parse(a).is_ok())
        && transform
            .chromashift
            .as_deref()
            .is_none_or(|a| crate::owned_chromashift::ChromaShift::parse(a).is_ok())
        && transform
            .avgblur
            .as_deref()
            .is_none_or(|args| crate::owned_avgblur::AverageBlur::parse(args).is_ok())
        && transform
            .boxblur
            .as_deref()
            .is_none_or(|args| crate::owned_boxblur::BoxBlurProgram::parse(args).is_ok())
        && transform
            .negate
            .as_deref()
            .is_none_or(|args| crate::owned_negate::Negate::parse(args).is_ok())
        && transform
            .input_format
            .as_deref()
            .is_none_or(|format| matches!(format, "y4m" | "yuv4mpegpipe"))
        && *transform
            == DecodeTransform {
                crop: transform.crop,
                scale: transform.scale,
                transpose: transform.transpose,
                rotate: transform.rotate,
                pad: transform.pad,
                unsharp: transform.unsharp.clone(),
                eq: transform.eq.clone(),
                hue: transform.hue.clone(),
                colorize: transform.colorize.clone(),
                monochrome: transform.monochrome.clone(),
                lutyuv: transform.lutyuv.clone(),
                colorhold: transform.colorhold.clone(),
                colorcontrast: transform.colorcontrast.clone(),
                vibrance: transform.vibrance.clone(),
                colorlevels: transform.colorlevels.clone(),
                colorchannelmixer: transform.colorchannelmixer.clone(),
                fade: transform.fade.clone(),
                lagfun: transform.lagfun.clone(),
                tmix: transform.tmix.clone(),
                hqdn3d: transform.hqdn3d.clone(),
                exposure: transform.exposure.clone(),
                colorbalance: transform.colorbalance.clone(),
                curves: transform.curves.clone(),
                vignette: transform.vignette.clone(),
                smartblur: transform.smartblur.clone(),
                sab: transform.sab.clone(),
                bitplanenoise: transform.bitplanenoise.clone(),
                gradfun: transform.gradfun.clone(),
                colorcorrect: transform.colorcorrect.clone(),
                cas: transform.cas.clone(),
                grayworld: transform.grayworld.clone(),
                negate: transform.negate.clone(),
                avgblur: transform.avgblur.clone(),
                gblur: transform.gblur.clone(),
                bilateral: transform.bilateral.clone(),
                boxblur: transform.boxblur.clone(),
                pixelize: transform.pixelize.clone(),
                chromashift: transform.chromashift.clone(),
                shuffleplanes: transform.shuffleplanes.clone(),
                sobel: transform.sobel.clone(),
                prewitt: transform.prewitt.clone(),
                roberts: transform.roberts.clone(),
                kirsch: transform.kirsch.clone(),
                scharr: transform.scharr.clone(),
                dilation: transform.dilation.clone(),
                erosion: transform.erosion.clone(),
                horizontal_flip: transform.horizontal_flip,
                vertical_flip: transform.vertical_flip,
                interval: transform.interval,
                overlay: transform.overlay.clone(),
                input_format: transform.input_format.clone(),
                framestep: transform.framestep.clone(),
                shuffleframes: transform.shuffleframes.clone(),
                reverse: transform.reverse.clone(),
                ..Default::default()
            }
}
pub(crate) fn supports_transformed(source: &Path, transform: &DecodeTransform) -> bool {
    supported_request(transform)
        && overlay_supported(source, transform)
        && supports(source)
        && framestep_clock_supported(source, transform)
        && (transform.transpose.is_none() || header_format(source) != Some(PixelFormat::Yuv411))
}
fn framestep_clock_supported(source: &Path, transform: &DecodeTransform) -> bool {
    let Some(args) = transform.framestep.as_deref() else {
        return true;
    };
    let qualify = || -> Result<()> {
        let mut input = BufReader::new(File::open(source).map_err(|e| e.to_string())?);
        let mut bytes = Vec::new();
        line(&mut input, &mut bytes)?;
        let rate = Header::parse(&bytes)?.frame_rate()?;
        crate::owned_framestep::FrameStep::parse(args)?.frame_rate(rate)?;
        Ok(())
    };
    qualify().is_ok()
}
fn overlay_supported(source: &Path, transform: &DecodeTransform) -> bool {
    let Some(spec) = &transform.overlay else {
        return true;
    };
    if transform.shuffleplanes.is_some() {
        return false;
    }
    let Ok(file) = File::open(source) else {
        return false;
    };
    let mut reader = BufReader::new(file);
    let mut bytes = Vec::new();
    if !line(&mut reader, &mut bytes).is_ok_and(|present| present) {
        return false;
    }
    let Ok(header) = Header::parse(&bytes) else {
        return false;
    };
    crate::owned_y4m_overlay::OverlayReader::open(&header, spec).is_ok()
}
fn header_format(source: &Path) -> Option<PixelFormat> {
    let mut r = BufReader::new(File::open(source).ok()?);
    let mut b = Vec::new();
    line(&mut r, &mut b).ok()?;
    Some(Header::parse(&b).ok()?.format)
}
pub fn decode_video_transformed(source: &Path, transform: DecodeTransform) -> Result<DecodeStats> {
    decode_reader_transformed(
        BufReader::new(File::open(source).map_err(|e| e.to_string())?),
        &transform,
    )
}
pub(crate) fn crop_geometry(header: &Header, crop: Option<CropRect>) -> Result<(CropRect, usize)> {
    let crop = crop.unwrap_or(CropRect {
        x: 0,
        y: 0,
        width: header.width,
        height: header.height,
    });
    let (sx, sy) = header.format.subsampling();
    if crop.width == 0
        || crop.height == 0
        || (crop.width % sx != 0 && crop.x.checked_add(crop.width) != Some(header.width))
        || (crop.height % sy != 0 && crop.y.checked_add(crop.height) != Some(header.height))
        || crop.x % sx != 0
        || crop.y % sy != 0
        || crop
            .x
            .checked_add(crop.width)
            .is_none_or(|end| end > header.width)
        || crop
            .y
            .checked_add(crop.height)
            .is_none_or(|end| end > header.height)
    {
        return Err("invalid or unaligned Y4M crop".into());
    }
    let area = crop
        .width
        .checked_mul(crop.height)
        .ok_or("Y4M crop size overflow")?;
    let size = area
        .checked_add(
            crop.width.div_ceil(sx)
                .checked_mul(crop.height.div_ceil(sy))
                .and_then(|n| n.checked_mul(2))
                .ok_or("Y4M crop size overflow")?,
        )
        .and_then(|n| n.checked_mul(if header.depth() == 8 { 1 } else { 2 }))
        .ok_or("Y4M crop size overflow")?;
    Ok((crop, size))
}
pub(crate) fn output_geometry(
    header: &Header,
    crop: CropRect,
    scale: Option<ScaleSize>,
    transpose: Option<TransposeMode>,
    pad: Option<PadRect>,
) -> Result<(usize, usize, usize)> {
    if transpose.is_some() && header.format == PixelFormat::Yuv411 {
        return Err("owned Y4M transpose does not yet implement 4:1:1 output conversion".into());
    }
    let (cw, ch) = if transpose.is_some() {
        (crop.height, crop.width)
    } else {
        (crop.width, crop.height)
    };
    if let Some(p) = pad {
        p.validate(
            u32::try_from(cw).map_err(|_| "Y4M pad input width overflow")?,
            u32::try_from(ch).map_err(|_| "Y4M pad input height overflow")?,
        )?;
    }
    let (cw, ch) = pad.map_or((cw, ch), |p| (p.width as usize, p.height as usize));
    let (w, h) = scale.map_or((cw, ch), |s| (s.width as usize, s.height as usize));
    if scale.is_some() && (w == 0 || h == 0 || w > 8192 || h > 4320 || w % 2 != 0 || h % 2 != 0) {
        return Err("scale must be even and within 1..=8192 x 1..=4320".into());
    }
    let (sx, sy) = header.format.subsampling();
    let (ox, oy) = if transpose.is_some() {
        (sy, sx)
    } else {
        (sx, sy)
    };
    let area = w.checked_mul(h).ok_or("Y4M scale size overflow")?;
    let size = area
        .checked_add(
            w.div_ceil(ox)
                .checked_mul(h.div_ceil(oy))
                .and_then(|n| n.checked_mul(2))
                .ok_or("Y4M scale size overflow")?,
        )
        .and_then(|n| n.checked_mul(if header.depth() == 8 { 1 } else { 2 }))
        .ok_or("Y4M scale size overflow")?;
    Ok((w, h, size))
}
pub(crate) fn requested_geometry(
    header: &Header,
    transform: &DecodeTransform,
) -> Result<(usize, usize, usize)> {
    let (crop, _) = crop_geometry(header, transform.crop)?;
    if let Some(angle) = transform.rotate {
        let (w, h, _) = output_geometry(header, crop, None, transform.transpose, None)?;
        let (w, h) = angle.size(
            u32::try_from(w).map_err(|_| "rotation width overflow")?,
            u32::try_from(h).map_err(|_| "rotation height overflow")?,
        );
        let mut rotated = header.clone();
        rotated.width = w as usize;
        rotated.height = h as usize;
        if transform.transpose.is_some() {
            rotated.format = match rotated.format {
                PixelFormat::Yuv422 => PixelFormat::Yuv440,
                PixelFormat::Yuv440 => PixelFormat::Yuv422,
                format => format,
            };
        }
        output_geometry(
            &rotated,
            CropRect {
                x: 0,
                y: 0,
                width: rotated.width,
                height: rotated.height,
            },
            transform.scale,
            None,
            transform.pad,
        )
    } else {
        output_geometry(
            header,
            crop,
            transform.scale,
            transform.transpose,
            transform.pad,
        )
    }
}
fn point_sample(index: usize, input: usize, output: usize) -> usize {
    let increment = (((input as u128) << 16) + output as u128 / 2) / output as u128;
    (((index as u128 * increment + increment / 2) >> 16) as usize).min(input - 1)
}
/// Transform planar pixels using the same request subset as owned decode.
pub fn transform_frame_requested(
    header: &Header,
    frame: &[u8],
    transform: &DecodeTransform,
) -> Result<Vec<u8>> {
    transform_frame_requested_cached(header, frame, transform, None, 6)
}
pub(crate) fn transform_frame_requested_cached(
    header: &Header,
    frame: &[u8],
    transform: &DecodeTransform,
    lut: Option<&crate::owned_lutyuv::LutYuv>,
    matrix: u8,
) -> Result<Vec<u8>> {
    transform_frame_requested_cached_at(header,frame,transform,lut,matrix,0,None)
}
pub(crate) fn transform_frame_requested_cached_at(
    header:&Header, frame:&[u8], transform:&DecodeTransform,
    lut:Option<&crate::owned_lutyuv::LutYuv>, matrix:u8, n:u64,t:Option<f64>,
)->Result<Vec<u8>> {

    transform_frame_requested_clock(header, frame, transform, lut, matrix, n, t, None, None, None)
}
pub(crate) fn transform_frame_requested_clock(
    header: &Header,
    frame: &[u8],
    transform: &DecodeTransform,
    lut: Option<&crate::owned_lutyuv::LutYuv>,
    matrix: u8,
    n: u64,
    t: Option<f64>,
    fade: Option<&crate::owned_fade::FadeClock>,
    clock: Option<crate::owned_fade::FrameTime>,
    history: Option<&crate::owned_pixel_context::PixelContext>,
) -> Result<Vec<u8>> {
    if !supported_request(transform) {
        return Err("owned Y4M decoder does not yet implement requested transform options".into());
    }
    if transform.overlay.is_some() {
        return Err("scheduled overlay requires the streaming frame API".into());
    }
    let mut output = transform_frame_geometry_requested(header, frame, transform)?;
    apply_pixel_filters_clock(header, transform, &mut output, lut, matrix,n,t,
        fade,
        clock,
        history,
    )?;
    Ok(output)
}
pub(crate) fn transform_frame_geometry_requested(
    header: &Header,
    frame: &[u8],
    transform: &DecodeTransform,
) -> Result<Vec<u8>> {
    if let Some(angle) = transform.rotate {
        let before = DecodeTransform {
            rotate: None,
            pad: None,
            scale: None,
            ..transform.clone()
        };
        let pixels = transform_frame_geometry_requested(header, frame, &before)?;
        let (w, h, _) = requested_geometry(header, &before)?;
        let mut intermediate = header.clone();
        intermediate.width = w;
        intermediate.height = h;
        if transform.transpose.is_some() {
            intermediate.format = match intermediate.format {
                PixelFormat::Yuv422 => PixelFormat::Yuv440,
                PixelFormat::Yuv440 => PixelFormat::Yuv422,
                format => format,
            };
        }
        let (sx, sy) = intermediate.format.subsampling();
        let rotated = crate::owned_rotate::rotate(
            &crate::owned_frame::GeometryFrame {
                width: w,
                height: h,
                subsampling: Some([sx, sy]),
                data: pixels,
            },
            angle.degrees,
            header.depth(),
            header.full_range()?,
        )?;
        intermediate.width = rotated.width;
        intermediate.height = rotated.height;
        return transform_frame_geometry_requested(
            &intermediate,
            &rotated.data,
            &DecodeTransform {
                pad: transform.pad,
                scale: transform.scale,
                ..Default::default()
            },
        );
    }
    let mut output = Vec::new();
    transform_frame_into(
        header,
        frame,
        transform.crop,
        transform.horizontal_flip,
        transform.vertical_flip,
        transform.scale,
        transform.transpose,
        transform.pad,
        &mut output,
    )?;
    Ok(output)
}
fn gradients(t: &DecodeTransform) -> [(crate::owned_gradient::GradientKind, &Option<String>); 5] {
    use crate::owned_gradient::GradientKind::*;
    [
        (Sobel, &t.sobel),
        (Prewitt, &t.prewitt),
        (Roberts, &t.roberts),
        (Kirsch, &t.kirsch),
        (Scharr, &t.scharr),
    ]
}
fn morphology(
    t: &DecodeTransform,
) -> [(crate::owned_morphology::MorphologyKind, &Option<String>); 2] {
    use crate::owned_morphology::MorphologyKind::*;
    [(Dilation, &t.dilation), (Erosion, &t.erosion)]
}
pub(crate) fn apply_pixel_filters(
    header: &Header,
    transform: &DecodeTransform,
    output: &mut Vec<u8>,
) -> Result<()> {
    apply_pixel_filters_cached(header, transform, output, None, 6)
}
pub(crate) fn apply_pixel_filters_cached(
    header: &Header,
    transform: &DecodeTransform,
    output: &mut Vec<u8>,
    lut: Option<&crate::owned_lutyuv::LutYuv>,
    matrix: u8,
) -> Result<()> {
    apply_pixel_filters_cached_at(header, transform, output, lut, matrix, 0, None)
}
pub(crate) fn apply_pixel_filters_cached_at(
    header: &Header, transform: &DecodeTransform, output: &mut Vec<u8>,
    lut: Option<&crate::owned_lutyuv::LutYuv>, matrix: u8, n:u64, t:Option<f64>,
) -> Result<()> {
    apply_pixel_filters_clock(header, transform, output, lut, matrix, n, t, None, None, None)
}
pub(crate) fn apply_pixel_filters_clock(
    header: &Header,
    transform: &DecodeTransform,
    output: &mut Vec<u8>,
    lut: Option<&crate::owned_lutyuv::LutYuv>,
    matrix: u8,
    n: u64,
    t: Option<f64>,
    fade: Option<&crate::owned_fade::FadeClock>,
    clock: Option<crate::owned_fade::FrameTime>,
    history: Option<&crate::owned_pixel_context::PixelContext>,
) -> Result<()> {
    if transform.unsharp.is_some()
        || transform.unsharp.is_some()
        || transform.eq.is_some()
        || transform.hue.is_some()
        || transform.colorize.is_some()
        || transform.monochrome.is_some()
        || transform.lutyuv.is_some()
        || transform.colorhold.is_some()
        || transform.colorcontrast.is_some()
        || transform.vibrance.is_some()
        || transform.colorlevels.is_some()
        || transform.colorchannelmixer.is_some()
        || transform.fade.is_some()
        || transform.lagfun.is_some()
        || transform.tmix.is_some()
        || transform.hqdn3d.is_some()
        || transform.exposure.is_some()
        || transform.colorbalance.is_some()
        || transform.curves.is_some()
        || transform.vignette.is_some()
        || transform.smartblur.is_some()
        || transform.sab.is_some()
        || transform.bitplanenoise.is_some()
        || transform.gradfun.is_some()
        || transform.colorcorrect.is_some()
        || transform.cas.is_some()
        || transform.grayworld.is_some()
        || transform.gblur.is_some()
        || transform.bilateral.is_some()
        || transform.avgblur.is_some()
        || transform.boxblur.is_some()
        || transform.pixelize.is_some()
        || transform.chromashift.is_some()
        || transform.shuffleplanes.is_some()
        || gradients(transform).iter().any(|(_, a)| a.is_some())
        || morphology(transform).iter().any(|(_, a)| a.is_some())
    {
        let (width, height, _) = requested_geometry(header, transform)?;
        let (sx, sy) = header.format.subsampling();
        let subsampling = if transform.transpose.is_some() {
            [sy, sx]
        } else {
            [sx, sy]
        };
        let mut frame = crate::owned_frame::GeometryFrame {
            width,
            height,
            subsampling: Some(subsampling),
            data: std::mem::take(output),
        };
        let result: Result<()> = (|| {
            if transform.tmix.is_some() {
                history.and_then(|h|h.tmix.as_ref()).ok_or("tmix requires persistent streaming history")?.apply(&mut frame,header.depth(),n)?;
            }
            if transform.hqdn3d.is_some() {
                history.and_then(|h|h.hqdn3d.as_ref()).ok_or("hqdn3d requires persistent streaming history")?.apply(&mut frame,header.depth(),n,t)?;
            }
            if let Some(args) = transform.eq.as_deref() {
                if let Some(eq)=history.and_then(|context|context.eq.as_ref()) {eq.apply(&mut frame,header.depth(),n,t)?;}
                else {crate::owned_eq::EqualizerProgram::parse(args)?.apply(&mut frame,header.depth(),n,t)?;}
            }
            if let Some(args) = transform.unsharp.as_deref() {
                crate::owned_unsharp::Unsharp::parse(args)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.hue.as_deref() {
                crate::owned_hue::HueProgram::parse(args)?.at(n,t)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.gblur.as_deref() {
                crate::owned_gblur::GaussianBlur::parse(args)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.avgblur.as_deref() {
                crate::owned_avgblur::AverageBlur::parse(args)?
                    .apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.boxblur.as_deref() {
                if let Some(filter) = history.and_then(|context| context.boxblur.as_ref()) {
                    filter.apply(&mut frame, header.depth())?;
                } else {
                    crate::owned_boxblur::BoxBlurProgram::parse(args)?.apply(&mut frame, header.depth())?;
                }
            }
            if let Some(args) = transform.negate.as_deref() {
                crate::owned_negate::Negate::parse(args)?.apply(&mut frame.data, header.depth())?;
            }
            for (kind, args) in gradients(transform) {
                if let Some(args) = args.as_deref() {
                    crate::owned_gradient::Gradient::parse(kind, args)?
                        .apply(&mut frame, header.depth())?;
                }
            }
            if let Some(args) = transform.smartblur.as_deref() {
                if let Some(filter)=history.and_then(|h|h.smartblur.as_ref()) {filter.apply(&mut frame,header.depth(),n,t)?;}
                else {crate::owned_smartblur::SmartBlur::parse(args)?.apply(&mut frame,header.depth(),n,t)?;}
            }
            if let Some(args) = transform.sab.as_deref() {
                if let Some(filter)=history.and_then(|h|h.sab.as_ref()) {filter.apply(&mut frame,header.depth(),n,t)?;}
                else {crate::owned_sab::Sab::parse(args)?.apply(&mut frame,header.depth(),n,t)?;}
            }
            if let Some(args) = transform.bilateral.as_deref() {
                crate::owned_bilateral::Bilateral::parse(args)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.cas.as_deref() {
                crate::owned_cas::Cas::parse(args)?.apply(&mut frame,header.depth())?;
            }
            if transform.vignette.is_some() {
                if let Some(filter)=history.and_then(|h|h.vignette.as_ref()) {let rate=if header.tokens.iter().any(|t|t.starts_with('F')) {Some(header.frame_rate()?)} else {None};
                    let aspect=header.pixel_aspect()?;
                    let mut sar=aspect.0 as f64/aspect.1 as f64;
                    let crop=transform.crop.unwrap_or(fvid_media_info::CropRect{x:0,y:0,width:header.width,height:header.height});
                    let (mut cw,mut ch)=(u32::try_from(crop.width).map_err(|_|"vignette crop width overflow")?,u32::try_from(crop.height).map_err(|_|"vignette crop height overflow")?);
                    if transform.transpose.is_some() {sar=sar.recip();std::mem::swap(&mut cw,&mut ch);}
                    if let Some(angle)=transform.rotate {(cw,ch)=angle.size(cw,ch);}
                    if let Some(pad)=transform.pad {(cw,ch)=(pad.width,pad.height);}
                    if transform.scale.is_some() {sar*=cw as f64*frame.height as f64/(ch as f64*frame.width as f64);}
                    filter.apply_clock(&mut frame,header.depth(),crate::owned_vignette::Clock {
                        n,t,pts: clock.map(|c|c.ticks as f64/c.quantum as f64),
                        rate:rate.map(|r|r[0] as f64/r[1] as f64),
                        time_base: clock.map(|c|c.quantum as f64/c.scale as f64),
                        sample_aspect:sar,
                    })?;}
                else {return Err("vignette requires persistent streaming context".into());}
            }
            if let Some(args) = transform.curves.as_deref() {
                let apply = |filter:&crate::owned_curves::Curves,frame:&mut crate::owned_frame::GeometryFrame| filter.apply_yuv(frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,n,t);
                if let Some(filter)=history.and_then(|h|h.curves.as_ref()) {apply(filter,&mut frame)?;} else {apply(&crate::owned_curves::Curves::parse(args)?,&mut frame)?;}
            }
            if let Some(args) = transform.colorbalance.as_deref() {
                crate::owned_colorbalance::ColorBalance::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            if let Some(args) = transform.colorlevels.as_deref() {
                crate::owned_colorlevels::ColorLevels::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            if let Some(args) = transform.colorchannelmixer.as_deref() {
                crate::owned_colorchannelmixer::ColorChannelMixer::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            if let Some(args) = transform.monochrome.as_deref() {
                crate::owned_monochrome::Monochrome::parse(args)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.grayworld.as_deref() {
                if crate::owned_timeline::Timeline::grayworld(args)?.enabled(n,t,frame.width,frame.height,
                )? {
                    crate::owned_grayworld::GrayWorld::default().apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                    )?;
                }
            }
            if transform.lagfun.is_some() {
                history.and_then(|h|h.lagfun.as_ref()).ok_or("lagfun requires persistent streaming history")?.apply(&mut frame,header.depth(),n,t)?;
            }
            if let Some(args) = transform.bitplanenoise.as_deref() {
                if let Some(filter)=history.and_then(|h|h.bitplanenoise.as_ref()) {let _=filter.apply(&mut frame,header.depth(),n,t)?;}
                else {let _=crate::owned_bitplanenoise::BitPlaneNoise::parse(args)?.apply(&mut frame,header.depth(),n,t)?;}
            }
            if let Some(args) = transform.gradfun.as_deref() {
                if let Some(filter)=history.and_then(|h|h.gradfun.as_ref()) {filter.apply(&mut frame,header.depth(),n,t)?;}
                else {crate::owned_gradfun::GradFun::parse(args)?.apply(&mut frame,header.depth(),n,t)?;}
            }
            if let Some(args) = transform.pixelize.as_deref() {
                crate::owned_pixelize::Pixelize::parse(args)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.vibrance.as_deref() {
                crate::owned_vibrance::Vibrance::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            for (kind, args) in morphology(transform) {
                if let Some(args) = args.as_deref() {
                    crate::owned_morphology::Morphology::parse(kind, args)?
                        .apply(&mut frame, header.depth())?;
                }
            }
            if let Some(args) = transform.colorize.as_deref() {
                crate::owned_colorize::Colorize::parse(args)?.apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.exposure.as_deref() {
                crate::owned_exposure::Exposure::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            if let Some(args) = transform.chromashift.as_deref() {
                crate::owned_chromashift::ChromaShift::parse(args)?
                    .apply(&mut frame, header.depth())?;
            }
            if let Some(args) = transform.colorcontrast.as_deref() {
                crate::owned_colorcontrast::ColorContrast::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            if let Some(args) = transform.colorcorrect.as_deref() {
                crate::owned_colorcorrect::ColorCorrect::parse(args)?.apply(&mut frame,header.depth())?;
            }
            if let Some(args) = transform.shuffleplanes.as_deref() {
                frame.subsampling = Some(
                    crate::owned_shuffleplanes::ShufflePlanes::parse(args)?.apply_yuv(
                        &mut frame.data,
                        width,
                        height,
                        subsampling,
                        header.depth(),
                    )?,
                );
            }
            if let Some(args) = transform.lutyuv.as_deref() {
                if let Some(lut) = lut {lut.apply(&mut frame, header.depth(), header.full_range()?)?;}
                else {crate::owned_lutyuv::LutYuv::parse(args)?.apply(&mut frame, header.depth(), header.full_range()?,
                    )?;}
            }
            if let Some(args) = transform.colorhold.as_deref() {
                crate::owned_colorhold::ColorHold::parse(args)?.apply_yuv(&mut frame,header.depth(),header.full_range()?,crate::owned_yuv_rgb::Matrix::from_code(matrix)?,
                )?;
            }
            if let Some(args)=transform.fade.as_deref() {
                let filter = if let Some(fade) = fade {
                    fade.at(n, clock)?
                } else {
                    crate::owned_fade::Fade::parse(args)?
                };
                filter.apply_colour(&mut frame,header.depth(),header.full_range()?,matrix,n)?;}
            Ok(())
        })();
        *output = frame.data;
        return result;
    }
    if let Some(args) = transform.negate.as_deref() {
        crate::owned_negate::Negate::parse(args)?.apply(output, header.depth())?;
    }
    Ok(())
}
/// Apply crop and reflections to each plane, keeping multibyte samples intact.
pub fn transform_frame(
    header: &Header,
    frame: &[u8],
    crop: Option<CropRect>,
    horizontal: bool,
    vertical: bool,
) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    transform_frame_into(
        header,
        frame,
        crop,
        horizontal,
        vertical,
        None,
        None,
        None,
        &mut output,
    )?;
    Ok(output)
}
fn transform_frame_into(
    header: &Header,
    frame: &[u8],
    crop: Option<CropRect>,
    horizontal: bool,
    vertical: bool,
    scale: Option<ScaleSize>,
    transpose: Option<TransposeMode>,
    pad: Option<PadRect>,
    output: &mut Vec<u8>,
) -> Result<()> {
    if frame.len() != header.frame_len()? {
        return Err("Y4M frame size mismatch".into());
    }
    let (crop, _) = crop_geometry(header, crop)?;
    let (ow, oh, output_size) = output_geometry(header, crop, scale, transpose, pad)?;
    let (sx, sy) = header.format.subsampling();
    let step = if header.depth() == 8 { 1 } else { 2 };
    output.clear();
    output
        .try_reserve_exact(output_size)
        .map_err(|e| e.to_string())?;
    let mut offset = 0;
    for (plane, (dx, dy)) in [(1, 1), (sx, sy), (sx, sy)].into_iter().enumerate() {
        let stride = header.width.div_ceil(dx) * step;
        if scale.is_some() || transpose.is_some() || pad.is_some() {
            let (odx, ody) = if transpose.is_some() {
                (dy, dx)
            } else {
                (dx, dy)
            };
            let (iw, ih, dw, dh) = (crop.width.div_ceil(dx), crop.height.div_ceil(dy), ow.div_ceil(odx), oh.div_ceil(ody),
            );
            let (tw, th) = if transpose.is_some() {
                (ih, iw)
            } else {
                (iw, ih)
            };
            let (canvas_w, canvas_h, px, py) = pad.map_or((tw, th, 0, 0), |p| {
                (
                    p.width as usize / odx,
                    p.height as usize / ody,
                    p.x as usize / odx,
                    p.y as usize / ody,
                )
            });
            let full = header.tokens.iter().any(|t| t == "XCOLORRANGE=FULL");
            let black = if full {
                if plane == 0 {
                    0
                } else {
                    128u16 << (header.depth() - 8)
                }
            } else {
                let code = if plane == 0 { 16u32 } else { 128 };
                ((code * ((1u32 << header.depth()) - 1) + 127) / 255) as u16
            }
            .to_le_bytes();
            for row in 0..dh {
                let cy = point_sample(row, canvas_h, dh);
                for col in 0..dw {
                    let cx = point_sample(col, canvas_w, dw);
                    if cx < px || cy < py || cx - px >= tw || cy - py >= th {
                        output.extend_from_slice(&black[..step]);
                        continue;
                    }
                    let (tx, ty) = (cx - px, cy - py);
                    let (mut x, mut y) = match transpose {
                        None => (tx, ty),
                        Some(TransposeMode::Clock) => (ty, ih - 1 - tx),
                        Some(TransposeMode::CClock) => (iw - 1 - ty, tx),
                        Some(TransposeMode::ClockFlip) => (iw - 1 - ty, ih - 1 - tx),
                        Some(TransposeMode::CClockFlip) => (ty, tx),
                    };
                    if vertical {
                        y = ih - 1 - y;
                    }
                    if horizontal {
                        x = iw - 1 - x;
                    }
                    let at = offset + (crop.y / dy + y) * stride + (crop.x / dx + x) * step;
                    output.extend_from_slice(&frame[at..at + step]);
                }
            }
            offset += stride * (header.height.div_ceil(dy));
            continue;
        }
        let row_bytes = crop.width.div_ceil(dx) * step;
        for row in 0..crop.height.div_ceil(dy) {
            let row = if vertical {
                crop.height.div_ceil(dy) - 1 - row
            } else {
                row
            };
            let start = offset + (crop.y / dy + row) * stride + crop.x / dx * step;
            let at = output.len();
            output.extend_from_slice(&frame[start..start + row_bytes]);
            if horizontal {
                fvid_cpu::hflip_row(&mut output[at..], crop.width.div_ceil(dx), step);
            }
        }
        offset += stride * (header.height.div_ceil(dy));
    }
    Ok(())
}
pub fn decode_reader_transformed(
    source: impl BufRead,
    transform: &DecodeTransform,
) -> Result<DecodeStats> {
    decode_reader_frames(source, transform, None, None).map(|(stats, _)| stats)
}

/// Deliver transformed planar frames without retaining the video. Timestamps
/// stay on the source timeline, including interval selection. Callback failure
/// stops processing immediately; callers must discard partial exports.
pub fn visit_reader_transformed(
    source: impl BufRead,
    transform: &DecodeTransform,
    mut visit: impl FnMut(&Header, &[u8], u64, u64) -> Result<()>,
) -> Result<DecodeStats> {
    visit_reader_transformed_limited(source, transform, None, &mut visit)
}

pub(crate) fn visit_reader_transformed_limited(
    source: impl BufRead,
    transform: &DecodeTransform,
    max_frames: Option<u64>,
    visit: &mut FrameVisitor<'_>,
) -> Result<DecodeStats> {
    visit_reader_transformed_counted(source, transform, max_frames, visit).map(|(stats, _)| stats)
}

/// Internal accounting retains every completely consumed frame, including
/// interval preroll and frames discarded by temporal selection.
pub(crate) fn visit_reader_transformed_counted(
    source: impl BufRead,
    transform: &DecodeTransform,
    max_frames: Option<u64>,
    visit: &mut FrameVisitor<'_>,
) -> Result<(DecodeStats, u64)> {
    decode_reader_frames(source, transform, Some(visit), max_frames)
}

type FrameVisitor<'a> = dyn FnMut(&Header, &[u8], u64, u64) -> Result<()> + 'a;
fn decode_reader_frames(
    mut source: impl BufRead,
    transform: &DecodeTransform,
    mut visit: Option<&mut FrameVisitor<'_>>,
    max_frames: Option<u64>,
) -> Result<(DecodeStats, u64)> {
    if !supported_request(transform) {
        return Err("owned Y4M decoder does not yet implement requested transform options".into());
    }
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode interval requires 0 <= from < to".into());
    }
    let mut bytes = Vec::new();
    if !line(&mut source, &mut bytes)? {
        return Err("empty Y4M input".into());
    }
    let header = Header::parse(&bytes)?;
    let [rate_n, rate_d] = header.frame_rate()?;
    let (crop, _) = crop_geometry(&header, transform.crop)?;
    let (ow, oh, _) = requested_geometry(&header, transform)?;
    let width = u32::try_from(ow).map_err(|_| "Y4M width exceeds decode API range")?;
    let height = u32::try_from(oh).map_err(|_| "Y4M height exceeds decode API range")?;
    let promote = transform
        .shuffleplanes
        .as_deref()
        .map(crate::owned_shuffleplanes::ShufflePlanes::parse)
        .transpose()?
        .is_some_and(|f| f.mapping[0] != 0 || f.mapping[1] == 0 || f.mapping[2] == 0);
    let mut presented_header = header.clone();
    presented_header.width = ow;
    presented_header.height = oh;
    presented_header.format = match header.format {
        _ if promote => PixelFormat::Yuv444,
        PixelFormat::Yuv422 if transform.transpose.is_some() => PixelFormat::Yuv440,
        PixelFormat::Yuv440 if transform.transpose.is_some() => PixelFormat::Yuv422,
        format => format,
    };
    header.full_range()?;
    let (n, d) = header.pixel_aspect()?;
    let (mut n, mut d) = (u128::from(n), u128::from(d));
    let (mut w, mut h) = (crop.width as u128, crop.height as u128);
    if transform.transpose.is_some() {
        std::mem::swap(&mut n, &mut d);
        std::mem::swap(&mut w, &mut h);
    }
    if let Some(angle) = transform.rotate {
        let (rw, rh) = angle.size(u32::try_from(w).map_err(|_| "rotation width overflow")?, u32::try_from(h).map_err(|_| "rotation height overflow")?,
        );
        (w, h) = (u128::from(rw), u128::from(rh));
    }
    if let Some(pad) = transform.pad {
        w = u128::from(pad.width);
        h = u128::from(pad.height);
    }
    if transform.scale.is_some() {
        n = n
            .checked_mul(w)
            .and_then(|v| v.checked_mul(oh as u128))
            .ok_or("pixel aspect overflow")?;
        d = d
            .checked_mul(h)
            .and_then(|v| v.checked_mul(ow as u128))
            .ok_or("pixel aspect overflow")?;
    }
    let (mut a, mut b) = (n, d);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let aspect = (
        u32::try_from(n / a).map_err(|_| "pixel aspect overflow")?,
        u32::try_from(d / a).map_err(|_| "pixel aspect overflow")?,
    );
    presented_header.tokens.retain(|t| !t.starts_with('A'));
    presented_header
        .tokens
        .push(format!("A{}:{}", aspect.0, aspect.1));
    let mut overlay = transform
        .overlay
        .as_ref()
        .map(|spec| crate::owned_y4m_overlay::OverlayReader::open(&presented_header, spec))
        .transpose()?;
    let step =
        crate::owned_framestep::FrameStep::parse(transform.framestep.as_deref().unwrap_or(""))?;
    if transform.framestep.is_some() {
        let output_rate = step.frame_rate([rate_n, rate_d])?;
        presented_header
            .tokens
            .retain(|token| !token.starts_with('F'));
        presented_header
            .tokens
            .push(format!("F{}:{}", output_rate[0], output_rate[1]));
    }
    let mut shuffle = transform
        .shuffleframes
        .as_deref()
        .map(crate::owned_shuffleframes::ShuffleFrames::parse)
        .transpose()?;
    let mut reverse = if transform.reverse.is_some() && visit.is_some() {
        Some(crate::owned_reverse::Reverse::new()?)
    } else {
        None
    };
    let frame_bytes = header.frame_len()?;
    let geometry = transform.overlay.is_some()
        || transform.crop.is_some()
        || transform.horizontal_flip
        || transform.vertical_flip
        || transform.scale.is_some()
        || transform.transpose.is_some()
        || transform.rotate.is_some()
        || transform.pad.is_some()
        || transform.unsharp.is_some()
        || transform.eq.is_some()
        || transform.hue.is_some()
        || transform.colorize.is_some()
        || transform.monochrome.is_some()
        || transform.lutyuv.is_some()
        || transform.colorhold.is_some()
        || transform.colorcontrast.is_some()
        || transform.vibrance.is_some()
        || transform.colorlevels.is_some()
        || transform.colorchannelmixer.is_some()
        || transform.fade.is_some()
        || transform.lagfun.is_some()
        || transform.tmix.is_some()
        || transform.hqdn3d.is_some()
        || transform.exposure.is_some()
        || transform.colorbalance.is_some()
        || transform.curves.is_some()
        || transform.vignette.is_some()
        || transform.smartblur.is_some()
        || transform.sab.is_some()
        || transform.bitplanenoise.is_some()
        || transform.gradfun.is_some()
        || transform.colorcorrect.is_some()
        || transform.cas.is_some()
        || transform.grayworld.is_some()
        || transform.negate.is_some()
        || transform.gblur.is_some()
        || transform.bilateral.is_some()
        || transform.avgblur.is_some()
        || transform.boxblur.is_some()
        || transform.pixelize.is_some()
        || transform.chromashift.is_some()
        || transform.shuffleplanes.is_some()
        || gradients(transform).iter().any(|(_, a)| a.is_some())
        || morphology(transform).iter().any(|(_, a)| a.is_some());
    let mut input = Vec::new();
    if geometry || visit.is_some() {
        input
            .try_reserve_exact(frame_bytes)
            .map_err(|e| e.to_string())?;
        input.resize(frame_bytes, 0);
    }
    let mut output = Vec::new();
    let mut scratch = [0u8; 8192];
    let mut filtered_frames=0u64;
    let mut index = 0u64;
    let mut frames = 0u64;
    let mut selected_inputs = 0u64;
    let lut = transform.lutyuv.as_deref().map(crate::owned_lutyuv::LutYuv::parse).transpose()?;

    let history = crate::owned_pixel_context::PixelContext::parse(transform)?;
    let fade = transform
        .fade
        .as_deref()
        .map(crate::owned_fade::FadeClock::parse)
        .transpose()?;
    loop {
        let clock = u128::from(index) * rate_d as u128 * 1_000_000;
        if transform
            .interval
            .is_some_and(|(_, to)| clock >= to as u128 * rate_n as u128)
        {
            break;
        }
        if !line(&mut source, &mut bytes)? {
            break;
        }
        if bytes != b"FRAME\n" && !bytes.starts_with(b"FRAME ") {
            return Err("expected Y4M FRAME marker".into());
        }
        let in_interval = transform
            .interval
            .is_none_or(|(from, _)| clock >= from as u128 * rate_n as u128);
        let selected = in_interval && step.emits(selected_inputs);
        if in_interval {
            selected_inputs = selected_inputs
                .checked_add(1)
                .ok_or("Y4M frame count overflow")?;
        }
        if max_frames.is_some_and(|limit| index >= limit) {
            return Err("Y4M input packet count exceeds limit".into());
        }
        let mut remaining = frame_bytes;
        while remaining != 0 {
            let count = remaining.min(scratch.len());
            let buffer = if (geometry || visit.is_some()) && in_interval {
                let at = frame_bytes - remaining;
                &mut input[at..at + count]
            } else {
                &mut scratch[..count]
            };
            source.read_exact(buffer).map_err(|error| {
                if error.kind() == std::io::ErrorKind::UnexpectedEof {
                    "truncated Y4M frame payload".into()
                } else {
                    error.to_string()
                }
            })?;
            remaining -= count;
        }
        if in_interval {
            if geometry {
                if transform.rotate.is_some() {
                    output = transform_frame_geometry_requested(&header, &input, transform)?;
                } else {
                    transform_frame_into(&header, &input, transform.crop,
                        transform.horizontal_flip, transform.vertical_flip, transform.scale,
                        transform.transpose, transform.pad, &mut output,
                    )?;
                }
                if let Some(overlay) = overlay.as_mut() {
                    overlay.apply(&presented_header, &mut output, index)?;
                }
                apply_pixel_filters_clock(&header, transform, &mut output, lut.as_ref(), 6, filtered_frames, Some(index as f64 * rate_d as f64 / rate_n as f64),
                    fade.as_ref(),
                    Some(
                        crate::owned_fade::FrameTime::new(
                            u128::from(index) * rate_d as u128,
                            rate_n as u64,
                        )?
                        .with_quantum(rate_d as u64)?,
                    ),
                    Some(&history),
                )?;
                filtered_frames=filtered_frames.checked_add(1).ok_or("timeline frame count overflow")?;
                std::hint::black_box(&output);
            }
            if selected {
            let (pts, duration) = if visit.is_some() {
                let start = u128::from(index) * rate_d as u128 * 1_000_000_000 / rate_n as u128;
                let end = (u128::from(index) + 1) * rate_d as u128 * 1_000_000_000 / rate_n as u128;
                (
                    u64::try_from(start).map_err(|_| "Y4M timestamp overflow")?,
                    u64::try_from(end - start).map_err(|_| "Y4M duration overflow")?,
                )
            } else {
                (0, 0)
            };
            let pixels = if geometry { &output } else { &input };
            let retain_pixels = visit.is_some();
            let mut emit = |data: &[u8], pts, duration| -> Result<()> {
                if let Some(reverse) = reverse.as_mut() {
                    reverse.push(data, pts, duration)?;
                } else if let Some(callback) = visit.as_deref_mut() {
                    callback(&presented_header, data, pts, duration)?;
                }
                Ok(())
            };
            let emitted = if let Some(shuffle) = shuffle.as_mut() {
                shuffle.push(
                    if retain_pixels { pixels } else { &[] },
                    pts,
                    duration,
                    &mut emit,
                )?
            } else {
                emit(pixels, pts, duration)?;
                1
            };
            frames = frames
                .checked_add(emitted)
                .ok_or("Y4M frame count overflow")?;
            }
        }
        index = index.checked_add(1).ok_or("Y4M frame count overflow")?;
    }
    // Release upstream frame/group storage before the single-buffer replay.
    drop(input);
    drop(output);
    drop(shuffle);
    if let Some(reverse) = reverse.as_mut() {
        reverse.flush(&mut |data, pts, duration| {
            if let Some(callback) = visit.as_deref_mut() {
                callback(&presented_header, data, pts, duration)?;
            }
            Ok(())
        })?;
    }
    let layout = match presented_header.format {
        PixelFormat::Yuv420 => "420",
        PixelFormat::Yuv422 => "422",
        PixelFormat::Yuv444 => "444",
        PixelFormat::Yuv440 => "440",
        PixelFormat::Yuv411 => "411",
        PixelFormat::Yuv410 => "410",
    };
    let pixel_format = if header.depth() == 8 {
        format!("yuv{layout}p")
    } else {
        format!("yuv{layout}p{}le", header.depth())
    };
    Ok((
        DecodeStats {
            backend: if geometry {
                "owned Y4M planar decode"
            } else {
                "owned Y4M raw decode"
            },
            video_frames: frames,
            width,
            height,
            pixel_format,
            decode_errors: 0,
        },
        index,
    ))
}

#[cfg(test)]
mod framestep_tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn selects_frames_and_restarts_count_at_the_selected_interval() {
        let source =
            include_bytes!("../../../tests/fixtures/playback-errors/framestep-six-frames.y4m");
        for (interval, expected) in [
            (None, vec![0, 2, 4]),
            (Some((250_000, 1_500_000)), vec![1, 3, 5]),
        ] {
            let transform = DecodeTransform {
                framestep: Some("step=2".into()),
                interval,
                ..Default::default()
            };
            let mut seen = Vec::new();
            let stats = visit_reader_transformed(
                Cursor::new(source),
                &transform,
                |header, pixels, pts, duration| {
                    let index = pixels[0] - 10;
                    let mut expected_pixels = vec![10 + index; 16];
                    expected_pixels.extend([128; 8]);
                    assert_eq!(pixels, expected_pixels);
                    assert_eq!(header.frame_rate().unwrap(), [2, 1]);
                    assert_eq!(pts, u64::from(index) * 250_000_000);
                    assert_eq!(duration, 250_000_000);
                    seen.push(index);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(seen, expected);
            assert_eq!(stats.video_frames, 3);
        }
    }
    #[test]
    fn stepped_overlay_reaches_owned_ffv1_encoder_on_the_source_clock() {
        let root =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors");
        let source = root.join("framestep-six-frames.y4m");
        let transform = DecodeTransform {
            framestep: Some("2".into()),
            overlay: Some(fvid_media_info::OverlaySpec {
                path: root.join("overlay-secondary-clock.y4m"),
                x: 2,
                y: 2,
            }),
            ..Default::default()
        };
        assert!(supports_transformed(&source, &transform));
        assert_eq!(
            crate::decode_video_transformed(&source, transform.clone())
                .unwrap()
                .video_frames,
            3
        );
        let mut decoded = crate::owned_ffv1_decoder::Decoder::new(4, 4, 1 << 20).unwrap();
        let mut seen = Vec::new();
        let stats = crate::owned_ffv1_encoder::encode_y4m(
            BufReader::new(File::open(source).unwrap()),
            &transform,
            |header, packet, pts, duration| {
                assert_eq!(header.frame_rate().unwrap(), [2, 1]);
                let frame = decoded.decode(packet).unwrap().frame;
                let index = frame.data[0] - 10;
                let (luma, u, v) = if index == 0 {
                    (50, 80, 160)
                } else {
                    (100, 90, 170)
                };
                let mut expected = vec![10 + index; 16];
                for at in [10, 11, 14, 15] {
                    expected[at] = luma;
                }
                expected.extend([128, 128, 128, u, 128, 128, 128, v]);
                assert_eq!(frame.data, expected);
                assert_eq!(
                    (pts, duration),
                    (u64::from(index) * 250_000_000, 250_000_000)
                );
                seen.push(index);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(seen, [0, 2, 4]);
        assert_eq!(stats.video_frames, 3);
    }
}

#[cfg(test)]
mod shuffleframes_tests {
    use super::*;
    use std::io::Cursor;
    #[test]
    fn shuffled_group_pixels_keep_position_timestamps_and_discard_incomplete_tail() {
        let bytes = include_bytes!(
            "../../../tests/fixtures/playback-errors/shuffleframes-seven-frames.y4m"
        );
        for (mapping, interval, expected) in [
            (
                "2 1 0",
                None,
                vec![(2u8, 0u64), (1, 1), (0, 2), (5, 3), (4, 4), (3, 5)],
            ),
            ("mapping=2|-1|2", None, vec![(2, 0), (2, 2), (5, 3), (5, 5)]),
            (
                "2 1 0",
                Some((250_000, 1_500_000)),
                vec![(3, 1), (2, 2), (1, 3)],
            ),
        ] {
            let transform = DecodeTransform {
                shuffleframes: Some(mapping.into()),
                interval,
                ..Default::default()
            };
            let mut seen = Vec::new();
            let stats = visit_reader_transformed(
                Cursor::new(bytes),
                &transform,
                |header, pixels, pts, duration| {
                    let index = pixels[0] - 10;
                    let mut expected = vec![10 + index; 16];
                    expected.extend([128; 8]);
                    assert_eq!(pixels, expected);
                    assert_eq!(header.frame_rate().unwrap(), [4, 1]);
                    assert_eq!(duration, 250_000_000);
                    seen.push((index, pts / 250_000_000));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(stats.video_frames, expected.len() as u64);
            assert_eq!(seen, expected);
        }
    }
}

#[cfg(test)]
mod reverse_tests {
    use super::*;
    #[test]
    fn reversed_payloads_keep_forward_timing_in_full_and_interval_decode() {
        let bytes =
            include_bytes!("../../../tests/fixtures/playback-errors/reverse-six-frames.y4m");
        for (interval, values, positions) in [
            (None, vec![5u8, 4, 3, 2, 1, 0], vec![0u64, 1, 2, 3, 4, 5]),
            (Some((250_000, 1_000_000)), vec![3, 2, 1], vec![1, 2, 3]),
        ] {
            let transform = DecodeTransform {
                reverse: Some(String::new()),
                interval,
                ..Default::default()
            };
            let mut seen = Vec::new();
            let stats = visit_reader_transformed(
                std::io::Cursor::new(bytes),
                &transform,
                |_, pixels, pts, duration| {
                    let value = pixels[0] - 10;
                    let mut expected = vec![value + 10; 16];
                    expected.extend([128; 8]);
                    assert_eq!(pixels, expected);
                    assert_eq!(duration, 250_000_000);
                    seen.push((value, pts / 250_000_000));
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(seen, values.into_iter().zip(positions).collect::<Vec<_>>());
            assert_eq!(stats.video_frames, seen.len() as u64);
        }
    }
}
