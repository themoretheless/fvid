//! Owned MP4 AVC/HEVC to FFV1/Matroska, retaining companion AAC packets.
use crate::{
    Result,
    container::{
        matroska_write::{Encoding, FileMetadata, PacketWriter, TrackSpec},
        mp4::Mp4Reader,
        mp4_matroska,
    },
    invalid,
    playback_native::{NativeReader, RawFrame},
};
use fvid_control::{CancelFlag, ProgressEvent, ProgressHook};
use std::{
    cmp::Reverse,
    collections::BinaryHeap,
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::Path,
};
fn check(cancel: Option<&CancelFlag>) -> Result<()> {
    if cancel.is_some_and(|c| c.is_cancelled()) {
        Err(invalid("media operation cancelled"))
    } else {
        Ok(())
    }
}
/// All tracks must be represented; no unsupported stream may disappear.
pub fn eligible(source: &Path) -> Result<bool> {
    if crate::native_lossless_y4m::eligible(source)? { return Ok(true); }
    let mut input = File::open(source)?;
    let mut prefix = [0; 8];
    if input.read(&mut prefix)? != 8 || !crate::container::mp4::recognizes_prefix(&prefix) {
        return Ok(false);
    }
    let input = Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    Ok(mp4_matroska::eligible(&input)
        && input
            .tracks()
            .iter()
            .filter(|t| t.handler == *b"vide")
            .count()
            == 1)
}
/// Caller owns atomic publication. Progress never reports done before publication.
pub fn write_mp4<W: Write + Seek>(
    source: &Path,
    output: &mut W,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    write_mp4_transformed(
        source,
        output,
        &Default::default(),
        &Default::default(),
        cancel,
        progress,
    )
}

/// Apply owned geometry/pixel operations while retaining every AAC companion.
pub fn write_mp4_transformed<W: Write + Seek>(
    source: &Path,
    output: &mut W,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    cancel: Option<&CancelFlag>,
    progress: Option<&ProgressHook>,
 ) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    write_mp4_processed(source,output,geometry,filters,cancel,progress,None)
}

/// Frame callback used by owned lossless exporters, with the caller's lifetime.
pub type FrameProcessor<'a> =
    dyn FnMut(&mut crate::native_geometry::GeometryFrame, u8, u64) -> Result<()> + 'a;

/// Run an owned frame processor before FFV1 encoding, retaining timing and AAC.
/// Processor input has stored rotation materialized; dimensions/sampling must
/// remain unchanged. Publication remains the caller's responsibility.
pub fn write_mp4_processed<W: Write + Seek>(
    source: &Path, output: &mut W,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    cancel: Option<&CancelFlag>, progress: Option<&ProgressHook>,
    processor: Option<&mut FrameProcessor<'_>>,
) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    write_mp4_selected(source, output, geometry, filters, cancel, progress, processor,
        fvid_media::owned_framestep::FrameStep::parse("").map_err(|e| invalid(&e))?,
    )
}

