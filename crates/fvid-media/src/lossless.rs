//! Decode/crop/FFV1 encode without pixel-format conversion; other streams are copied.
use super::*;
const AGAIN: i32 = -libc::EAGAIN;
pub(crate) struct Codec(pub(crate) *mut AVCodecContext);
impl Drop for Codec {
    fn drop(&mut self) {
        // SAFETY: Codec exclusively owns the allocated context, including any codec state.
        unsafe {
            avcodec_free_context(&mut self.0);
        }
    }
}
pub(crate) struct Frame(pub(crate) *mut AVFrame);
impl Frame {
    pub(crate) fn new() -> Result<Self> {
        // SAFETY: Fresh allocation; null is checked before any use.
        let p = unsafe { av_frame_alloc() };
        if p.is_null() {
            Err("frame allocation failed".into())
        } else {
            Ok(Self(p))
        }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        // SAFETY: Frame owns its reference and all underlying buffer references.
        unsafe {
            av_frame_free(&mut self.0);
        }
    }
}
pub(super) struct Parameters(pub(super) *mut AVCodecParameters);
impl Drop for Parameters {
    fn drop(&mut self) {
        // SAFETY: Parameters exclusively owns the allocation from avcodec_parameters_alloc.
        unsafe {
            avcodec_parameters_free(&mut self.0);
        }
    }
}

struct DecodeAudioTrack {
    stream_index: usize,
    mapped: usize,
    decoder: Codec,
    format: AVSampleFormat,
    decoded_sample_frames: u64,
    written_sample_frames: u64,
    sample_bounds: Option<(u64, u64)>,
    pool: super::audio::PacketPool,
    done: bool,
}
pub use fvid_media_info::{CropRect, ScaleSize, OverlaySpec, XfadeSpec, LosslessTransform};

pub use fvid_media_info::LosslessStats;

fn drain_encoder(
    encoder: &mut Codec,
    output: &mut Output,
    packet: &mut Packet,
    index: usize,
    stats: &mut LosslessStats,
) -> Result<()> {
    loop {
        // SAFETY: Valid codec and packet; receive overwrites the empty packet reference.
        let code = unsafe { avcodec_receive_packet(encoder.0, packet.0) };
        if code == AGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive lossless packet")?;
        // SAFETY: Encoder is initialized and its time base remains fixed.
        let tb = unsafe { (*encoder.0).time_base };
        output.write(packet, index, tb)?;
        stats.video_packets += 1;
    }
}

fn send_encoder_frame(
    encoder: &mut Codec,
    output: &mut Output,
    packet: &mut Packet,
    index: usize,
    frame: *mut AVFrame,
    stats: &mut LosslessStats,
) -> Result<()> {
    loop {
        let code = unsafe { avcodec_send_frame(encoder.0, frame) };
        if code == AGAIN {
            let before = stats.video_packets;
            drain_encoder(encoder, output, packet, index, stats)?;
            if stats.video_packets == before {
                return Err("encoder stalled (EAGAIN with no packets)".into());
            }
            continue;
        }
        check(code, "send frame to encoder")?;
        return Ok(());
    }
}

#[derive(Clone, Copy)]
struct CropStage<'a> {
    index: usize,
    crop: CropRect,
    vertical_flip: bool,
    horizontal_flip: bool,
    scale: Option<ScaleSize>,
    epx: Option<&'a str>,
    transpose: Option<TransposeMode>,
    rotate: Option<RotateAngle>,
    pad: Option<PadRect>,
    burn_subs: Option<&'a Path>,
    overlay: Option<&'a OverlaySpec>,
    xfade: Option<&'a XfadeSpec>,
    yadif: Option<&'a str>,
    bwdif: Option<&'a str>,
    w3fdif: Option<&'a str>,
    tblend: Option<&'a str>,
    tmix: Option<&'a str>,
    hqdn3d: Option<&'a str>,
    gblur: Option<&'a str>,
    eq: Option<&'a str>,
    unsharp: Option<&'a str>,
    hue: Option<&'a str>,
    avgblur: Option<&'a str>,
    boxblur: Option<&'a str>,
    negate: Option<&'a str>,
    edgedetect: Option<&'a str>,
    sobel: Option<&'a str>,
    prewitt: Option<&'a str>,
    roberts: Option<&'a str>,
    kirsch: Option<&'a str>,
    scharr: Option<&'a str>,
    atadenoise: Option<&'a str>,
    owdenoise: Option<&'a str>,
    vaguedenoiser: Option<&'a str>,
    nlmeans: Option<&'a str>,
    bm3d: Option<&'a str>,
    dctdnoiz: Option<&'a str>,
    fftdnoiz: Option<&'a str>,
    smartblur: Option<&'a str>,
    sab: Option<&'a str>,
    bilateral: Option<&'a str>,
    cas: Option<&'a str>,
    vignette: Option<&'a str>,
    curves: Option<&'a str>,
    colorbalance: Option<&'a str>,
    colorlevels: Option<&'a str>,
    colorchannelmixer: Option<&'a str>,
    deflicker: Option<&'a str>,
    photosensitivity: Option<&'a str>,
    monochrome: Option<&'a str>,
    grayworld: Option<&'a str>,
    drawbox: Option<&'a str>,
    drawgrid: Option<&'a str>,
    lagfun: Option<&'a str>,
    amplify: Option<&'a str>,
    bitplanenoise: Option<&'a str>,
    deband: Option<&'a str>,
    gradfun: Option<&'a str>,
    lenscorrection: Option<&'a str>,
    pixelize: Option<&'a str>,
    removegrain: Option<&'a str>,
    yaepblur: Option<&'a str>,
    vibrance: Option<&'a str>,
    dilation: Option<&'a str>,
    erosion: Option<&'a str>,
    colorize: Option<&'a str>,
    exposure: Option<&'a str>,
    chromashift: Option<&'a str>,
    colorcontrast: Option<&'a str>,
    colorcorrect: Option<&'a str>,
    histeq: Option<&'a str>,
    shuffleplanes: Option<&'a str>,
    lutyuv: Option<&'a str>,
    colorhold: Option<&'a str>,
    fade: Option<&'a str>,
    perspective: Option<&'a str>,
    lumakey: Option<&'a str>,
    chromakey: Option<&'a str>,
    colorkey: Option<&'a str>,
    despill: Option<&'a str>,
    selectivecolor: Option<&'a str>,
    stereo3d: Option<&'a str>,
    field: Option<&'a str>,
    hqx: Option<&'a str>,
    xbr: Option<&'a str>,
    il: Option<&'a str>,
    super2xsai: Option<&'a str>,
    kerndeint: Option<&'a str>,
    phase: Option<&'a str>,
    estdif: Option<&'a str>,
    tinterlace: Option<&'a str>,
    separatefields: Option<&'a str>,
    weave: Option<&'a str>,
    doubleweave: Option<&'a str>,
    framepack: Option<&'a str>,
    telecine: Option<&'a str>,
    pullup: Option<&'a str>,
    decimate: Option<&'a str>,
    mpdecimate: Option<&'a str>,
    framestep: Option<&'a str>,
    tile: Option<&'a str>,
    untile: Option<&'a str>,
    shuffleframes: Option<&'a str>,
    reverse: Option<&'a str>,
    r#loop: Option<&'a str>,
    thumbnail: Option<&'a str>,
    freezedetect: Option<&'a str>,
    pseudocolor: Option<&'a str>,
    minterpolate: Option<&'a str>,
    fps: Option<&'a str>,
    colorspace: Option<&'a str>,
    zscale: Option<&'a str>,
    tonemap: Option<&'a str>,
    pix_fmt: Option<AVPixelFormat>,
    interval: Option<(i64, i64)>,
}
fn drain_decoder(
    decoder: &mut Codec,
    encoder: &mut Codec,
    output: &mut Output,
    frame: &mut Frame,
    compact: &mut [Frame],
    compact_i: &mut usize,
    transposed: &mut Frame,
    transpose_graph: &mut Option<filter::FilterGraph>,
    rotated: &mut Frame,
    rotate_graph: &mut Option<filter::FilterGraph>,
    padded: &mut Frame,
    pad_graph: &mut Option<filter::FilterGraph>,
    scaled: &mut Frame,
    sws: &mut Option<Sws>,
    epxed: &mut Frame,
    epx_graph: &mut Option<filter::FilterGraph>,
    burned: &mut Frame,
    burn_graph: &mut Option<filter::FilterGraph>,
    overlaid: &mut Frame,
    overlay_graph: &mut Option<filter::OverlayGraph>,
    xfade_dst: &mut Frame,
    xfade_graph: &mut Option<filter::XfadeGraph>,
    deinterlaced: &mut Frame,
    yadif_graph: &mut Option<filter::FilterGraph>,
    bwdif_out: &mut Frame,
    bwdif_graph: &mut Option<filter::FilterGraph>,
    w3fdif_out: &mut Frame,
    w3fdif_graph: &mut Option<filter::FilterGraph>,
    tblended: &mut Frame,
    tblend_graph: &mut Option<filter::FilterGraph>,
    tmixed: &mut Frame,
    tmix_graph: &mut Option<filter::FilterGraph>,
    denoised: &mut Frame,
    hqdn3d_graph: &mut Option<filter::FilterGraph>,
    blurred: &mut Frame,
    gblur_graph: &mut Option<filter::FilterGraph>,
    equalized: &mut Frame,
    eq_graph: &mut Option<filter::FilterGraph>,
    sharpened: &mut Frame,
    unsharp_graph: &mut Option<filter::FilterGraph>,
    hued: &mut Frame,
    hue_graph: &mut Option<filter::FilterGraph>,
    avgblurred: &mut Frame,
    avgblur_graph: &mut Option<filter::FilterGraph>,
    boxblurred: &mut Frame,
    boxblur_graph: &mut Option<filter::FilterGraph>,
    negated: &mut Frame,
    negate_graph: &mut Option<filter::FilterGraph>,
    edged: &mut Frame,
    edgedetect_graph: &mut Option<filter::FilterGraph>,
    sobeled: &mut Frame,
    sobel_graph: &mut Option<filter::FilterGraph>,
    prewitted: &mut Frame,
    prewitt_graph: &mut Option<filter::FilterGraph>,
    robertsed: &mut Frame,
    roberts_graph: &mut Option<filter::FilterGraph>,
    kirsched: &mut Frame,
    kirsch_graph: &mut Option<filter::FilterGraph>,
    scharred: &mut Frame,
    scharr_graph: &mut Option<filter::FilterGraph>,
    atdenoised: &mut Frame,
    atadenoise_graph: &mut Option<filter::FilterGraph>,
    owdenoised: &mut Frame,
    owdenoise_graph: &mut Option<filter::FilterGraph>,
    vaguedenoised: &mut Frame,
    vaguedenoiser_graph: &mut Option<filter::FilterGraph>,
    nldenoised: &mut Frame,
    nlmeans_graph: &mut Option<filter::FilterGraph>,
    bm3ded: &mut Frame,
    bm3d_graph: &mut Option<filter::FilterGraph>,
    dctdnoized: &mut Frame,
    dctdnoiz_graph: &mut Option<filter::FilterGraph>,
    fftdnoized: &mut Frame,
    fftdnoiz_graph: &mut Option<filter::FilterGraph>,
    smartblurred: &mut Frame,
    smartblur_graph: &mut Option<filter::FilterGraph>,
    sabbed: &mut Frame,
    sab_graph: &mut Option<filter::FilterGraph>,
    bilateraled: &mut Frame,
    bilateral_graph: &mut Option<filter::FilterGraph>,
    cased: &mut Frame,
    cas_graph: &mut Option<filter::FilterGraph>,
    vignetted: &mut Frame,
    vignette_graph: &mut Option<filter::FilterGraph>,
    curved: &mut Frame,
    curves_graph: &mut Option<filter::FilterGraph>,
    colorbalanced: &mut Frame,
    colorbalance_graph: &mut Option<filter::FilterGraph>,
    colorleveled: &mut Frame,
    colorlevels_graph: &mut Option<filter::FilterGraph>,
    colorchannelmixed: &mut Frame,
    colorchannelmixer_graph: &mut Option<filter::FilterGraph>,
    deflickered: &mut Frame,
    deflicker_graph: &mut Option<filter::FilterGraph>,
    photosensitized: &mut Frame,
    photosensitivity_graph: &mut Option<filter::FilterGraph>,
    monochromed: &mut Frame,
    monochrome_graph: &mut Option<filter::FilterGraph>,
    grayworlded: &mut Frame,
    grayworld_graph: &mut Option<filter::FilterGraph>,
    drawboxed: &mut Frame,
    drawbox_graph: &mut Option<filter::FilterGraph>,
    drawgridd: &mut Frame,
    drawgrid_graph: &mut Option<filter::FilterGraph>,
    lagfuned: &mut Frame,
    lagfun_graph: &mut Option<filter::FilterGraph>,
    amplified: &mut Frame,
    amplify_graph: &mut Option<filter::FilterGraph>,
    bitplanenoised: &mut Frame,
    bitplanenoise_graph: &mut Option<filter::FilterGraph>,
    debanded: &mut Frame,
    deband_graph: &mut Option<filter::FilterGraph>,
    gradfuned: &mut Frame,
    gradfun_graph: &mut Option<filter::FilterGraph>,
    lenscorrected: &mut Frame,
    lenscorrection_graph: &mut Option<filter::FilterGraph>,
    pixelized: &mut Frame,
    pixelize_graph: &mut Option<filter::FilterGraph>,
    removegrained: &mut Frame,
    removegrain_graph: &mut Option<filter::FilterGraph>,
    yaepblurred: &mut Frame,
    yaepblur_graph: &mut Option<filter::FilterGraph>,
    vibranced: &mut Frame,
    vibrance_graph: &mut Option<filter::FilterGraph>,
    dilated: &mut Frame,
    dilation_graph: &mut Option<filter::FilterGraph>,
    eroded: &mut Frame,
    erosion_graph: &mut Option<filter::FilterGraph>,
    colorized: &mut Frame,
    colorize_graph: &mut Option<filter::FilterGraph>,
    exposured: &mut Frame,
    exposure_graph: &mut Option<filter::FilterGraph>,
    chromashifted: &mut Frame,
    chromashift_graph: &mut Option<filter::FilterGraph>,
    colorcontrasted: &mut Frame,
    colorcontrast_graph: &mut Option<filter::FilterGraph>,
    colorcorrected: &mut Frame,
    colorcorrect_graph: &mut Option<filter::FilterGraph>,
    histeqed: &mut Frame,
    histeq_graph: &mut Option<filter::FilterGraph>,
    shuffleplaned: &mut Frame,
    shuffleplanes_graph: &mut Option<filter::FilterGraph>,
    lutyuved: &mut Frame,
    lutyuv_graph: &mut Option<filter::FilterGraph>,
    colorholded: &mut Frame,
    colorhold_graph: &mut Option<filter::FilterGraph>,
    faded: &mut Frame,
    fade_graph: &mut Option<filter::FilterGraph>,
    fade_push_mode: &mut bool,
    perspectived: &mut Frame,
    perspective_graph: &mut Option<filter::FilterGraph>,
    lumakeyed: &mut Frame,
    lumakey_graph: &mut Option<filter::FilterGraph>,
    chromakeyed: &mut Frame,
    chromakey_graph: &mut Option<filter::FilterGraph>,
    colorkeyed: &mut Frame,
    colorkey_graph: &mut Option<filter::FilterGraph>,
    despilled: &mut Frame,
    despill_graph: &mut Option<filter::FilterGraph>,
    selectivecolored: &mut Frame,
    selectivecolor_graph: &mut Option<filter::FilterGraph>,
    stereo3ded: &mut Frame,
    stereo3d_graph: &mut Option<filter::FilterGraph>,
    fielded: &mut Frame,
    field_graph: &mut Option<filter::FilterGraph>,
    hqxd: &mut Frame,
    hqx_graph: &mut Option<filter::FilterGraph>,
    xbrd: &mut Frame,
    xbr_graph: &mut Option<filter::FilterGraph>,
    ild: &mut Frame,
    il_graph: &mut Option<filter::FilterGraph>,
    super2xsaid: &mut Frame,
    super2xsai_graph: &mut Option<filter::FilterGraph>,
    kerndeintd: &mut Frame,
    kerndeint_graph: &mut Option<filter::FilterGraph>,
    phased: &mut Frame,
    phase_graph: &mut Option<filter::FilterGraph>,
    phase_push_mode: &mut bool,
    estdifd: &mut Frame,
    estdif_graph: &mut Option<filter::FilterGraph>,
    tinterlaced: &mut Frame,
    tinterlace_graph: &mut Option<filter::FilterGraph>,
    separatefieldsd: &mut Frame,
    separatefields_graph: &mut Option<filter::FilterGraph>,
    weaved: &mut Frame,
    weave_graph: &mut Option<filter::FilterGraph>,
    doubleweaved: &mut Frame,
    doubleweave_graph: &mut Option<filter::FilterGraph>,
    framepacked: &mut Frame,
    framepack_graph: &mut Option<filter::FramepackGraph>,
    telecined: &mut Frame,
    telecine_graph: &mut Option<filter::FilterGraph>,
    pulledup: &mut Frame,
    pullup_graph: &mut Option<filter::FilterGraph>,
    decimated: &mut Frame,
    decimate_graph: &mut Option<filter::FilterGraph>,
    mpdecimated: &mut Frame,
    mpdecimate_graph: &mut Option<filter::FilterGraph>,
    framestepped: &mut Frame,
    framestep_graph: &mut Option<filter::FilterGraph>,
    tiled: &mut Frame,
    tile_graph: &mut Option<filter::FilterGraph>,
    untiled: &mut Frame,
    untile_graph: &mut Option<filter::FilterGraph>,
    shuffled: &mut Frame,
    shuffleframes_graph: &mut Option<filter::FilterGraph>,
    reversed: &mut Frame,
    reverse_graph: &mut Option<filter::FilterGraph>,
    looped: &mut Frame,
    loop_graph: &mut Option<filter::FilterGraph>,
    thumbnailed: &mut Frame,
    thumbnail_graph: &mut Option<filter::FilterGraph>,
    freezedetectd: &mut Frame,
    freezedetect_graph: &mut Option<filter::FilterGraph>,
    pseudocolored: &mut Frame,
    pseudocolor_graph: &mut Option<filter::FilterGraph>,
    minterpolate_dst: &mut Frame,
    minterpolate_graph: &mut Option<filter::FilterGraph>,
    fps_dst: &mut Frame,
    fps_graph: &mut Option<filter::FilterGraph>,
    colorspaced: &mut Frame,
    colorspace_graph: &mut Option<filter::FilterGraph>,
    zscaled: &mut Frame,
    zscale_graph: &mut Option<filter::FilterGraph>,
    tonemapped: &mut Frame,
    tonemap_graph: &mut Option<filter::FilterGraph>,
    converted: &mut Frame,
    fmt_sws: &mut Option<Sws>,
    packet: &mut Packet,
    stage: CropStage<'_>,
    stats: &mut LosslessStats,
) -> Result<()> {
    let CropStage {
        index,
        crop,
        vertical_flip,
        horizontal_flip,
        scale,
        epx,
        transpose,
        rotate,
        pad,
        burn_subs,
        overlay,
        xfade,
        yadif,
        bwdif,
        w3fdif,
        tblend,
        tmix,
        hqdn3d,
        gblur,
        eq,
        unsharp,
        hue,
        avgblur,
        boxblur,
        negate,
        edgedetect,
        sobel,
        prewitt,
        roberts,
        kirsch,
        scharr,
        atadenoise,
        owdenoise,
        vaguedenoiser,
        nlmeans,
        bm3d,
        dctdnoiz,
        fftdnoiz,
        smartblur,
        sab,
        bilateral,
        cas,
        vignette,
        curves,
        colorbalance,
        colorlevels,
        colorchannelmixer,
        deflicker,
        photosensitivity,
        monochrome,
        grayworld,
        drawbox,
        drawgrid,
        lagfun,
        amplify,
        bitplanenoise,
        deband,
        gradfun,
        lenscorrection,
        pixelize,
        removegrain,
        yaepblur,
        vibrance,
        dilation,
        erosion,
        colorize,
        exposure,
        chromashift,
        colorcontrast,
        colorcorrect,
        histeq,
        shuffleplanes,
        lutyuv,
        colorhold,
        fade,
        perspective,
        lumakey,
        chromakey,
        colorkey,
        despill,
        selectivecolor,
        stereo3d,
        field,
        hqx,
        xbr,
        il,
        super2xsai,
        kerndeint,
        phase,
        estdif,
        tinterlace,
        separatefields,
        weave,
        doubleweave,
        framepack,
        telecine,
        pullup,
        decimate,
        mpdecimate,
        framestep,
        tile,
        untile,
        shuffleframes,
        reverse,
        r#loop,
        thumbnail,
        freezedetect,
        pseudocolor,
        minterpolate,
        fps,
        colorspace,
        zscale,
        tonemap,
        pix_fmt,
        interval,
    } = stage;
    let full_w = unsafe { (*decoder.0).width };
    let full_h = unsafe { (*decoder.0).height };
    let src_fmt = unsafe { (*decoder.0).pix_fmt };
    loop {
        // SAFETY: Decoder/frame are live and exclusively borrowed.
        let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
        if code == AGAIN || code == EOF {
            return Ok(());
        }
        check(code, "receive decoded frame")?;
        stats.decoded_frames += 1;
        // SAFETY: Decoded frame owns its planes. crop bounds/subsampling are checked
        // before av_frame_apply_cropping adjusts views. It does not copy pixel payloads.
        unsafe {
            let f = &mut *frame.0;
            if f.width != full_w
                || f.height != full_h
                || f.format != src_fmt
                || (f.width as usize) < crop.x + crop.width
                || (f.height as usize) < crop.y + crop.height
            {
                return Err(
                    "dynamic frame geometry/format is not qualified for lossless export".into(),
                );
            }
            if f.pts == NOPTS {
                f.pts = f.best_effort_timestamp;
            }
            if f.pts == NOPTS {
                return Err("lossless export requires frame timestamps".into());
            }
            // Filters (especially libass) need a valid time_base matching packet PTS.
            if f.time_base.num <= 0 || f.time_base.den <= 0 {
                f.time_base = (*decoder.0).pkt_timebase;
            }
            if let Some((start, end)) = interval {
                if f.pts < start || f.pts >= end {
                    av_frame_unref(frame.0);
                    continue;
                }
                f.pts = f.pts.checked_sub(start).ok_or("frame timestamp overflow")?;
            }
            // Decoder pict_type (I/P/B from the source bitstream) must not force
            // libx264 picture types — FFmpeg filters clear this to NONE.
            f.pict_type = 0;
            f.quality = 0;
            if f.flags & AV_FRAME_FLAG_INTERLACED as i32 != 0 {
                return Err("interlaced crop is not qualified".into());
            }
            let full_frame = crop.x == 0
                && crop.y == 0
                && crop.width == f.width as usize
                && crop.height == f.height as usize;
            if !full_frame {
                f.crop_left = crop.x;
                f.crop_top = crop.y;
                f.crop_right = f.width as usize - crop.x - crop.width;
                f.crop_bottom = f.height as usize - crop.y - crop.height;
                check(
                    av_frame_apply_cropping(frame.0, AV_FRAME_CROP_UNALIGNED as i32),
                    "apply exact crop view",
                )?;
            }
            // Match FFmpeg vf order: crop → hflip → vflip → transpose → rotate → pad → scale → subtitles.
            let send = if horizontal_flip {
                let i = *compact_i;
                *compact_i = (i + 1) % compact.len();
                let slot = &mut compact[i];
                horizontal_copy_frame(slot.0, frame.0)?;
                (*slot.0).pict_type = 0;
                (*slot.0).quality = 0;
                (*slot.0).pts = f.pts;
                (*slot.0).duration = f.duration;
                if vertical_flip {
                    flip_view(slot.0)?;
                }
                slot.0
            } else {
                if vertical_flip {
                    flip_view(frame.0)?;
                }
                frame.0
            };
            let send = if let Some(mode) = transpose {
                filter::transpose_frame(transpose_graph, transposed.0, send, mode)?;
                transposed.0
            } else {
                send
            };
            let send = if let Some(angle) = rotate {
                filter::rotate_frame(rotate_graph, rotated.0, send, angle)?;
                rotated.0
            } else {
                send
            };
            let send = if let Some(pad) = pad {
                filter::pad_frame(pad_graph, padded.0, send, pad)?;
                padded.0
            } else {
                send
            };
            // FFmpeg merges `scale=flags=neighbor,format=` into one neighbor
            // swscale pass when no later materialize (burn/overlay) intervenes.
            let (send, format_done) = if let (Some(size), Some(fmt)) = (scale, pix_fmt) {
                if burn_subs.is_none()
                    && overlay.is_none()
                    && xfade.is_none()
                    && epx.is_none()
                    && yadif.is_none()
                    && bwdif.is_none()
                    && w3fdif.is_none()
                    && tblend.is_none()
                    && tmix.is_none()
                    && hqdn3d.is_none()
                    && gblur.is_none()
                    && eq.is_none()
                    && unsharp.is_none()
                    && hue.is_none()
                    && avgblur.is_none()
                    && boxblur.is_none()
                    && negate.is_none()
                    && edgedetect.is_none()
                    && sobel.is_none()
                    && prewitt.is_none()
                    && roberts.is_none()
                    && kirsch.is_none()
                    && scharr.is_none()
                    && atadenoise.is_none()
                    && owdenoise.is_none()
                    && vaguedenoiser.is_none()
                    && nlmeans.is_none()
                    && bm3d.is_none()
                    && dctdnoiz.is_none()
                    && fftdnoiz.is_none()
                    && smartblur.is_none()
                    && sab.is_none()
                    && bilateral.is_none()
                    && cas.is_none()
                    && vignette.is_none()
                    && curves.is_none()
                    && colorbalance.is_none()
                    && colorlevels.is_none()
                    && colorchannelmixer.is_none()
                    && deflicker.is_none()
                    && photosensitivity.is_none()
                    && monochrome.is_none()
                    && grayworld.is_none()
                    && drawbox.is_none()
                    && drawgrid.is_none()
                    && lagfun.is_none()
                    && amplify.is_none()
                    && bitplanenoise.is_none()
                    && deband.is_none()
                    && gradfun.is_none()
                    && lenscorrection.is_none()
                    && pixelize.is_none()
                    && removegrain.is_none()
                    && yaepblur.is_none()
                    && vibrance.is_none()
                    && dilation.is_none()
                    && erosion.is_none()
                    && colorize.is_none()
                    && exposure.is_none()
                    && chromashift.is_none()
                    && colorcontrast.is_none()
                    && colorcorrect.is_none()
                    && histeq.is_none()
                    && shuffleplanes.is_none()
                    && lutyuv.is_none()
                    && colorhold.is_none()
                    && fade.is_none()
                    && perspective.is_none()
                    && lumakey.is_none()
                    && chromakey.is_none()
                    && colorkey.is_none()
                    && despill.is_none()
                    && selectivecolor.is_none()
                    && stereo3d.is_none()
                    && field.is_none()
                    && hqx.is_none()
                    && xbr.is_none()
                    && il.is_none()
                    && super2xsai.is_none()
                    && kerndeint.is_none()
                    && phase.is_none()
                    && estdif.is_none()
                    && tinterlace.is_none()
                    && separatefields.is_none()
                    && weave.is_none()
                    && doubleweave.is_none()
                    && framepack.is_none()
                    && telecine.is_none()
                    && pullup.is_none()
                    && decimate.is_none()
                    && mpdecimate.is_none()
                    && framestep.is_none()
                    && pseudocolor.is_none()
                    && minterpolate.is_none()
                    && fps.is_none()
                    && colorspace.is_none()
                    && zscale.is_none()
                    && tonemap.is_none()
                {
                    scale_convert_frame(
                        sws,
                        scaled.0,
                        send,
                        size.width as i32,
                        size.height as i32,
                        fmt,
                    )?;
                    (scaled.0, true)
                } else {
                    scale_frame(sws, scaled.0, send, size.width as i32, size.height as i32)?;
                    (scaled.0, false)
                }
            } else if let Some(size) = scale {
                scale_frame(sws, scaled.0, send, size.width as i32, size.height as i32)?;
                (scaled.0, false)
            } else {
                (send, false)
            };
            let send = if let Some(args) = epx {
                filter::epx_frame(epx_graph, epxed.0, send, args)?;
                epxed.0
            } else {
                send
            };
            let send = if let Some(path) = burn_subs {
                filter::subtitles_frame(burn_graph, burned.0, send, path)?;
                burned.0
            } else {
                send
            };
            let send = if let Some(spec) = overlay {
                filter::overlay_frame(overlay_graph, overlaid.0, send, &spec.path, spec.x, spec.y)?;
                overlaid.0
            } else {
                send
            };
            let send = if let Some(spec) = xfade {
                let produced = filter::xfade_frame(
                    xfade_graph,
                    xfade_dst.0,
                    send,
                    &spec.path,
                    &spec.transition,
                    spec.duration_us,
                    spec.offset_us,
                    spec.fps_num,
                    spec.fps_den,
                )?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                xfade_dst.0
            } else {
                send
            };
            let send = if let Some(args) = yadif {
                let produced = filter::yadif_push_frame(yadif_graph, deinterlaced.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                deinterlaced.0
            } else {
                send
            };
            let send = if let Some(args) = bwdif {
                let produced = filter::bwdif_push_frame(bwdif_graph, bwdif_out.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                bwdif_out.0
            } else {
                send
            };
            let send = if let Some(args) = w3fdif {
                let produced = filter::w3fdif_push_frame(w3fdif_graph, w3fdif_out.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                w3fdif_out.0
            } else {
                send
            };
            let send = if let Some(args) = tblend {
                let produced = filter::tblend_frame(tblend_graph, tblended.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                tblended.0
            } else {
                send
            };
            let send = if let Some(args) = tmix {
                let produced = filter::tmix_push_frame(tmix_graph, tmixed.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                tmixed.0
            } else {
                send
            };
            let send = if let Some(args) = hqdn3d {
                filter::hqdn3d_frame(hqdn3d_graph, denoised.0, send, args)?;
                denoised.0
            } else {
                send
            };
            let send = if let Some(args) = gblur {
                filter::gblur_frame(gblur_graph, blurred.0, send, args)?;
                blurred.0
            } else {
                send
            };
            let send = if let Some(args) = eq {
                filter::eq_frame(eq_graph, equalized.0, send, args)?;
                equalized.0
            } else {
                send
            };
            let send = if let Some(args) = unsharp {
                filter::unsharp_frame(unsharp_graph, sharpened.0, send, args)?;
                sharpened.0
            } else {
                send
            };
            let send = if let Some(args) = hue {
                filter::hue_frame(hue_graph, hued.0, send, args)?;
                hued.0
            } else {
                send
            };
            let send = if let Some(args) = avgblur {
                filter::avgblur_frame(avgblur_graph, avgblurred.0, send, args)?;
                avgblurred.0
            } else {
                send
            };
            let send = if let Some(args) = boxblur {
                filter::boxblur_frame(boxblur_graph, boxblurred.0, send, args)?;
                boxblurred.0
            } else {
                send
            };
            let send = if let Some(args) = negate {
                filter::negate_frame(negate_graph, negated.0, send, args)?;
                negated.0
            } else {
                send
            };
            let send = if let Some(args) = edgedetect {
                filter::edgedetect_frame(edgedetect_graph, edged.0, send, args)?;
                let out = edged.0;
                // colormix materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                if args.contains("mode=colormix") && (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = sobel {
                filter::sobel_frame(sobel_graph, sobeled.0, send, args)?;
                sobeled.0
            } else {
                send
            };
            let send = if let Some(args) = prewitt {
                filter::prewitt_frame(prewitt_graph, prewitted.0, send, args)?;
                prewitted.0
            } else {
                send
            };
            let send = if let Some(args) = roberts {
                filter::roberts_frame(roberts_graph, robertsed.0, send, args)?;
                robertsed.0
            } else {
                send
            };
            let send = if let Some(args) = kirsch {
                filter::kirsch_frame(kirsch_graph, kirsched.0, send, args)?;
                kirsched.0
            } else {
                send
            };
            let send = if let Some(args) = scharr {
                filter::scharr_frame(scharr_graph, scharred.0, send, args)?;
                scharred.0
            } else {
                send
            };
            let send = if let Some(args) = atadenoise {
                let produced =
                    filter::atadenoise_push_frame(atadenoise_graph, atdenoised.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                atdenoised.0
            } else {
                send
            };
            let send = if let Some(args) = owdenoise {
                filter::owdenoise_frame(owdenoise_graph, owdenoised.0, send, args)?;
                owdenoised.0
            } else {
                send
            };
            let send = if let Some(args) = vaguedenoiser {
                filter::vaguedenoiser_frame(vaguedenoiser_graph, vaguedenoised.0, send, args)?;
                vaguedenoised.0
            } else {
                send
            };
            let send = if let Some(args) = nlmeans {
                filter::nlmeans_frame(nlmeans_graph, nldenoised.0, send, args)?;
                nldenoised.0
            } else {
                send
            };
            let send = if let Some(args) = bm3d {
                filter::bm3d_frame(bm3d_graph, bm3ded.0, send, args)?;
                bm3ded.0
            } else {
                send
            };
            let send = if let Some(args) = dctdnoiz {
                let src_fmt = (*send).format;
                filter::dctdnoiz_frame(dctdnoiz_graph, dctdnoized.0, send, args)?;
                let out = dctdnoized.0;
                // dctdnoiz materializes rgb24; fair-pair reverts via libswscale like photosensitivity.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = fftdnoiz {
                filter::fftdnoiz_frame(fftdnoiz_graph, fftdnoized.0, send, args)?;
                fftdnoized.0
            } else {
                send
            };
            let send = if let Some(args) = smartblur {
                filter::smartblur_frame(smartblur_graph, smartblurred.0, send, args)?;
                smartblurred.0
            } else {
                send
            };
            let send = if let Some(args) = sab {
                filter::sab_frame(sab_graph, sabbed.0, send, args)?;
                sabbed.0
            } else {
                send
            };
            let send = if let Some(args) = bilateral {
                filter::bilateral_frame(bilateral_graph, bilateraled.0, send, args)?;
                bilateraled.0
            } else {
                send
            };
            let send = if let Some(args) = cas {
                filter::cas_frame(cas_graph, cased.0, send, args)?;
                cased.0
            } else {
                send
            };
            let send = if let Some(args) = vignette {
                filter::vignette_frame(vignette_graph, vignetted.0, send, args)?;
                vignetted.0
            } else {
                send
            };
            let send = if let Some(args) = curves {
                let src_fmt = (*send).format;
                filter::curves_frame(curves_graph, curved.0, send, args)?;
                let out = curved.0;
                // curves materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorbalance {
                let src_fmt = (*send).format;
                filter::colorbalance_frame(colorbalance_graph, colorbalanced.0, send, args)?;
                let out = colorbalanced.0;
                // colorbalance materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorlevels {
                let src_fmt = (*send).format;
                filter::colorlevels_frame(colorlevels_graph, colorleveled.0, send, args)?;
                let out = colorleveled.0;
                // colorlevels materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorchannelmixer {
                let src_fmt = (*send).format;
                filter::colorchannelmixer_frame(
                    colorchannelmixer_graph,
                    colorchannelmixed.0,
                    send,
                    args,
                )?;
                let out = colorchannelmixed.0;
                // colorchannelmixer materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = deflicker {
                let produced =
                    filter::deflicker_push_frame(deflicker_graph, deflickered.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                deflickered.0
            } else {
                send
            };
            let send = if let Some(args) = photosensitivity {
                let src_fmt = (*send).format;
                filter::photosensitivity_frame(
                    photosensitivity_graph,
                    photosensitized.0,
                    send,
                    args,
                )?;
                let out = photosensitized.0;
                // photosensitivity materializes rgb24; fair-pair reverts via libswscale like colorbalance.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = monochrome {
                filter::monochrome_frame(monochrome_graph, monochromed.0, send, args)?;
                monochromed.0
            } else {
                send
            };
            let send = if let Some(args) = grayworld {
                let src_fmt = (*send).format;
                filter::grayworld_frame(grayworld_graph, grayworlded.0, send, args)?;
                let out = grayworlded.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = drawbox {
                filter::drawbox_frame(drawbox_graph, drawboxed.0, send, args)?;
                drawboxed.0
            } else {
                send
            };
            let send = if let Some(args) = drawgrid {
                filter::drawgrid_frame(drawgrid_graph, drawgridd.0, send, args)?;
                drawgridd.0
            } else {
                send
            };
            let send = if let Some(args) = lagfun {
                let produced = filter::lagfun_push_frame(lagfun_graph, lagfuned.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                lagfuned.0
            } else {
                send
            };
            let send = if let Some(args) = amplify {
                let produced = filter::amplify_push_frame(amplify_graph, amplified.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                amplified.0
            } else {
                send
            };
            let send = if let Some(args) = bitplanenoise {
                filter::bitplanenoise_frame(bitplanenoise_graph, bitplanenoised.0, send, args)?;
                bitplanenoised.0
            } else {
                send
            };
            let send = if let Some(args) = deband {
                filter::deband_frame(deband_graph, debanded.0, send, args)?;
                debanded.0
            } else {
                send
            };
            let send = if let Some(args) = gradfun {
                let src_fmt = (*send).format;
                filter::gradfun_frame(gradfun_graph, gradfuned.0, send, args)?;
                let out = gradfuned.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = lenscorrection {
                let src_fmt = (*send).format;
                filter::lenscorrection_frame(lenscorrection_graph, lenscorrected.0, send, args)?;
                let out = lenscorrected.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = pixelize {
                let src_fmt = (*send).format;
                filter::pixelize_frame(pixelize_graph, pixelized.0, send, args)?;
                let out = pixelized.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = removegrain {
                let src_fmt = (*send).format;
                filter::removegrain_frame(removegrain_graph, removegrained.0, send, args)?;
                let out = removegrained.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = yaepblur {
                let src_fmt = (*send).format;
                filter::yaepblur_frame(yaepblur_graph, yaepblurred.0, send, args)?;
                let out = yaepblurred.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = vibrance {
                let src_fmt = (*send).format;
                filter::vibrance_frame(vibrance_graph, vibranced.0, send, args)?;
                let out = vibranced.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = dilation {
                let src_fmt = (*send).format;
                filter::dilation_frame(dilation_graph, dilated.0, send, args)?;
                let out = dilated.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = erosion {
                let src_fmt = (*send).format;
                filter::erosion_frame(erosion_graph, eroded.0, send, args)?;
                let out = eroded.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorize {
                let src_fmt = (*send).format;
                filter::colorize_frame(colorize_graph, colorized.0, send, args)?;
                let out = colorized.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = exposure {
                let src_fmt = (*send).format;
                filter::exposure_frame(exposure_graph, exposured.0, send, args)?;
                let out = exposured.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = chromashift {
                let src_fmt = (*send).format;
                filter::chromashift_frame(chromashift_graph, chromashifted.0, send, args)?;
                let out = chromashifted.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorcontrast {
                let src_fmt = (*send).format;
                filter::colorcontrast_frame(colorcontrast_graph, colorcontrasted.0, send, args)?;
                let out = colorcontrasted.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorcorrect {
                let src_fmt = (*send).format;
                filter::colorcorrect_frame(colorcorrect_graph, colorcorrected.0, send, args)?;
                let out = colorcorrected.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = histeq {
                let src_fmt = (*send).format;
                filter::histeq_frame(histeq_graph, histeqed.0, send, args)?;
                let out = histeqed.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = shuffleplanes {
                let src_fmt = (*send).format;
                filter::shuffleplanes_frame(shuffleplanes_graph, shuffleplaned.0, send, args)?;
                let out = shuffleplaned.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = lutyuv {
                let src_fmt = (*send).format;
                filter::lutyuv_frame(lutyuv_graph, lutyuved.0, send, args)?;
                let out = lutyuved.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorhold {
                let src_fmt = (*send).format;
                filter::colorhold_frame(colorhold_graph, colorholded.0, send, args)?;
                let out = colorholded.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = fade {
                let src_fmt = (*send).format;
                let produced =
                    filter::fade_apply_frame(fade_graph, faded.0, send, args, fade_push_mode)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                let out = faded.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = perspective {
                let src_fmt = (*send).format;
                filter::perspective_frame(perspective_graph, perspectived.0, send, args)?;
                let out = perspectived.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = lumakey {
                filter::lumakey_frame(lumakey_graph, lumakeyed.0, send, args)?;
                lumakeyed.0
            } else {
                send
            };
            let send = if let Some(args) = chromakey {
                filter::chromakey_frame(chromakey_graph, chromakeyed.0, send, args)?;
                chromakeyed.0
            } else {
                send
            };
            let send = if let Some(args) = colorkey {
                filter::colorkey_frame(colorkey_graph, colorkeyed.0, send, args)?;
                colorkeyed.0
            } else {
                send
            };
            let send = if let Some(args) = despill {
                let src_fmt = (*send).format;
                filter::despill_frame(despill_graph, despilled.0, send, args)?;
                let out = despilled.0;
                // despill materializes gbrp/rgb; fair-pair reverts via libswscale like colorbalance.
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = selectivecolor {
                let src_fmt = (*send).format;
                filter::selectivecolor_frame(selectivecolor_graph, selectivecolored.0, send, args)?;
                let out = selectivecolored.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = stereo3d {
                let src_fmt = (*send).format;
                filter::stereo3d_frame(stereo3d_graph, stereo3ded.0, send, args)?;
                let out = stereo3ded.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = field {
                let src_fmt = (*send).format;
                filter::field_frame(field_graph, fielded.0, send, args)?;
                let out = fielded.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = hqx {
                filter::hqx_frame(hqx_graph, hqxd.0, send, args)?;
                hqxd.0
            } else {
                send
            };
            let send = if let Some(args) = xbr {
                filter::xbr_frame(xbr_graph, xbrd.0, send, args)?;
                xbrd.0
            } else {
                send
            };
            let send = if let Some(args) = il {
                let src_fmt = (*send).format;
                filter::il_frame(il_graph, ild.0, send, args)?;
                let out = ild.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = super2xsai {
                filter::super2xsai_frame(super2xsai_graph, super2xsaid.0, send, args)?;
                super2xsaid.0
            } else {
                send
            };
            let send = if let Some(args) = kerndeint {
                let src_fmt = (*send).format;
                filter::kerndeint_frame(kerndeint_graph, kerndeintd.0, send, args)?;
                let out = kerndeintd.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = phase {
                let src_fmt = (*send).format;
                let produced =
                    filter::phase_apply_frame(phase_graph, phased.0, send, args, phase_push_mode)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                let out = phased.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = estdif {
                let src_fmt = (*send).format;
                let produced = filter::estdif_push_frame(estdif_graph, estdifd.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                let out = estdifd.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = tinterlace {
                let src_fmt = (*send).format;
                let produced =
                    filter::tinterlace_push_frame(tinterlace_graph, tinterlaced.0, send, args)?;
                if !produced {
                    av_frame_unref(frame.0);
                    continue;
                }
                let out = tinterlaced.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            if let Some(args) = separatefields {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::separatefields_push_frame(
                    separatefields_graph,
                    separatefieldsd.0,
                    send,
                    args,
                    |field| {
                        let mut out = field;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let mut doubleweave_field = |mut out: *mut AVFrame| -> Result<()> {
                            let out = if let Some(args) = freezedetect {
                                let src_fmt = (*out).format;
                                filter::freezedetect_frame(
                                    freezedetect_graph,
                                    freezedetectd.0,
                                    out,
                                    args,
                                )?;
                                let o = freezedetectd.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = pseudocolor {
                                let src_fmt = (*out).format;
                                filter::pseudocolor_frame(
                                    pseudocolor_graph,
                                    pseudocolored.0,
                                    out,
                                    args,
                                )?;
                                let o = pseudocolored.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = colorspace {
                                filter::colorspace_frame(
                                    colorspace_graph,
                                    colorspaced.0,
                                    out,
                                    args,
                                )?;
                                colorspaced.0
                            } else {
                                out
                            };
                            let (out, mut format_done) = if let Some(args) = zscale {
                                let fmt = pix_fmt.ok_or(
                                    "--zscale requires --pix-fmt for format= after zscale",
                                )?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown zscale output pixel format".into());
                                }
                                filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                                (zscaled.0, true)
                            } else {
                                (out, format_done)
                            };
                            let (out, format_done) = if let Some(args) = tonemap {
                                let fmt = pix_fmt.ok_or(
                                    "--tonemap requires --pix-fmt for format= after tonemap",
                                )?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown tonemap output pixel format".into());
                                }
                                filter::tonemap_frame(
                                    tonemap_graph,
                                    tonemapped.0,
                                    out,
                                    args,
                                    &name,
                                )?;
                                (tonemapped.0, format_done)
                            } else {
                                (out, format_done)
                            };
                            let out = if let Some(fmt) = pix_fmt {
                                if !format_done && (*out).format != fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                                    converted.0
                                } else {
                                    out
                                }
                            } else {
                                out
                            };
                            if minterpolate.is_some() || fps.is_some() {
                                filter::temporal_push_frame(
                                    minterpolate_graph,
                                    minterpolate_dst.0,
                                    minterpolate,
                                    fps_graph,
                                    fps_dst.0,
                                    fps,
                                    out,
                                    |o| {
                                        send_encoder_frame(
                                            encoder, output, packet, index, o, stats,
                                        )?;
                                        emitted += 1;
                                        Ok(())
                                    },
                                )?;
                            } else {
                                send_encoder_frame(encoder, output, packet, index, out, stats)?;
                                emitted += 1;
                            }
                            Ok(())
                        };
                        let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                            if shuffleframes.is_some() {
                                filter::push_shuffleframes_or_emit(
                                    shuffleframes_graph,
                                    shuffled.0,
                                    out,
                                    shuffleframes.as_deref(),
                                    |f| doubleweave_field(f),
                                )
                            } else {
                                doubleweave_field(out)
                            }
                        };
                        let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                            if untile.is_some() {
                                filter::push_untile_or_emit(
                                    untile_graph,
                                    untiled.0,
                                    out,
                                    untile.as_deref(),
                                    |f| apply_shuffleframes(f),
                                )
                            } else {
                                apply_shuffleframes(out)
                            }
                        };
                        let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                            if tile.is_some() {
                                filter::push_tile_or_emit(
                                    tile_graph,
                                    tiled.0,
                                    out,
                                    tile.as_deref(),
                                    |f| apply_untile(f),
                                )
                            } else {
                                apply_untile(out)
                            }
                        };

                        let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                            if framestep.is_some() {
                                filter::push_framestep_or_emit(
                                    framestep_graph,
                                    framestepped.0,
                                    out,
                                    framestep,
                                    |f| apply_tile(f),
                                )
                            } else {
                                apply_tile(out)
                            }
                        };
                        let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(mpd_args) = mpdecimate {
                                filter::mpdecimate_push_frame(
                                    mpdecimate_graph,
                                    mpdecimated.0,
                                    out,
                                    mpd_args,
                                    |mpd| apply_framestep(mpd),
                                )
                            } else {
                                apply_framestep(out)
                            }
                        };
                        let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(dc_args) = decimate {
                                filter::decimate_push_frame(
                                    decimate_graph,
                                    decimated.0,
                                    out,
                                    dc_args,
                                    |dc| apply_mpdecimate(dc),
                                )?;
                            } else {
                                apply_mpdecimate(out)?;
                            }
                            Ok(())
                        };
                        let mut apply_pullup = |out: *mut AVFrame| -> Result<()> {
                            if let Some(pu_args) = pullup {
                                filter::pullup_push_frame(
                                    pullup_graph,
                                    pulledup.0,
                                    out,
                                    pu_args,
                                    |pu| apply_decimate(pu),
                                )?;
                            } else {
                                apply_decimate(out)?;
                            }
                            Ok(())
                        };
                        let mut apply_telecine = |out: *mut AVFrame| -> Result<()> {
                            if let Some(tc_args) = telecine {
                                filter::telecine_push_frame(
                                    telecine_graph,
                                    telecined.0,
                                    out,
                                    tc_args,
                                    |tc| apply_pullup(tc),
                                )?;
                            } else {
                                apply_pullup(out)?;
                            }
                            Ok(())
                        };
                        let mut apply_framepack = |out: *mut AVFrame| -> Result<()> {
                            if let Some(fp_args) = framepack {
                                filter::framepack_push_frame(
                                    framepack_graph,
                                    framepacked.0,
                                    out,
                                    fp_args,
                                    |packed| apply_telecine(packed),
                                )?;
                            } else {
                                apply_telecine(out)?;
                            }
                            Ok(())
                        };
                        let mut apply_doubleweave = |out: *mut AVFrame| -> Result<()> {
                            if let Some(dw_args) = doubleweave {
                                filter::doubleweave_push_frame(
                                    doubleweave_graph,
                                    doubleweaved.0,
                                    out,
                                    dw_args,
                                    |doubled| apply_framepack(doubled),
                                )?;
                            } else {
                                apply_framepack(out)?;
                            }
                            Ok(())
                        };
                        if let Some(weave_args) = weave {
                            filter::weave_push_frame(
                                weave_graph,
                                weaved.0,
                                out,
                                weave_args,
                                |woven| apply_doubleweave(woven),
                            )?;
                        } else {
                            apply_doubleweave(out)?;
                        }
                        Ok(())
                    },
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = weave {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::weave_push_frame(weave_graph, weaved.0, send, args, |woven| {
                    let mut out = woven;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                        out = converted.0;
                    }
                    let mut finish = |mut out: *mut AVFrame| -> Result<()> {
                        let out = if let Some(args) = freezedetect {
                            let src_fmt = (*out).format;
                            filter::freezedetect_frame(
                                freezedetect_graph,
                                freezedetectd.0,
                                out,
                                args,
                            )?;
                            let o = freezedetectd.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = pseudocolor {
                            let src_fmt = (*out).format;
                            filter::pseudocolor_frame(
                                pseudocolor_graph,
                                pseudocolored.0,
                                out,
                                args,
                            )?;
                            let o = pseudocolored.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = colorspace {
                            filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                            colorspaced.0
                        } else {
                            out
                        };
                        let (out, mut format_done) = if let Some(args) = zscale {
                            let fmt = pix_fmt
                                .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                            (zscaled.0, true)
                        } else {
                            (out, format_done)
                        };
                        let (out, format_done) = if let Some(args) = tonemap {
                            let fmt = pix_fmt
                                .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                            (tonemapped.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if minterpolate.is_some() || fps.is_some() {
                            filter::temporal_push_frame(
                                minterpolate_graph,
                                minterpolate_dst.0,
                                minterpolate,
                                fps_graph,
                                fps_dst.0,
                                fps,
                                out,
                                |o| {
                                    send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            send_encoder_frame(encoder, output, packet, index, out, stats)?;
                            emitted += 1;
                        }
                        Ok(())
                    };
                    let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                        if shuffleframes.is_some() {
                            filter::push_shuffleframes_or_emit(
                                shuffleframes_graph,
                                shuffled.0,
                                out,
                                shuffleframes.as_deref(),
                                |f| finish(f),
                            )
                        } else {
                            finish(out)
                        }
                    };
                    let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                        if untile.is_some() {
                            filter::push_untile_or_emit(
                                untile_graph,
                                untiled.0,
                                out,
                                untile.as_deref(),
                                |f| apply_shuffleframes(f),
                            )
                        } else {
                            apply_shuffleframes(out)
                        }
                    };
                    let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                        if tile.is_some() {
                            filter::push_tile_or_emit(
                                tile_graph,
                                tiled.0,
                                out,
                                tile.as_deref(),
                                |f| apply_untile(f),
                            )
                        } else {
                            apply_untile(out)
                        }
                    };

                    let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                        if framestep.is_some() {
                            filter::push_framestep_or_emit(
                                framestep_graph,
                                framestepped.0,
                                out,
                                framestep,
                                |f| apply_tile(f),
                            )
                        } else {
                            apply_tile(out)
                        }
                    };
                    let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(mpd_args) = mpdecimate {
                            filter::mpdecimate_push_frame(
                                mpdecimate_graph,
                                mpdecimated.0,
                                out,
                                mpd_args,
                                |mpd| apply_framestep(mpd),
                            )
                        } else {
                            apply_framestep(out)
                        }
                    };
                    let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(dc_args) = decimate {
                            filter::decimate_push_frame(
                                decimate_graph,
                                decimated.0,
                                out,
                                dc_args,
                                |dc| apply_mpdecimate(dc),
                            )?;
                        } else {
                            apply_mpdecimate(out)?;
                        }
                        Ok(())
                    };
                    let mut apply_pullup = |out: *mut AVFrame| -> Result<()> {
                        if let Some(pu_args) = pullup {
                            filter::pullup_push_frame(
                                pullup_graph,
                                pulledup.0,
                                out,
                                pu_args,
                                |pu| apply_decimate(pu),
                            )?;
                        } else {
                            apply_decimate(out)?;
                        }
                        Ok(())
                    };
                    let mut apply_telecine = |out: *mut AVFrame| -> Result<()> {
                        if let Some(tc_args) = telecine {
                            filter::telecine_push_frame(
                                telecine_graph,
                                telecined.0,
                                out,
                                tc_args,
                                |tc| apply_pullup(tc),
                            )?;
                        } else {
                            apply_pullup(out)?;
                        }
                        Ok(())
                    };
                    let mut apply_framepack = |out: *mut AVFrame| -> Result<()> {
                        if let Some(fp_args) = framepack {
                            filter::framepack_push_frame(
                                framepack_graph,
                                framepacked.0,
                                out,
                                fp_args,
                                |packed| apply_telecine(packed),
                            )?;
                        } else {
                            apply_telecine(out)?;
                        }
                        Ok(())
                    };
                    if let Some(dw_args) = doubleweave {
                        filter::doubleweave_push_frame(
                            doubleweave_graph,
                            doubleweaved.0,
                            out,
                            dw_args,
                            |doubled| apply_framepack(doubled),
                        )?;
                    } else {
                        apply_framepack(out)?;
                    }
                    Ok(())
                })?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = doubleweave {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::doubleweave_push_frame(
                    doubleweave_graph,
                    doubleweaved.0,
                    send,
                    args,
                    |doubled| {
                        let mut out = doubled;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let mut finish = |mut out: *mut AVFrame| -> Result<()> {
                            let out = if let Some(args) = freezedetect {
                                let src_fmt = (*out).format;
                                filter::freezedetect_frame(
                                    freezedetect_graph,
                                    freezedetectd.0,
                                    out,
                                    args,
                                )?;
                                let o = freezedetectd.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = pseudocolor {
                                let src_fmt = (*out).format;
                                filter::pseudocolor_frame(
                                    pseudocolor_graph,
                                    pseudocolored.0,
                                    out,
                                    args,
                                )?;
                                let o = pseudocolored.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = colorspace {
                                filter::colorspace_frame(
                                    colorspace_graph,
                                    colorspaced.0,
                                    out,
                                    args,
                                )?;
                                colorspaced.0
                            } else {
                                out
                            };
                            let (out, mut format_done) = if let Some(args) = zscale {
                                let fmt = pix_fmt.ok_or(
                                    "--zscale requires --pix-fmt for format= after zscale",
                                )?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown zscale output pixel format".into());
                                }
                                filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                                (zscaled.0, true)
                            } else {
                                (out, format_done)
                            };
                            let (out, format_done) = if let Some(args) = tonemap {
                                let fmt = pix_fmt.ok_or(
                                    "--tonemap requires --pix-fmt for format= after tonemap",
                                )?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown tonemap output pixel format".into());
                                }
                                filter::tonemap_frame(
                                    tonemap_graph,
                                    tonemapped.0,
                                    out,
                                    args,
                                    &name,
                                )?;
                                (tonemapped.0, format_done)
                            } else {
                                (out, format_done)
                            };
                            let out = if let Some(fmt) = pix_fmt {
                                if !format_done && (*out).format != fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                                    converted.0
                                } else {
                                    out
                                }
                            } else {
                                out
                            };
                            if minterpolate.is_some() || fps.is_some() {
                                filter::temporal_push_frame(
                                    minterpolate_graph,
                                    minterpolate_dst.0,
                                    minterpolate,
                                    fps_graph,
                                    fps_dst.0,
                                    fps,
                                    out,
                                    |o| {
                                        send_encoder_frame(
                                            encoder, output, packet, index, o, stats,
                                        )?;
                                        emitted += 1;
                                        Ok(())
                                    },
                                )?;
                            } else {
                                send_encoder_frame(encoder, output, packet, index, out, stats)?;
                                emitted += 1;
                            }
                            Ok(())
                        };
                        let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                            if shuffleframes.is_some() {
                                filter::push_shuffleframes_or_emit(
                                    shuffleframes_graph,
                                    shuffled.0,
                                    out,
                                    shuffleframes.as_deref(),
                                    |f| finish(f),
                                )
                            } else {
                                finish(out)
                            }
                        };
                        let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                            if untile.is_some() {
                                filter::push_untile_or_emit(
                                    untile_graph,
                                    untiled.0,
                                    out,
                                    untile.as_deref(),
                                    |f| apply_shuffleframes(f),
                                )
                            } else {
                                apply_shuffleframes(out)
                            }
                        };
                        let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                            if tile.is_some() {
                                filter::push_tile_or_emit(
                                    tile_graph,
                                    tiled.0,
                                    out,
                                    tile.as_deref(),
                                    |f| apply_untile(f),
                                )
                            } else {
                                apply_untile(out)
                            }
                        };

                        let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                            if framestep.is_some() {
                                filter::push_framestep_or_emit(
                                    framestep_graph,
                                    framestepped.0,
                                    out,
                                    framestep,
                                    |f| apply_tile(f),
                                )
                            } else {
                                apply_tile(out)
                            }
                        };
                        let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(mpd_args) = mpdecimate {
                                filter::mpdecimate_push_frame(
                                    mpdecimate_graph,
                                    mpdecimated.0,
                                    out,
                                    mpd_args,
                                    |mpd| apply_framestep(mpd),
                                )
                            } else {
                                apply_framestep(out)
                            }
                        };
                        let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(dc_args) = decimate {
                                filter::decimate_push_frame(
                                    decimate_graph,
                                    decimated.0,
                                    out,
                                    dc_args,
                                    |dc| apply_mpdecimate(dc),
                                )?;
                            } else {
                                apply_mpdecimate(out)?;
                            }
                            Ok(())
                        };
                        let mut apply_pullup = |out: *mut AVFrame| -> Result<()> {
                            if let Some(pu_args) = pullup {
                                filter::pullup_push_frame(
                                    pullup_graph,
                                    pulledup.0,
                                    out,
                                    pu_args,
                                    |pu| apply_decimate(pu),
                                )?;
                            } else {
                                apply_decimate(out)?;
                            }
                            Ok(())
                        };
                        let mut apply_telecine = |out: *mut AVFrame| -> Result<()> {
                            if let Some(tc_args) = telecine {
                                filter::telecine_push_frame(
                                    telecine_graph,
                                    telecined.0,
                                    out,
                                    tc_args,
                                    |tc| apply_pullup(tc),
                                )?;
                            } else {
                                apply_pullup(out)?;
                            }
                            Ok(())
                        };
                        if let Some(fp_args) = framepack {
                            filter::framepack_push_frame(
                                framepack_graph,
                                framepacked.0,
                                out,
                                fp_args,
                                |packed| apply_telecine(packed),
                            )?;
                        } else {
                            apply_telecine(out)?;
                        }
                        Ok(())
                    },
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = framepack {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                let mut finish = |mut out: *mut AVFrame| -> Result<()> {
                    let out = if let Some(args) = freezedetect {
                        let src_fmt = (*out).format;
                        filter::freezedetect_frame(freezedetect_graph, freezedetectd.0, out, args)?;
                        let o = freezedetectd.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = pseudocolor {
                        let src_fmt = (*out).format;
                        filter::pseudocolor_frame(pseudocolor_graph, pseudocolored.0, out, args)?;
                        let o = pseudocolored.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = colorspace {
                        filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                        colorspaced.0
                    } else {
                        out
                    };
                    let (out, mut format_done) = if let Some(args) = zscale {
                        let fmt = pix_fmt
                            .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                        (zscaled.0, true)
                    } else {
                        (out, format_done)
                    };
                    let (out, format_done) = if let Some(args) = tonemap {
                        let fmt = pix_fmt
                            .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                        (tonemapped.0, format_done)
                    } else {
                        (out, format_done)
                    };
                    let out = if let Some(fmt) = pix_fmt {
                        if !format_done && (*out).format != fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                            converted.0
                        } else {
                            out
                        }
                    } else {
                        out
                    };
                    if minterpolate.is_some() || fps.is_some() {
                        filter::temporal_push_frame(
                            minterpolate_graph,
                            minterpolate_dst.0,
                            minterpolate,
                            fps_graph,
                            fps_dst.0,
                            fps,
                            out,
                            |o| {
                                send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                    } else {
                        send_encoder_frame(encoder, output, packet, index, out, stats)?;
                        emitted += 1;
                    }
                    Ok(())
                };
                let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                    if shuffleframes.is_some() {
                        filter::push_shuffleframes_or_emit(
                            shuffleframes_graph,
                            shuffled.0,
                            out,
                            shuffleframes.as_deref(),
                            |f| finish(f),
                        )
                    } else {
                        finish(out)
                    }
                };
                let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                    if untile.is_some() {
                        filter::push_untile_or_emit(
                            untile_graph,
                            untiled.0,
                            out,
                            untile.as_deref(),
                            |f| apply_shuffleframes(f),
                        )
                    } else {
                        apply_shuffleframes(out)
                    }
                };
                let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                    if tile.is_some() {
                        filter::push_tile_or_emit(tile_graph, tiled.0, out, tile.as_deref(), |f| {
                            apply_untile(f)
                        })
                    } else {
                        apply_untile(out)
                    }
                };

                let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                    if framestep.is_some() {
                        filter::push_framestep_or_emit(
                            framestep_graph,
                            framestepped.0,
                            out,
                            framestep,
                            |f| apply_tile(f),
                        )
                    } else {
                        apply_tile(out)
                    }
                };
                let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                    if let Some(mpd_args) = mpdecimate {
                        filter::mpdecimate_push_frame(
                            mpdecimate_graph,
                            mpdecimated.0,
                            out,
                            mpd_args,
                            |mpd| apply_framestep(mpd),
                        )
                    } else {
                        apply_framestep(out)
                    }
                };
                let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                    if let Some(dc_args) = decimate {
                        filter::decimate_push_frame(
                            decimate_graph,
                            decimated.0,
                            out,
                            dc_args,
                            |dc| apply_mpdecimate(dc),
                        )?;
                    } else {
                        apply_mpdecimate(out)?;
                    }
                    Ok(())
                };
                let mut apply_pullup = |out: *mut AVFrame| -> Result<()> {
                    if let Some(pu_args) = pullup {
                        filter::pullup_push_frame(pullup_graph, pulledup.0, out, pu_args, |pu| {
                            apply_decimate(pu)
                        })?;
                    } else {
                        apply_decimate(out)?;
                    }
                    Ok(())
                };
                let mut apply_telecine = |out: *mut AVFrame| -> Result<()> {
                    if let Some(tc_args) = telecine {
                        filter::telecine_push_frame(
                            telecine_graph,
                            telecined.0,
                            out,
                            tc_args,
                            |tc| apply_pullup(tc),
                        )?;
                    } else {
                        apply_pullup(out)?;
                    }
                    Ok(())
                };
                filter::framepack_push_frame(
                    framepack_graph,
                    framepacked.0,
                    send,
                    args,
                    |packed| apply_telecine(packed),
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = telecine {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::telecine_push_frame(telecine_graph, telecined.0, send, args, |tc| {
                    let mut after_decimate = |mut out: *mut AVFrame| -> Result<()> {
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let out = if let Some(args) = freezedetect {
                            let src_fmt = (*out).format;
                            filter::freezedetect_frame(
                                freezedetect_graph,
                                freezedetectd.0,
                                out,
                                args,
                            )?;
                            let o = freezedetectd.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = pseudocolor {
                            let src_fmt = (*out).format;
                            filter::pseudocolor_frame(
                                pseudocolor_graph,
                                pseudocolored.0,
                                out,
                                args,
                            )?;
                            let o = pseudocolored.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = colorspace {
                            filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                            colorspaced.0
                        } else {
                            out
                        };
                        let (out, mut format_done) = if let Some(args) = zscale {
                            let fmt = pix_fmt
                                .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                            (zscaled.0, true)
                        } else {
                            (out, format_done)
                        };
                        let (out, format_done) = if let Some(args) = tonemap {
                            let fmt = pix_fmt
                                .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                            (tonemapped.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if minterpolate.is_some() || fps.is_some() {
                            filter::temporal_push_frame(
                                minterpolate_graph,
                                minterpolate_dst.0,
                                minterpolate,
                                fps_graph,
                                fps_dst.0,
                                fps,
                                out,
                                |o| {
                                    send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            send_encoder_frame(encoder, output, packet, index, out, stats)?;
                            emitted += 1;
                        }
                        Ok(())
                    };
                    let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                        if shuffleframes.is_some() {
                            filter::push_shuffleframes_or_emit(
                                shuffleframes_graph,
                                shuffled.0,
                                out,
                                shuffleframes.as_deref(),
                                |f| after_decimate(f),
                            )
                        } else {
                            after_decimate(out)
                        }
                    };
                    let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                        if untile.is_some() {
                            filter::push_untile_or_emit(
                                untile_graph,
                                untiled.0,
                                out,
                                untile.as_deref(),
                                |f| apply_shuffleframes(f),
                            )
                        } else {
                            apply_shuffleframes(out)
                        }
                    };
                    let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                        if tile.is_some() {
                            filter::push_tile_or_emit(
                                tile_graph,
                                tiled.0,
                                out,
                                tile.as_deref(),
                                |f| apply_untile(f),
                            )
                        } else {
                            apply_untile(out)
                        }
                    };

                    let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                        if framestep.is_some() {
                            filter::push_framestep_or_emit(
                                framestep_graph,
                                framestepped.0,
                                out,
                                framestep,
                                |f| apply_tile(f),
                            )
                        } else {
                            apply_tile(out)
                        }
                    };
                    let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(mpd_args) = mpdecimate {
                            filter::mpdecimate_push_frame(
                                mpdecimate_graph,
                                mpdecimated.0,
                                out,
                                mpd_args,
                                |mpd| apply_framestep(mpd),
                            )
                        } else {
                            apply_framestep(out)
                        }
                    };
                    let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(dc_args) = decimate {
                            filter::decimate_push_frame(
                                decimate_graph,
                                decimated.0,
                                out,
                                dc_args,
                                |dc| apply_mpdecimate(dc),
                            )
                        } else {
                            apply_mpdecimate(out)
                        }
                    };
                    if let Some(pu_args) = pullup {
                        filter::pullup_push_frame(pullup_graph, pulledup.0, tc, pu_args, |pu| {
                            apply_decimate(pu)
                        })
                    } else {
                        apply_decimate(tc)
                    }
                })?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = pullup {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                let mut after_decimate = |mut out: *mut AVFrame| -> Result<()> {
                    let mut out = out;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                        out = converted.0;
                    }
                    let out = if let Some(args) = freezedetect {
                        let src_fmt = (*out).format;
                        filter::freezedetect_frame(freezedetect_graph, freezedetectd.0, out, args)?;
                        let o = freezedetectd.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = pseudocolor {
                        let src_fmt = (*out).format;
                        filter::pseudocolor_frame(pseudocolor_graph, pseudocolored.0, out, args)?;
                        let o = pseudocolored.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = colorspace {
                        filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                        colorspaced.0
                    } else {
                        out
                    };
                    let (out, mut format_done) = if let Some(args) = zscale {
                        let fmt = pix_fmt
                            .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                        (zscaled.0, true)
                    } else {
                        (out, format_done)
                    };
                    let (out, format_done) = if let Some(args) = tonemap {
                        let fmt = pix_fmt
                            .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                        (tonemapped.0, format_done)
                    } else {
                        (out, format_done)
                    };
                    let out = if let Some(fmt) = pix_fmt {
                        if !format_done && (*out).format != fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                            converted.0
                        } else {
                            out
                        }
                    } else {
                        out
                    };
                    if minterpolate.is_some() || fps.is_some() {
                        filter::temporal_push_frame(
                            minterpolate_graph,
                            minterpolate_dst.0,
                            minterpolate,
                            fps_graph,
                            fps_dst.0,
                            fps,
                            out,
                            |o| {
                                send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                    } else {
                        send_encoder_frame(encoder, output, packet, index, out, stats)?;
                        emitted += 1;
                    }
                    Ok(())
                };
                let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                    if shuffleframes.is_some() {
                        filter::push_shuffleframes_or_emit(
                            shuffleframes_graph,
                            shuffled.0,
                            out,
                            shuffleframes.as_deref(),
                            |f| after_decimate(f),
                        )
                    } else {
                        after_decimate(out)
                    }
                };
                let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                    if untile.is_some() {
                        filter::push_untile_or_emit(
                            untile_graph,
                            untiled.0,
                            out,
                            untile.as_deref(),
                            |f| apply_shuffleframes(f),
                        )
                    } else {
                        apply_shuffleframes(out)
                    }
                };
                let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                    if tile.is_some() {
                        filter::push_tile_or_emit(tile_graph, tiled.0, out, tile.as_deref(), |f| {
                            apply_untile(f)
                        })
                    } else {
                        apply_untile(out)
                    }
                };

                let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                    if framestep.is_some() {
                        filter::push_framestep_or_emit(
                            framestep_graph,
                            framestepped.0,
                            out,
                            framestep,
                            |f| apply_tile(f),
                        )
                    } else {
                        apply_tile(out)
                    }
                };
                let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                    if let Some(mpd_args) = mpdecimate {
                        filter::mpdecimate_push_frame(
                            mpdecimate_graph,
                            mpdecimated.0,
                            out,
                            mpd_args,
                            |mpd| apply_framestep(mpd),
                        )
                    } else {
                        apply_framestep(out)
                    }
                };
                let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                    if let Some(dc_args) = decimate {
                        filter::decimate_push_frame(
                            decimate_graph,
                            decimated.0,
                            out,
                            dc_args,
                            |dc| apply_mpdecimate(dc),
                        )
                    } else {
                        apply_mpdecimate(out)
                    }
                };
                filter::pullup_push_frame(pullup_graph, pulledup.0, send, args, |pu| {
                    apply_decimate(pu)
                })?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = decimate {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                let mut after_tile = |mut out: *mut AVFrame| -> Result<()> {
                    let mut out = out;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                        out = converted.0;
                    }
                    let out = if let Some(args) = freezedetect {
                        let src_fmt = (*out).format;
                        filter::freezedetect_frame(freezedetect_graph, freezedetectd.0, out, args)?;
                        let o = freezedetectd.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = pseudocolor {
                        let src_fmt = (*out).format;
                        filter::pseudocolor_frame(pseudocolor_graph, pseudocolored.0, out, args)?;
                        let o = pseudocolored.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = colorspace {
                        filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                        colorspaced.0
                    } else {
                        out
                    };
                    let (out, mut format_done) = if let Some(args) = zscale {
                        let fmt = pix_fmt
                            .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                        (zscaled.0, true)
                    } else {
                        (out, format_done)
                    };
                    let (out, format_done) = if let Some(args) = tonemap {
                        let fmt = pix_fmt
                            .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                        (tonemapped.0, format_done)
                    } else {
                        (out, format_done)
                    };
                    let out = if let Some(fmt) = pix_fmt {
                        if !format_done && (*out).format != fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                            converted.0
                        } else {
                            out
                        }
                    } else {
                        out
                    };
                    if minterpolate.is_some() || fps.is_some() {
                        filter::temporal_push_frame(
                            minterpolate_graph,
                            minterpolate_dst.0,
                            minterpolate,
                            fps_graph,
                            fps_dst.0,
                            fps,
                            out,
                            |o| {
                                send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                    } else {
                        send_encoder_frame(encoder, output, packet, index, out, stats)?;
                        emitted += 1;
                    }
                    Ok(())
                };
                let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                    if shuffleframes.is_some() {
                        filter::push_shuffleframes_or_emit(
                            shuffleframes_graph,
                            shuffled.0,
                            out,
                            shuffleframes.as_deref(),
                            |f| after_tile(f),
                        )
                    } else {
                        after_tile(out)
                    }
                };
                let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                    if untile.is_some() {
                        filter::push_untile_or_emit(
                            untile_graph,
                            untiled.0,
                            out,
                            untile.as_deref(),
                            |f| apply_shuffleframes(f),
                        )
                    } else {
                        apply_shuffleframes(out)
                    }
                };
                let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                    if framestep.is_some() {
                        filter::push_framestep_or_emit(
                            framestep_graph,
                            framestepped.0,
                            out,
                            framestep,
                            |f| apply_untile(f),
                        )
                    } else {
                        apply_untile(out)
                    }
                };
                let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                    if let Some(mpd_args) = mpdecimate {
                        filter::mpdecimate_push_frame(
                            mpdecimate_graph,
                            mpdecimated.0,
                            out,
                            mpd_args,
                            |mpd| apply_framestep(mpd),
                        )
                    } else {
                        apply_framestep(out)
                    }
                };
                filter::decimate_push_frame(decimate_graph, decimated.0, send, args, |dc| {
                    apply_mpdecimate(dc)
                })?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = mpdecimate {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                let mut after_tile = |mut out: *mut AVFrame| -> Result<()> {
                    let mut out = out;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                        out = converted.0;
                    }
                    let out = if let Some(args) = freezedetect {
                        let src_fmt = (*out).format;
                        filter::freezedetect_frame(freezedetect_graph, freezedetectd.0, out, args)?;
                        let o = freezedetectd.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = pseudocolor {
                        let src_fmt = (*out).format;
                        filter::pseudocolor_frame(pseudocolor_graph, pseudocolored.0, out, args)?;
                        let o = pseudocolored.0;
                        if (*o).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                            converted.0
                        } else {
                            o
                        }
                    } else {
                        out
                    };
                    let out = if let Some(args) = colorspace {
                        filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                        colorspaced.0
                    } else {
                        out
                    };
                    let (out, mut format_done) = if let Some(args) = zscale {
                        let fmt = pix_fmt
                            .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                        (zscaled.0, true)
                    } else {
                        (out, format_done)
                    };
                    let (out, format_done) = if let Some(args) = tonemap {
                        let fmt = pix_fmt
                            .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                        (tonemapped.0, format_done)
                    } else {
                        (out, format_done)
                    };
                    let out = if let Some(fmt) = pix_fmt {
                        if !format_done && (*out).format != fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                            converted.0
                        } else {
                            out
                        }
                    } else {
                        out
                    };
                    if minterpolate.is_some() || fps.is_some() {
                        filter::temporal_push_frame(
                            minterpolate_graph,
                            minterpolate_dst.0,
                            minterpolate,
                            fps_graph,
                            fps_dst.0,
                            fps,
                            out,
                            |o| {
                                send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                    } else {
                        send_encoder_frame(encoder, output, packet, index, out, stats)?;
                        emitted += 1;
                    }
                    Ok(())
                };
                let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                    if shuffleframes.is_some() {
                        filter::push_shuffleframes_or_emit(
                            shuffleframes_graph,
                            shuffled.0,
                            out,
                            shuffleframes.as_deref(),
                            |f| after_tile(f),
                        )
                    } else {
                        after_tile(out)
                    }
                };
                let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                    if untile.is_some() {
                        filter::push_untile_or_emit(
                            untile_graph,
                            untiled.0,
                            out,
                            untile.as_deref(),
                            |f| apply_shuffleframes(f),
                        )
                    } else {
                        apply_shuffleframes(out)
                    }
                };
                let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                    if framestep.is_some() {
                        filter::push_framestep_or_emit(
                            framestep_graph,
                            framestepped.0,
                            out,
                            framestep,
                            |f| apply_untile(f),
                        )
                    } else {
                        apply_untile(out)
                    }
                };
                filter::mpdecimate_push_frame(
                    mpdecimate_graph,
                    mpdecimated.0,
                    send,
                    args,
                    |out| apply_framestep(out),
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = reverse {
                if shuffleframes.is_none() {
                    let src_fmt = (*send).format;
                    let mut emitted = 0u64;
                    filter::reverse_push_frame(
                        reverse_graph,
                        reversed.0,
                        send,
                        args,
                        |out| unsafe {
                            filter::push_loop_or_emit(
                                loop_graph,
                                looped.0,
                                out,
                                r#loop.as_deref(),
                                |out| unsafe {
                                    filter::push_thumbnail_or_emit(
                                        thumbnail_graph,
                                        thumbnailed.0,
                                        out,
                                        thumbnail.as_deref(),
                                        |mut out| {
                                            let mut out = out;
                                            if (*out).format != src_fmt {
                                                convert_pix_fmt_frame(
                                                    fmt_sws,
                                                    converted.0,
                                                    out,
                                                    src_fmt,
                                                )?;
                                                out = converted.0;
                                            }
                                            if minterpolate.is_some() || fps.is_some() {
                                                filter::temporal_push_frame(
                                                    minterpolate_graph,
                                                    minterpolate_dst.0,
                                                    minterpolate,
                                                    fps_graph,
                                                    fps_dst.0,
                                                    fps,
                                                    out,
                                                    |o| {
                                                        send_encoder_frame(
                                                            encoder, output, packet, index, o,
                                                            stats,
                                                        )?;
                                                        emitted += 1;
                                                        Ok(())
                                                    },
                                                )?;
                                            } else {
                                                send_encoder_frame(
                                                    encoder, output, packet, index, out, stats,
                                                )?;
                                                emitted += 1;
                                            }
                                            Ok(())
                                        },
                                    )
                                },
                            )
                        },
                    )?;
                    stats.video_frames += emitted;
                    av_frame_unref(frame.0);
                    continue;
                }
            }
            if let Some(args) = r#loop {
                if shuffleframes.is_none() && reverse.is_none() {
                    let src_fmt = (*send).format;
                    let mut emitted = 0u64;
                    filter::loop_push_frame(loop_graph, looped.0, send, args, |out| unsafe {
                        filter::push_thumbnail_or_emit(
                            thumbnail_graph,
                            thumbnailed.0,
                            out,
                            thumbnail.as_deref(),
                            |mut out| {
                                let mut out = out;
                                if (*out).format != src_fmt {
                                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                                    out = converted.0;
                                }
                                if minterpolate.is_some() || fps.is_some() {
                                    filter::temporal_push_frame(
                                        minterpolate_graph,
                                        minterpolate_dst.0,
                                        minterpolate,
                                        fps_graph,
                                        fps_dst.0,
                                        fps,
                                        out,
                                        |o| {
                                            send_encoder_frame(
                                                encoder, output, packet, index, o, stats,
                                            )?;
                                            emitted += 1;
                                            Ok(())
                                        },
                                    )?;
                                } else {
                                    send_encoder_frame(encoder, output, packet, index, out, stats)?;
                                    emitted += 1;
                                }
                                Ok(())
                            },
                        )
                    })?;
                    stats.video_frames += emitted;
                    av_frame_unref(frame.0);
                    continue;
                }
            }
            if let Some(args) = thumbnail {
                if shuffleframes.is_none() && reverse.is_none() && r#loop.is_none() {
                    let src_fmt = (*send).format;
                    let mut emitted = 0u64;
                    filter::thumbnail_push_frame(
                        thumbnail_graph,
                        thumbnailed.0,
                        send,
                        args,
                        |mut out| {
                            let mut out = out;
                            if (*out).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                                out = converted.0;
                            }
                            if minterpolate.is_some() || fps.is_some() {
                                filter::temporal_push_frame(
                                    minterpolate_graph,
                                    minterpolate_dst.0,
                                    minterpolate,
                                    fps_graph,
                                    fps_dst.0,
                                    fps,
                                    out,
                                    |o| {
                                        send_encoder_frame(
                                            encoder, output, packet, index, o, stats,
                                        )?;
                                        emitted += 1;
                                        Ok(())
                                    },
                                )?;
                            } else {
                                send_encoder_frame(encoder, output, packet, index, out, stats)?;
                                emitted += 1;
                            }
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                    av_frame_unref(frame.0);
                    continue;
                }
            }
            if let Some(args) = shuffleframes {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::shuffleframes_push_frame(
                    shuffleframes_graph,
                    shuffled.0,
                    send,
                    args,
                    |out| unsafe {
                        filter::push_reverse_or_emit(
                            reverse_graph,
                            reversed.0,
                            out,
                            reverse.as_deref(),
                            |out| unsafe {
                                filter::push_loop_or_emit(
                                    loop_graph,
                                    looped.0,
                                    out,
                                    r#loop.as_deref(),
                                    |out| unsafe {
                                        filter::push_thumbnail_or_emit(
                                            thumbnail_graph,
                                            thumbnailed.0,
                                            out,
                                            thumbnail.as_deref(),
                                            |mut out| {
                                                let mut out = out;
                                                if (*out).format != src_fmt {
                                                    convert_pix_fmt_frame(
                                                        fmt_sws,
                                                        converted.0,
                                                        out,
                                                        src_fmt,
                                                    )?;
                                                    out = converted.0;
                                                }
                                                if minterpolate.is_some() || fps.is_some() {
                                                    filter::temporal_push_frame(
                                                        minterpolate_graph,
                                                        minterpolate_dst.0,
                                                        minterpolate,
                                                        fps_graph,
                                                        fps_dst.0,
                                                        fps,
                                                        out,
                                                        |o| {
                                                            send_encoder_frame(
                                                                encoder, output, packet, index, o,
                                                                stats,
                                                            )?;
                                                            emitted += 1;
                                                            Ok(())
                                                        },
                                                    )?;
                                                } else {
                                                    send_encoder_frame(
                                                        encoder, output, packet, index, out, stats,
                                                    )?;
                                                    emitted += 1;
                                                }
                                                Ok(())
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = tile {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::tile_push_frame(tile_graph, tiled.0, send, args, |out| {
                    filter::push_untile_or_emit(
                        untile_graph,
                        untiled.0,
                        out,
                        untile.as_deref(),
                        |out| {
                            filter::push_shuffleframes_or_emit(
                                shuffleframes_graph,
                                shuffled.0,
                                out,
                                shuffleframes.as_deref(),
                                |out| {
                                    filter::push_reverse_or_emit(
                                        reverse_graph,
                                        reversed.0,
                                        out,
                                        reverse.as_deref(),
                                        |out| unsafe {
                                            filter::push_loop_or_emit(
                                                loop_graph,
                                                looped.0,
                                                out,
                                                r#loop.as_deref(),
                                                |out| unsafe {
                                                    filter::push_thumbnail_or_emit(
                                                        thumbnail_graph,
                                                        thumbnailed.0,
                                                        out,
                                                        thumbnail.as_deref(),
                                                        |mut out| {
                                                            let mut out = out;
                                                            if (*out).format != src_fmt {
                                                                convert_pix_fmt_frame(
                                                                    fmt_sws,
                                                                    converted.0,
                                                                    out,
                                                                    src_fmt,
                                                                )?;
                                                                out = converted.0;
                                                            }
                                                            let out =
                                                                if let Some(args) = freezedetect {
                                                                    let src_fmt = (*out).format;
                                                                    filter::freezedetect_frame(
                                                                        freezedetect_graph,
                                                                        freezedetectd.0,
                                                                        out,
                                                                        args,
                                                                    )?;
                                                                    let o = freezedetectd.0;
                                                                    if (*o).format != src_fmt {
                                                                        convert_pix_fmt_frame(
                                                                            fmt_sws,
                                                                            converted.0,
                                                                            o,
                                                                            src_fmt,
                                                                        )?;
                                                                        converted.0
                                                                    } else {
                                                                        o
                                                                    }
                                                                } else {
                                                                    out
                                                                };
                                                            let out =
                                                                if let Some(args) = pseudocolor {
                                                                    let src_fmt = (*out).format;
                                                                    filter::pseudocolor_frame(
                                                                        pseudocolor_graph,
                                                                        pseudocolored.0,
                                                                        out,
                                                                        args,
                                                                    )?;
                                                                    let o = pseudocolored.0;
                                                                    if (*o).format != src_fmt {
                                                                        convert_pix_fmt_frame(
                                                                            fmt_sws,
                                                                            converted.0,
                                                                            o,
                                                                            src_fmt,
                                                                        )?;
                                                                        converted.0
                                                                    } else {
                                                                        o
                                                                    }
                                                                } else {
                                                                    out
                                                                };
                                                            let out = if let Some(args) = colorspace
                                                            {
                                                                filter::colorspace_frame(
                                                                    colorspace_graph,
                                                                    colorspaced.0,
                                                                    out,
                                                                    args,
                                                                )?;
                                                                colorspaced.0
                                                            } else {
                                                                out
                                                            };
                                                            let (out, mut format_done) =
                                                                if let Some(args) = zscale {
                                                                    let fmt =
                                pix_fmt.ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                                                                    let name = string(
                                                                        av_get_pix_fmt_name(fmt),
                                                                    );
                                                                    if name.is_empty() {
                                                                        return Err("unknown zscale output pixel format".into());
                                                                    }
                                                                    filter::zscale_frame(
                                                                        zscale_graph,
                                                                        zscaled.0,
                                                                        out,
                                                                        args,
                                                                        &name,
                                                                    )?;
                                                                    (zscaled.0, true)
                                                                } else {
                                                                    (out, format_done)
                                                                };
                                                            let (out, format_done) = if let Some(
                                                                args,
                                                            ) = tonemap
                                                            {
                                                                let fmt = pix_fmt
                                .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                                                                let name = string(
                                                                    av_get_pix_fmt_name(fmt),
                                                                );
                                                                if name.is_empty() {
                                                                    return Err("unknown tonemap output pixel format".into());
                                                                }
                                                                filter::tonemap_frame(
                                                                    tonemap_graph,
                                                                    tonemapped.0,
                                                                    out,
                                                                    args,
                                                                    &name,
                                                                )?;
                                                                (tonemapped.0, format_done)
                                                            } else {
                                                                (out, format_done)
                                                            };
                                                            let out = if let Some(fmt) = pix_fmt {
                                                                if !format_done
                                                                    && (*out).format != fmt
                                                                {
                                                                    convert_pix_fmt_frame(
                                                                        fmt_sws,
                                                                        converted.0,
                                                                        out,
                                                                        fmt,
                                                                    )?;
                                                                    converted.0
                                                                } else {
                                                                    out
                                                                }
                                                            } else {
                                                                out
                                                            };
                                                            if minterpolate.is_some()
                                                                || fps.is_some()
                                                            {
                                                                filter::temporal_push_frame(
                                                                    minterpolate_graph,
                                                                    minterpolate_dst.0,
                                                                    minterpolate,
                                                                    fps_graph,
                                                                    fps_dst.0,
                                                                    fps,
                                                                    out,
                                                                    |o| {
                                                                        send_encoder_frame(
                                                                            encoder, output,
                                                                            packet, index, o,
                                                                            stats,
                                                                        )?;
                                                                        emitted += 1;
                                                                        Ok(())
                                                                    },
                                                                )?;
                                                            } else {
                                                                send_encoder_frame(
                                                                    encoder, output, packet, index,
                                                                    out, stats,
                                                                )?;
                                                                emitted += 1;
                                                            }
                                                            Ok(())
                                                        },
                                                    )
                                                },
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                })?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = untile {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::untile_push_frame(untile_graph, untiled.0, send, args, |out| unsafe {
                    filter::push_shuffleframes_or_emit(
                        shuffleframes_graph,
                        shuffled.0,
                        out,
                        shuffleframes.as_deref(),
                        |out| {
                            filter::push_reverse_or_emit(
                                reverse_graph,
                                reversed.0,
                                out,
                                reverse.as_deref(),
                                |out| unsafe {
                                    filter::push_loop_or_emit(
                                        loop_graph,
                                        looped.0,
                                        out,
                                        r#loop.as_deref(),
                                        |out| unsafe {
                                            filter::push_thumbnail_or_emit(
                                                thumbnail_graph,
                                                thumbnailed.0,
                                                out,
                                                thumbnail.as_deref(),
                                                |mut out| {
                                                    let mut out = out;
                                                    if (*out).format != src_fmt {
                                                        convert_pix_fmt_frame(
                                                            fmt_sws,
                                                            converted.0,
                                                            out,
                                                            src_fmt,
                                                        )?;
                                                        out = converted.0;
                                                    }
                                                    let out = if let Some(args) = freezedetect {
                                                        let src_fmt = (*out).format;
                                                        filter::freezedetect_frame(
                                                            freezedetect_graph,
                                                            freezedetectd.0,
                                                            out,
                                                            args,
                                                        )?;
                                                        let o = freezedetectd.0;
                                                        if (*o).format != src_fmt {
                                                            convert_pix_fmt_frame(
                                                                fmt_sws,
                                                                converted.0,
                                                                o,
                                                                src_fmt,
                                                            )?;
                                                            converted.0
                                                        } else {
                                                            o
                                                        }
                                                    } else {
                                                        out
                                                    };
                                                    let out = if let Some(args) = pseudocolor {
                                                        let src_fmt = (*out).format;
                                                        filter::pseudocolor_frame(
                                                            pseudocolor_graph,
                                                            pseudocolored.0,
                                                            out,
                                                            args,
                                                        )?;
                                                        let o = pseudocolored.0;
                                                        if (*o).format != src_fmt {
                                                            convert_pix_fmt_frame(
                                                                fmt_sws,
                                                                converted.0,
                                                                o,
                                                                src_fmt,
                                                            )?;
                                                            converted.0
                                                        } else {
                                                            o
                                                        }
                                                    } else {
                                                        out
                                                    };
                                                    let out = if let Some(args) = colorspace {
                                                        filter::colorspace_frame(
                                                            colorspace_graph,
                                                            colorspaced.0,
                                                            out,
                                                            args,
                                                        )?;
                                                        colorspaced.0
                                                    } else {
                                                        out
                                                    };
                                                    let (out, mut format_done) = if let Some(args) =
                                                        zscale
                                                    {
                                                        let fmt =
                                pix_fmt.ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                                                        let name = string(av_get_pix_fmt_name(fmt));
                                                        if name.is_empty() {
                                                            return Err("unknown zscale output pixel format".into());
                                                        }
                                                        filter::zscale_frame(
                                                            zscale_graph,
                                                            zscaled.0,
                                                            out,
                                                            args,
                                                            &name,
                                                        )?;
                                                        (zscaled.0, true)
                                                    } else {
                                                        (out, format_done)
                                                    };
                                                    let (out, format_done) = if let Some(args) =
                                                        tonemap
                                                    {
                                                        let fmt = pix_fmt
                                .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                                                        let name = string(av_get_pix_fmt_name(fmt));
                                                        if name.is_empty() {
                                                            return Err("unknown tonemap output pixel format".into());
                                                        }
                                                        filter::tonemap_frame(
                                                            tonemap_graph,
                                                            tonemapped.0,
                                                            out,
                                                            args,
                                                            &name,
                                                        )?;
                                                        (tonemapped.0, format_done)
                                                    } else {
                                                        (out, format_done)
                                                    };
                                                    let out = if let Some(fmt) = pix_fmt {
                                                        if !format_done && (*out).format != fmt {
                                                            convert_pix_fmt_frame(
                                                                fmt_sws,
                                                                converted.0,
                                                                out,
                                                                fmt,
                                                            )?;
                                                            converted.0
                                                        } else {
                                                            out
                                                        }
                                                    } else {
                                                        out
                                                    };
                                                    if minterpolate.is_some() || fps.is_some() {
                                                        filter::temporal_push_frame(
                                                            minterpolate_graph,
                                                            minterpolate_dst.0,
                                                            minterpolate,
                                                            fps_graph,
                                                            fps_dst.0,
                                                            fps,
                                                            out,
                                                            |o| {
                                                                send_encoder_frame(
                                                                    encoder, output, packet, index,
                                                                    o, stats,
                                                                )?;
                                                                emitted += 1;
                                                                Ok(())
                                                            },
                                                        )?;
                                                    } else {
                                                        send_encoder_frame(
                                                            encoder, output, packet, index, out,
                                                            stats,
                                                        )?;
                                                        emitted += 1;
                                                    }
                                                    Ok(())
                                                },
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                })?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            if let Some(args) = framestep {
                let src_fmt = (*send).format;
                let mut emitted = 0u64;
                filter::framestep_push_frame(
                    framestep_graph,
                    framestepped.0,
                    send,
                    args,
                    |mut out| {
                        let mut out = out;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let out = if let Some(args) = freezedetect {
                            let src_fmt = (*out).format;
                            filter::freezedetect_frame(
                                freezedetect_graph,
                                freezedetectd.0,
                                out,
                                args,
                            )?;
                            let o = freezedetectd.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = pseudocolor {
                            let src_fmt = (*out).format;
                            filter::pseudocolor_frame(
                                pseudocolor_graph,
                                pseudocolored.0,
                                out,
                                args,
                            )?;
                            let o = pseudocolored.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = colorspace {
                            filter::colorspace_frame(colorspace_graph, colorspaced.0, out, args)?;
                            colorspaced.0
                        } else {
                            out
                        };
                        let (out, mut format_done) = if let Some(args) = zscale {
                            let fmt = pix_fmt
                                .ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            filter::zscale_frame(zscale_graph, zscaled.0, out, args, &name)?;
                            (zscaled.0, true)
                        } else {
                            (out, format_done)
                        };
                        let (out, format_done) = if let Some(args) = tonemap {
                            let fmt = pix_fmt
                                .ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            filter::tonemap_frame(tonemap_graph, tonemapped.0, out, args, &name)?;
                            (tonemapped.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if minterpolate.is_some() || fps.is_some() {
                            filter::temporal_push_frame(
                                minterpolate_graph,
                                minterpolate_dst.0,
                                minterpolate,
                                fps_graph,
                                fps_dst.0,
                                fps,
                                out,
                                |o| {
                                    send_encoder_frame(encoder, output, packet, index, o, stats)?;
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            send_encoder_frame(encoder, output, packet, index, out, stats)?;
                            emitted += 1;
                        }
                        Ok(())
                    },
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            let send = if let Some(args) = freezedetect {
                let src_fmt = (*send).format;
                filter::freezedetect_frame(freezedetect_graph, freezedetectd.0, send, args)?;
                let out = freezedetectd.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = pseudocolor {
                let src_fmt = (*send).format;
                filter::pseudocolor_frame(pseudocolor_graph, pseudocolored.0, send, args)?;
                let out = pseudocolored.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, out, src_fmt)?;
                    converted.0
                } else {
                    out
                }
            } else {
                send
            };
            let send = if let Some(args) = colorspace {
                filter::colorspace_frame(colorspace_graph, colorspaced.0, send, args)?;
                colorspaced.0
            } else {
                send
            };
            let (send, format_done) = if let Some(args) = zscale {
                let fmt = pix_fmt.ok_or("--zscale requires --pix-fmt for format= after zscale")?;
                let name = string(av_get_pix_fmt_name(fmt));
                if name.is_empty() {
                    return Err("unknown zscale output pixel format".into());
                }
                filter::zscale_frame(zscale_graph, zscaled.0, send, args, &name)?;
                (zscaled.0, true)
            } else {
                (send, format_done)
            };
            let (send, format_done) = if let Some(args) = tonemap {
                let fmt =
                    pix_fmt.ok_or("--tonemap requires --pix-fmt for format= after tonemap")?;
                let name = string(av_get_pix_fmt_name(fmt));
                if name.is_empty() {
                    return Err("unknown tonemap output pixel format".into());
                }
                filter::tonemap_frame(tonemap_graph, tonemapped.0, send, args, &name)?;
                (tonemapped.0, true)
            } else {
                (send, format_done)
            };
            let send = if let Some(fmt) = pix_fmt {
                if !format_done && (*send).format != fmt {
                    convert_pix_fmt_frame(fmt_sws, converted.0, send, fmt)?;
                    converted.0
                } else {
                    send
                }
            } else {
                send
            };
            if minterpolate.is_some() || fps.is_some() {
                let mut emitted = 0u64;
                filter::temporal_push_frame(
                    minterpolate_graph,
                    minterpolate_dst.0,
                    minterpolate,
                    fps_graph,
                    fps_dst.0,
                    fps,
                    send,
                    |out| {
                        send_encoder_frame(encoder, output, packet, index, out, stats)?;
                        emitted += 1;
                        Ok(())
                    },
                )?;
                stats.video_frames += emitted;
                av_frame_unref(frame.0);
                continue;
            }
            // Drain only on encoder EAGAIN (inside send) so libx264 keeps depth.
            send_encoder_frame(encoder, output, packet, index, send, stats)?;
            av_frame_unref(frame.0);
        }
        stats.video_frames += 1;
    }
}

unsafe fn alloc_like_frame(dst: *mut AVFrame, src: *const AVFrame) -> Result<()> {
    unsafe {
        let s = &*src;
        let d = &mut *dst;
        if d.data[0].is_null()
            || d.width != s.width
            || d.height != s.height
            || d.format != s.format
            || av_frame_is_writable(dst) == 0
        {
            av_frame_unref(dst);
            d.format = s.format;
            d.width = s.width;
            d.height = s.height;
            // Default align (0 → 32) matches FFmpeg filter frames.
            check(av_frame_get_buffer(dst, 0), "alloc compact encode frame")?;
        }
    }
    Ok(())
}

/// Single-pass horizontal flip from `src` into a pooled writable `dst`.
pub(crate) unsafe fn horizontal_copy_frame(dst: *mut AVFrame, src: *mut AVFrame) -> Result<()> {
    unsafe {
        alloc_like_frame(dst, src)?;
        // Avoid av_frame_copy_props (side-data walk); set encode-critical fields only.
        let s = &*src;
        let d = &mut *dst;
        d.pts = s.pts;
        d.duration = s.duration;
        d.pict_type = 0;
        d.quality = 0;
        d.flags = s.flags & !(AV_FRAME_FLAG_KEY as i32);
        d.sample_aspect_ratio = s.sample_aspect_ratio;
        d.color_range = s.color_range;
        d.color_primaries = s.color_primaries;
        d.color_trc = s.color_trc;
        d.colorspace = s.colorspace;
        d.chroma_location = s.chroma_location;
        // AV_PIX_FMT_YUV420P is the dominant decode/bench path. Its three
        // byte-planar planes are fixed, so avoid walking the pixel descriptor
        // and components for every frame.
        if s.format == 0 && s.width > 0 && s.height > 0 {
            let dimensions = [
                (s.width as usize, s.height as usize),
                (
                    (s.width as usize).div_ceil(2),
                    (s.height as usize).div_ceil(2),
                ),
                (
                    (s.width as usize).div_ceil(2),
                    (s.height as usize).div_ceil(2),
                ),
            ];
            for (plane, (width, height)) in dimensions.into_iter().enumerate() {
                if s.data[plane].is_null()
                    || d.data[plane].is_null()
                    || width > s.linesize[plane].unsigned_abs() as usize
                    || width > d.linesize[plane].unsigned_abs() as usize
                {
                    return Err("invalid yuv420p horizontal-filter plane".into());
                }
                let src_ls = s.linesize[plane] as isize;
                let dst_ls = d.linesize[plane] as isize;
                fvid_cpu::hflip_plane_copy(
                    d.data[plane],
                    dst_ls,
                    s.data[plane],
                    src_ls,
                    width,
                    height,
                );
            }
            return Ok(());
        }
        let desc = av_pix_fmt_desc_get(s.format);
        if desc.is_null() || s.width <= 0 || s.height <= 0 {
            return Err("invalid horizontal-filter geometry".into());
        }
        let desc = &*desc;
        let planes = av_pix_fmt_count_planes(s.format);
        if !(1..=4).contains(&planes)
            || desc.flags
                & (AV_PIX_FMT_FLAG_HWACCEL | AV_PIX_FMT_FLAG_PAL | AV_PIX_FMT_FLAG_BITSTREAM) as u64
                != 0
            || (planes == 1
                && desc.log2_chroma_w != 0
                && desc.flags & AV_PIX_FMT_FLAG_RGB as u64 == 0)
        {
            return Err(
                "horizontal filter requires software planes with uniform pixel groups".into(),
            );
        }
        for plane in 0..planes as usize {
            let mut step = None;
            for component in &desc.comp[..desc.nb_components as usize] {
                if component.plane as usize == plane {
                    if step.is_some_and(|v| v != component.step) {
                        return Err("mixed pixel steps in plane".into());
                    }
                    step = Some(component.step);
                }
            }
            let step = usize::try_from(step.ok_or("plane has no components")?)
                .map_err(|_| "invalid pixel step")?;
            let chroma = (plane == 1 || plane == 2) && desc.flags & AV_PIX_FMT_FLAG_RGB as u64 == 0;
            let width =
                (s.width as usize).div_ceil(1usize << if chroma { desc.log2_chroma_w } else { 0 });
            let height =
                (s.height as usize).div_ceil(1usize << if chroma { desc.log2_chroma_h } else { 0 });
            let src_stride = s.linesize[plane].unsigned_abs() as usize;
            let dst_stride = d.linesize[plane].unsigned_abs() as usize;
            if step == 0
                || s.data[plane].is_null()
                || d.data[plane].is_null()
                || width
                    .checked_mul(step)
                    .is_none_or(|bytes| bytes > src_stride || bytes > dst_stride)
            {
                return Err("invalid horizontal-filter row extent".into());
            }
            let row_bytes = width * step;
            let src_ls = s.linesize[plane] as isize;
            let dst_ls = d.linesize[plane] as isize;
            let sp = s.data[plane];
            let dp = d.data[plane];
            for row in 0..height {
                let src_row =
                    std::slice::from_raw_parts(sp.offset((row as isize) * src_ls), row_bytes);
                let dst_row =
                    std::slice::from_raw_parts_mut(dp.offset((row as isize) * dst_ls), row_bytes);
                fvid_cpu::hflip_row_copy(dst_row, src_row, width, step);
            }
        }
    }
    Ok(())
}

/// Reverse row traversal while keeping the decoder-owned AVBuffer references.
/// SAFETY: frame must be a live, writable AVFrame with software video planes.
pub(crate) unsafe fn flip_view(frame: *mut AVFrame) -> Result<()> {
    // SAFETY: Caller owns the decoded frame; validated descriptor/plane bounds below
    // keep offsets within each decoder-provided plane. No buffer ownership changes.
    unsafe {
        let f = &mut *frame;
        let desc = av_pix_fmt_desc_get(f.format);
        if desc.is_null()
            || (*desc).flags
                & (AV_PIX_FMT_FLAG_HWACCEL | AV_PIX_FMT_FLAG_BITSTREAM | AV_PIX_FMT_FLAG_PAL) as u64
                != 0
        {
            return Err("vertical flip requires ordinary software pixel planes".into());
        }
        let planes = av_pix_fmt_count_planes(f.format);
        if !(1..=4).contains(&planes) || f.height <= 0 {
            return Err("invalid video plane geometry".into());
        }
        for plane in 0..planes as usize {
            let chroma =
                (plane == 1 || plane == 2) && (*desc).flags & AV_PIX_FMT_FLAG_RGB as u64 == 0;
            let shift = if chroma { (*desc).log2_chroma_h } else { 0 };
            let height = (f.height as usize).div_ceil(1usize << shift);
            let stride = f.linesize[plane];
            if f.data[plane].is_null() || stride == 0 || stride == i32::MIN {
                return Err("invalid video plane stride".into());
            }
            let offset = (height - 1)
                .checked_mul(stride.unsigned_abs() as usize)
                .and_then(|v| isize::try_from(v).ok())
                .ok_or("plane offset overflow")?;
            f.data[plane] = f.data[plane].offset(if stride < 0 { -offset } else { offset });
            f.linesize[plane] = -stride;
        }
        // Video extended_data normally aliases data; keep a separate table in sync too.
        if f.extended_data != f.data.as_mut_ptr() {
            for plane in 0..planes as usize {
                *f.extended_data.add(plane) = f.data[plane];
            }
        }
        Ok(())
    }
}
pub(crate) struct Sws {
    /// Direct swscale context, used for point scaling.
    context: *mut SwsContext,
    /// Same-size format conversions run through a slice-threaded
    /// buffersrc->(auto-inserted scale)->buffersink mini-graph so the restore
    /// conversion after each filter uses graph worker threads instead of a
    /// single-threaded sws_scale call.
    graph: Option<Box<ConvertGraph>>,
}

impl Sws {
    fn direct(context: *mut SwsContext) -> Self {
        Self { context, graph: None }
    }

    fn graph() -> Self {
        Self {
            context: ptr::null_mut(),
            graph: None,
        }
    }
}

impl Drop for Sws {
    fn drop(&mut self) {
        unsafe {
            if !self.context.is_null() {
                sws_freeContext(self.context);
            }
        }
    }
}

struct ConvertGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
    width: i32,
    height: i32,
    src_format: i32,
    dst_format: i32,
}

impl Drop for ConvertGraph {
    fn drop(&mut self) {
        // SAFETY: Owns the graph; source/sink contexts are freed with it.
        unsafe { avfilter_graph_free(&mut self.graph) }
    }
}

impl ConvertGraph {
    unsafe fn open(
        width: i32,
        height: i32,
        src_format: i32,
        dst_format: i32,
    ) -> Result<Self> {
        unsafe {
            let src_name = string(av_get_pix_fmt_name(src_format));
            let dst_name = string(av_get_pix_fmt_name(dst_format));
            if src_name.is_empty() || dst_name.is_empty() {
                return Err("unknown pixel format for conversion graph".into());
            }
            let buffersrc = avfilter_get_by_name(c"buffer".as_ptr());
            let format = avfilter_get_by_name(c"format".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffersrc.is_null() || format.is_null() || buffersink.is_null() {
                return Err("buffer/format/buffersink unavailable in linked libavfilter".into());
            }
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err("conversion graph allocation failed".into());
            }
            let mut built = Self {
                graph,
                src: ptr::null_mut(),
                sink: ptr::null_mut(),
                width,
                height,
                src_format,
                dst_format,
            };
            let args = format!(
                "video_size={width}x{height}:pix_fmt={src_name}:time_base=1/1000000:pixel_aspect=1/1"
            );
            let args = cstring(&args)?;
            check(
                avfilter_graph_create_filter(
                    &mut built.src,
                    buffersrc,
                    c"in".as_ptr(),
                    args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create conversion source",
            )?;
            check(
                avfilter_graph_create_filter(
                    &mut built.sink,
                    buffersink,
                    c"out".as_ptr(),
                    ptr::null(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create conversion sink",
            )?;
            let mut fmt = ptr::null_mut();
            let fmt_args = format!("pix_fmts={dst_name}");
            let fmt_args = cstring(&fmt_args)?;
            check(
                avfilter_graph_create_filter(
                    &mut fmt,
                    format,
                    c"want".as_ptr(),
                    fmt_args.as_ptr(),
                    ptr::null_mut(),
                    built.graph,
                ),
                "create conversion format filter",
            )?;
            check(
                avfilter_link(built.src, 0, fmt, 0),
                "link conversion source to format",
            )?;
            check(
                avfilter_link(fmt, 0, built.sink, 0),
                "link format to conversion sink",
            )?;
            check(
                configure_filter_graph(built.graph),
                "configure conversion graph",
            )?;
            Ok(built)
        }
    }
}

/// Nearest-neighbor scaler flag (`SwsFlags::SWS_POINT` = 1<<4).
const SWS_POINT: i32 = 1 << 4;

/// Neighbor (point) scale into a reusable destination frame.
pub(crate) unsafe fn scale_frame(
    sws: &mut Option<Sws>,
    dst: *mut AVFrame,
    src: *const AVFrame,
    out_w: i32,
    out_h: i32,
) -> Result<()> {
    unsafe {
        let s = &*src;
        scale_or_convert_frame(sws, dst, src, out_w, out_h, s.format, SWS_POINT)
    }
}

/// Convert pixel format at the same size (FFmpeg `format=PIX_FMT`) through a
/// slice-threaded conversion graph.
pub(crate) unsafe fn convert_pix_fmt_frame(
    sws: &mut Option<Sws>,
    dst: *mut AVFrame,
    src: *const AVFrame,
    out_fmt: AVPixelFormat,
) -> Result<()> {
    unsafe {
        let s = &*src;
        let sws = sws.get_or_insert_with(Sws::graph);
        let (width, height, src_format) = (s.width, s.height, s.format);
        let stale = match sws.graph.as_deref() {
            Some(g) => {
                !(g.width == width
                    && g.height == height
                    && g.src_format == src_format
                    && g.dst_format == out_fmt)
            }
            None => true,
        };
        if stale {
            sws.graph = Some(Box::new(ConvertGraph::open(
                width, height, src_format, out_fmt,
            )?));
        }
        let graph = sws.graph.as_deref_mut().unwrap_unchecked();
        check(
            av_buffersrc_write_frame(graph.src, src),
            "feed conversion source",
        )?;
        av_frame_unref(dst);
        check(
            av_buffersink_get_frame(graph.sink, dst),
            "receive converted frame",
        )?;
        let d = &mut *dst;
        d.pts = s.pts;
        d.duration = s.duration;
        d.pict_type = 0;
        d.quality = 0;
        d.flags = s.flags & !(AV_FRAME_FLAG_KEY as i32);
        d.sample_aspect_ratio = s.sample_aspect_ratio;
        d.color_range = s.color_range;
        d.color_primaries = s.color_primaries;
        d.color_trc = s.color_trc;
        d.colorspace = s.colorspace;
        d.chroma_location = s.chroma_location;
    }
    Ok(())
}

/// Neighbor resize that may also change pixel format (FFmpeg merges
/// `scale=W:H:flags=neighbor,format=FMT` into one point swscale).
pub(crate) unsafe fn scale_convert_frame(
    sws: &mut Option<Sws>,
    dst: *mut AVFrame,
    src: *const AVFrame,
    out_w: i32,
    out_h: i32,
    out_fmt: AVPixelFormat,
) -> Result<()> {
    unsafe { scale_or_convert_frame(sws, dst, src, out_w, out_h, out_fmt, SWS_POINT) }
}

unsafe fn scale_or_convert_frame(
    sws: &mut Option<Sws>,
    dst: *mut AVFrame,
    src: *const AVFrame,
    out_w: i32,
    out_h: i32,
    out_fmt: AVPixelFormat,
    flags: i32,
) -> Result<()> {
    unsafe {
        let s = &*src;
        let sws = sws.get_or_insert_with(Sws::graph);
        if sws.context.is_null() {
            let context = sws_getContext(
                s.width,
                s.height,
                s.format,
                out_w,
                out_h,
                out_fmt,
                flags,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            );
            if context.is_null() {
                return Err("scale/convert context allocation failed".into());
            }
            sws.context = context;
        }
        let d = &mut *dst;
        if d.data[0].is_null()
            || d.width != out_w
            || d.height != out_h
            || d.format != out_fmt
            || av_frame_is_writable(dst) == 0
        {
            av_frame_unref(dst);
            d.format = out_fmt;
            d.width = out_w;
            d.height = out_h;
            check(
                av_frame_get_buffer(dst, 32),
                "allocate converted frame buffer",
            )?;
        }
        d.pts = s.pts;
        d.duration = s.duration;
        d.pict_type = 0;
        d.quality = 0;
        d.flags = s.flags & !(AV_FRAME_FLAG_KEY as i32);
        d.sample_aspect_ratio = s.sample_aspect_ratio;
        d.color_range = s.color_range;
        d.color_primaries = s.color_primaries;
        d.color_trc = s.color_trc;
        d.colorspace = s.colorspace;
        d.chroma_location = s.chroma_location;
        let code = sws_scale(
            sws.context,
            s.data.as_ptr() as *const *const u8,
            s.linesize.as_ptr(),
            0,
            s.height,
            d.data.as_ptr(),
            d.linesize.as_ptr(),
        );
        if code <= 0 {
            return Err("scale/convert frame failed".into());
        }
    }
    Ok(())
}

pub(crate) fn parse_pix_fmt(name: &str) -> Result<AVPixelFormat> {
    if name.is_empty() || name.len() > 32 || name.contains('\0') {
        return Err("pix_fmt name must be 1..=32 bytes without NUL".into());
    }
    let c = std::ffi::CString::new(name).map_err(|_| "invalid pix_fmt name")?;
    // SAFETY: CString is NUL-terminated; av_get_pix_fmt reads a static table.
    let fmt = unsafe { av_get_pix_fmt(c.as_ptr()) };
    if fmt == AVPixelFormat_AV_PIX_FMT_NONE {
        return Err(format!("unknown pixel format: {name}"));
    }
    Ok(fmt)
}

pub(crate) fn validate_scale(scale: ScaleSize) -> Result<()> {
    if scale.width == 0 || scale.height == 0 || scale.width > 8192 || scale.height > 4320 {
        return Err("scale must be within 1..=8192 x 1..=4320".into());
    }
    if scale.width % 2 != 0 || scale.height % 2 != 0 {
        return Err("scale size must be even for 4:2:0 chroma".into());
    }
    Ok(())
}

/// Lossless FFV1 in Matroska; all selected non-video packets are remuxed.
/// Strictly preserves pixel format/subsampling/bit depth rather than converting silently.
pub fn crop_lossless(
    source: &Path,
    destination: &Path,
    crop: CropRect,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    transcode_lossless(
        source,
        destination,
        LosslessTransform {
            crop: Some(crop),
            vertical_flip: false,
            horizontal_flip: false,
            scale: None,
            epx: None,
            transpose: None,
            rotate: None,
            pad: None,
            burn_subs: None,
            overlay: None,
            xfade: None,
            yadif: None,
            bwdif: None,
            w3fdif: None,
            tblend: None,
            tmix: None,
            hqdn3d: None,
            gblur: None,
            eq: None,
            unsharp: None,
            hue: None,
            avgblur: None,
            boxblur: None,
            negate: None,
            edgedetect: None,
            sobel: None,
            prewitt: None,
            roberts: None,
            kirsch: None,
            scharr: None,
            atadenoise: None,
            owdenoise: None,
            vaguedenoiser: None,
            nlmeans: None,
            bm3d: None,
            dctdnoiz: None,
            fftdnoiz: None,
            smartblur: None,
            sab: None,
            bilateral: None,
            cas: None,
            vignette: None,
            curves: None,
            colorbalance: None,
            colorlevels: None,
            colorchannelmixer: None,
            deflicker: None,
            photosensitivity: None,
            monochrome: None,
            grayworld: None,
            drawbox: None,
            drawgrid: None,
            lagfun: None,
            amplify: None,
            bitplanenoise: None,
            deband: None,
            gradfun: None,
            lenscorrection: None,
            pixelize: None,
            removegrain: None,
            yaepblur: None,
            vibrance: None,
            dilation: None,
            erosion: None,
            colorize: None,
            exposure: None,
            chromashift: None,
            colorcontrast: None,
            colorcorrect: None,
            histeq: None,
            shuffleplanes: None,
            lutyuv: None,
            colorhold: None,
            fade: None,
            perspective: None,
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
            zscale: None,
            tonemap: None,
            pix_fmt: None,
            interval: None,
            seek: false,
        },
        options,
    )
}
/// Exact packet-boundary copy of a secondary non-reordered video from a fresh demuxer.
/// Used when the primary path seeks so secondary packets are not missed or mis-aligned.
fn mux_exact_video_interval_from_path(
    source: &Path,
    output: &mut Output,
    stream_index: usize,
    mapped: usize,
    from_us: i64,
    to_us: i64,
    options: &CopyOptions,
) -> Result<u64> {
    let mut input = Input::open_fast(source)?;
    if stream_index >= input.streams().len() {
        return Err("secondary video stream index out of range".into());
    }
    // SAFETY: Index checked; live stream table.
    let tb = unsafe { (*input.streams()[stream_index]).time_base };
    let codec = unsafe { &*(*input.streams()[stream_index]).codecpar };
    if codec.codec_type != AVMediaType_AVMEDIA_TYPE_VIDEO {
        return Err("exact secondary interval requires a video stream".into());
    }
    let format_start_us = unsafe {
        if (*input.0).start_time == NOPTS {
            0
        } else {
            (*input.0).start_time
        }
    };
    let ticks = |time: i64| -> Result<i64> {
        let us = format_start_us
            .checked_add(time)
            .ok_or("interval timestamp overflow")?;
        let numerator = i128::from(us) * i128::from(tb.den);
        let denominator = 1_000_000i128 * i128::from(tb.num);
        if denominator <= 0 || tb.den <= 0 || numerator % denominator != 0 {
            return Err("interval boundary is not exact in secondary video time base".into());
        }
        i64::try_from(numerator / denominator).map_err(|_| "interval timestamp overflow".into())
    };
    let start = ticks(from_us)?;
    check(
        unsafe { avformat_seek_file(input.0, stream_index as i32, i64::MIN, start, start, 0) },
        "seek before exact secondary video interval",
    )?;
    let mut packet = Packet::new()?;
    let mut copied = 0u64;
    while packet.read(&mut input)? {
        check_budget(options, copied)?;
        let (index, _) = packet_info(&packet, &input, options)?;
        if index != stream_index {
            continue;
        }
        if !copy_secondary_video_interval(&mut packet, &input, index, from_us, to_us)? {
            // Fully before window after undershoot, or past end.
            // SAFETY: Live packet PTS for stop condition.
            let pts = unsafe { (*packet.0).pts };
            if pts != NOPTS && pts >= start {
                let end = ticks(to_us)?;
                if pts >= end {
                    break;
                }
            }
            continue;
        }
        output.write(&mut packet, mapped, tb)?;
        copied += 1;
    }
    if copied == 0 {
        return Err("exact secondary interval copied no packets".into());
    }
    Ok(copied)
}

/// Exact packet-boundary copy of a secondary non-reordered video inside `[from,to)` µs.
/// Returns false when the packet lies fully outside the window (caller should skip).
fn copy_secondary_video_interval(
    packet: &mut Packet,
    input: &Input,
    index: usize,
    from_us: i64,
    to_us: i64,
) -> Result<bool> {
    // SAFETY: Live packet and stream; only timestamps are mutated on include.
    unsafe {
        let p = &mut *packet.0;
        let stream = &*input.streams()[index];
        let tb = stream.time_base;
        if p.pts == NOPTS || p.dts == NOPTS || p.duration <= 0 {
            return Err("secondary video interval requires PTS, DTS and positive duration".into());
        }
        if p.pts != p.dts {
            return Err("secondary video interval rejects reordered packets".into());
        }
        let origin = if (*input.0).start_time == NOPTS {
            0
        } else {
            (*input.0).start_time
        };
        let ticks = |time: i64| -> Result<i64> {
            let us = origin
                .checked_add(time)
                .ok_or("interval timestamp overflow")?;
            let numerator = i128::from(us) * i128::from(tb.den);
            let denominator = 1_000_000i128 * i128::from(tb.num);
            if denominator <= 0 || tb.den <= 0 || numerator % denominator != 0 {
                return Err("interval boundary is not exact in secondary video time base".into());
            }
            i64::try_from(numerator / denominator).map_err(|_| "interval timestamp overflow".into())
        };
        let start = ticks(from_us)?;
        let end = ticks(to_us)?;
        let packet_end = p.pts.checked_add(p.duration).ok_or("timestamp overflow")?;
        if packet_end <= start || p.pts >= end {
            return Ok(false);
        }
        if p.pts < start || packet_end > end {
            return Err(
                "secondary video interval cuts through a packet; exact stream copy impossible"
                    .into(),
            );
        }
        if p.pts == start && p.flags & AV_PKT_FLAG_KEY as i32 == 0 {
            return Err("secondary video interval must begin on a keyframe".into());
        }
        p.pts = p.pts.checked_sub(start).ok_or("PTS overflow")?;
        p.dts = p.dts.checked_sub(start).ok_or("DTS overflow")?;
        Ok(true)
    }
}

/// Clip chapter ranges and rebase them to the selected presentation interval.
/// All arithmetic is validated before mutating the input-owned chapter table.
pub(super) fn retime_chapters(input: &mut Input, from: i64, to: i64) -> Result<()> {
    // SAFETY: Input exclusively owns the context, chapter array and metadata. Removed
    // entries are freed once; kept pointers are compacted and nb_chapters updated.
    unsafe {
        let context = &mut *input.0;
        let count = context.nb_chapters as usize;
        if count > 4096 {
            return Err("too many chapters".into());
        }
        let origin = if context.start_time == NOPTS {
            0
        } else {
            context.start_time
        };
        let start = origin
            .checked_add(from)
            .ok_or("chapter interval overflow")?;
        let end = origin.checked_add(to).ok_or("chapter interval overflow")?;
        let mut ranges = Vec::with_capacity(count);
        for index in 0..count {
            let chapter = &**context.chapters.add(index);
            let micros = |value: i64| -> Result<i64> {
                let tb = chapter.time_base;
                if tb.num <= 0 || tb.den <= 0 {
                    return Err("invalid chapter time base".into());
                }
                let numerator = i128::from(value) * i128::from(tb.num) * 1_000_000;
                if numerator % i128::from(tb.den) != 0 {
                    return Err("chapter time cannot be represented exactly in microseconds".into());
                }
                i64::try_from(numerator / i128::from(tb.den))
                    .map_err(|_| "chapter timestamp overflow".into())
            };
            let left = micros(chapter.start)?;
            let right = micros(chapter.end)?;
            if right < left {
                return Err("reversed chapter range".into());
            }
            let left = left.max(start);
            let right = right.min(end);
            ranges.push(if left < right {
                Some((
                    left.checked_sub(start)
                        .ok_or("chapter timestamp overflow")?,
                    right
                        .checked_sub(start)
                        .ok_or("chapter timestamp overflow")?,
                ))
            } else {
                None
            });
        }
        context.nb_chapters = 0;
        for (index, range) in ranges.into_iter().enumerate() {
            let chapter = *context.chapters.add(index);
            if let Some((start, end)) = range {
                (*chapter).start = start;
                (*chapter).end = end;
                (*chapter).time_base = AVRational {
                    num: 1,
                    den: 1_000_000,
                };
                *context.chapters.add(context.nb_chapters as usize) = chapter;
                context.nb_chapters += 1;
            } else {
                av_dict_free(&mut (*chapter).metadata);
                av_free(chapter.cast());
            }
        }
    }
    Ok(())
}
/// Preserve decoded samples in FFV1; optional crop and vertical flip use plane views.
pub fn transcode_lossless(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
) -> Result<LosslessStats> {
    if destination.extension().and_then(|v| v.to_str()) != Some("mkv") {
        return Err("lossless export requires FFV1 in .mkv".into());
    }
    if transform.crop.is_none()
        && !transform.vertical_flip
        && !transform.horizontal_flip
        && transform.scale.is_none()
        && transform.epx.is_none()
        && transform.transpose.is_none()
        && transform.rotate.is_none()
        && transform.pad.is_none()
        && transform.burn_subs.is_none()
        && transform.overlay.is_none()
        && transform.xfade.is_none()
        && transform.yadif.is_none()
        && transform.bwdif.is_none()
        && transform.w3fdif.is_none()
        && transform.tblend.is_none()
        && transform.tmix.is_none()
        && transform.hqdn3d.is_none()
        && transform.gblur.is_none()
        && transform.eq.is_none()
        && transform.unsharp.is_none()
        && transform.hue.is_none()
        && transform.avgblur.is_none()
        && transform.boxblur.is_none()
        && transform.negate.is_none()
        && transform.edgedetect.is_none()
        && transform.sobel.is_none()
        && transform.prewitt.is_none()
        && transform.roberts.is_none()
        && transform.kirsch.is_none()
        && transform.scharr.is_none()
        && transform.atadenoise.is_none()
        && transform.owdenoise.is_none()
        && transform.vaguedenoiser.is_none()
        && transform.nlmeans.is_none()
        && transform.bm3d.is_none()
        && transform.dctdnoiz.is_none()
        && transform.fftdnoiz.is_none()
        && transform.smartblur.is_none()
        && transform.sab.is_none()
        && transform.bilateral.is_none()
        && transform.cas.is_none()
        && transform.vignette.is_none()
        && transform.curves.is_none()
        && transform.colorbalance.is_none()
        && transform.colorlevels.is_none()
        && transform.colorchannelmixer.is_none()
        && transform.deflicker.is_none()
        && transform.photosensitivity.is_none()
        && transform.monochrome.is_none()
        && transform.grayworld.is_none()
        && transform.drawbox.is_none()
        && transform.drawgrid.is_none()
        && transform.lagfun.is_none()
        && transform.amplify.is_none()
        && transform.bitplanenoise.is_none()
        && transform.deband.is_none()
        && transform.gradfun.is_none()
        && transform.lenscorrection.is_none()
        && transform.pixelize.is_none()
        && transform.removegrain.is_none()
        && transform.yaepblur.is_none()
        && transform.vibrance.is_none()
        && transform.dilation.is_none()
        && transform.erosion.is_none()
        && transform.colorize.is_none()
        && transform.exposure.is_none()
        && transform.chromashift.is_none()
        && transform.colorcontrast.is_none()
        && transform.colorcorrect.is_none()
        && transform.histeq.is_none()
        && transform.shuffleplanes.is_none()
        && transform.lutyuv.is_none()
        && transform.colorhold.is_none()
        && transform.fade.is_none()
        && transform.perspective.is_none()
        && transform.lumakey.is_none()
        && transform.chromakey.is_none()
        && transform.colorkey.is_none()
        && transform.despill.is_none()
        && transform.selectivecolor.is_none()
        && transform.stereo3d.is_none()
        && transform.field.is_none()
        && transform.hqx.is_none()
        && transform.xbr.is_none()
        && transform.il.is_none()
        && transform.super2xsai.is_none()
        && transform.kerndeint.is_none()
        && transform.phase.is_none()
        && transform.estdif.is_none()
        && transform.tinterlace.is_none()
        && transform.separatefields.is_none()
        && transform.weave.is_none()
        && transform.doubleweave.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
        && transform.framestep.is_none()
        && transform.freezedetect.is_none()
        && transform.pseudocolor.is_none()
        && transform.minterpolate.is_none()
        && transform.fps.is_none()
        && transform.colorspace.is_none()
        && transform.zscale.is_none()
        && transform.tonemap.is_none()
        && transform.pix_fmt.is_none()
        && transform.interval.is_none()
        && !transform.seek
    {
        // Matroska headers already expose codec identity; avoid a second full
        // packet probe before the stream-copy planner (fair-pair vs ffmpeg -c copy).
        let mkv = source
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("mkv"));
        let input = Input::open_with_stream_info(source, !mkv, None)?;
        let selected = selection(&input, options)?;
        if selected.len() == 1
            && unsafe {
                let parameters = &*(*input.streams()[selected[0]]).codecpar;
                parameters.codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
                    && parameters.codec_id == AVCodecID_AV_CODEC_ID_FFV1
            }
        {
            let pixel_format = unsafe {
                string(av_get_pix_fmt_name(
                    (*(*input.streams()[selected[0]]).codecpar).format,
                ))
            };
            let copied = remux_input(input, destination, options)?;
            return Ok(LosslessStats {
                backend: "native FFV1 stream copy",
                video_frames: copied.packets,
                decoded_frames: 0,
                seek_used: false,
                video_packets: copied.packets,
                copied_packets: 0,
                trimmed_audio_sample_frames: 0,
                pixel_format,
                encoder: "ffv1 (stream copy)".into(),
                fvid_crop_payload_copies: 0,
                vertical_flip: false,
                horizontal_flip: false,
            });
        }
    }
    transcode(
        source,
        destination,
        transform,
        options,
        &EncoderSettings {
            name: "ffv1".into(),
            options: Vec::new(),
        },
    )
}
pub use fvid_media_info::EncoderSettings;
struct CodecOptions(*mut AVDictionary);
impl Drop for CodecOptions {
    fn drop(&mut self) {
        // SAFETY: This guard exclusively owns its dictionary, including leftovers.
        unsafe {
            av_dict_free(&mut self.0);
        }
    }
}
pub fn transcode(
    source: &Path,
    destination: &Path,
    transform: LosslessTransform,
    options: &CopyOptions,
    settings: &EncoderSettings,
) -> Result<LosslessStats> {
    settings.validate()?;
    let encoder_name = cstring(&settings.name)?;
    let mut codec_options = CodecOptions(ptr::null_mut());
    for (key, value) in &settings.options {
        let key = cstring(key)?;
        let value = cstring(value)?;
        // SAFETY: Dictionary owns copies of the valid NUL-terminated strings.
        check(
            unsafe { av_dict_set(&mut codec_options.0, key.as_ptr(), value.as_ptr(), 0) },
            "set encoder option",
        )?;
    }
    if transform.seek && transform.interval.is_none() {
        return Err("seek requires a lossless interval".into());
    }
    let mut input = Input::open(source)?;
    let selected = selection(&input, options)?;
    let scratch_frames = 6 + usize::from(transform.horizontal_flip) * 16;
    crate::budget::admit_input_controlled_budget(&input, options, scratch_frames, true)?;
    crate::budget::check_rss_budget(options)?;
    // SAFETY: Selection points into Input's live stream table and codec parameters.
    let videos: Vec<_> = selected
        .iter()
        .copied()
        .filter(|&i| unsafe {
            (*(*input.streams()[i]).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
        })
        .collect();
    if videos.is_empty() {
        return Err("lossless crop requires at least one selected video stream".into());
    }
    let video = videos[0];
    // Dedicated demuxers keep secondary intervals correct when the primary path seeks.
    let mut reordered_secondary: Vec<(usize, usize)> = Vec::new();
    let mut exact_secondary: Vec<(usize, usize)> = Vec::new();
    if transform.interval.is_some() {
        for &index in &videos[1..] {
            // SAFETY: Selected video indices belong to the live input.
            let codec = unsafe { &*(*input.streams()[index]).codecpar };
            let reordered = codec.video_delay != 0
                || codec.codec_id == AVCodecID_AV_CODEC_ID_H264
                || codec.codec_id == AVCodecID_AV_CODEC_ID_HEVC;
            let mapped = selected
                .iter()
                .position(|&i| i == index)
                .ok_or("secondary video stream mapping missing")?;
            if reordered {
                if codec.codec_id != AVCodecID_AV_CODEC_ID_H264
                    && codec.codec_id != AVCodecID_AV_CODEC_ID_HEVC
                {
                    return Err(
                        "secondary video in lossless interval requires a non-reordered codec or H.264/HEVC closed-GOP stream copy"
                            .into(),
                    );
                }
                reordered_secondary.push((index, mapped));
            } else if transform.seek {
                exact_secondary.push((index, mapped));
            }
        }
    }
    let interval = if let Some((from, to)) = transform.interval {
        if from < 0 || to <= from {
            return Err("lossless interval requires 0 <= from < to".into());
        }
        // Seek with PCM or compressed audio is allowed: demuxer seek lands at/before
        // `from`. PCM uses sample-exact packet trim; AAC/MP3/FLAC use frame PTS to
        // place the absolute sample window after the shared seek point.
        // SAFETY: The input context and selected stream remain live.
        unsafe {
            let origin = if (*input.0).start_time == NOPTS {
                0
            } else {
                (*input.0).start_time
            };
            let tb = (*input.streams()[videos[0]]).time_base;
            let ticks = |time: i64| -> Result<i64> {
                let us = origin
                    .checked_add(time)
                    .ok_or("interval timestamp overflow")?;
                let numerator = i128::from(us) * i128::from(tb.den);
                let denominator = 1_000_000i128 * i128::from(tb.num);
                if denominator <= 0 || tb.den <= 0 || numerator % denominator != 0 {
                    return Err("interval boundary is not exact in video time base".into());
                }
                i64::try_from(numerator / denominator)
                    .map_err(|_| "interval timestamp overflow".into())
            };
            Some((ticks(from)?, ticks(to)?))
        }
    } else {
        None
    };

    let crop = transform.crop.unwrap_or_else(|| {
        // SAFETY: Selected stream belongs to the live input.
        let p = unsafe { &*(*input.streams()[video]).codecpar };
        CropRect {
            x: 0,
            y: 0,
            width: p.width.max(0) as usize,
            height: p.height.max(0) as usize,
        }
    });
    let mapped = selected.iter().position(|&i| i == video).unwrap();
    // SAFETY: Contexts allocated below are held in RAII guards before subsequent
    // fallible calls. Codec pointers and source parameters are library-owned/live.
    let (mut decoder, mut encoder, parameters, tb, pixel_format) = unsafe {
        let s = &*input.streams()[video];
        let p = &*s.codecpar;
        if p.nb_coded_side_data != 0 {
            return Err(
                "lossless crop with codec side data requires explicit metadata handling".into(),
            );
        }
        let dec = avcodec_find_decoder(p.codec_id);
        if dec.is_null() {
            return Err("decoder unavailable".into());
        }
        let decoder = Codec(avcodec_alloc_context3(dec));
        if decoder.0.is_null() {
            return Err("decoder allocation failed".into());
        }
        check(
            avcodec_parameters_to_context(decoder.0, s.codecpar),
            "configure decoder",
        )?;
        (*decoder.0).pkt_timebase = s.time_base;
        (*decoder.0).thread_count = 0;
        (*decoder.0).max_pixels = 8192 * 4320;
        check(
            avcodec_open2(decoder.0, dec, ptr::null_mut()),
            "open decoder",
        )?;
        let format = (*decoder.0).pix_fmt;
        let encode_format = if let Some(ref name) = transform.pix_fmt {
            parse_pix_fmt(name)?
        } else if transform.lumakey.is_some()
            || transform.chromakey.is_some()
            || transform.colorkey.is_some()
        {
            parse_pix_fmt("yuva420p")?
        } else {
            format
        };
        let descriptor = av_pix_fmt_desc_get(format);
        if descriptor.is_null() {
            return Err("unknown decoded pixel format".into());
        }
        let sx = 1usize << (*descriptor).log2_chroma_w;
        let sy = 1usize << (*descriptor).log2_chroma_h;
        if crop.width == 0
            || crop.height == 0
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|v| v > p.width.max(0) as usize)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|v| v > p.height.max(0) as usize)
            || !crop.x.is_multiple_of(sx)
            || !crop.y.is_multiple_of(sy)
        {
            return Err("crop must be bounded with a chroma-aligned origin".into());
        }
        let (out_w, out_h) = {
            let (mut w, mut h) = if let Some(mode) = transform.transpose {
                let (tw, th) = mode.size(crop.width as u32, crop.height as u32);
                (tw as i32, th as i32)
            } else {
                (crop.width as i32, crop.height as i32)
            };
            if let Some(angle) = transform.rotate {
                let (rw, rh) = angle.size(w as u32, h as u32);
                w = rw as i32;
                h = rh as i32;
            }
            if let Some(pad) = transform.pad {
                pad.validate(w as u32, h as u32)?;
                w = pad.width as i32;
                h = pad.height as i32;
            }
            let (mut ow, mut oh) = if let Some(scale) = transform.scale {
                validate_scale(scale)?;
                (scale.width as i32, scale.height as i32)
            } else {
                (w, h)
            };
            if let Some(ref args) = transform.epx {
                filter::validate_epx_args(args)?;
                let (ew, eh) = filter::epx_output_size(ow as u32, oh as u32, args)?;
                ow = ew as i32;
                oh = eh as i32;
            }
            if let Some(ref args) = transform.stereo3d {
                filter::validate_stereo3d_args(args)?;
                let (sw, sh) = filter::stereo3d_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.field {
                filter::validate_field_args(args)?;
                let (fw, fh) = filter::field_output_size(ow as u32, oh as u32, args)?;
                ow = fw as i32;
                oh = fh as i32;
            }
            if let Some(ref args) = transform.hqx {
                filter::validate_hqx_args(args)?;
                let (hw, hh) = filter::hqx_output_size(ow as u32, oh as u32, args)?;
                ow = hw as i32;
                oh = hh as i32;
            }
            if let Some(ref args) = transform.xbr {
                filter::validate_xbr_args(args)?;
                let (xw, xh) = filter::xbr_output_size(ow as u32, oh as u32, args)?;
                ow = xw as i32;
                oh = xh as i32;
            }
            if let Some(ref args) = transform.super2xsai {
                filter::validate_super2xsai_args(args)?;
                let (sw, sh) = filter::super2xsai_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.separatefields {
                filter::validate_separatefields_args(args)?;
                let (sw, sh) = filter::separatefields_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.weave {
                filter::validate_weave_args(args)?;
                let (sw, sh) = filter::weave_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.doubleweave {
                filter::validate_doubleweave_args(args)?;
                let (sw, sh) = filter::doubleweave_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.framepack {
                filter::validate_framepack_args(args)?;
                let (sw, sh) = filter::framepack_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.tile {
                filter::validate_tile_args(args)?;
                let (sw, sh) = filter::tile_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            if let Some(ref args) = transform.untile {
                filter::validate_untile_args(args)?;
                let (sw, sh) = filter::untile_output_size(ow as u32, oh as u32, args)?;
                ow = sw as i32;
                oh = sh as i32;
            }
            (ow, oh)
        };
        let enc = avcodec_find_encoder_by_name(encoder_name.as_ptr());
        if enc.is_null() {
            return Err(format!("encoder unavailable: {}", settings.name));
        }
        let encoder = Codec(avcodec_alloc_context3(enc));
        if encoder.0.is_null() {
            return Err("encoder allocation failed".into());
        }
        (*encoder.0).width = out_w;
        (*encoder.0).height = out_h;
        (*encoder.0).pix_fmt = encode_format;
        // Preserve source PTS/durations exactly, including VFR gaps. Average
        // frame rate remains an encoder hint, not a timestamp rewrite policy.
        let fr = s.avg_frame_rate;
        (*encoder.0).time_base = s.time_base;
        (*encoder.0).framerate = fr;
        (*encoder.0).sample_aspect_ratio = p.sample_aspect_ratio;
        (*encoder.0).color_range = p.color_range;
        (*encoder.0).color_primaries = p.color_primaries;
        (*encoder.0).color_trc = p.color_trc;
        (*encoder.0).colorspace = p.color_space;
        (*encoder.0).chroma_sample_location = p.chroma_location;
        (*encoder.0).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as i32;
        if (*enc).id == AVCodecID_AV_CODEC_ID_FFV1 {
            (*encoder.0).level = 3;
        }
        (*encoder.0).thread_count = 0;
        check(
            avcodec_open2(encoder.0, enc, &mut codec_options.0),
            "open selected encoder without pixel conversion",
        )?;
        if av_dict_count(codec_options.0) != 0 {
            return Err("unrecognized or unused encoder option".into());
        }
        if (*encoder.0).pix_fmt != encode_format {
            return Err("encoder changed pixel format".into());
        }
        let parameters = Parameters(avcodec_parameters_alloc());
        if parameters.0.is_null() {
            return Err("codec parameter allocation failed".into());
        }
        check(
            avcodec_parameters_from_context(parameters.0, encoder.0),
            "export FFV1 parameters",
        )?;
        let enc_tb = (*encoder.0).time_base;
        (
            decoder,
            encoder,
            parameters,
            enc_tb,
            string(av_get_pix_fmt_name(encode_format)),
        )
    };
    if let Some((from, to)) = transform.interval {
        retime_chapters(&mut input, from, to)?;
    }
    if transform.seek {
        let (start, _) = interval.ok_or("seek requires interval")?;
        // SAFETY: Live demuxer, valid stream index/time base. The upper bound forces
        // a seek point at or before the requested start; no decoder packets sent yet.
        // Compressed audio for seek is decoded on a second demuxer (from start) so this
        // seek only accelerates the video path.
        check(
            unsafe { avformat_seek_file(input.0, video as i32, i64::MIN, start, start, 0) },
            "seek before lossless interval",
        )?;
    }
    let interval_us = transform.interval;
    let mut audio_param_holders = Vec::new();
    let mut decode_audio: Vec<DecodeAudioTrack> = Vec::new();
    let mut seek_audio: Vec<(usize, usize)> = Vec::new();
    let mut overrides = vec![(video, parameters.0 as *const _, tb)];
    if interval_us.is_some() {
        for &index in &selected {
            if index == video {
                continue;
            }
            // SAFETY: Selected streams belong to the live input.
            let codecpar = unsafe { &*(*input.streams()[index]).codecpar };
            if codecpar.codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO {
                // Secondary video is stream-copied in the packet loop.
                continue;
            }
            if pcm::classify_interval_audio(codecpar)? != pcm::IntervalAudio::Decode {
                continue;
            }
            let (pcm_params, audio_tb, format) =
                audio::pcm_parameters_for_interval_decode(codecpar)?;
            let mapped = selected
                .iter()
                .position(|&i| i == index)
                .ok_or("audio stream mapping missing")?;
            overrides.push((index, pcm_params.0 as *const _, audio_tb));
            audio_param_holders.push(pcm_params);
            if transform.seek {
                // Video demuxer will seek; keep sample-exact AAC/MP3/FLAC via a
                // second from-start demux instead of mid-stream decoder state.
                seek_audio.push((index, mapped));
                continue;
            }
            let decoder = unsafe {
                let codec = avcodec_find_decoder(codecpar.codec_id);
                if codec.is_null() {
                    return Err("audio decoder unavailable".into());
                }
                let decoder = Codec(avcodec_alloc_context3(codec));
                if decoder.0.is_null() {
                    return Err("audio decoder allocation failed".into());
                }
                check(
                    avcodec_parameters_to_context(decoder.0, codecpar),
                    "configure audio decoder",
                )?;
                (*decoder.0).pkt_timebase = (*input.streams()[index]).time_base;
                check(
                    avcodec_open2(decoder.0, codec, ptr::null_mut()),
                    "open audio decoder",
                )?;
                decoder
            };
            decode_audio.push(DecodeAudioTrack {
                stream_index: index,
                mapped,
                decoder,
                format,
                decoded_sample_frames: 0,
                written_sample_frames: 0,
                sample_bounds: None,
                pool: audio::PacketPool::default(),
                done: false,
            });
        }
    }
    let mut output =
        Output::with_overrides(destination, &input, &selected, &overrides, true, None)?
            .without_interleave();
    drop(audio_param_holders);
    let mut stats = LosslessStats {
        backend: "native libavcodec + Fvid crop view",
        video_frames: 0,
        decoded_frames: 0,
        seek_used: transform.seek,
        video_packets: 0,
        copied_packets: 0,
        trimmed_audio_sample_frames: 0,
        pixel_format,
        encoder: settings.name.clone(),
        fvid_crop_payload_copies: 0,
        vertical_flip: transform.vertical_flip,
        horizontal_flip: transform.horizontal_flip,
    };
    if let Some((from, to)) = interval_us {
        for &(index, mapped) in &seek_audio {
            stats.trimmed_audio_sample_frames = audio::mux_interval_pcm_from_path(
                source,
                &mut output,
                index,
                mapped,
                (from, to),
                options.max_packet_bytes,
            )?;
        }
        for &(index, mapped) in &reordered_secondary {
            stats.copied_packets += crate::edit::mux_reordered_video_interval(
                source,
                &mut output,
                index,
                mapped,
                from,
                to,
                options,
            )?;
        }
        for &(index, mapped) in &exact_secondary {
            stats.copied_packets += mux_exact_video_interval_from_path(
                source,
                &mut output,
                index,
                mapped,
                from,
                to,
                options,
            )?;
        }
    }
    let mut packet = Packet::new()?;
    let mut encoded = Packet::new()?;
    let mut frame = Frame::new()?;
    let mut compact: Vec<Frame> = if transform.horizontal_flip {
        (0..16).map(|_| Frame::new()).collect::<Result<Vec<_>>>()?
    } else {
        Vec::new()
    };
    let mut compact_i = 0usize;
    let mut transposed = Frame::new()?;
    let mut transpose_graph = None;
    let mut rotated = Frame::new()?;
    let mut rotate_graph = None;
    let mut padded = Frame::new()?;
    let mut pad_graph = None;
    let mut scaled = Frame::new()?;
    let mut sws = None;
    let mut epxed = Frame::new()?;
    let mut epx_graph = None;
    let mut burned = Frame::new()?;
    let mut burn_graph = None;
    let mut overlaid = Frame::new()?;
    let mut overlay_graph = None;
    let mut xfade_dst = Frame::new()?;
    let mut xfade_graph = None;
    let mut deinterlaced = Frame::new()?;
    let mut yadif_graph = None;
    let mut bwdif_out = Frame::new()?;
    let mut bwdif_graph = None;
    let mut w3fdif_out = Frame::new()?;
    let mut w3fdif_graph = None;
    let mut tblended = Frame::new()?;
    let mut tblend_graph = None;
    let mut tmixed = Frame::new()?;
    let mut tmix_graph = None;
    let mut denoised = Frame::new()?;
    let mut hqdn3d_graph = None;
    let mut blurred = Frame::new()?;
    let mut gblur_graph = None;
    let mut equalized = Frame::new()?;
    let mut eq_graph = None;
    let mut sharpened = Frame::new()?;
    let mut unsharp_graph = None;
    let mut hued = Frame::new()?;
    let mut hue_graph = None;
    let mut avgblurred = Frame::new()?;
    let mut avgblur_graph = None;
    let mut boxblurred = Frame::new()?;
    let mut boxblur_graph = None;
    let mut negated = Frame::new()?;
    let mut negate_graph = None;
    let mut edged = Frame::new()?;
    let mut edgedetect_graph = None;
    let mut sobeled = Frame::new()?;
    let mut sobel_graph = None;
    let mut prewitted = Frame::new()?;
    let mut prewitt_graph = None;
    let mut robertsed = Frame::new()?;
    let mut roberts_graph = None;
    let mut kirsched = Frame::new()?;
    let mut kirsch_graph = None;
    let mut scharred = Frame::new()?;
    let mut scharr_graph = None;
    let mut atdenoised = Frame::new()?;
    let mut atadenoise_graph = None;
    let mut owdenoised = Frame::new()?;
    let mut owdenoise_graph = None;
    let mut vaguedenoised = Frame::new()?;
    let mut vaguedenoiser_graph = None;
    let mut nldenoised = Frame::new()?;
    let mut nlmeans_graph = None;
    let mut bm3ded = Frame::new()?;
    let mut bm3d_graph = None;
    let mut dctdnoized = Frame::new()?;
    let mut dctdnoiz_graph = None;
    let mut fftdnoized = Frame::new()?;
    let mut fftdnoiz_graph = None;
    let mut smartblurred = Frame::new()?;
    let mut smartblur_graph = None;
    let mut sabbed = Frame::new()?;
    let mut sab_graph = None;
    let mut bilateraled = Frame::new()?;
    let mut bilateral_graph = None;
    let mut cased = Frame::new()?;
    let mut cas_graph = None;
    let mut vignetted = Frame::new()?;
    let mut vignette_graph = None;
    let mut curved = Frame::new()?;
    let mut curves_graph = None;
    let mut colorbalanced = Frame::new()?;
    let mut colorbalance_graph = None;
    let mut colorleveled = Frame::new()?;
    let mut colorlevels_graph = None;
    let mut colorchannelmixed = Frame::new()?;
    let mut colorchannelmixer_graph = None;
    let mut deflickered = Frame::new()?;
    let mut deflicker_graph = None;
    let mut photosensitized = Frame::new()?;
    let mut photosensitivity_graph = None;
    let mut monochromed = Frame::new()?;
    let mut monochrome_graph = None;
    let mut grayworlded = Frame::new()?;
    let mut grayworld_graph = None;
    let mut drawboxed = Frame::new()?;
    let mut drawbox_graph = None;
    let mut drawgridd = Frame::new()?;
    let mut drawgrid_graph = None;
    let mut lagfuned = Frame::new()?;
    let mut lagfun_graph = None;
    let mut amplified = Frame::new()?;
    let mut amplify_graph = None;
    let mut bitplanenoised = Frame::new()?;
    let mut bitplanenoise_graph = None;
    let mut debanded = Frame::new()?;
    let mut deband_graph = None;
    let mut gradfuned = Frame::new()?;
    let mut gradfun_graph = None;
    let mut lenscorrected = Frame::new()?;
    let mut lenscorrection_graph = None;
    let mut pixelized = Frame::new()?;
    let mut pixelize_graph = None;
    let mut removegrained = Frame::new()?;
    let mut removegrain_graph = None;
    let mut yaepblurred = Frame::new()?;
    let mut yaepblur_graph = None;
    let mut vibranced = Frame::new()?;
    let mut vibrance_graph = None;
    let mut dilated = Frame::new()?;
    let mut dilation_graph = None;
    let mut eroded = Frame::new()?;
    let mut erosion_graph = None;
    let mut colorized = Frame::new()?;
    let mut colorize_graph = None;
    let mut exposured = Frame::new()?;
    let mut exposure_graph = None;
    let mut chromashifted = Frame::new()?;
    let mut chromashift_graph = None;
    let mut colorcontrasted = Frame::new()?;
    let mut colorcontrast_graph = None;
    let mut colorcorrected = Frame::new()?;
    let mut colorcorrect_graph = None;
    let mut histeqed = Frame::new()?;
    let mut histeq_graph = None;
    let mut shuffleplaned = Frame::new()?;
    let mut shuffleplanes_graph = None;
    let mut lutyuved = Frame::new()?;
    let mut lutyuv_graph = None;
    let mut colorholded = Frame::new()?;
    let mut colorhold_graph = None;
    let mut faded = Frame::new()?;
    let mut fade_graph = None;
    let mut fade_push_mode = false;
    let mut perspectived = Frame::new()?;
    let mut perspective_graph = None;
    let mut lumakeyed = Frame::new()?;
    let mut lumakey_graph = None;
    let mut chromakeyed = Frame::new()?;
    let mut chromakey_graph = None;
    let mut colorkeyed = Frame::new()?;
    let mut colorkey_graph = None;
    let mut despilled = Frame::new()?;
    let mut despill_graph = None;
    let mut selectivecolored = Frame::new()?;
    let mut selectivecolor_graph = None;
    let mut stereo3ded = Frame::new()?;
    let mut stereo3d_graph = None;
    let mut fielded = Frame::new()?;
    let mut field_graph = None;
    let mut hqxd = Frame::new()?;
    let mut hqx_graph = None;
    let mut xbrd = Frame::new()?;
    let mut xbr_graph = None;
    let mut ild = Frame::new()?;
    let mut il_graph = None;
    let mut super2xsaid = Frame::new()?;
    let mut super2xsai_graph = None;
    let mut kerndeintd = Frame::new()?;
    let mut kerndeint_graph = None;
    let mut phased = Frame::new()?;
    let mut phase_graph = None;
    let mut phase_push_mode = false;
    let mut estdifd = Frame::new()?;
    let mut estdif_graph = None;
    let mut tinterlaced = Frame::new()?;
    let mut tinterlace_graph = None;
    let mut separatefieldsd = Frame::new()?;
    let mut separatefields_graph = None;
    let mut weaved = Frame::new()?;
    let mut weave_graph = None;
    let mut doubleweaved = Frame::new()?;
    let mut doubleweave_graph = None;
    let mut framepacked = Frame::new()?;
    let mut framepack_graph = None;
    let mut telecined = Frame::new()?;
    let mut telecine_graph = None;
    let mut pulledup = Frame::new()?;
    let mut pullup_graph = None;
    let mut decimated = Frame::new()?;
    let mut decimate_graph = None;
    let mut mpdecimated = Frame::new()?;
    let mut mpdecimate_graph = None;
    let mut framestepped = Frame::new()?;
    let mut framestep_graph = None;
    let mut tiled = Frame::new()?;
    let mut tile_graph = None;
    let mut untiled = Frame::new()?;
    let mut untile_graph = None;
    let mut shuffled = Frame::new()?;
    let mut shuffleframes_graph = None;
    let mut reversed = Frame::new()?;
    let mut reverse_graph = None;
    let mut looped = Frame::new()?;
    let mut loop_graph = None;
    let mut thumbnailed = Frame::new()?;
    let mut thumbnail_graph = None;
    let mut freezedetectd = Frame::new()?;
    let mut freezedetect_graph = None;
    let mut pseudocolored = Frame::new()?;
    let mut pseudocolor_graph = None;
    let mut minterpolate_dst = Frame::new()?;
    let mut minterpolate_graph = None;
    let mut fps_dst = Frame::new()?;
    let mut fps_graph = None;
    let mut colorspaced = Frame::new()?;
    let mut colorspace_graph = None;
    let mut zscaled = Frame::new()?;
    let mut zscale_graph = None;
    let mut tonemapped = Frame::new()?;
    let mut tonemap_graph = None;
    let mut converted = Frame::new()?;
    let mut fmt_sws = None;
    let target_pix_fmt = transform
        .pix_fmt
        .as_deref()
        .map(parse_pix_fmt)
        .transpose()?;
    if transform.overlay.is_some() && transform.xfade.is_some() {
        return Err("overlay and xfade are mutually exclusive".into());
    }
    if let Some(ref spec) = transform.xfade {
        filter::validate_xfade_transition(&spec.transition)?;
    }
    if let Some(ref args) = transform.yadif {
        filter::validate_yadif_args(args)?;
    }
    if let Some(ref args) = transform.bwdif {
        filter::validate_bwdif_args(args)?;
    }
    if let Some(ref args) = transform.w3fdif {
        filter::validate_w3fdif_args(args)?;
    }
    if let Some(ref args) = transform.tblend {
        filter::validate_tblend_args(args)?;
    }
    if let Some(ref args) = transform.tmix {
        filter::validate_tmix_args(args)?;
    }
    if let Some(ref args) = transform.hqdn3d {
        filter::validate_hqdn3d_args(args)?;
    }
    if let Some(ref args) = transform.gblur {
        filter::validate_gblur_args(args)?;
    }
    if let Some(ref args) = transform.eq {
        filter::validate_eq_args(args)?;
    }
    if let Some(ref args) = transform.unsharp {
        filter::validate_unsharp_args(args)?;
    }
    if let Some(ref args) = transform.hue {
        filter::validate_hue_args(args)?;
    }
    if let Some(ref args) = transform.avgblur {
        filter::validate_avgblur_args(args)?;
    }
    if let Some(ref args) = transform.boxblur {
        filter::validate_boxblur_args(args)?;
    }
    if let Some(ref args) = transform.negate {
        filter::validate_negate_args(args)?;
    }
    if let Some(ref args) = transform.edgedetect {
        filter::validate_edgedetect_args(args)?;
    }
    if let Some(ref args) = transform.sobel {
        filter::validate_sobel_args(args)?;
    }
    if let Some(ref args) = transform.prewitt {
        filter::validate_prewitt_args(args)?;
    }
    if let Some(ref args) = transform.roberts {
        filter::validate_roberts_args(args)?;
    }
    if let Some(ref args) = transform.kirsch {
        filter::validate_kirsch_args(args)?;
    }
    if let Some(ref args) = transform.scharr {
        filter::validate_scharr_args(args)?;
    }
    if let Some(ref args) = transform.atadenoise {
        filter::validate_atadenoise_args(args)?;
    }
    if let Some(ref args) = transform.owdenoise {
        filter::validate_owdenoise_args(args)?;
    }
    if let Some(ref args) = transform.vaguedenoiser {
        filter::validate_vaguedenoiser_args(args)?;
    }
    if let Some(ref args) = transform.nlmeans {
        filter::validate_nlmeans_args(args)?;
    }
    if let Some(ref args) = transform.bm3d {
        filter::validate_bm3d_args(args)?;
    }
    if let Some(ref args) = transform.dctdnoiz {
        filter::validate_dctdnoiz_args(args)?;
    }
    if let Some(ref args) = transform.fftdnoiz {
        filter::validate_fftdnoiz_args(args)?;
    }
    if let Some(ref args) = transform.smartblur {
        filter::validate_smartblur_args(args)?;
    }
    if let Some(ref args) = transform.sab {
        filter::validate_sab_args(args)?;
    }
    if let Some(ref args) = transform.bilateral {
        filter::validate_bilateral_args(args)?;
    }
    if let Some(ref args) = transform.cas {
        filter::validate_cas_args(args)?;
    }
    if let Some(ref args) = transform.epx {
        filter::validate_epx_args(args)?;
    }
    if let Some(ref args) = transform.vignette {
        filter::validate_vignette_args(args)?;
    }
    if let Some(ref args) = transform.curves {
        filter::validate_curves_args(args)?;
    }
    if let Some(ref args) = transform.colorbalance {
        filter::validate_colorbalance_args(args)?;
    }
    if let Some(ref args) = transform.colorlevels {
        filter::validate_colorlevels_args(args)?;
    }
    if let Some(ref args) = transform.colorchannelmixer {
        filter::validate_colorchannelmixer_args(args)?;
    }
    if let Some(ref args) = transform.deflicker {
        filter::validate_deflicker_args(args)?;
    }
    if let Some(ref args) = transform.photosensitivity {
        filter::validate_photosensitivity_args(args)?;
    }
    if let Some(ref args) = transform.monochrome {
        filter::validate_monochrome_args(args)?;
    }
    if let Some(ref args) = transform.grayworld {
        filter::validate_grayworld_args(args)?;
    }
    if let Some(ref args) = transform.drawbox {
        filter::validate_drawbox_args(args)?;
    }
    if let Some(ref args) = transform.drawgrid {
        filter::validate_drawgrid_args(args)?;
    }
    if let Some(ref args) = transform.lagfun {
        filter::validate_lagfun_args(args)?;
    }
    if let Some(ref args) = transform.amplify {
        filter::validate_amplify_args(args)?;
    }
    if let Some(ref args) = transform.bitplanenoise {
        filter::validate_bitplanenoise_args(args)?;
    }
    if let Some(ref args) = transform.deband {
        filter::validate_deband_args(args)?;
    }
    if let Some(ref args) = transform.gradfun {
        filter::validate_gradfun_args(args)?;
    }
    if let Some(ref args) = transform.lenscorrection {
        filter::validate_lenscorrection_args(args)?;
    }
    if let Some(ref args) = transform.pixelize {
        filter::validate_pixelize_args(args)?;
    }
    if let Some(ref args) = transform.removegrain {
        filter::validate_removegrain_args(args)?;
    }
    if let Some(ref args) = transform.yaepblur {
        filter::validate_yaepblur_args(args)?;
    }
    if let Some(ref args) = transform.vibrance {
        filter::validate_vibrance_args(args)?;
    }
    if let Some(ref args) = transform.dilation {
        filter::validate_dilation_args(args)?;
    }
    if let Some(ref args) = transform.erosion {
        filter::validate_erosion_args(args)?;
    }
    if let Some(ref args) = transform.colorize {
        filter::validate_colorize_args(args)?;
    }
    if let Some(ref args) = transform.exposure {
        filter::validate_exposure_args(args)?;
    }
    if let Some(ref args) = transform.chromashift {
        filter::validate_chromashift_args(args)?;
    }
    if let Some(ref args) = transform.colorcontrast {
        filter::validate_colorcontrast_args(args)?;
    }
    if let Some(ref args) = transform.colorcorrect {
        filter::validate_colorcorrect_args(args)?;
    }
    if let Some(ref args) = transform.histeq {
        filter::validate_histeq_args(args)?;
    }
    if let Some(ref args) = transform.shuffleplanes {
        filter::validate_shuffleplanes_args(args)?;
    }
    if let Some(ref args) = transform.lutyuv {
        filter::validate_lutyuv_args(args)?;
    }
    if let Some(ref args) = transform.colorhold {
        filter::validate_colorhold_args(args)?;
    }
    if let Some(ref args) = transform.fade {
        filter::validate_fade_args(args)?;
    }
    if let Some(ref args) = transform.perspective {
        filter::validate_perspective_args(args)?;
    }
    if let Some(ref args) = transform.lumakey {
        filter::validate_lumakey_args(args)?;
    }
    if let Some(ref args) = transform.chromakey {
        filter::validate_chromakey_args(args)?;
    }
    if let Some(ref args) = transform.colorkey {
        filter::validate_colorkey_args(args)?;
    }
    if let Some(ref args) = transform.despill {
        filter::validate_despill_args(args)?;
    }
    if let Some(ref args) = transform.selectivecolor {
        filter::validate_selectivecolor_args(args)?;
    }
    if let Some(ref args) = transform.stereo3d {
        filter::validate_stereo3d_args(args)?;
    }
    if let Some(ref args) = transform.field {
        filter::validate_field_args(args)?;
    }
    if let Some(ref args) = transform.hqx {
        filter::validate_hqx_args(args)?;
    }
    if let Some(ref args) = transform.xbr {
        filter::validate_xbr_args(args)?;
    }
    if let Some(ref args) = transform.il {
        filter::validate_il_args(args)?;
    }
    if let Some(ref args) = transform.super2xsai {
        filter::validate_super2xsai_args(args)?;
    }
    if let Some(ref args) = transform.kerndeint {
        filter::validate_kerndeint_args(args)?;
    }
    if let Some(ref args) = transform.phase {
        filter::validate_phase_args(args)?;
    }
    if let Some(ref args) = transform.estdif {
        filter::validate_estdif_args(args)?;
    }
    if let Some(ref args) = transform.tinterlace {
        filter::validate_tinterlace_args(args)?;
    }
    if let Some(ref args) = transform.separatefields {
        filter::validate_separatefields_args(args)?;
    }
    if let Some(ref args) = transform.weave {
        filter::validate_weave_args(args)?;
    }
    if let Some(ref args) = transform.doubleweave {
        filter::validate_doubleweave_args(args)?;
    }
    if let Some(ref args) = transform.framepack {
        filter::validate_framepack_args(args)?;
    }
    if let Some(ref args) = transform.telecine {
        filter::validate_telecine_args(args)?;
    }
    if let Some(ref args) = transform.pullup {
        filter::validate_pullup_args(args)?;
    }
    if let Some(ref args) = transform.decimate {
        filter::validate_decimate_args(args)?;
    }
    if let Some(ref args) = transform.mpdecimate {
        filter::validate_mpdecimate_args(args)?;
    }
    if let Some(ref args) = transform.framestep {
        filter::validate_framestep_args(args)?;
    }
    if let Some(ref args) = transform.tile {
        filter::validate_tile_args(args)?;
    }
    if let Some(ref args) = transform.untile {
        filter::validate_untile_args(args)?;
    }
    if let Some(ref args) = transform.shuffleframes {
        filter::validate_shuffleframes_args(args)?;
    }
    if let Some(ref args) = transform.reverse {
        filter::validate_reverse_args(args)?;
    }
    if let Some(ref args) = transform.r#loop {
        filter::validate_loop_args(args)?;
    }
    if let Some(ref args) = transform.thumbnail {
        filter::validate_thumbnail_args(args)?;
    }
    if let Some(ref args) = transform.freezedetect {
        filter::validate_freezedetect_args(args)?;
    }
    if let Some(ref args) = transform.pseudocolor {
        filter::validate_pseudocolor_args(args)?;
    }
    if let Some(ref args) = transform.minterpolate {
        filter::validate_minterpolate_args(args)?;
    }
    if let Some(ref args) = transform.fps {
        filter::validate_fps_args(args)?;
    }
    if let Some(ref args) = transform.colorspace {
        filter::validate_colorspace_args(args)?;
    }
    if let Some(ref args) = transform.zscale {
        filter::validate_zscale_args(args)?;
        if transform.pix_fmt.is_none() {
            return Err("--zscale requires --pix-fmt".into());
        }
    }
    if let Some(ref args) = transform.tonemap {
        filter::validate_tonemap_args(args)?;
        if transform.pix_fmt.is_none() {
            return Err("--tonemap requires --pix-fmt".into());
        }
    }
    while packet.read(&mut input)? {
        crate::check_budget_with_bytes(
            options,
            stats.video_packets.saturating_add(stats.copied_packets),
            0,
        )?;
        let (index, _) = packet_info(&packet, &input, options)?;
        if index == video {
            // SAFETY: Decoder retains any packet references it needs after the call.
            check(
                unsafe { avcodec_send_packet(decoder.0, packet.0) },
                "send compressed video packet",
            )?;
            drain_decoder(
                &mut decoder,
                &mut encoder,
                &mut output,
                &mut frame,
                &mut compact,
                &mut compact_i,
                &mut transposed,
                &mut transpose_graph,
                &mut rotated,
                &mut rotate_graph,
                &mut padded,
                &mut pad_graph,
                &mut scaled,
                &mut sws,
                &mut epxed,
                &mut epx_graph,
                &mut burned,
                &mut burn_graph,
                &mut overlaid,
                &mut overlay_graph,
                &mut xfade_dst,
                &mut xfade_graph,
                &mut deinterlaced,
                &mut yadif_graph,
                &mut bwdif_out,
                &mut bwdif_graph,
                &mut w3fdif_out,
                &mut w3fdif_graph,
                &mut tblended,
                &mut tblend_graph,
                &mut tmixed,
                &mut tmix_graph,
                &mut denoised,
                &mut hqdn3d_graph,
                &mut blurred,
                &mut gblur_graph,
                &mut equalized,
                &mut eq_graph,
                &mut sharpened,
                &mut unsharp_graph,
                &mut hued,
                &mut hue_graph,
                &mut avgblurred,
                &mut avgblur_graph,
                &mut boxblurred,
                &mut boxblur_graph,
                &mut negated,
                &mut negate_graph,
                &mut edged,
                &mut edgedetect_graph,
                &mut sobeled,
                &mut sobel_graph,
                &mut prewitted,
                &mut prewitt_graph,
                &mut robertsed,
                &mut roberts_graph,
                &mut kirsched,
                &mut kirsch_graph,
                &mut scharred,
                &mut scharr_graph,
                &mut atdenoised,
                &mut atadenoise_graph,
                &mut owdenoised,
                &mut owdenoise_graph,
                &mut vaguedenoised,
                &mut vaguedenoiser_graph,
                &mut nldenoised,
                &mut nlmeans_graph,
                &mut bm3ded,
                &mut bm3d_graph,
                &mut dctdnoized,
                &mut dctdnoiz_graph,
                &mut fftdnoized,
                &mut fftdnoiz_graph,
                &mut smartblurred,
                &mut smartblur_graph,
                &mut sabbed,
                &mut sab_graph,
                &mut bilateraled,
                &mut bilateral_graph,
                &mut cased,
                &mut cas_graph,
                &mut vignetted,
                &mut vignette_graph,
                &mut curved,
                &mut curves_graph,
                &mut colorbalanced,
                &mut colorbalance_graph,
                &mut colorleveled,
                &mut colorlevels_graph,
                &mut colorchannelmixed,
                &mut colorchannelmixer_graph,
                &mut deflickered,
                &mut deflicker_graph,
                &mut photosensitized,
                &mut photosensitivity_graph,
                &mut monochromed,
                &mut monochrome_graph,
                &mut grayworlded,
                &mut grayworld_graph,
                &mut drawboxed,
                &mut drawbox_graph,
                &mut drawgridd,
                &mut drawgrid_graph,
                &mut lagfuned,
                &mut lagfun_graph,
                &mut amplified,
                &mut amplify_graph,
                &mut bitplanenoised,
                &mut bitplanenoise_graph,
                &mut debanded,
                &mut deband_graph,
                &mut gradfuned,
                &mut gradfun_graph,
                &mut lenscorrected,
                &mut lenscorrection_graph,
                &mut pixelized,
                &mut pixelize_graph,
                &mut removegrained,
                &mut removegrain_graph,
                &mut yaepblurred,
                &mut yaepblur_graph,
                &mut vibranced,
                &mut vibrance_graph,
                &mut dilated,
                &mut dilation_graph,
                &mut eroded,
                &mut erosion_graph,
                &mut colorized,
                &mut colorize_graph,
                &mut exposured,
                &mut exposure_graph,
                &mut chromashifted,
                &mut chromashift_graph,
                &mut colorcontrasted,
                &mut colorcontrast_graph,
                &mut colorcorrected,
                &mut colorcorrect_graph,
                &mut histeqed,
                &mut histeq_graph,
                &mut shuffleplaned,
                &mut shuffleplanes_graph,
                &mut lutyuved,
                &mut lutyuv_graph,
                &mut colorholded,
                &mut colorhold_graph,
                &mut faded,
                &mut fade_graph,
                &mut fade_push_mode,
                &mut perspectived,
                &mut perspective_graph,
                &mut lumakeyed,
                &mut lumakey_graph,
                &mut chromakeyed,
                &mut chromakey_graph,
                &mut colorkeyed,
                &mut colorkey_graph,
                &mut despilled,
                &mut despill_graph,
                &mut selectivecolored,
                &mut selectivecolor_graph,
                &mut stereo3ded,
                &mut stereo3d_graph,
                &mut fielded,
                &mut field_graph,
                &mut hqxd,
                &mut hqx_graph,
                &mut xbrd,
                &mut xbr_graph,
                &mut ild,
                &mut il_graph,
                &mut super2xsaid,
                &mut super2xsai_graph,
                &mut kerndeintd,
                &mut kerndeint_graph,
                &mut phased,
                &mut phase_graph,
                &mut phase_push_mode,
                &mut estdifd,
                &mut estdif_graph,
                &mut tinterlaced,
                &mut tinterlace_graph,
                &mut separatefieldsd,
                &mut separatefields_graph,
                &mut weaved,
                &mut weave_graph,
                &mut doubleweaved,
                &mut doubleweave_graph,
                &mut framepacked,
                &mut framepack_graph,
                &mut telecined,
                &mut telecine_graph,
                &mut pulledup,
                &mut pullup_graph,
                &mut decimated,
                &mut decimate_graph,
                &mut mpdecimated,
                &mut mpdecimate_graph,
                &mut framestepped,
                &mut framestep_graph,
                &mut tiled,
                &mut tile_graph,
                &mut untiled,
                &mut untile_graph,
                &mut shuffled,
                &mut shuffleframes_graph,
                &mut reversed,
                &mut reverse_graph,
                &mut looped,
                &mut loop_graph,
                &mut thumbnailed,
                &mut thumbnail_graph,
                &mut freezedetectd,
                &mut freezedetect_graph,
                &mut pseudocolored,
                &mut pseudocolor_graph,
                &mut minterpolate_dst,
                &mut minterpolate_graph,
                &mut fps_dst,
                &mut fps_graph,
                &mut colorspaced,
                &mut colorspace_graph,
                &mut zscaled,
                &mut zscale_graph,
                &mut tonemapped,
                &mut tonemap_graph,
                &mut converted,
                &mut fmt_sws,
                &mut encoded,
                CropStage {
                    index: mapped,
                    crop,
                    vertical_flip: transform.vertical_flip,
                    horizontal_flip: transform.horizontal_flip,
                    scale: transform.scale,
                    epx: transform.epx.as_deref(),
                    transpose: transform.transpose,
                    rotate: transform.rotate,
                    pad: transform.pad,
                    burn_subs: transform.burn_subs.as_deref(),
                    overlay: transform.overlay.as_ref(),
                    xfade: transform.xfade.as_ref(),
                    yadif: transform.yadif.as_deref(),
                    bwdif: transform.bwdif.as_deref(),
                    w3fdif: transform.w3fdif.as_deref(),
                    tblend: transform.tblend.as_deref(),
                    tmix: transform.tmix.as_deref(),
                    hqdn3d: transform.hqdn3d.as_deref(),
                    gblur: transform.gblur.as_deref(),
                    eq: transform.eq.as_deref(),
                    unsharp: transform.unsharp.as_deref(),
                    hue: transform.hue.as_deref(),
                    avgblur: transform.avgblur.as_deref(),
                    boxblur: transform.boxblur.as_deref(),
                    negate: transform.negate.as_deref(),
                    edgedetect: transform.edgedetect.as_deref(),
                    sobel: transform.sobel.as_deref(),
                    prewitt: transform.prewitt.as_deref(),
                    roberts: transform.roberts.as_deref(),
                    kirsch: transform.kirsch.as_deref(),
                    scharr: transform.scharr.as_deref(),
                    atadenoise: transform.atadenoise.as_deref(),
                    owdenoise: transform.owdenoise.as_deref(),
                    vaguedenoiser: transform.vaguedenoiser.as_deref(),
                    nlmeans: transform.nlmeans.as_deref(),
                    bm3d: transform.bm3d.as_deref(),
                    dctdnoiz: transform.dctdnoiz.as_deref(),
                    fftdnoiz: transform.fftdnoiz.as_deref(),
                    smartblur: transform.smartblur.as_deref(),
                    sab: transform.sab.as_deref(),
                    bilateral: transform.bilateral.as_deref(),
                    cas: transform.cas.as_deref(),
                    vignette: transform.vignette.as_deref(),
                    curves: transform.curves.as_deref(),
                    colorbalance: transform.colorbalance.as_deref(),
                    colorlevels: transform.colorlevels.as_deref(),
                    colorchannelmixer: transform.colorchannelmixer.as_deref(),
                    deflicker: transform.deflicker.as_deref(),
                    photosensitivity: transform.photosensitivity.as_deref(),
                    monochrome: transform.monochrome.as_deref(),
                    grayworld: transform.grayworld.as_deref(),
                    drawbox: transform.drawbox.as_deref(),
                    drawgrid: transform.drawgrid.as_deref(),
                    lagfun: transform.lagfun.as_deref(),
                    amplify: transform.amplify.as_deref(),
                    bitplanenoise: transform.bitplanenoise.as_deref(),
                    deband: transform.deband.as_deref(),
                    gradfun: transform.gradfun.as_deref(),
                    lenscorrection: transform.lenscorrection.as_deref(),
                    pixelize: transform.pixelize.as_deref(),
                    removegrain: transform.removegrain.as_deref(),
                    yaepblur: transform.yaepblur.as_deref(),
                    vibrance: transform.vibrance.as_deref(),
                    dilation: transform.dilation.as_deref(),
                    erosion: transform.erosion.as_deref(),
                    colorize: transform.colorize.as_deref(),
                    exposure: transform.exposure.as_deref(),
                    chromashift: transform.chromashift.as_deref(),
                    colorcontrast: transform.colorcontrast.as_deref(),
                    colorcorrect: transform.colorcorrect.as_deref(),
                    histeq: transform.histeq.as_deref(),
                    shuffleplanes: transform.shuffleplanes.as_deref(),
                    lutyuv: transform.lutyuv.as_deref(),
                    colorhold: transform.colorhold.as_deref(),
                    fade: transform.fade.as_deref(),
                    perspective: transform.perspective.as_deref(),
                    lumakey: transform.lumakey.as_deref(),
                    chromakey: transform.chromakey.as_deref(),
                    colorkey: transform.colorkey.as_deref(),
                    despill: transform.despill.as_deref(),
                    selectivecolor: transform.selectivecolor.as_deref(),
                    stereo3d: transform.stereo3d.as_deref(),
                    field: transform.field.as_deref(),
                    hqx: transform.hqx.as_deref(),
                    xbr: transform.xbr.as_deref(),
                    il: transform.il.as_deref(),
                    super2xsai: transform.super2xsai.as_deref(),
                    kerndeint: transform.kerndeint.as_deref(),
                    phase: transform.phase.as_deref(),
                    estdif: transform.estdif.as_deref(),
                    tinterlace: transform.tinterlace.as_deref(),
                    separatefields: transform.separatefields.as_deref(),
                    weave: transform.weave.as_deref(),
                    doubleweave: transform.doubleweave.as_deref(),
                    framepack: transform.framepack.as_deref(),
                    telecine: transform.telecine.as_deref(),
                    pullup: transform.pullup.as_deref(),
                    decimate: transform.decimate.as_deref(),
                    mpdecimate: transform.mpdecimate.as_deref(),
                    framestep: transform.framestep.as_deref(),
                    tile: transform.tile.as_deref(),
                    untile: transform.untile.as_deref(),
                    shuffleframes: transform.shuffleframes.as_deref(),
                    reverse: transform.reverse.as_deref(),
                    r#loop: transform.r#loop.as_deref(),
                    thumbnail: transform.thumbnail.as_deref(),
                    freezedetect: transform.freezedetect.as_deref(),
                    pseudocolor: transform.pseudocolor.as_deref(),
                    minterpolate: transform.minterpolate.as_deref(),
                    fps: transform.fps.as_deref(),
                    colorspace: transform.colorspace.as_deref(),
                    zscale: transform.zscale.as_deref(),
                    tonemap: transform.tonemap.as_deref(),
                    pix_fmt: target_pix_fmt,
                    interval,
                },
                &mut stats,
            )?;
        } else if let Some(track) = decode_audio
            .iter_mut()
            .find(|track| track.stream_index == index)
        {
            if track.done {
                continue;
            }
            let (from, to) = interval_us.ok_or("compressed audio interval missing")?;
            check(
                unsafe { avcodec_send_packet(track.decoder.0, packet.0) },
                "send compressed audio packet",
            )?;
            loop {
                let code = unsafe { avcodec_receive_frame(track.decoder.0, frame.0) };
                if code == AGAIN {
                    break;
                }
                check(code, "receive decoded audio frame")?;
                track.done = audio::write_interval_pcm_frame(
                    &mut output,
                    track.mapped,
                    &frame,
                    &mut encoded,
                    &mut track.pool,
                    track.format,
                    &mut track.decoded_sample_frames,
                    &mut track.written_sample_frames,
                    &mut track.sample_bounds,
                    (from, to),
                    options.max_packet_bytes,
                )?;
                stats.trimmed_audio_sample_frames = track.written_sample_frames;
                unsafe {
                    av_frame_unref(frame.0);
                }
                if track.done {
                    break;
                }
            }
        } else if seek_audio.iter().any(|&(stream, _)| stream == index) {
            // Already written via secondary from-start demux for seek+compressed.
            continue;
        } else if reordered_secondary
            .iter()
            .any(|&(stream, _)| stream == index)
            || exact_secondary.iter().any(|&(stream, _)| stream == index)
        {
            // Secondary video written from a dedicated demuxer (seek-safe).
            continue;
        } else if let Some(mapped) = selected.iter().position(|&i| i == index) {
            // SAFETY: Index checked by packet_info; Input is live.
            let tb = unsafe { (*input.streams()[index]).time_base };
            let is_video = unsafe {
                (*(*input.streams()[index]).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
            };
            if let Some((from, to)) = transform.interval {
                if is_video {
                    // Secondary non-reordered video: exact packet-boundary stream-copy
                    // inside the presentation window, timestamps rebased to interval start.
                    if !copy_secondary_video_interval(&mut packet, &input, index, from, to)? {
                        continue;
                    }
                } else {
                    let samples = pcm::trim(&mut packet, &input, index, from, to)?;
                    if samples == 0 {
                        continue;
                    }
                    stats.trimmed_audio_sample_frames += samples;
                }
            }
            output.write(&mut packet, mapped, tb)?;
            stats.copied_packets += 1;
        }
    }
    // SAFETY: Null packets/frames are the documented drain signals for open codecs.
    check(
        unsafe { avcodec_send_packet(decoder.0, ptr::null()) },
        "drain decoder",
    )?;
    drain_decoder(
        &mut decoder,
        &mut encoder,
        &mut output,
        &mut frame,
        &mut compact,
        &mut compact_i,
        &mut transposed,
        &mut transpose_graph,
        &mut rotated,
        &mut rotate_graph,
        &mut padded,
        &mut pad_graph,
        &mut scaled,
        &mut sws,
        &mut epxed,
        &mut epx_graph,
        &mut burned,
        &mut burn_graph,
        &mut overlaid,
        &mut overlay_graph,
        &mut xfade_dst,
        &mut xfade_graph,
        &mut deinterlaced,
        &mut yadif_graph,
        &mut bwdif_out,
        &mut bwdif_graph,
        &mut w3fdif_out,
        &mut w3fdif_graph,
        &mut tblended,
        &mut tblend_graph,
        &mut tmixed,
        &mut tmix_graph,
        &mut denoised,
        &mut hqdn3d_graph,
        &mut blurred,
        &mut gblur_graph,
        &mut equalized,
        &mut eq_graph,
        &mut sharpened,
        &mut unsharp_graph,
        &mut hued,
        &mut hue_graph,
        &mut avgblurred,
        &mut avgblur_graph,
        &mut boxblurred,
        &mut boxblur_graph,
        &mut negated,
        &mut negate_graph,
        &mut edged,
        &mut edgedetect_graph,
        &mut sobeled,
        &mut sobel_graph,
        &mut prewitted,
        &mut prewitt_graph,
        &mut robertsed,
        &mut roberts_graph,
        &mut kirsched,
        &mut kirsch_graph,
        &mut scharred,
        &mut scharr_graph,
        &mut atdenoised,
        &mut atadenoise_graph,
        &mut owdenoised,
        &mut owdenoise_graph,
        &mut vaguedenoised,
        &mut vaguedenoiser_graph,
        &mut nldenoised,
        &mut nlmeans_graph,
        &mut bm3ded,
        &mut bm3d_graph,
        &mut dctdnoized,
        &mut dctdnoiz_graph,
        &mut fftdnoized,
        &mut fftdnoiz_graph,
        &mut smartblurred,
        &mut smartblur_graph,
        &mut sabbed,
        &mut sab_graph,
        &mut bilateraled,
        &mut bilateral_graph,
        &mut cased,
        &mut cas_graph,
        &mut vignetted,
        &mut vignette_graph,
        &mut curved,
        &mut curves_graph,
        &mut colorbalanced,
        &mut colorbalance_graph,
        &mut colorleveled,
        &mut colorlevels_graph,
        &mut colorchannelmixed,
        &mut colorchannelmixer_graph,
        &mut deflickered,
        &mut deflicker_graph,
        &mut photosensitized,
        &mut photosensitivity_graph,
        &mut monochromed,
        &mut monochrome_graph,
        &mut grayworlded,
        &mut grayworld_graph,
        &mut drawboxed,
        &mut drawbox_graph,
        &mut drawgridd,
        &mut drawgrid_graph,
        &mut lagfuned,
        &mut lagfun_graph,
        &mut amplified,
        &mut amplify_graph,
        &mut bitplanenoised,
        &mut bitplanenoise_graph,
        &mut debanded,
        &mut deband_graph,
        &mut gradfuned,
        &mut gradfun_graph,
        &mut lenscorrected,
        &mut lenscorrection_graph,
        &mut pixelized,
        &mut pixelize_graph,
        &mut removegrained,
        &mut removegrain_graph,
        &mut yaepblurred,
        &mut yaepblur_graph,
        &mut vibranced,
        &mut vibrance_graph,
        &mut dilated,
        &mut dilation_graph,
        &mut eroded,
        &mut erosion_graph,
        &mut colorized,
        &mut colorize_graph,
        &mut exposured,
        &mut exposure_graph,
        &mut chromashifted,
        &mut chromashift_graph,
        &mut colorcontrasted,
        &mut colorcontrast_graph,
        &mut colorcorrected,
        &mut colorcorrect_graph,
        &mut histeqed,
        &mut histeq_graph,
        &mut shuffleplaned,
        &mut shuffleplanes_graph,
        &mut lutyuved,
        &mut lutyuv_graph,
        &mut colorholded,
        &mut colorhold_graph,
        &mut faded,
        &mut fade_graph,
        &mut fade_push_mode,
        &mut perspectived,
        &mut perspective_graph,
        &mut lumakeyed,
        &mut lumakey_graph,
        &mut chromakeyed,
        &mut chromakey_graph,
        &mut colorkeyed,
        &mut colorkey_graph,
        &mut despilled,
        &mut despill_graph,
        &mut selectivecolored,
        &mut selectivecolor_graph,
        &mut stereo3ded,
        &mut stereo3d_graph,
        &mut fielded,
        &mut field_graph,
        &mut hqxd,
        &mut hqx_graph,
        &mut xbrd,
        &mut xbr_graph,
        &mut ild,
        &mut il_graph,
        &mut super2xsaid,
        &mut super2xsai_graph,
        &mut kerndeintd,
        &mut kerndeint_graph,
        &mut phased,
        &mut phase_graph,
        &mut phase_push_mode,
        &mut estdifd,
        &mut estdif_graph,
        &mut tinterlaced,
        &mut tinterlace_graph,
        &mut separatefieldsd,
        &mut separatefields_graph,
        &mut weaved,
        &mut weave_graph,
        &mut doubleweaved,
        &mut doubleweave_graph,
        &mut framepacked,
        &mut framepack_graph,
        &mut telecined,
        &mut telecine_graph,
        &mut pulledup,
        &mut pullup_graph,
        &mut decimated,
        &mut decimate_graph,
        &mut mpdecimated,
        &mut mpdecimate_graph,
        &mut framestepped,
        &mut framestep_graph,
        &mut tiled,
        &mut tile_graph,
        &mut untiled,
        &mut untile_graph,
        &mut shuffled,
        &mut shuffleframes_graph,
        &mut reversed,
        &mut reverse_graph,
        &mut looped,
        &mut loop_graph,
        &mut thumbnailed,
        &mut thumbnail_graph,
        &mut freezedetectd,
        &mut freezedetect_graph,
        &mut pseudocolored,
        &mut pseudocolor_graph,
        &mut minterpolate_dst,
        &mut minterpolate_graph,
        &mut fps_dst,
        &mut fps_graph,
        &mut colorspaced,
        &mut colorspace_graph,
        &mut zscaled,
        &mut zscale_graph,
        &mut tonemapped,
        &mut tonemap_graph,
        &mut converted,
        &mut fmt_sws,
        &mut encoded,
        CropStage {
            index: mapped,
            crop,
            vertical_flip: transform.vertical_flip,
            horizontal_flip: transform.horizontal_flip,
            scale: transform.scale,
            epx: transform.epx.as_deref(),
            transpose: transform.transpose,
            rotate: transform.rotate,
            pad: transform.pad,
            burn_subs: transform.burn_subs.as_deref(),
            overlay: transform.overlay.as_ref(),
            xfade: transform.xfade.as_ref(),
            yadif: transform.yadif.as_deref(),
            bwdif: transform.bwdif.as_deref(),
            w3fdif: transform.w3fdif.as_deref(),
            tblend: transform.tblend.as_deref(),
            tmix: transform.tmix.as_deref(),
            hqdn3d: transform.hqdn3d.as_deref(),
            gblur: transform.gblur.as_deref(),
            eq: transform.eq.as_deref(),
            unsharp: transform.unsharp.as_deref(),
            hue: transform.hue.as_deref(),
            avgblur: transform.avgblur.as_deref(),
            boxblur: transform.boxblur.as_deref(),
            negate: transform.negate.as_deref(),
            edgedetect: transform.edgedetect.as_deref(),
            sobel: transform.sobel.as_deref(),
            prewitt: transform.prewitt.as_deref(),
            roberts: transform.roberts.as_deref(),
            kirsch: transform.kirsch.as_deref(),
            scharr: transform.scharr.as_deref(),
            atadenoise: transform.atadenoise.as_deref(),
            owdenoise: transform.owdenoise.as_deref(),
            vaguedenoiser: transform.vaguedenoiser.as_deref(),
            nlmeans: transform.nlmeans.as_deref(),
            bm3d: transform.bm3d.as_deref(),
            dctdnoiz: transform.dctdnoiz.as_deref(),
            fftdnoiz: transform.fftdnoiz.as_deref(),
            smartblur: transform.smartblur.as_deref(),
            sab: transform.sab.as_deref(),
            bilateral: transform.bilateral.as_deref(),
            cas: transform.cas.as_deref(),
            vignette: transform.vignette.as_deref(),
            curves: transform.curves.as_deref(),
            colorbalance: transform.colorbalance.as_deref(),
            colorlevels: transform.colorlevels.as_deref(),
            colorchannelmixer: transform.colorchannelmixer.as_deref(),
            deflicker: transform.deflicker.as_deref(),
            photosensitivity: transform.photosensitivity.as_deref(),
            monochrome: transform.monochrome.as_deref(),
            grayworld: transform.grayworld.as_deref(),
            drawbox: transform.drawbox.as_deref(),
            drawgrid: transform.drawgrid.as_deref(),
            lagfun: transform.lagfun.as_deref(),
            amplify: transform.amplify.as_deref(),
            bitplanenoise: transform.bitplanenoise.as_deref(),
            deband: transform.deband.as_deref(),
            gradfun: transform.gradfun.as_deref(),
            lenscorrection: transform.lenscorrection.as_deref(),
            pixelize: transform.pixelize.as_deref(),
            removegrain: transform.removegrain.as_deref(),
            yaepblur: transform.yaepblur.as_deref(),
            vibrance: transform.vibrance.as_deref(),
            dilation: transform.dilation.as_deref(),
            erosion: transform.erosion.as_deref(),
            colorize: transform.colorize.as_deref(),
            exposure: transform.exposure.as_deref(),
            chromashift: transform.chromashift.as_deref(),
            colorcontrast: transform.colorcontrast.as_deref(),
            colorcorrect: transform.colorcorrect.as_deref(),
            histeq: transform.histeq.as_deref(),
            shuffleplanes: transform.shuffleplanes.as_deref(),
            lutyuv: transform.lutyuv.as_deref(),
            colorhold: transform.colorhold.as_deref(),
            fade: transform.fade.as_deref(),
            perspective: transform.perspective.as_deref(),
            lumakey: transform.lumakey.as_deref(),
            chromakey: transform.chromakey.as_deref(),
            colorkey: transform.colorkey.as_deref(),
            despill: transform.despill.as_deref(),
            selectivecolor: transform.selectivecolor.as_deref(),
            stereo3d: transform.stereo3d.as_deref(),
            field: transform.field.as_deref(),
            hqx: transform.hqx.as_deref(),
            xbr: transform.xbr.as_deref(),
            il: transform.il.as_deref(),
            super2xsai: transform.super2xsai.as_deref(),
            kerndeint: transform.kerndeint.as_deref(),
            phase: transform.phase.as_deref(),
            estdif: transform.estdif.as_deref(),
            tinterlace: transform.tinterlace.as_deref(),
            separatefields: transform.separatefields.as_deref(),
            weave: transform.weave.as_deref(),
            doubleweave: transform.doubleweave.as_deref(),
            framepack: transform.framepack.as_deref(),
            telecine: transform.telecine.as_deref(),
            pullup: transform.pullup.as_deref(),
            decimate: transform.decimate.as_deref(),
            mpdecimate: transform.mpdecimate.as_deref(),
            framestep: transform.framestep.as_deref(),
            tile: transform.tile.as_deref(),
            untile: transform.untile.as_deref(),
            shuffleframes: transform.shuffleframes.as_deref(),
            reverse: transform.reverse.as_deref(),
            r#loop: transform.r#loop.as_deref(),
            thumbnail: transform.thumbnail.as_deref(),
            freezedetect: transform.freezedetect.as_deref(),
            pseudocolor: transform.pseudocolor.as_deref(),
            minterpolate: transform.minterpolate.as_deref(),
            fps: transform.fps.as_deref(),
            colorspace: transform.colorspace.as_deref(),
            zscale: transform.zscale.as_deref(),
            tonemap: transform.tonemap.as_deref(),
            pix_fmt: target_pix_fmt,
            interval,
        },
        &mut stats,
    )?;
    if transform.fade.is_some() && fade_push_mode {
        unsafe {
            filter::fade_flush(&mut fade_graph, faded.0, |mut send| {
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.phase.is_some() && phase_push_mode {
        unsafe {
            filter::phase_flush(&mut phase_graph, phased.0, |mut send| {
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.estdif.is_some() {
        unsafe {
            filter::estdif_flush(&mut estdif_graph, estdifd.0, |mut send| {
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.tinterlace.is_some() {
        unsafe {
            filter::tinterlace_flush(&mut tinterlace_graph, tinterlaced.0, |mut send| {
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.separatefields.is_some() {
        unsafe {
            filter::separatefields_flush(
                &mut separatefields_graph,
                separatefieldsd.0,
                |mut send| {
                    let mut finish = |mut send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.freezedetect {
                            let src_fmt = (*send).format;
                            filter::freezedetect_frame(
                                &mut freezedetect_graph,
                                freezedetectd.0,
                                send,
                                args,
                            )?;
                            let out = freezedetectd.0;
                            if (*out).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                                send = converted.0;
                            } else {
                                send = out;
                            }
                        }
                        if let Some(ref args) = transform.pseudocolor {
                            let src_fmt = (*send).format;
                            filter::pseudocolor_frame(
                                &mut pseudocolor_graph,
                                pseudocolored.0,
                                send,
                                args,
                            )?;
                            let out = pseudocolored.0;
                            if (*out).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                                send = converted.0;
                            } else {
                                send = out;
                            }
                        }
                        if let Some(ref args) = transform.colorspace {
                            filter::colorspace_frame(
                                &mut colorspace_graph,
                                colorspaced.0,
                                send,
                                args,
                            )?;
                            send = colorspaced.0;
                        }
                        let mut format_done = false;
                        if let Some(ref args) = transform.zscale {
                            let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                            send = zscaled.0;
                            format_done = true;
                        }
                        if let Some(ref args) = transform.tonemap {
                            let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            filter::tonemap_frame(
                                &mut tonemap_graph,
                                tonemapped.0,
                                send,
                                args,
                                &name,
                            )?;
                            send = tonemapped.0;
                            format_done = true;
                        }
                        if let Some(fmt) = target_pix_fmt {
                            if !format_done && (*send).format != fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                                send = converted.0;
                            }
                        }
                        if transform.minterpolate.is_some() || transform.fps.is_some() {
                            let mut emitted = 0u64;
                            filter::temporal_push_frame(
                                &mut minterpolate_graph,
                                minterpolate_dst.0,
                                transform.minterpolate.as_deref(),
                                &mut fps_graph,
                                fps_dst.0,
                                transform.fps.as_deref(),
                                send,
                                |out| {
                                    send_encoder_frame(
                                        &mut encoder,
                                        &mut output,
                                        &mut encoded,
                                        mapped,
                                        out,
                                        &mut stats,
                                    )?;
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                            stats.video_frames += emitted;
                        } else {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                send,
                                &mut stats,
                            )?;
                            stats.video_frames += 1;
                        }
                        Ok(())
                    };
                    let mut apply_framestep = |send: *mut AVFrame| -> Result<()> {
                        if transform.framestep.is_some() {
                            filter::push_framestep_or_emit(
                                &mut framestep_graph,
                                framestepped.0,
                                send,
                                transform.framestep.as_deref(),
                                |f| finish(f),
                            )
                        } else {
                            finish(send)
                        }
                    };
                    let mut apply_mpdecimate = |send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.mpdecimate {
                            filter::mpdecimate_push_frame(
                                &mut mpdecimate_graph,
                                mpdecimated.0,
                                send,
                                args,
                                |mpd| apply_framestep(mpd),
                            )
                        } else {
                            apply_framestep(send)
                        }
                    };
                    let mut apply_decimate = |send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.decimate {
                            filter::decimate_push_frame(
                                &mut decimate_graph,
                                decimated.0,
                                send,
                                args,
                                |dc| apply_mpdecimate(dc),
                            )?;
                        } else {
                            apply_mpdecimate(send)?;
                        }
                        Ok(())
                    };
                    let mut apply_pullup = |send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.pullup {
                            filter::pullup_push_frame(
                                &mut pullup_graph,
                                pulledup.0,
                                send,
                                args,
                                |pu| apply_decimate(pu),
                            )?;
                        } else {
                            apply_decimate(send)?;
                        }
                        Ok(())
                    };
                    let mut apply_telecine = |send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.telecine {
                            filter::telecine_push_frame(
                                &mut telecine_graph,
                                telecined.0,
                                send,
                                args,
                                |tc| apply_pullup(tc),
                            )?;
                        } else {
                            apply_pullup(send)?;
                        }
                        Ok(())
                    };
                    let mut apply_framepack = |send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.framepack {
                            filter::framepack_push_frame(
                                &mut framepack_graph,
                                framepacked.0,
                                send,
                                args,
                                |packed| apply_telecine(packed),
                            )?;
                        } else {
                            apply_telecine(send)?;
                        }
                        Ok(())
                    };
                    let mut apply_doubleweave = |send: *mut AVFrame| -> Result<()> {
                        if let Some(ref args) = transform.doubleweave {
                            filter::doubleweave_push_frame(
                                &mut doubleweave_graph,
                                doubleweaved.0,
                                send,
                                args,
                                |doubled| apply_framepack(doubled),
                            )?;
                        } else {
                            apply_framepack(send)?;
                        }
                        Ok(())
                    };
                    if let Some(ref args) = transform.weave {
                        filter::weave_push_frame(
                            &mut weave_graph,
                            weaved.0,
                            send,
                            args,
                            |woven| apply_doubleweave(woven),
                        )?;
                    } else {
                        apply_doubleweave(send)?;
                    }
                    Ok(())
                },
            )?;
        }
    }
    if transform.weave.is_some() {
        unsafe {
            filter::weave_flush(&mut weave_graph, weaved.0, |mut send| {
                let mut finish = |mut send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.freezedetect {
                        let src_fmt = (*send).format;
                        filter::freezedetect_frame(
                            &mut freezedetect_graph,
                            freezedetectd.0,
                            send,
                            args,
                        )?;
                        let out = freezedetectd.0;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            send = converted.0;
                        } else {
                            send = out;
                        }
                    }
                    if let Some(ref args) = transform.pseudocolor {
                        let src_fmt = (*send).format;
                        filter::pseudocolor_frame(
                            &mut pseudocolor_graph,
                            pseudocolored.0,
                            send,
                            args,
                        )?;
                        let out = pseudocolored.0;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            send = converted.0;
                        } else {
                            send = out;
                        }
                    }
                    if let Some(ref args) = transform.colorspace {
                        filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                        send = colorspaced.0;
                    }
                    let mut format_done = false;
                    if let Some(ref args) = transform.zscale {
                        let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                        send = zscaled.0;
                        format_done = true;
                    }
                    if let Some(ref args) = transform.tonemap {
                        let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                        send = tonemapped.0;
                        format_done = true;
                    }
                    if let Some(fmt) = target_pix_fmt {
                        if !format_done && (*send).format != fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                            send = converted.0;
                        }
                    }
                    if transform.minterpolate.is_some() || transform.fps.is_some() {
                        let mut emitted = 0u64;
                        filter::temporal_push_frame(
                            &mut minterpolate_graph,
                            minterpolate_dst.0,
                            transform.minterpolate.as_deref(),
                            &mut fps_graph,
                            fps_dst.0,
                            transform.fps.as_deref(),
                            send,
                            |out| {
                                send_encoder_frame(
                                    &mut encoder,
                                    &mut output,
                                    &mut encoded,
                                    mapped,
                                    out,
                                    &mut stats,
                                )?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                        stats.video_frames += emitted;
                    } else {
                        send_encoder_frame(
                            &mut encoder,
                            &mut output,
                            &mut encoded,
                            mapped,
                            send,
                            &mut stats,
                        )?;
                        stats.video_frames += 1;
                    }
                    Ok(())
                };
                let mut apply_framestep = |send: *mut AVFrame| -> Result<()> {
                    if transform.framestep.is_some() {
                        filter::push_framestep_or_emit(
                            &mut framestep_graph,
                            framestepped.0,
                            send,
                            transform.framestep.as_deref(),
                            |f| finish(f),
                        )
                    } else {
                        finish(send)
                    }
                };
                let mut apply_mpdecimate = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.mpdecimate {
                        filter::mpdecimate_push_frame(
                            &mut mpdecimate_graph,
                            mpdecimated.0,
                            send,
                            args,
                            |mpd| apply_framestep(mpd),
                        )
                    } else {
                        apply_framestep(send)
                    }
                };
                let mut apply_decimate = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.decimate {
                        filter::decimate_push_frame(
                            &mut decimate_graph,
                            decimated.0,
                            send,
                            args,
                            |dc| apply_mpdecimate(dc),
                        )?;
                    } else {
                        apply_mpdecimate(send)?;
                    }
                    Ok(())
                };
                let mut apply_pullup = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.pullup {
                        filter::pullup_push_frame(
                            &mut pullup_graph,
                            pulledup.0,
                            send,
                            args,
                            |pu| apply_decimate(pu),
                        )?;
                    } else {
                        apply_decimate(send)?;
                    }
                    Ok(())
                };
                let mut apply_telecine = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.telecine {
                        filter::telecine_push_frame(
                            &mut telecine_graph,
                            telecined.0,
                            send,
                            args,
                            |tc| apply_pullup(tc),
                        )?;
                    } else {
                        apply_pullup(send)?;
                    }
                    Ok(())
                };
                let mut apply_framepack = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.framepack {
                        filter::framepack_push_frame(
                            &mut framepack_graph,
                            framepacked.0,
                            send,
                            args,
                            |packed| apply_telecine(packed),
                        )?;
                    } else {
                        apply_telecine(send)?;
                    }
                    Ok(())
                };
                if let Some(ref args) = transform.doubleweave {
                    filter::doubleweave_push_frame(
                        &mut doubleweave_graph,
                        doubleweaved.0,
                        send,
                        args,
                        |doubled| apply_framepack(doubled),
                    )?;
                } else {
                    apply_framepack(send)?;
                }
                Ok(())
            })?;
        }
    }
    if transform.doubleweave.is_some() {
        unsafe {
            filter::doubleweave_flush(&mut doubleweave_graph, doubleweaved.0, |mut send| {
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.framepack.is_some() {
        unsafe {
            filter::framepack_flush(&mut framepack_graph, framepacked.0, |mut send| {
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.telecine.is_some() && transform.framepack.is_none() {
        unsafe {
            filter::telecine_flush(&mut telecine_graph, telecined.0, |mut send| {
                let mut after_decimate = |mut send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.freezedetect {
                        let src_fmt = (*send).format;
                        filter::freezedetect_frame(
                            &mut freezedetect_graph,
                            freezedetectd.0,
                            send,
                            args,
                        )?;
                        let out = freezedetectd.0;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            send = converted.0;
                        } else {
                            send = out;
                        }
                    }
                    if let Some(ref args) = transform.pseudocolor {
                        let src_fmt = (*send).format;
                        filter::pseudocolor_frame(
                            &mut pseudocolor_graph,
                            pseudocolored.0,
                            send,
                            args,
                        )?;
                        let out = pseudocolored.0;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            send = converted.0;
                        } else {
                            send = out;
                        }
                    }
                    if let Some(ref args) = transform.colorspace {
                        filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                        send = colorspaced.0;
                    }
                    let mut format_done = false;
                    if let Some(ref args) = transform.zscale {
                        let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                        send = zscaled.0;
                        format_done = true;
                    }
                    if let Some(ref args) = transform.tonemap {
                        let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                        send = tonemapped.0;
                        format_done = true;
                    }
                    if let Some(fmt) = target_pix_fmt {
                        if !format_done && (*send).format != fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                            send = converted.0;
                        }
                    }
                    if transform.minterpolate.is_some() || transform.fps.is_some() {
                        let mut emitted = 0u64;
                        filter::temporal_push_frame(
                            &mut minterpolate_graph,
                            minterpolate_dst.0,
                            transform.minterpolate.as_deref(),
                            &mut fps_graph,
                            fps_dst.0,
                            transform.fps.as_deref(),
                            send,
                            |out| {
                                send_encoder_frame(
                                    &mut encoder,
                                    &mut output,
                                    &mut encoded,
                                    mapped,
                                    out,
                                    &mut stats,
                                )?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                        stats.video_frames += emitted;
                    } else {
                        send_encoder_frame(
                            &mut encoder,
                            &mut output,
                            &mut encoded,
                            mapped,
                            send,
                            &mut stats,
                        )?;
                        stats.video_frames += 1;
                    }
                    Ok(())
                };
                let mut apply_framestep = |send: *mut AVFrame| -> Result<()> {
                    if transform.framestep.is_some() {
                        filter::push_framestep_or_emit(
                            &mut framestep_graph,
                            framestepped.0,
                            send,
                            transform.framestep.as_deref(),
                            |f| after_decimate(f),
                        )
                    } else {
                        after_decimate(send)
                    }
                };
                let mut apply_mpdecimate = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.mpdecimate {
                        filter::mpdecimate_push_frame(
                            &mut mpdecimate_graph,
                            mpdecimated.0,
                            send,
                            args,
                            |mpd| apply_framestep(mpd),
                        )
                    } else {
                        apply_framestep(send)
                    }
                };
                let mut apply_decimate = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.decimate {
                        filter::decimate_push_frame(
                            &mut decimate_graph,
                            decimated.0,
                            send,
                            args,
                            |dc| apply_mpdecimate(dc),
                        )
                    } else {
                        apply_mpdecimate(send)
                    }
                };
                if let Some(ref args) = transform.pullup {
                    filter::pullup_push_frame(&mut pullup_graph, pulledup.0, send, args, |pu| {
                        apply_decimate(pu)
                    })
                } else {
                    apply_decimate(send)
                }
            })?;
        }
    }
    if transform.pullup.is_some() && transform.framepack.is_none() && transform.telecine.is_none() {
        unsafe {
            filter::pullup_flush(&mut pullup_graph, pulledup.0, |send| {
                let mut after_decimate = |mut send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.freezedetect {
                        let src_fmt = (*send).format;
                        filter::freezedetect_frame(
                            &mut freezedetect_graph,
                            freezedetectd.0,
                            send,
                            args,
                        )?;
                        let out = freezedetectd.0;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            send = converted.0;
                        } else {
                            send = out;
                        }
                    }
                    if let Some(ref args) = transform.pseudocolor {
                        let src_fmt = (*send).format;
                        filter::pseudocolor_frame(
                            &mut pseudocolor_graph,
                            pseudocolored.0,
                            send,
                            args,
                        )?;
                        let out = pseudocolored.0;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            send = converted.0;
                        } else {
                            send = out;
                        }
                    }
                    if let Some(ref args) = transform.colorspace {
                        filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                        send = colorspaced.0;
                    }
                    let mut format_done = false;
                    if let Some(ref args) = transform.zscale {
                        let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown zscale output pixel format".into());
                        }
                        filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                        send = zscaled.0;
                        format_done = true;
                    }
                    if let Some(ref args) = transform.tonemap {
                        let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                        let name = string(av_get_pix_fmt_name(fmt));
                        if name.is_empty() {
                            return Err("unknown tonemap output pixel format".into());
                        }
                        filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                        send = tonemapped.0;
                        format_done = true;
                    }
                    if let Some(fmt) = target_pix_fmt {
                        if !format_done && (*send).format != fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                            send = converted.0;
                        }
                    }
                    if transform.minterpolate.is_some() || transform.fps.is_some() {
                        let mut emitted = 0u64;
                        filter::temporal_push_frame(
                            &mut minterpolate_graph,
                            minterpolate_dst.0,
                            transform.minterpolate.as_deref(),
                            &mut fps_graph,
                            fps_dst.0,
                            transform.fps.as_deref(),
                            send,
                            |out| {
                                send_encoder_frame(
                                    &mut encoder,
                                    &mut output,
                                    &mut encoded,
                                    mapped,
                                    out,
                                    &mut stats,
                                )?;
                                emitted += 1;
                                Ok(())
                            },
                        )?;
                        stats.video_frames += emitted;
                    } else {
                        send_encoder_frame(
                            &mut encoder,
                            &mut output,
                            &mut encoded,
                            mapped,
                            send,
                            &mut stats,
                        )?;
                        stats.video_frames += 1;
                    }
                    Ok(())
                };
                let mut apply_framestep = |send: *mut AVFrame| -> Result<()> {
                    if transform.framestep.is_some() {
                        filter::push_framestep_or_emit(
                            &mut framestep_graph,
                            framestepped.0,
                            send,
                            transform.framestep.as_deref(),
                            |f| after_decimate(f),
                        )
                    } else {
                        after_decimate(send)
                    }
                };
                let mut apply_mpdecimate = |send: *mut AVFrame| -> Result<()> {
                    if let Some(ref args) = transform.mpdecimate {
                        filter::mpdecimate_push_frame(
                            &mut mpdecimate_graph,
                            mpdecimated.0,
                            send,
                            args,
                            |mpd| apply_framestep(mpd),
                        )
                    } else {
                        apply_framestep(send)
                    }
                };
                if let Some(ref args) = transform.decimate {
                    filter::decimate_push_frame(
                        &mut decimate_graph,
                        decimated.0,
                        send,
                        args,
                        |dc| apply_mpdecimate(dc),
                    )
                } else {
                    apply_mpdecimate(send)
                }
            })?;
        }
    }
    if transform.decimate.is_some()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
    {
        unsafe {
            let mut after_tile = |mut send: *mut AVFrame| -> Result<()> {
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            };
            let mut apply_thumbnail = |send: *mut AVFrame| -> Result<()> {
                if transform.thumbnail.is_some() {
                    filter::push_thumbnail_or_emit(
                        &mut thumbnail_graph,
                        thumbnailed.0,
                        send,
                        transform.thumbnail.as_deref(),
                        |f| after_tile(f),
                    )
                } else {
                    after_tile(send)
                }
            };
            let mut apply_loop = |send: *mut AVFrame| -> Result<()> {
                if transform.r#loop.is_some() {
                    filter::push_loop_or_emit(
                        &mut loop_graph,
                        looped.0,
                        send,
                        transform.r#loop.as_deref(),
                        |f| apply_thumbnail(f),
                    )
                } else {
                    apply_thumbnail(send)
                }
            };
            let mut apply_reverse = |send: *mut AVFrame| -> Result<()> {
                if transform.reverse.is_some() {
                    filter::push_reverse_or_emit(
                        &mut reverse_graph,
                        reversed.0,
                        send,
                        transform.reverse.as_deref(),
                        |f| apply_loop(f),
                    )
                } else {
                    apply_loop(send)
                }
            };
            let mut apply_shuffleframes = |send: *mut AVFrame| -> Result<()> {
                if transform.shuffleframes.is_some() {
                    filter::push_shuffleframes_or_emit(
                        &mut shuffleframes_graph,
                        shuffled.0,
                        send,
                        transform.shuffleframes.as_deref(),
                        |f| apply_reverse(f),
                    )
                } else {
                    apply_reverse(send)
                }
            };
            let mut apply_untile = |send: *mut AVFrame| -> Result<()> {
                if transform.untile.is_some() {
                    filter::push_untile_or_emit(
                        &mut untile_graph,
                        untiled.0,
                        send,
                        transform.untile.as_deref(),
                        |f| apply_shuffleframes(f),
                    )
                } else {
                    apply_shuffleframes(send)
                }
            };
            let mut apply_framestep = |send: *mut AVFrame| -> Result<()> {
                if transform.framestep.is_some() {
                    filter::push_framestep_or_emit(
                        &mut framestep_graph,
                        framestepped.0,
                        send,
                        transform.framestep.as_deref(),
                        |f| apply_untile(f),
                    )
                } else {
                    apply_untile(send)
                }
            };
            let mut apply_mpdecimate = |send: *mut AVFrame| -> Result<()> {
                if let Some(ref args) = transform.mpdecimate {
                    filter::mpdecimate_push_frame(
                        &mut mpdecimate_graph,
                        mpdecimated.0,
                        send,
                        args,
                        |mpd| apply_framestep(mpd),
                    )
                } else {
                    apply_framestep(send)
                }
            };
            filter::decimate_flush(&mut decimate_graph, decimated.0, |send| {
                apply_mpdecimate(send)
            })?;
        }
    }
    let mut flush_after_tile = |mut send: *mut AVFrame| -> Result<()> {
        unsafe {
            if let Some(ref args) = transform.freezedetect {
                let src_fmt = (*send).format;
                filter::freezedetect_frame(&mut freezedetect_graph, freezedetectd.0, send, args)?;
                let out = freezedetectd.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                    send = converted.0;
                } else {
                    send = out;
                }
            }
            if let Some(ref args) = transform.pseudocolor {
                let src_fmt = (*send).format;
                filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                let out = pseudocolored.0;
                if (*out).format != src_fmt {
                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                    send = converted.0;
                } else {
                    send = out;
                }
            }
            if let Some(ref args) = transform.colorspace {
                filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                send = colorspaced.0;
            }
            let mut format_done = false;
            if let Some(ref args) = transform.zscale {
                let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                let name = string(av_get_pix_fmt_name(fmt));
                if name.is_empty() {
                    return Err("unknown zscale output pixel format".into());
                }
                filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                send = zscaled.0;
                format_done = true;
            }
            if let Some(ref args) = transform.tonemap {
                let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                let name = string(av_get_pix_fmt_name(fmt));
                if name.is_empty() {
                    return Err("unknown tonemap output pixel format".into());
                }
                filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                send = tonemapped.0;
                format_done = true;
            }
            if let Some(fmt) = target_pix_fmt {
                if !format_done && (*send).format != fmt {
                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                    send = converted.0;
                }
            }
            if transform.minterpolate.is_some() || transform.fps.is_some() {
                let mut emitted = 0u64;
                filter::temporal_push_frame(
                    &mut minterpolate_graph,
                    minterpolate_dst.0,
                    transform.minterpolate.as_deref(),
                    &mut fps_graph,
                    fps_dst.0,
                    transform.fps.as_deref(),
                    send,
                    |out| {
                        send_encoder_frame(
                            &mut encoder,
                            &mut output,
                            &mut encoded,
                            mapped,
                            out,
                            &mut stats,
                        )?;
                        emitted += 1;
                        Ok(())
                    },
                )?;
                stats.video_frames += emitted;
            } else {
                send_encoder_frame(
                    &mut encoder,
                    &mut output,
                    &mut encoded,
                    mapped,
                    send,
                    &mut stats,
                )?;
                stats.video_frames += 1;
            }
        }
        Ok(())
    };
    if transform.mpdecimate.is_some()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
    {
        unsafe {
            filter::mpdecimate_flush(&mut mpdecimate_graph, mpdecimated.0, |send| {
                if transform.framestep.is_some() {
                    filter::push_framestep_or_emit(
                        &mut framestep_graph,
                        framestepped.0,
                        send,
                        transform.framestep.as_deref(),
                        |send| {
                            filter::push_tile_or_emit(
                                &mut tile_graph,
                                tiled.0,
                                send,
                                transform.tile.as_deref(),
                                |send| {
                                    filter::push_untile_or_emit(
                                        &mut untile_graph,
                                        untiled.0,
                                        send,
                                        transform.untile.as_deref(),
                                        |send| {
                                            filter::push_shuffleframes_or_emit(
                                                &mut shuffleframes_graph,
                                                shuffled.0,
                                                send,
                                                transform.shuffleframes.as_deref(),
                                                |send| {
                                                    filter::push_reverse_or_emit(
                                                        &mut reverse_graph,
                                                        reversed.0,
                                                        send,
                                                        transform.reverse.as_deref(),
                                                        |send| unsafe {
                                                            filter::push_loop_or_emit(
                                                                &mut loop_graph,
                                                                looped.0,
                                                                send,
                                                                transform.r#loop.as_deref(),
                                                                |send| {
                                                                    if transform.thumbnail.is_some()
                                                                    {
                                                                        filter::push_thumbnail_or_emit(
                                                                &mut thumbnail_graph,
                                                                thumbnailed.0,
                                                                send,
                                                                transform.thumbnail.as_deref(),
                                                                |f| flush_after_tile(f),
                                                            )
                                                                    } else {
                                                                        flush_after_tile(send)
                                                                    }
                                                                },
                                                            )
                                                        },
                                                    )
                                                },
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                } else {
                    filter::push_tile_or_emit(
                        &mut tile_graph,
                        tiled.0,
                        send,
                        transform.tile.as_deref(),
                        |send| {
                            filter::push_untile_or_emit(
                                &mut untile_graph,
                                untiled.0,
                                send,
                                transform.untile.as_deref(),
                                |send| {
                                    filter::push_shuffleframes_or_emit(
                                        &mut shuffleframes_graph,
                                        shuffled.0,
                                        send,
                                        transform.shuffleframes.as_deref(),
                                        |send| {
                                            filter::push_reverse_or_emit(
                                                &mut reverse_graph,
                                                reversed.0,
                                                send,
                                                transform.reverse.as_deref(),
                                                |send| unsafe {
                                                    filter::push_loop_or_emit(
                                                        &mut loop_graph,
                                                        looped.0,
                                                        send,
                                                        transform.r#loop.as_deref(),
                                                        |send| {
                                                            if transform.thumbnail.is_some() {
                                                                filter::push_thumbnail_or_emit(
                                                                    &mut thumbnail_graph,
                                                                    thumbnailed.0,
                                                                    send,
                                                                    transform.thumbnail.as_deref(),
                                                                    |f| flush_after_tile(f),
                                                                )
                                                            } else {
                                                                flush_after_tile(send)
                                                            }
                                                        },
                                                    )
                                                },
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                }
            })?;
        }
    }
    if transform.framestep.is_some()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::framestep_flush(&mut framestep_graph, framestepped.0, |send| {
                filter::push_tile_or_emit(
                    &mut tile_graph,
                    tiled.0,
                    send,
                    transform.tile.as_deref(),
                    |send| {
                        filter::push_untile_or_emit(
                            &mut untile_graph,
                            untiled.0,
                            send,
                            transform.untile.as_deref(),
                            |send| {
                                filter::push_shuffleframes_or_emit(
                                    &mut shuffleframes_graph,
                                    shuffled.0,
                                    send,
                                    transform.shuffleframes.as_deref(),
                                    |send| {
                                        filter::push_reverse_or_emit(
                                            &mut reverse_graph,
                                            reversed.0,
                                            send,
                                            transform.reverse.as_deref(),
                                            |send| unsafe {
                                                filter::push_loop_or_emit(
                                                    &mut loop_graph,
                                                    looped.0,
                                                    send,
                                                    transform.r#loop.as_deref(),
                                                    |send| {
                                                        if transform.thumbnail.is_some() {
                                                            filter::push_thumbnail_or_emit(
                                                                &mut thumbnail_graph,
                                                                thumbnailed.0,
                                                                send,
                                                                transform.thumbnail.as_deref(),
                                                                |f| flush_after_tile(f),
                                                            )
                                                        } else {
                                                            flush_after_tile(send)
                                                        }
                                                    },
                                                )
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                )
            })?;
        }
    }
    if transform.tile.is_some()
        && transform.framestep.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::tile_flush(&mut tile_graph, tiled.0, |send| {
                filter::push_untile_or_emit(
                    &mut untile_graph,
                    untiled.0,
                    send,
                    transform.untile.as_deref(),
                    |send| {
                        filter::push_shuffleframes_or_emit(
                            &mut shuffleframes_graph,
                            shuffled.0,
                            send,
                            transform.shuffleframes.as_deref(),
                            |send| {
                                filter::push_reverse_or_emit(
                                    &mut reverse_graph,
                                    reversed.0,
                                    send,
                                    transform.reverse.as_deref(),
                                    |send| unsafe {
                                        filter::push_loop_or_emit(
                                            &mut loop_graph,
                                            looped.0,
                                            send,
                                            transform.r#loop.as_deref(),
                                            |send| {
                                                if transform.thumbnail.is_some() {
                                                    filter::push_thumbnail_or_emit(
                                                        &mut thumbnail_graph,
                                                        thumbnailed.0,
                                                        send,
                                                        transform.thumbnail.as_deref(),
                                                        |f| flush_after_tile(f),
                                                    )
                                                } else {
                                                    flush_after_tile(send)
                                                }
                                            },
                                        )
                                    },
                                )
                            },
                        )
                    },
                )
            })?;
        }
    }
    if transform.untile.is_some()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::untile_flush(&mut untile_graph, untiled.0, |send| {
                filter::push_shuffleframes_or_emit(
                    &mut shuffleframes_graph,
                    shuffled.0,
                    send,
                    transform.shuffleframes.as_deref(),
                    |send| {
                        filter::push_reverse_or_emit(
                            &mut reverse_graph,
                            reversed.0,
                            send,
                            transform.reverse.as_deref(),
                            |send| unsafe {
                                filter::push_loop_or_emit(
                                    &mut loop_graph,
                                    looped.0,
                                    send,
                                    transform.r#loop.as_deref(),
                                    |send| {
                                        if transform.thumbnail.is_some() {
                                            filter::push_thumbnail_or_emit(
                                                &mut thumbnail_graph,
                                                thumbnailed.0,
                                                send,
                                                transform.thumbnail.as_deref(),
                                                |f| flush_after_tile(f),
                                            )
                                        } else {
                                            flush_after_tile(send)
                                        }
                                    },
                                )
                            },
                        )
                    },
                )
            })?;
        }
    }
    if transform.shuffleframes.is_some()
        && transform.untile.is_none()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::shuffleframes_flush(&mut shuffleframes_graph, shuffled.0, |send| {
                filter::push_reverse_or_emit(
                    &mut reverse_graph,
                    reversed.0,
                    send,
                    transform.reverse.as_deref(),
                    |send| unsafe {
                        filter::push_loop_or_emit(
                            &mut loop_graph,
                            looped.0,
                            send,
                            transform.r#loop.as_deref(),
                            |send| {
                                if transform.thumbnail.is_some() {
                                    filter::push_thumbnail_or_emit(
                                        &mut thumbnail_graph,
                                        thumbnailed.0,
                                        send,
                                        transform.thumbnail.as_deref(),
                                        |f| flush_after_tile(f),
                                    )
                                } else {
                                    flush_after_tile(send)
                                }
                            },
                        )
                    },
                )
            })?;
        }
    }
    if transform.reverse.is_some()
        && transform.shuffleframes.is_none()
        && transform.untile.is_none()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::reverse_flush(&mut reverse_graph, reversed.0, |send| unsafe {
                filter::push_loop_or_emit(
                    &mut loop_graph,
                    looped.0,
                    send,
                    transform.r#loop.as_deref(),
                    |send| {
                        if transform.thumbnail.is_some() {
                            filter::push_thumbnail_or_emit(
                                &mut thumbnail_graph,
                                thumbnailed.0,
                                send,
                                transform.thumbnail.as_deref(),
                                |f| flush_after_tile(f),
                            )
                        } else {
                            flush_after_tile(send)
                        }
                    },
                )
            })?;
        }
    }
    if transform.r#loop.is_some()
        && transform.reverse.is_none()
        && transform.shuffleframes.is_none()
        && transform.untile.is_none()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::loop_flush(&mut loop_graph, looped.0, |send| {
                if transform.thumbnail.is_some() {
                    filter::push_thumbnail_or_emit(
                        &mut thumbnail_graph,
                        thumbnailed.0,
                        send,
                        transform.thumbnail.as_deref(),
                        |f| flush_after_tile(f),
                    )
                } else {
                    flush_after_tile(send)
                }
            })?;
        }
    }
    if transform.thumbnail.is_some()
        && transform.r#loop.is_none()
        && transform.reverse.is_none()
        && transform.shuffleframes.is_none()
        && transform.untile.is_none()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.framepack.is_none()
        && transform.telecine.is_none()
        && transform.pullup.is_none()
        && transform.decimate.is_none()
        && transform.mpdecimate.is_none()
    {
        unsafe {
            filter::thumbnail_flush(&mut thumbnail_graph, thumbnailed.0, |send| {
                flush_after_tile(send)
            })?;
        }
    }
    if transform.yadif.is_some() {
        unsafe {
            filter::yadif_flush(&mut yadif_graph, deinterlaced.0, |mut send| {
                if let Some(ref args) = transform.bwdif {
                    let produced =
                        filter::bwdif_push_frame(&mut bwdif_graph, bwdif_out.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = bwdif_out.0;
                }
                if let Some(ref args) = transform.w3fdif {
                    let produced =
                        filter::w3fdif_push_frame(&mut w3fdif_graph, w3fdif_out.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = w3fdif_out.0;
                }
                if let Some(ref args) = transform.tblend {
                    let produced = filter::tblend_frame(&mut tblend_graph, tblended.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = tblended.0;
                }
                if let Some(ref args) = transform.tmix {
                    let produced = filter::tmix_push_frame(&mut tmix_graph, tmixed.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = tmixed.0;
                }
                if let Some(ref args) = transform.hqdn3d {
                    filter::hqdn3d_frame(&mut hqdn3d_graph, denoised.0, send, args)?;
                    send = denoised.0;
                }
                if let Some(ref args) = transform.gblur {
                    filter::gblur_frame(&mut gblur_graph, blurred.0, send, args)?;
                    send = blurred.0;
                }
                if let Some(ref args) = transform.eq {
                    filter::eq_frame(&mut eq_graph, equalized.0, send, args)?;
                    send = equalized.0;
                }
                if let Some(ref args) = transform.unsharp {
                    filter::unsharp_frame(&mut unsharp_graph, sharpened.0, send, args)?;
                    send = sharpened.0;
                }
                if let Some(ref args) = transform.hue {
                    filter::hue_frame(&mut hue_graph, hued.0, send, args)?;
                    send = hued.0;
                }
                if let Some(ref args) = transform.avgblur {
                    filter::avgblur_frame(&mut avgblur_graph, avgblurred.0, send, args)?;
                    send = avgblurred.0;
                }
                if let Some(ref args) = transform.boxblur {
                    filter::boxblur_frame(&mut boxblur_graph, boxblurred.0, send, args)?;
                    send = boxblurred.0;
                }
                if let Some(ref args) = transform.negate {
                    filter::negate_frame(&mut negate_graph, negated.0, send, args)?;
                    send = negated.0;
                }
                if let Some(ref args) = transform.edgedetect {
                    filter::edgedetect_frame(&mut edgedetect_graph, edged.0, send, args)?;
                    let out = edged.0;
                    if args.contains("mode=colormix") && (*out).format != (*send).format {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, (*send).format)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.sobel {
                    filter::sobel_frame(&mut sobel_graph, sobeled.0, send, args)?;
                    send = sobeled.0;
                }
                if let Some(ref args) = transform.prewitt {
                    filter::prewitt_frame(&mut prewitt_graph, prewitted.0, send, args)?;
                    send = prewitted.0;
                }
                if let Some(ref args) = transform.roberts {
                    filter::roberts_frame(&mut roberts_graph, robertsed.0, send, args)?;
                    send = robertsed.0;
                }
                if let Some(ref args) = transform.kirsch {
                    filter::kirsch_frame(&mut kirsch_graph, kirsched.0, send, args)?;
                    send = kirsched.0;
                }
                if let Some(ref args) = transform.scharr {
                    filter::scharr_frame(&mut scharr_graph, scharred.0, send, args)?;
                    send = scharred.0;
                }
                if let Some(ref args) = transform.atadenoise {
                    let produced = filter::atadenoise_push_frame(
                        &mut atadenoise_graph,
                        atdenoised.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = atdenoised.0;
                }
                if let Some(ref args) = transform.owdenoise {
                    filter::owdenoise_frame(&mut owdenoise_graph, owdenoised.0, send, args)?;
                    send = owdenoised.0;
                }
                if let Some(ref args) = transform.vaguedenoiser {
                    filter::vaguedenoiser_frame(
                        &mut vaguedenoiser_graph,
                        vaguedenoised.0,
                        send,
                        args,
                    )?;
                    send = vaguedenoised.0;
                }
                if let Some(ref args) = transform.nlmeans {
                    filter::nlmeans_frame(&mut nlmeans_graph, nldenoised.0, send, args)?;
                    send = nldenoised.0;
                }
                if let Some(ref args) = transform.bm3d {
                    filter::bm3d_frame(&mut bm3d_graph, bm3ded.0, send, args)?;
                    send = bm3ded.0;
                }
                if let Some(ref args) = transform.dctdnoiz {
                    let src_fmt = (*send).format;
                    filter::dctdnoiz_frame(&mut dctdnoiz_graph, dctdnoized.0, send, args)?;
                    let out = dctdnoized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fftdnoiz {
                    filter::fftdnoiz_frame(&mut fftdnoiz_graph, fftdnoized.0, send, args)?;
                    send = fftdnoized.0;
                }
                if let Some(ref args) = transform.smartblur {
                    filter::smartblur_frame(&mut smartblur_graph, smartblurred.0, send, args)?;
                    send = smartblurred.0;
                }
                if let Some(ref args) = transform.sab {
                    filter::sab_frame(&mut sab_graph, sabbed.0, send, args)?;
                    send = sabbed.0;
                }
                if let Some(ref args) = transform.bilateral {
                    filter::bilateral_frame(&mut bilateral_graph, bilateraled.0, send, args)?;
                    send = bilateraled.0;
                }
                if let Some(ref args) = transform.cas {
                    filter::cas_frame(&mut cas_graph, cased.0, send, args)?;
                    send = cased.0;
                }
                if let Some(ref args) = transform.vignette {
                    filter::vignette_frame(&mut vignette_graph, vignetted.0, send, args)?;
                    send = vignetted.0;
                }
                if let Some(ref args) = transform.curves {
                    let src_fmt = (*send).format;
                    filter::curves_frame(&mut curves_graph, curved.0, send, args)?;
                    let out = curved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorbalance {
                    let src_fmt = (*send).format;
                    filter::colorbalance_frame(
                        &mut colorbalance_graph,
                        colorbalanced.0,
                        send,
                        args,
                    )?;
                    let out = colorbalanced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorlevels {
                    let src_fmt = (*send).format;
                    filter::colorlevels_frame(&mut colorlevels_graph, colorleveled.0, send, args)?;
                    let out = colorleveled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorchannelmixer {
                    let src_fmt = (*send).format;
                    filter::colorchannelmixer_frame(
                        &mut colorchannelmixer_graph,
                        colorchannelmixed.0,
                        send,
                        args,
                    )?;
                    let out = colorchannelmixed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.deflicker {
                    let produced = filter::deflicker_push_frame(
                        &mut deflicker_graph,
                        deflickered.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = deflickered.0;
                }
                if let Some(ref args) = transform.photosensitivity {
                    let src_fmt = (*send).format;
                    filter::photosensitivity_frame(
                        &mut photosensitivity_graph,
                        photosensitized.0,
                        send,
                        args,
                    )?;
                    let out = photosensitized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.monochrome {
                    filter::monochrome_frame(&mut monochrome_graph, monochromed.0, send, args)?;
                    send = monochromed.0;
                }
                if let Some(ref args) = transform.grayworld {
                    let src_fmt = (*send).format;
                    filter::grayworld_frame(&mut grayworld_graph, grayworlded.0, send, args)?;
                    let out = grayworlded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.drawbox {
                    filter::drawbox_frame(&mut drawbox_graph, drawboxed.0, send, args)?;
                    send = drawboxed.0;
                }
                if let Some(ref args) = transform.drawgrid {
                    filter::drawgrid_frame(&mut drawgrid_graph, drawgridd.0, send, args)?;
                    send = drawgridd.0;
                }
                if let Some(ref args) = transform.lagfun {
                    let produced =
                        filter::lagfun_push_frame(&mut lagfun_graph, lagfuned.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = lagfuned.0;
                }
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.bwdif.is_some() {
        unsafe {
            filter::bwdif_flush(&mut bwdif_graph, bwdif_out.0, |mut send| {
                if let Some(ref args) = transform.w3fdif {
                    let produced =
                        filter::w3fdif_push_frame(&mut w3fdif_graph, w3fdif_out.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = w3fdif_out.0;
                }
                if let Some(ref args) = transform.tblend {
                    let produced = filter::tblend_frame(&mut tblend_graph, tblended.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = tblended.0;
                }
                if let Some(ref args) = transform.tmix {
                    let produced = filter::tmix_push_frame(&mut tmix_graph, tmixed.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = tmixed.0;
                }
                if let Some(ref args) = transform.hqdn3d {
                    filter::hqdn3d_frame(&mut hqdn3d_graph, denoised.0, send, args)?;
                    send = denoised.0;
                }
                if let Some(ref args) = transform.gblur {
                    filter::gblur_frame(&mut gblur_graph, blurred.0, send, args)?;
                    send = blurred.0;
                }
                if let Some(ref args) = transform.eq {
                    filter::eq_frame(&mut eq_graph, equalized.0, send, args)?;
                    send = equalized.0;
                }
                if let Some(ref args) = transform.unsharp {
                    filter::unsharp_frame(&mut unsharp_graph, sharpened.0, send, args)?;
                    send = sharpened.0;
                }
                if let Some(ref args) = transform.hue {
                    filter::hue_frame(&mut hue_graph, hued.0, send, args)?;
                    send = hued.0;
                }
                if let Some(ref args) = transform.avgblur {
                    filter::avgblur_frame(&mut avgblur_graph, avgblurred.0, send, args)?;
                    send = avgblurred.0;
                }
                if let Some(ref args) = transform.boxblur {
                    filter::boxblur_frame(&mut boxblur_graph, boxblurred.0, send, args)?;
                    send = boxblurred.0;
                }
                if let Some(ref args) = transform.negate {
                    filter::negate_frame(&mut negate_graph, negated.0, send, args)?;
                    send = negated.0;
                }
                if let Some(ref args) = transform.edgedetect {
                    filter::edgedetect_frame(&mut edgedetect_graph, edged.0, send, args)?;
                    let out = edged.0;
                    if args.contains("mode=colormix") && (*out).format != (*send).format {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, (*send).format)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.sobel {
                    filter::sobel_frame(&mut sobel_graph, sobeled.0, send, args)?;
                    send = sobeled.0;
                }
                if let Some(ref args) = transform.prewitt {
                    filter::prewitt_frame(&mut prewitt_graph, prewitted.0, send, args)?;
                    send = prewitted.0;
                }
                if let Some(ref args) = transform.roberts {
                    filter::roberts_frame(&mut roberts_graph, robertsed.0, send, args)?;
                    send = robertsed.0;
                }
                if let Some(ref args) = transform.kirsch {
                    filter::kirsch_frame(&mut kirsch_graph, kirsched.0, send, args)?;
                    send = kirsched.0;
                }
                if let Some(ref args) = transform.scharr {
                    filter::scharr_frame(&mut scharr_graph, scharred.0, send, args)?;
                    send = scharred.0;
                }
                if let Some(ref args) = transform.atadenoise {
                    let produced = filter::atadenoise_push_frame(
                        &mut atadenoise_graph,
                        atdenoised.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = atdenoised.0;
                }
                if let Some(ref args) = transform.owdenoise {
                    filter::owdenoise_frame(&mut owdenoise_graph, owdenoised.0, send, args)?;
                    send = owdenoised.0;
                }
                if let Some(ref args) = transform.vaguedenoiser {
                    filter::vaguedenoiser_frame(
                        &mut vaguedenoiser_graph,
                        vaguedenoised.0,
                        send,
                        args,
                    )?;
                    send = vaguedenoised.0;
                }
                if let Some(ref args) = transform.nlmeans {
                    filter::nlmeans_frame(&mut nlmeans_graph, nldenoised.0, send, args)?;
                    send = nldenoised.0;
                }
                if let Some(ref args) = transform.bm3d {
                    filter::bm3d_frame(&mut bm3d_graph, bm3ded.0, send, args)?;
                    send = bm3ded.0;
                }
                if let Some(ref args) = transform.dctdnoiz {
                    let src_fmt = (*send).format;
                    filter::dctdnoiz_frame(&mut dctdnoiz_graph, dctdnoized.0, send, args)?;
                    let out = dctdnoized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fftdnoiz {
                    filter::fftdnoiz_frame(&mut fftdnoiz_graph, fftdnoized.0, send, args)?;
                    send = fftdnoized.0;
                }
                if let Some(ref args) = transform.smartblur {
                    filter::smartblur_frame(&mut smartblur_graph, smartblurred.0, send, args)?;
                    send = smartblurred.0;
                }
                if let Some(ref args) = transform.sab {
                    filter::sab_frame(&mut sab_graph, sabbed.0, send, args)?;
                    send = sabbed.0;
                }
                if let Some(ref args) = transform.bilateral {
                    filter::bilateral_frame(&mut bilateral_graph, bilateraled.0, send, args)?;
                    send = bilateraled.0;
                }
                if let Some(ref args) = transform.cas {
                    filter::cas_frame(&mut cas_graph, cased.0, send, args)?;
                    send = cased.0;
                }
                if let Some(ref args) = transform.vignette {
                    filter::vignette_frame(&mut vignette_graph, vignetted.0, send, args)?;
                    send = vignetted.0;
                }
                if let Some(ref args) = transform.curves {
                    let src_fmt = (*send).format;
                    filter::curves_frame(&mut curves_graph, curved.0, send, args)?;
                    let out = curved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorbalance {
                    let src_fmt = (*send).format;
                    filter::colorbalance_frame(
                        &mut colorbalance_graph,
                        colorbalanced.0,
                        send,
                        args,
                    )?;
                    let out = colorbalanced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorlevels {
                    let src_fmt = (*send).format;
                    filter::colorlevels_frame(&mut colorlevels_graph, colorleveled.0, send, args)?;
                    let out = colorleveled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorchannelmixer {
                    let src_fmt = (*send).format;
                    filter::colorchannelmixer_frame(
                        &mut colorchannelmixer_graph,
                        colorchannelmixed.0,
                        send,
                        args,
                    )?;
                    let out = colorchannelmixed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.deflicker {
                    let produced = filter::deflicker_push_frame(
                        &mut deflicker_graph,
                        deflickered.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = deflickered.0;
                }
                if let Some(ref args) = transform.photosensitivity {
                    let src_fmt = (*send).format;
                    filter::photosensitivity_frame(
                        &mut photosensitivity_graph,
                        photosensitized.0,
                        send,
                        args,
                    )?;
                    let out = photosensitized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.monochrome {
                    filter::monochrome_frame(&mut monochrome_graph, monochromed.0, send, args)?;
                    send = monochromed.0;
                }
                if let Some(ref args) = transform.grayworld {
                    let src_fmt = (*send).format;
                    filter::grayworld_frame(&mut grayworld_graph, grayworlded.0, send, args)?;
                    let out = grayworlded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.drawbox {
                    filter::drawbox_frame(&mut drawbox_graph, drawboxed.0, send, args)?;
                    send = drawboxed.0;
                }
                if let Some(ref args) = transform.drawgrid {
                    filter::drawgrid_frame(&mut drawgrid_graph, drawgridd.0, send, args)?;
                    send = drawgridd.0;
                }
                if let Some(ref args) = transform.lagfun {
                    let produced =
                        filter::lagfun_push_frame(&mut lagfun_graph, lagfuned.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = lagfuned.0;
                }
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.w3fdif.is_some() {
        unsafe {
            filter::w3fdif_flush(&mut w3fdif_graph, w3fdif_out.0, |mut send| {
                if let Some(ref args) = transform.tblend {
                    let produced = filter::tblend_frame(&mut tblend_graph, tblended.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = tblended.0;
                }
                if let Some(ref args) = transform.tmix {
                    let produced = filter::tmix_push_frame(&mut tmix_graph, tmixed.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = tmixed.0;
                }
                if let Some(ref args) = transform.hqdn3d {
                    filter::hqdn3d_frame(&mut hqdn3d_graph, denoised.0, send, args)?;
                    send = denoised.0;
                }
                if let Some(ref args) = transform.gblur {
                    filter::gblur_frame(&mut gblur_graph, blurred.0, send, args)?;
                    send = blurred.0;
                }
                if let Some(ref args) = transform.eq {
                    filter::eq_frame(&mut eq_graph, equalized.0, send, args)?;
                    send = equalized.0;
                }
                if let Some(ref args) = transform.unsharp {
                    filter::unsharp_frame(&mut unsharp_graph, sharpened.0, send, args)?;
                    send = sharpened.0;
                }
                if let Some(ref args) = transform.hue {
                    filter::hue_frame(&mut hue_graph, hued.0, send, args)?;
                    send = hued.0;
                }
                if let Some(ref args) = transform.avgblur {
                    filter::avgblur_frame(&mut avgblur_graph, avgblurred.0, send, args)?;
                    send = avgblurred.0;
                }
                if let Some(ref args) = transform.boxblur {
                    filter::boxblur_frame(&mut boxblur_graph, boxblurred.0, send, args)?;
                    send = boxblurred.0;
                }
                if let Some(ref args) = transform.negate {
                    filter::negate_frame(&mut negate_graph, negated.0, send, args)?;
                    send = negated.0;
                }
                if let Some(ref args) = transform.edgedetect {
                    filter::edgedetect_frame(&mut edgedetect_graph, edged.0, send, args)?;
                    let out = edged.0;
                    if args.contains("mode=colormix") && (*out).format != (*send).format {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, (*send).format)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.sobel {
                    filter::sobel_frame(&mut sobel_graph, sobeled.0, send, args)?;
                    send = sobeled.0;
                }
                if let Some(ref args) = transform.prewitt {
                    filter::prewitt_frame(&mut prewitt_graph, prewitted.0, send, args)?;
                    send = prewitted.0;
                }
                if let Some(ref args) = transform.roberts {
                    filter::roberts_frame(&mut roberts_graph, robertsed.0, send, args)?;
                    send = robertsed.0;
                }
                if let Some(ref args) = transform.kirsch {
                    filter::kirsch_frame(&mut kirsch_graph, kirsched.0, send, args)?;
                    send = kirsched.0;
                }
                if let Some(ref args) = transform.scharr {
                    filter::scharr_frame(&mut scharr_graph, scharred.0, send, args)?;
                    send = scharred.0;
                }
                if let Some(ref args) = transform.atadenoise {
                    let produced = filter::atadenoise_push_frame(
                        &mut atadenoise_graph,
                        atdenoised.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = atdenoised.0;
                }
                if let Some(ref args) = transform.owdenoise {
                    filter::owdenoise_frame(&mut owdenoise_graph, owdenoised.0, send, args)?;
                    send = owdenoised.0;
                }
                if let Some(ref args) = transform.vaguedenoiser {
                    filter::vaguedenoiser_frame(
                        &mut vaguedenoiser_graph,
                        vaguedenoised.0,
                        send,
                        args,
                    )?;
                    send = vaguedenoised.0;
                }
                if let Some(ref args) = transform.nlmeans {
                    filter::nlmeans_frame(&mut nlmeans_graph, nldenoised.0, send, args)?;
                    send = nldenoised.0;
                }
                if let Some(ref args) = transform.bm3d {
                    filter::bm3d_frame(&mut bm3d_graph, bm3ded.0, send, args)?;
                    send = bm3ded.0;
                }
                if let Some(ref args) = transform.dctdnoiz {
                    let src_fmt = (*send).format;
                    filter::dctdnoiz_frame(&mut dctdnoiz_graph, dctdnoized.0, send, args)?;
                    let out = dctdnoized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fftdnoiz {
                    filter::fftdnoiz_frame(&mut fftdnoiz_graph, fftdnoized.0, send, args)?;
                    send = fftdnoized.0;
                }
                if let Some(ref args) = transform.smartblur {
                    filter::smartblur_frame(&mut smartblur_graph, smartblurred.0, send, args)?;
                    send = smartblurred.0;
                }
                if let Some(ref args) = transform.sab {
                    filter::sab_frame(&mut sab_graph, sabbed.0, send, args)?;
                    send = sabbed.0;
                }
                if let Some(ref args) = transform.bilateral {
                    filter::bilateral_frame(&mut bilateral_graph, bilateraled.0, send, args)?;
                    send = bilateraled.0;
                }
                if let Some(ref args) = transform.cas {
                    filter::cas_frame(&mut cas_graph, cased.0, send, args)?;
                    send = cased.0;
                }
                if let Some(ref args) = transform.vignette {
                    filter::vignette_frame(&mut vignette_graph, vignetted.0, send, args)?;
                    send = vignetted.0;
                }
                if let Some(ref args) = transform.curves {
                    let src_fmt = (*send).format;
                    filter::curves_frame(&mut curves_graph, curved.0, send, args)?;
                    let out = curved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorbalance {
                    let src_fmt = (*send).format;
                    filter::colorbalance_frame(
                        &mut colorbalance_graph,
                        colorbalanced.0,
                        send,
                        args,
                    )?;
                    let out = colorbalanced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorlevels {
                    let src_fmt = (*send).format;
                    filter::colorlevels_frame(&mut colorlevels_graph, colorleveled.0, send, args)?;
                    let out = colorleveled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorchannelmixer {
                    let src_fmt = (*send).format;
                    filter::colorchannelmixer_frame(
                        &mut colorchannelmixer_graph,
                        colorchannelmixed.0,
                        send,
                        args,
                    )?;
                    let out = colorchannelmixed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.deflicker {
                    let produced = filter::deflicker_push_frame(
                        &mut deflicker_graph,
                        deflickered.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = deflickered.0;
                }
                if let Some(ref args) = transform.photosensitivity {
                    let src_fmt = (*send).format;
                    filter::photosensitivity_frame(
                        &mut photosensitivity_graph,
                        photosensitized.0,
                        send,
                        args,
                    )?;
                    let out = photosensitized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.monochrome {
                    filter::monochrome_frame(&mut monochrome_graph, monochromed.0, send, args)?;
                    send = monochromed.0;
                }
                if let Some(ref args) = transform.grayworld {
                    let src_fmt = (*send).format;
                    filter::grayworld_frame(&mut grayworld_graph, grayworlded.0, send, args)?;
                    let out = grayworlded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.drawbox {
                    filter::drawbox_frame(&mut drawbox_graph, drawboxed.0, send, args)?;
                    send = drawboxed.0;
                }
                if let Some(ref args) = transform.drawgrid {
                    filter::drawgrid_frame(&mut drawgrid_graph, drawgridd.0, send, args)?;
                    send = drawgridd.0;
                }
                if let Some(ref args) = transform.lagfun {
                    let produced =
                        filter::lagfun_push_frame(&mut lagfun_graph, lagfuned.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = lagfuned.0;
                }
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.tmix.is_some() {
        unsafe {
            filter::tmix_flush(&mut tmix_graph, tmixed.0, |mut send| {
                if let Some(ref args) = transform.hqdn3d {
                    filter::hqdn3d_frame(&mut hqdn3d_graph, denoised.0, send, args)?;
                    send = denoised.0;
                }
                if let Some(ref args) = transform.gblur {
                    filter::gblur_frame(&mut gblur_graph, blurred.0, send, args)?;
                    send = blurred.0;
                }
                if let Some(ref args) = transform.eq {
                    filter::eq_frame(&mut eq_graph, equalized.0, send, args)?;
                    send = equalized.0;
                }
                if let Some(ref args) = transform.unsharp {
                    filter::unsharp_frame(&mut unsharp_graph, sharpened.0, send, args)?;
                    send = sharpened.0;
                }
                if let Some(ref args) = transform.hue {
                    filter::hue_frame(&mut hue_graph, hued.0, send, args)?;
                    send = hued.0;
                }
                if let Some(ref args) = transform.avgblur {
                    filter::avgblur_frame(&mut avgblur_graph, avgblurred.0, send, args)?;
                    send = avgblurred.0;
                }
                if let Some(ref args) = transform.boxblur {
                    filter::boxblur_frame(&mut boxblur_graph, boxblurred.0, send, args)?;
                    send = boxblurred.0;
                }
                if let Some(ref args) = transform.negate {
                    filter::negate_frame(&mut negate_graph, negated.0, send, args)?;
                    send = negated.0;
                }
                if let Some(ref args) = transform.edgedetect {
                    filter::edgedetect_frame(&mut edgedetect_graph, edged.0, send, args)?;
                    let out = edged.0;
                    if args.contains("mode=colormix") && (*out).format != (*send).format {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, (*send).format)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.sobel {
                    filter::sobel_frame(&mut sobel_graph, sobeled.0, send, args)?;
                    send = sobeled.0;
                }
                if let Some(ref args) = transform.prewitt {
                    filter::prewitt_frame(&mut prewitt_graph, prewitted.0, send, args)?;
                    send = prewitted.0;
                }
                if let Some(ref args) = transform.roberts {
                    filter::roberts_frame(&mut roberts_graph, robertsed.0, send, args)?;
                    send = robertsed.0;
                }
                if let Some(ref args) = transform.kirsch {
                    filter::kirsch_frame(&mut kirsch_graph, kirsched.0, send, args)?;
                    send = kirsched.0;
                }
                if let Some(ref args) = transform.scharr {
                    filter::scharr_frame(&mut scharr_graph, scharred.0, send, args)?;
                    send = scharred.0;
                }
                if let Some(ref args) = transform.atadenoise {
                    let produced = filter::atadenoise_push_frame(
                        &mut atadenoise_graph,
                        atdenoised.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = atdenoised.0;
                }
                if let Some(ref args) = transform.owdenoise {
                    filter::owdenoise_frame(&mut owdenoise_graph, owdenoised.0, send, args)?;
                    send = owdenoised.0;
                }
                if let Some(ref args) = transform.vaguedenoiser {
                    filter::vaguedenoiser_frame(
                        &mut vaguedenoiser_graph,
                        vaguedenoised.0,
                        send,
                        args,
                    )?;
                    send = vaguedenoised.0;
                }
                if let Some(ref args) = transform.nlmeans {
                    filter::nlmeans_frame(&mut nlmeans_graph, nldenoised.0, send, args)?;
                    send = nldenoised.0;
                }
                if let Some(ref args) = transform.bm3d {
                    filter::bm3d_frame(&mut bm3d_graph, bm3ded.0, send, args)?;
                    send = bm3ded.0;
                }
                if let Some(ref args) = transform.dctdnoiz {
                    let src_fmt = (*send).format;
                    filter::dctdnoiz_frame(&mut dctdnoiz_graph, dctdnoized.0, send, args)?;
                    let out = dctdnoized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fftdnoiz {
                    filter::fftdnoiz_frame(&mut fftdnoiz_graph, fftdnoized.0, send, args)?;
                    send = fftdnoized.0;
                }
                if let Some(ref args) = transform.smartblur {
                    filter::smartblur_frame(&mut smartblur_graph, smartblurred.0, send, args)?;
                    send = smartblurred.0;
                }
                if let Some(ref args) = transform.sab {
                    filter::sab_frame(&mut sab_graph, sabbed.0, send, args)?;
                    send = sabbed.0;
                }
                if let Some(ref args) = transform.bilateral {
                    filter::bilateral_frame(&mut bilateral_graph, bilateraled.0, send, args)?;
                    send = bilateraled.0;
                }
                if let Some(ref args) = transform.cas {
                    filter::cas_frame(&mut cas_graph, cased.0, send, args)?;
                    send = cased.0;
                }
                if let Some(ref args) = transform.vignette {
                    filter::vignette_frame(&mut vignette_graph, vignetted.0, send, args)?;
                    send = vignetted.0;
                }
                if let Some(ref args) = transform.curves {
                    let src_fmt = (*send).format;
                    filter::curves_frame(&mut curves_graph, curved.0, send, args)?;
                    let out = curved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorbalance {
                    let src_fmt = (*send).format;
                    filter::colorbalance_frame(
                        &mut colorbalance_graph,
                        colorbalanced.0,
                        send,
                        args,
                    )?;
                    let out = colorbalanced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorlevels {
                    let src_fmt = (*send).format;
                    filter::colorlevels_frame(&mut colorlevels_graph, colorleveled.0, send, args)?;
                    let out = colorleveled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorchannelmixer {
                    let src_fmt = (*send).format;
                    filter::colorchannelmixer_frame(
                        &mut colorchannelmixer_graph,
                        colorchannelmixed.0,
                        send,
                        args,
                    )?;
                    let out = colorchannelmixed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.deflicker {
                    let produced = filter::deflicker_push_frame(
                        &mut deflicker_graph,
                        deflickered.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = deflickered.0;
                }
                if let Some(ref args) = transform.photosensitivity {
                    let src_fmt = (*send).format;
                    filter::photosensitivity_frame(
                        &mut photosensitivity_graph,
                        photosensitized.0,
                        send,
                        args,
                    )?;
                    let out = photosensitized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.monochrome {
                    filter::monochrome_frame(&mut monochrome_graph, monochromed.0, send, args)?;
                    send = monochromed.0;
                }
                if let Some(ref args) = transform.grayworld {
                    let src_fmt = (*send).format;
                    filter::grayworld_frame(&mut grayworld_graph, grayworlded.0, send, args)?;
                    let out = grayworlded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.drawbox {
                    filter::drawbox_frame(&mut drawbox_graph, drawboxed.0, send, args)?;
                    send = drawboxed.0;
                }
                if let Some(ref args) = transform.drawgrid {
                    filter::drawgrid_frame(&mut drawgrid_graph, drawgridd.0, send, args)?;
                    send = drawgridd.0;
                }
                if let Some(ref args) = transform.lagfun {
                    let produced =
                        filter::lagfun_push_frame(&mut lagfun_graph, lagfuned.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = lagfuned.0;
                }
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.atadenoise.is_some() {
        unsafe {
            filter::atadenoise_flush(&mut atadenoise_graph, atdenoised.0, |mut send| {
                if let Some(ref args) = transform.owdenoise {
                    filter::owdenoise_frame(&mut owdenoise_graph, owdenoised.0, send, args)?;
                    send = owdenoised.0;
                }
                if let Some(ref args) = transform.vaguedenoiser {
                    filter::vaguedenoiser_frame(
                        &mut vaguedenoiser_graph,
                        vaguedenoised.0,
                        send,
                        args,
                    )?;
                    send = vaguedenoised.0;
                }
                if let Some(ref args) = transform.nlmeans {
                    filter::nlmeans_frame(&mut nlmeans_graph, nldenoised.0, send, args)?;
                    send = nldenoised.0;
                }
                if let Some(ref args) = transform.bm3d {
                    filter::bm3d_frame(&mut bm3d_graph, bm3ded.0, send, args)?;
                    send = bm3ded.0;
                }
                if let Some(ref args) = transform.dctdnoiz {
                    let src_fmt = (*send).format;
                    filter::dctdnoiz_frame(&mut dctdnoiz_graph, dctdnoized.0, send, args)?;
                    let out = dctdnoized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fftdnoiz {
                    filter::fftdnoiz_frame(&mut fftdnoiz_graph, fftdnoized.0, send, args)?;
                    send = fftdnoized.0;
                }
                if let Some(ref args) = transform.smartblur {
                    filter::smartblur_frame(&mut smartblur_graph, smartblurred.0, send, args)?;
                    send = smartblurred.0;
                }
                if let Some(ref args) = transform.sab {
                    filter::sab_frame(&mut sab_graph, sabbed.0, send, args)?;
                    send = sabbed.0;
                }
                if let Some(ref args) = transform.bilateral {
                    filter::bilateral_frame(&mut bilateral_graph, bilateraled.0, send, args)?;
                    send = bilateraled.0;
                }
                if let Some(ref args) = transform.cas {
                    filter::cas_frame(&mut cas_graph, cased.0, send, args)?;
                    send = cased.0;
                }
                if let Some(ref args) = transform.vignette {
                    filter::vignette_frame(&mut vignette_graph, vignetted.0, send, args)?;
                    send = vignetted.0;
                }
                if let Some(ref args) = transform.curves {
                    let src_fmt = (*send).format;
                    filter::curves_frame(&mut curves_graph, curved.0, send, args)?;
                    let out = curved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorbalance {
                    let src_fmt = (*send).format;
                    filter::colorbalance_frame(
                        &mut colorbalance_graph,
                        colorbalanced.0,
                        send,
                        args,
                    )?;
                    let out = colorbalanced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorlevels {
                    let src_fmt = (*send).format;
                    filter::colorlevels_frame(&mut colorlevels_graph, colorleveled.0, send, args)?;
                    let out = colorleveled.0;
                    // colorlevels materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorchannelmixer {
                    let src_fmt = (*send).format;
                    filter::colorchannelmixer_frame(
                        &mut colorchannelmixer_graph,
                        colorchannelmixed.0,
                        send,
                        args,
                    )?;
                    let out = colorchannelmixed.0;
                    // colorchannelmixer materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.deflicker {
                    let produced = filter::deflicker_push_frame(
                        &mut deflicker_graph,
                        deflickered.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    send = deflickered.0;
                }
                if let Some(ref args) = transform.photosensitivity {
                    let src_fmt = (*send).format;
                    filter::photosensitivity_frame(
                        &mut photosensitivity_graph,
                        photosensitized.0,
                        send,
                        args,
                    )?;
                    let out = photosensitized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.monochrome {
                    filter::monochrome_frame(&mut monochrome_graph, monochromed.0, send, args)?;
                    send = monochromed.0;
                }
                if let Some(ref args) = transform.grayworld {
                    let src_fmt = (*send).format;
                    filter::grayworld_frame(&mut grayworld_graph, grayworlded.0, send, args)?;
                    let out = grayworlded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.drawbox {
                    filter::drawbox_frame(&mut drawbox_graph, drawboxed.0, send, args)?;
                    send = drawboxed.0;
                }
                if let Some(ref args) = transform.drawgrid {
                    filter::drawgrid_frame(&mut drawgrid_graph, drawgridd.0, send, args)?;
                    send = drawgridd.0;
                }
                if let Some(ref args) = transform.lagfun {
                    let produced =
                        filter::lagfun_push_frame(&mut lagfun_graph, lagfuned.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = lagfuned.0;
                }
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.deflicker.is_some() {
        unsafe {
            filter::deflicker_flush(&mut deflicker_graph, deflickered.0, |mut send| {
                if let Some(ref args) = transform.photosensitivity {
                    let src_fmt = (*send).format;
                    filter::photosensitivity_frame(
                        &mut photosensitivity_graph,
                        photosensitized.0,
                        send,
                        args,
                    )?;
                    let out = photosensitized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.monochrome {
                    filter::monochrome_frame(&mut monochrome_graph, monochromed.0, send, args)?;
                    send = monochromed.0;
                }
                if let Some(ref args) = transform.grayworld {
                    let src_fmt = (*send).format;
                    filter::grayworld_frame(&mut grayworld_graph, grayworlded.0, send, args)?;
                    let out = grayworlded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.drawbox {
                    filter::drawbox_frame(&mut drawbox_graph, drawboxed.0, send, args)?;
                    send = drawboxed.0;
                }
                if let Some(ref args) = transform.drawgrid {
                    filter::drawgrid_frame(&mut drawgrid_graph, drawgridd.0, send, args)?;
                    send = drawgridd.0;
                }
                if let Some(ref args) = transform.lagfun {
                    let produced =
                        filter::lagfun_push_frame(&mut lagfun_graph, lagfuned.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = lagfuned.0;
                }
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.lagfun.is_some() {
        unsafe {
            filter::lagfun_flush(&mut lagfun_graph, lagfuned.0, |mut send| {
                if let Some(ref args) = transform.amplify {
                    let produced =
                        filter::amplify_push_frame(&mut amplify_graph, amplified.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = amplified.0;
                }
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.amplify.is_some() {
        unsafe {
            filter::amplify_flush(&mut amplify_graph, amplified.0, |mut send| {
                if let Some(ref args) = transform.bitplanenoise {
                    filter::bitplanenoise_frame(
                        &mut bitplanenoise_graph,
                        bitplanenoised.0,
                        send,
                        args,
                    )?;
                    send = bitplanenoised.0;
                }
                if let Some(ref args) = transform.deband {
                    filter::deband_frame(&mut deband_graph, debanded.0, send, args)?;
                    send = debanded.0;
                }
                if let Some(ref args) = transform.gradfun {
                    let src_fmt = (*send).format;
                    filter::gradfun_frame(&mut gradfun_graph, gradfuned.0, send, args)?;
                    let out = gradfuned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lenscorrection {
                    let src_fmt = (*send).format;
                    filter::lenscorrection_frame(
                        &mut lenscorrection_graph,
                        lenscorrected.0,
                        send,
                        args,
                    )?;
                    let out = lenscorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pixelize {
                    let src_fmt = (*send).format;
                    filter::pixelize_frame(&mut pixelize_graph, pixelized.0, send, args)?;
                    let out = pixelized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.removegrain {
                    let src_fmt = (*send).format;
                    filter::removegrain_frame(&mut removegrain_graph, removegrained.0, send, args)?;
                    let out = removegrained.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.yaepblur {
                    let src_fmt = (*send).format;
                    filter::yaepblur_frame(&mut yaepblur_graph, yaepblurred.0, send, args)?;
                    let out = yaepblurred.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.vibrance {
                    let src_fmt = (*send).format;
                    filter::vibrance_frame(&mut vibrance_graph, vibranced.0, send, args)?;
                    let out = vibranced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.dilation {
                    let src_fmt = (*send).format;
                    filter::dilation_frame(&mut dilation_graph, dilated.0, send, args)?;
                    let out = dilated.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.erosion {
                    let src_fmt = (*send).format;
                    filter::erosion_frame(&mut erosion_graph, eroded.0, send, args)?;
                    let out = eroded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorize {
                    let src_fmt = (*send).format;
                    filter::colorize_frame(&mut colorize_graph, colorized.0, send, args)?;
                    let out = colorized.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.exposure {
                    let src_fmt = (*send).format;
                    filter::exposure_frame(&mut exposure_graph, exposured.0, send, args)?;
                    let out = exposured.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.chromashift {
                    let src_fmt = (*send).format;
                    filter::chromashift_frame(&mut chromashift_graph, chromashifted.0, send, args)?;
                    let out = chromashifted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcontrast {
                    let src_fmt = (*send).format;
                    filter::colorcontrast_frame(
                        &mut colorcontrast_graph,
                        colorcontrasted.0,
                        send,
                        args,
                    )?;
                    let out = colorcontrasted.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorcorrect {
                    let src_fmt = (*send).format;
                    filter::colorcorrect_frame(
                        &mut colorcorrect_graph,
                        colorcorrected.0,
                        send,
                        args,
                    )?;
                    let out = colorcorrected.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.histeq {
                    let src_fmt = (*send).format;
                    filter::histeq_frame(&mut histeq_graph, histeqed.0, send, args)?;
                    let out = histeqed.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.shuffleplanes {
                    let src_fmt = (*send).format;
                    filter::shuffleplanes_frame(
                        &mut shuffleplanes_graph,
                        shuffleplaned.0,
                        send,
                        args,
                    )?;
                    let out = shuffleplaned.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lutyuv {
                    let src_fmt = (*send).format;
                    filter::lutyuv_frame(&mut lutyuv_graph, lutyuved.0, send, args)?;
                    let out = lutyuved.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorhold {
                    let src_fmt = (*send).format;
                    filter::colorhold_frame(&mut colorhold_graph, colorholded.0, send, args)?;
                    let out = colorholded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.fade {
                    let src_fmt = (*send).format;
                    let produced = filter::fade_apply_frame(
                        &mut fade_graph,
                        faded.0,
                        send,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = faded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.perspective {
                    let src_fmt = (*send).format;
                    filter::perspective_frame(&mut perspective_graph, perspectived.0, send, args)?;
                    let out = perspectived.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.lumakey {
                    filter::lumakey_frame(&mut lumakey_graph, lumakeyed.0, send, args)?;
                    send = lumakeyed.0;
                }
                if let Some(ref args) = transform.chromakey {
                    filter::chromakey_frame(&mut chromakey_graph, chromakeyed.0, send, args)?;
                    send = chromakeyed.0;
                }
                if let Some(ref args) = transform.colorkey {
                    filter::colorkey_frame(&mut colorkey_graph, colorkeyed.0, send, args)?;
                    send = colorkeyed.0;
                }
                if let Some(ref args) = transform.despill {
                    let src_fmt = (*send).format;
                    filter::despill_frame(&mut despill_graph, despilled.0, send, args)?;
                    let out = despilled.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.selectivecolor {
                    let src_fmt = (*send).format;
                    filter::selectivecolor_frame(
                        &mut selectivecolor_graph,
                        selectivecolored.0,
                        send,
                        args,
                    )?;
                    let out = selectivecolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.stereo3d {
                    let src_fmt = (*send).format;
                    filter::stereo3d_frame(&mut stereo3d_graph, stereo3ded.0, send, args)?;
                    let out = stereo3ded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.field {
                    let src_fmt = (*send).format;
                    filter::field_frame(&mut field_graph, fielded.0, send, args)?;
                    let out = fielded.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.hqx {
                    filter::hqx_frame(&mut hqx_graph, hqxd.0, send, args)?;
                    send = hqxd.0;
                }
                if let Some(ref args) = transform.xbr {
                    filter::xbr_frame(&mut xbr_graph, xbrd.0, send, args)?;
                    send = xbrd.0;
                }
                if let Some(ref args) = transform.il {
                    let src_fmt = (*send).format;
                    filter::il_frame(&mut il_graph, ild.0, send, args)?;
                    let out = ild.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.super2xsai {
                    filter::super2xsai_frame(&mut super2xsai_graph, super2xsaid.0, send, args)?;
                    send = super2xsaid.0;
                }
                if let Some(ref args) = transform.kerndeint {
                    let src_fmt = (*send).format;
                    filter::kerndeint_frame(&mut kerndeint_graph, kerndeintd.0, send, args)?;
                    let out = kerndeintd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.phase {
                    let src_fmt = (*send).format;
                    let produced = filter::phase_apply_frame(
                        &mut phase_graph,
                        phased.0,
                        send,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = phased.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.estdif {
                    let src_fmt = (*send).format;
                    let produced =
                        filter::estdif_push_frame(&mut estdif_graph, estdifd.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    let out = estdifd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.tinterlace {
                    let src_fmt = (*send).format;
                    let produced = filter::tinterlace_push_frame(
                        &mut tinterlace_graph,
                        tinterlaced.0,
                        send,
                        args,
                    )?;
                    if !produced {
                        return Ok(());
                    }
                    let out = tinterlaced.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.freezedetect {
                    let src_fmt = (*send).format;
                    filter::freezedetect_frame(
                        &mut freezedetect_graph,
                        freezedetectd.0,
                        send,
                        args,
                    )?;
                    let out = freezedetectd.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.pseudocolor {
                    let src_fmt = (*send).format;
                    filter::pseudocolor_frame(&mut pseudocolor_graph, pseudocolored.0, send, args)?;
                    let out = pseudocolored.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        send = converted.0;
                    } else {
                        send = out;
                    }
                }
                if let Some(ref args) = transform.colorspace {
                    filter::colorspace_frame(&mut colorspace_graph, colorspaced.0, send, args)?;
                    send = colorspaced.0;
                }
                let mut format_done = false;
                if let Some(ref args) = transform.zscale {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    filter::zscale_frame(&mut zscale_graph, zscaled.0, send, args, &name)?;
                    send = zscaled.0;
                    format_done = true;
                }
                if let Some(ref args) = transform.tonemap {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    filter::tonemap_frame(&mut tonemap_graph, tonemapped.0, send, args, &name)?;
                    send = tonemapped.0;
                    format_done = true;
                }
                if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*send).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, send, fmt)?;
                        send = converted.0;
                    }
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let mut emitted = 0u64;
                    filter::temporal_push_frame(
                        &mut minterpolate_graph,
                        minterpolate_dst.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst.0,
                        transform.fps.as_deref(),
                        send,
                        |out| {
                            send_encoder_frame(
                                &mut encoder,
                                &mut output,
                                &mut encoded,
                                mapped,
                                out,
                                &mut stats,
                            )?;
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    stats.video_frames += emitted;
                } else {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        send,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                }
                Ok(())
            })?;
        }
    }
    if transform.minterpolate.is_some() || transform.fps.is_some() {
        unsafe {
            filter::temporal_flush_frames(
                &mut minterpolate_graph,
                minterpolate_dst.0,
                transform.minterpolate.as_deref(),
                &mut fps_graph,
                fps_dst.0,
                transform.fps.as_deref(),
                |out| {
                    send_encoder_frame(
                        &mut encoder,
                        &mut output,
                        &mut encoded,
                        mapped,
                        out,
                        &mut stats,
                    )?;
                    stats.video_frames += 1;
                    Ok(())
                },
            )?;
        }
    }
    if let Some((from, to)) = interval_us {
        for track in &mut decode_audio {
            if track.done {
                continue;
            }
            check(
                unsafe { avcodec_send_packet(track.decoder.0, ptr::null()) },
                "drain audio decoder",
            )?;
            loop {
                let code = unsafe { avcodec_receive_frame(track.decoder.0, frame.0) };
                if code == AGAIN || code == EOF {
                    break;
                }
                check(code, "receive drained audio frame")?;
                track.done = audio::write_interval_pcm_frame(
                    &mut output,
                    track.mapped,
                    &frame,
                    &mut encoded,
                    &mut track.pool,
                    track.format,
                    &mut track.decoded_sample_frames,
                    &mut track.written_sample_frames,
                    &mut track.sample_bounds,
                    (from, to),
                    options.max_packet_bytes,
                )?;
                stats.trimmed_audio_sample_frames = track.written_sample_frames;
                unsafe {
                    av_frame_unref(frame.0);
                }
                if track.done {
                    break;
                }
            }
        }
    }
    // SAFETY: Decoder has finished; no new frames follow the encoder drain signal.
    loop {
        let code = unsafe { avcodec_send_frame(encoder.0, ptr::null()) };
        if code == AGAIN {
            let before = stats.video_packets;
            drain_encoder(&mut encoder, &mut output, &mut encoded, mapped, &mut stats)?;
            if stats.video_packets == before {
                return Err("encoder flush stalled".into());
            }
            continue;
        }
        if code == EOF {
            break;
        }
        check(code, "drain encoder")?;
        break;
    }
    drain_encoder(&mut encoder, &mut output, &mut encoded, mapped, &mut stats)?;
    if stats.video_frames == 0 {
        return Err("no video frames decoded".into());
    }
    crate::emit_progress_done(
        options,
        stats.video_packets.saturating_add(stats.copied_packets),
        0,
    );
    output.finish()?;
    Ok(stats)
}