#[expect(clippy::too_many_arguments, reason = "Preserve the public export/filter entrypoint signature for existing callers")]
pub fn write_mp4_selected<W: Write + Seek>(
    source: &Path, output: &mut W,
    geometry: &crate::native_geometry::VideoGeometry,
    filters: &crate::native_pixels::PixelFilters,
    cancel: Option<&CancelFlag>, progress: Option<&ProgressHook>,
    mut processor: Option<&mut FrameProcessor<'_>>,
    step: fvid_media::owned_framestep::FrameStep,
) -> Result<(crate::media_info::LosslessStats, ProgressEvent)> {
    check(cancel)?;
    let mut input = Mp4Reader::open(BufReader::new(File::open(source)?), Default::default())?;
    if !mp4_matroska::eligible(&input)
        || input
            .tracks()
            .iter()
            .filter(|t| t.handler == *b"vide")
            .count()
            != 1
    {
        return Err(invalid(
            "owned FFV1 export requires one AVC/HEVC video and supported AAC companion tracks",
        ));
    }
    let tracks = input.tracks().to_vec();
    let video = tracks.iter().position(|t| t.handler == *b"vide").unwrap();
    let plans = tracks
        .iter()
        .map(|t| mp4_matroska::plan(t, input.movie_timescale(), cancel))
        .collect::<Result<Vec<_>>>()?;
    let mut reader = NativeReader::software(BufReader::new(File::open(source)?), usize::MAX)?;
    let first = reader
        .read_frame_raw()?
        .ok_or_else(|| invalid("input has no decoded video frames"))?;
    check(cancel)?;
    let source_colour = reader.colour();
    filters.configure_vignette_source(&reader,geometry)?;
    let source_full_range = source_colour.full_range;
    let processed = processor.is_some();
    let bake_rotation = processor.is_some() || !geometry.is_identity() || !filters.is_empty();
    let prepare = |frame: &RawFrame, display: [usize; 2], n:u64, t:Option<f64>,
                   clock: Option<fvid_media::owned_fade::FrameTime>| -> Result<_> {
        let (w, h, depth) = match frame {
            RawFrame::Avc { picture, .. } => {
                let (w, h) = picture.dimensions();
                (w, h, picture.bit_depth)
            }
            RawFrame::Planar8(p) => (p.width, p.height, 8),
            _ => return Err(invalid("unexpected FFV1 input picture type")),
        };
        let mut samples = if bake_rotation {
            geometry.apply_display_media(frame, display[0], display[1], tracks[video].rotation)?
        } else {
            geometry.apply(frame, w, h)?
        };
        if !processed {filters.apply_colour_clock(&mut samples, depth, source_full_range, source_colour.matrix,n,t,
                clock,
            )?;}
        Ok(samples)
    };
    let first_samples = prepare(&first, reader.dimensions(),0,reader.frame_interval().map(|(start,_,scale)|start as f64/scale as f64),
        crate::native_pixels::frame_clock(&reader)?,
    )?;
    let output_dimensions = (first_samples.width, first_samples.height);
    let output_layout = first_samples.subsampling;
    let mut prepared_first = Some(first_samples);
    let mut first_frame = Some(first);
    let specs = tracks
        .iter()
        .enumerate()
        .map(|(index, t)| -> Result<_> {
            Ok(TrackSpec {
                encoding: if index == video {
                    Encoding::Ffv1V1 {
                        width: u32::try_from(output_dimensions.0)
                            .map_err(|_| invalid("FFV1 output width overflow"))?,
                        height: u32::try_from(output_dimensions.1)
                            .map_err(|_| invalid("FFV1 output height overflow"))?,
                    }
                } else {
                    Encoding::Aac {
                        configuration: crate::codec::config::aac_specific_config(&t.configuration)?,
                        sample_rate: t.sample_rate,
                        channels: t.channels,
                    }
                },
                name: &t.name,
                language: &t.language,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut options = plans.iter().map(|p| p.options).collect::<Vec<_>>();
    if bake_rotation {
        options[video].rotation = 0;
        let [width, height] = reader.dimensions();
        options[video]
            .video
            .as_mut()
            .ok_or_else(|| invalid("missing video metadata"))?
            .pixel_aspect = crate::native_export::transformed_aspect(
            reader.pixel_aspect(),
            width,
            height,
            geometry,
        )?;
    }
    let mut writer =
        PacketWriter::new_with_metadata(output, &specs, &options, &FileMetadata::from_mp4(&input))?;
    if let Some(hook) = progress {
        hook.emit(writer.event());
    }
    check(cancel)?;
    let mut audio = BinaryHeap::new();
    for (i, plan) in plans.iter().enumerate() {
        if i != video
            && let Some(first) = plan.packets.first() {
                audio.push(Reverse((first.dts, i, 0usize)));
            }
    }
    let mut payload = Vec::new();
    let mut stats = crate::media_info::LosslessStats {
        backend: "fvid",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: false,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format: String::new(),
        encoder: "ffv1".into(),
        fvid_crop_payload_copies: 0,
        vertical_flip: geometry.vertical_flip,
        horizontal_flip: geometry.horizontal_flip,
    };
    let mut depth = None;
    loop {
        check(cancel)?;
        let frame = if let Some(first) = first_frame.take() {
            Some(first)
        } else {
            reader.read_frame_raw()?
        };
        let time = if frame.is_some() {
            let (start, end, scale) = reader
                .frame_interval()
                .ok_or_else(|| invalid("missing FFV1 frame timing"))?;
            let ns = |n: u128| -> Result<u64> {
                if scale == 0 {
                    return Err(invalid("zero FFV1 clock"));
                }
                u64::try_from(
                    n.checked_mul(1_000_000_000)
                        .ok_or_else(|| invalid("FFV1 clock overflow"))?
                        / u128::from(scale),
                )
                .map_err(|_| invalid("FFV1 timestamp overflow"))
            };
            let start = ns(start)?;
            let end = ns(end)?;
            if end <= start {
                return Err(invalid("FFV1 frame duration is below one nanosecond"));
            }
            Some((start, end - start))
        } else {
            None
        };
        while audio
            .peek()
            .is_some_and(|Reverse((dts, _, _))| time.is_none_or(|(pts, _)| *dts <= i128::from(pts)))
        {
            check(cancel)?;
            let Reverse((_, track, index)) = audio.pop().unwrap();
            input.read_packet(track, index, &mut payload)?;
            let packet = &plans[track].packets[index];
            writer.write_packet_with_options(
                track,
                packet.pts,
                packet.duration,
                true,
                &payload,
                packet.options,
            )?;
            stats.copied_packets += 1;
            if let Some(next) = plans[track].packets.get(index + 1) {
                audio.push(Reverse((next.dts, track, index + 1)));
            }
            if let Some(hook) = progress {
                hook.emit(writer.event());
            }
        }
        let Some(frame) = frame else {
            break;
        };
        let (w, h, bit_depth) = match &frame {
            RawFrame::Avc { picture, .. } => {
                let (w, h) = picture.dimensions();
                (w, h, picture.bit_depth)
            }
            RawFrame::Planar8(planes) => (planes.width, planes.height, 8),
            _ => return Err(invalid("unexpected FFV1 input picture type")),
        };
        if w != usize::from(tracks[video].width)
            || h != usize::from(tracks[video].height)
            || depth.is_some_and(|d| d != bit_depth)
        {
            return Err(invalid("FFV1 stream geometry or depth changed"));
        }
        depth = Some(bit_depth);
        let mut samples = if let Some(samples) = prepared_first.take() {
            samples
        } else {
            prepare(&frame, reader.dimensions(),stats.decoded_frames,time.map(|(pts,_)|pts as f64/1e9),
                crate::native_pixels::frame_clock(&reader)?,
            )?
        };
        if let Some(process) = processor.as_deref_mut() {
            process(&mut samples,bit_depth,time.unwrap().0)?;
        }
        if (samples.width, samples.height) != output_dimensions
            || samples.subsampling != output_layout
        {
            return Err(invalid("FFV1 transformed geometry changed"));
        }
        if processed {filters.apply_colour_clock(&mut samples,bit_depth,source_full_range,source_colour.matrix,stats.decoded_frames,time.map(|(pts,_)|pts as f64/1e9),
                crate::native_pixels::frame_clock(&reader)?,
            )?;}
        let emit = step.emits(stats.decoded_frames);
        stats.decoded_frames += 1;
        if !emit { continue; }
        let packet = crate::codec::ffv1_encoder::encode(&samples, bit_depth)?;
        check(cancel)?;
        let (pts, duration) = time.unwrap();
        writer.write_packet(video, pts, duration, true, &packet)?;
        if geometry.crop.is_some() {
            stats.fvid_crop_payload_copies += 1;
        }
        stats.video_frames += 1;
        stats.video_packets += 1;
        let format = match samples.subsampling {
            Some([2, 2]) => "yuv420p",
            Some([2, 1]) => "yuv422p",
            Some([1, 1]) => "yuv444p",
            Some([1, 2]) => "yuv440p",
            _ => return Err(invalid("unexpected FFV1 input subsampling")),
        };
        stats.pixel_format = if bit_depth == 8 {
            format.into()
        } else {
            format!("{format}{bit_depth}le")
        };
        if let Some(hook) = progress {
            hook.emit(writer.event());
        }
    }
    if stats.video_frames == 0 {
        return Err(invalid("input has no decoded video frames"));
    }
    check(cancel)?;
    let event = writer.finish()?;
    Ok((stats, event))
}

/// Whether the supported request leaves samples and display geometry unchanged.
pub fn identity(transform: &crate::media_info::LosslessTransform) -> bool {
    supports(transform)
        && configuration(transform)
            .is_ok_and(|(geometry, filters)| geometry.is_identity() && filters.is_empty())
}

/// An overlay plus independently supported spatial/pixel transforms.
pub fn supports_overlay(transform:&crate::media_info::LosslessTransform)->bool {
    if transform.overlay.is_none() {return false;}
    let mut remaining=transform.clone();remaining.overlay=None;supports(&remaining)
}

/// A plain overlay request can share the dedicated owned compositor.
pub fn overlay_only(transform:&crate::media_info::LosslessTransform,
)->Option<&crate::media_info::OverlaySpec> {
    let spec=transform.overlay.as_ref()?;
    let mut remaining=transform.clone();remaining.overlay=None;
    identity(&remaining).then_some(spec)
}

/// Admission for currently owned spatial transformations.
pub fn supports(transform: &crate::media_info::LosslessTransform) -> bool {
    if transform.grayworld.as_deref().is_some_and(|a| fvid_media::owned_timeline::Timeline::grayworld(a).is_err()) { return false; }
    if transform.curves.as_deref().is_some_and(|a| fvid_media::owned_curves::Curves::parse(a).is_err()) {return false;}
    if transform.vignette.as_deref().is_some_and(|a| fvid_media::owned_vignette::Vignette::parse(a).is_err()) {return false;}
    if transform.smartblur.as_deref().is_some_and(|a| fvid_media::owned_smartblur::SmartBlur::parse(a).is_err()) {return false;}
    if transform.sab.as_deref().is_some_and(|a| fvid_media::owned_sab::Sab::parse(a).is_err()) {return false;}
    if transform.bitplanenoise.as_deref().is_some_and(|a| fvid_media::owned_bitplanenoise::BitPlaneNoise::parse(a).is_err()) {return false;}
    if transform.deband.as_deref().is_some_and(|a| fvid_media::owned_deband::Deband::parse(a).is_err()) {return false;}
    if transform.perspective.as_deref().is_some_and(|a| fvid_media::owned_perspective::Perspective::parse(a).is_err()) {return false;}
    if transform.gradfun.as_deref().is_some_and(|a| fvid_media::owned_gradfun::GradFun::parse(a).is_err()) {return false;}
    if transform.lenscorrection.as_deref().is_some_and(|a| fvid_media::owned_lenscorrection::LensCorrection::parse(a).is_err()) {return false;}
    if transform.drawbox.as_deref().is_some_and(|a| fvid_media::owned_draw::Draw::box_filter(a).is_err()) {return false;}
    if transform.drawgrid.as_deref().is_some_and(|a| fvid_media::owned_draw::Draw::grid_filter(a).is_err()) {return false;}
    if transform.removegrain.as_deref().is_some_and(|a| fvid_media::owned_removegrain::RemoveGrain::parse(a).is_err()) {return false;}
    if transform.yaepblur.as_deref().is_some_and(|a| fvid_media::owned_yaepblur::YaepBlur::parse(a).is_err()) {return false;}
    if transform.hqdn3d.as_deref().is_some_and(|a| fvid_media::owned_hqdn3d::HqDn3d::parse(a).is_err()) {return false;}
    if transform.boxblur.as_deref().is_some_and(|args| crate::native_boxblur::BoxBlurProgram::parse(args).is_err()) { return false; }
    matches!(
        transform,
        crate::media_info::LosslessTransform {
            crop: _,
            vertical_flip: _,
            horizontal_flip: _,
            scale: _,
            epx: None,
            transpose: _,
            rotate: _,
            pad: _,
            burn_subs: None,
            overlay: None,
            xfade: None,
            yadif: None,
            bwdif: None,
            w3fdif: None,
            tblend: None,
            tmix: _,
            hqdn3d: _,
            gblur: _,
            eq: _,
            unsharp: _,
            hue: _,
            avgblur: _,
            boxblur: _,
            negate: _,
            edgedetect: None,
            sobel: _,
            prewitt: _,
            roberts: _,
            kirsch: _,
            scharr: _,
            atadenoise: None,
            owdenoise: None,
            vaguedenoiser: None,
            nlmeans: None,
            bm3d: None,
            dctdnoiz: None,
            fftdnoiz: None,
            smartblur: _,
            sab: _,
            bitplanenoise: _,
            deband: _,
            perspective: _,
            gradfun: _,
            lenscorrection: _,
            removegrain: _,
            yaepblur: _,
            bilateral: _,
            cas: _,
            vignette: _,
            curves: _,
            colorbalance: _,
            colorlevels: _,
            colorchannelmixer: _,
            deflicker: None,
            photosensitivity: None,
            monochrome: _,
            grayworld: _,
            drawbox: _,
            drawgrid: _,
            lagfun: _,
            amplify: None,
            pixelize: _,
            vibrance: _,
            dilation: _,
            erosion: _,
            colorize: _,
            exposure: _,
            chromashift: _,
            colorcontrast: _,
            colorcorrect: _,
            histeq: None,
            shuffleplanes: _,
            lutyuv: _,
            colorhold: _,
            fade: _,
            lumakey: None,
            chromakey: None,
            colorkey: None,
            despill: None,
            selectivecolor: None,
            stereo3d: None,
            field: None,
            hqx: None,
            xbr: None,
            il: None,
            super2xsai: None,
            kerndeint: None,
            phase: None,
            estdif: None,
            tinterlace: None,
            separatefields: None,
            weave: None,
            doubleweave: None,
            framepack: None,
            telecine: None,
            pullup: None,
            decimate: None,
            mpdecimate: None,
            framestep: None,
            tile: None,
            untile: None,
            shuffleframes: None,
            reverse: None,
            r#loop: None,
            thumbnail: None,
            freezedetect: None,
            pseudocolor: None,
            minterpolate: None,
            fps: None,
            colorspace: None,
            zscale: _,
            tonemap: None,
            pix_fmt: None,
            interval: None,
            seek: false,
        }
    ) && configuration(transform).is_ok()
}

/// Convert shared lossless options to the owned spatial pipeline.
pub fn configuration(
    transform: &crate::media_info::LosslessTransform,
) -> Result<(
    crate::native_geometry::VideoGeometry,
    crate::native_pixels::PixelFilters,
)> {
    let geometry = crate::native_geometry::VideoGeometry {
        rotate: transform.rotate,
        crop: transform.crop.map(|r| [r.x, r.y, r.width, r.height]),
        horizontal_flip: transform.horizontal_flip,
        vertical_flip: transform.vertical_flip,
        scale: transform
            .scale
            .map(|r| [r.width as usize, r.height as usize]),
        transpose: transform
            .transpose
            .map(|r| crate::native_geometry::Transpose::parse(r.as_str()))
            .transpose()?,
        pad: transform.pad.map(|r| {
            [
                r.width as usize,
                r.height as usize,
                r.x as usize,
                r.y as usize,
            ]
        }),
    };
    let request = crate::media_info::DecodeTransform {
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
        exposure: transform.exposure.clone(),
        colorbalance: transform.colorbalance.clone(),
        curves: transform.curves.clone(),
        vignette: transform.vignette.clone(),
        smartblur: transform.smartblur.clone(),
        sab: transform.sab.clone(),
        bitplanenoise: transform.bitplanenoise.clone(),
        deband: transform.deband.clone(),
        perspective: transform.perspective.clone(),
        gradfun: transform.gradfun.clone(),
        lenscorrection: transform.lenscorrection.clone(),
        drawbox: transform.drawbox.clone(),
        drawgrid: transform.drawgrid.clone(),
        removegrain: transform.removegrain.clone(),
        yaepblur: transform.yaepblur.clone(),
        colorcorrect: transform.colorcorrect.clone(),
        cas: transform.cas.clone(),
        grayworld: transform.grayworld.clone(),
        lagfun: transform.lagfun.clone(),
        tmix: transform.tmix.clone(),
        hqdn3d: transform.hqdn3d.clone(),
        pixelize: transform.pixelize.clone(),
        boxblur: transform.boxblur.clone(),
        gblur: transform.gblur.clone(),
        bilateral: transform.bilateral.clone(),
        avgblur: transform.avgblur.clone(),
        negate: transform.negate.clone(),
        shuffleplanes: transform.shuffleplanes.clone(),
        sobel: transform.sobel.clone(),
        prewitt: transform.prewitt.clone(),
        roberts: transform.roberts.clone(),
        kirsch: transform.kirsch.clone(),
        scharr: transform.scharr.clone(),
        dilation: transform.dilation.clone(),
        erosion: transform.erosion.clone(),
        chromashift: transform.chromashift.clone(),
        ..Default::default()
    };
    Ok((
        geometry,
        crate::native_pixels::PixelFilters::from_request(&request)?,
    ))
}

/// Temporal selection is admitted only with otherwise supported spatial operations.
pub fn supports_framestep(transform: &crate::media_info::LosslessTransform) -> bool {
    let Some(args) = transform.framestep.as_deref() else { return false; };
    if fvid_media::owned_framestep::FrameStep::parse(args).is_err() { return false; }
    let mut spatial = transform.clone();
    spatial.framestep = None;
    supports(&spatial)
}
