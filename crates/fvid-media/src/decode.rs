//! Decode video and discard frames (throughput probe; no encode/mux).
use super::*;
use filter::{
    FilterGraph, FramepackGraph, OverlayGraph, amplify_flush, amplify_push_frame, atadenoise_flush,
    atadenoise_push_frame, avgblur_frame, bilateral_frame, bitplanenoise_frame, bm3d_frame,
    boxblur_frame, bwdif_flush, bwdif_push_frame, cas_frame, chromakey_frame, chromashift_frame,
    colorbalance_frame, colorchannelmixer_frame, colorcontrast_frame, colorcorrect_frame,
    colorhold_frame, colorize_frame, colorkey_frame, colorlevels_frame, colorspace_frame,
    curves_frame, dctdnoiz_frame, deband_frame, decimate_flush, decimate_push_frame,
    deflicker_flush, deflicker_push_frame, despill_frame, dilation_frame, doubleweave_flush,
    doubleweave_push_frame, drawbox_frame, drawgrid_frame, edgedetect_frame, epx_frame, eq_frame,
    erosion_frame, estdif_flush, estdif_push_frame, exposure_frame, fade_apply_frame, fade_flush,
    fftdnoiz_frame, field_frame, framepack_flush, framepack_push_frame, framestep_flush,
    framestep_push_frame, freezedetect_frame, gblur_frame, gradfun_frame, grayworld_frame,
    histeq_frame, hqdn3d_frame, hqx_frame, hue_frame, il_frame, kerndeint_frame, kirsch_frame,
    lagfun_flush, lagfun_push_frame, lenscorrection_frame, loop_flush, loop_push_frame,
    lumakey_frame, lutyuv_frame, monochrome_frame, mpdecimate_flush, mpdecimate_push_frame,
    negate_frame, nlmeans_frame, overlay_frame, owdenoise_frame, pad_frame, perspective_frame,
    phase_apply_frame, phase_flush, photosensitivity_frame, pixelize_frame, prewitt_frame,
    pseudocolor_frame, pullup_flush, pullup_push_frame, push_framestep_or_emit, push_loop_or_emit,
    push_reverse_or_emit, push_shuffleframes_or_emit, push_thumbnail_or_emit, push_tile_or_emit,
    push_untile_or_emit, removegrain_frame, reverse_flush, reverse_push_frame, roberts_frame,
    rotate_frame, sab_frame, scharr_frame, selectivecolor_frame, separatefields_flush,
    separatefields_push_frame, shuffleframes_flush, shuffleframes_push_frame, shuffleplanes_frame,
    smartblur_frame, sobel_frame, stereo3d_frame, subtitles_frame, super2xsai_frame, tblend_frame,
    telecine_flush, telecine_push_frame, temporal_flush_frames, temporal_push_frame,
    thumbnail_flush, thumbnail_push_frame, tile_flush, tile_push_frame, tinterlace_flush,
    tinterlace_push_frame, tmix_flush, tmix_push_frame, tonemap_frame, transpose_frame,
    unsharp_frame, untile_flush, untile_push_frame, vaguedenoiser_frame, validate_amplify_args,
    validate_atadenoise_args, validate_avgblur_args, validate_bilateral_args,
    validate_bitplanenoise_args, validate_bm3d_args, validate_boxblur_args, validate_bwdif_args,
    validate_cas_args, validate_chromakey_args, validate_chromashift_args,
    validate_colorbalance_args, validate_colorchannelmixer_args, validate_colorcontrast_args,
    validate_colorcorrect_args, validate_colorhold_args, validate_colorize_args,
    validate_colorkey_args, validate_colorlevels_args, validate_colorspace_args,
    validate_curves_args, validate_dctdnoiz_args, validate_deband_args, validate_decimate_args,
    validate_deflicker_args, validate_despill_args, validate_dilation_args,
    validate_doubleweave_args, validate_drawbox_args, validate_drawgrid_args,
    validate_edgedetect_args, validate_epx_args, validate_eq_args, validate_erosion_args,
    validate_estdif_args, validate_exposure_args, validate_fade_args, validate_fftdnoiz_args,
    validate_field_args, validate_fps_args, validate_framepack_args, validate_framestep_args,
    validate_freezedetect_args, validate_gblur_args, validate_gradfun_args,
    validate_grayworld_args, validate_histeq_args, validate_hqdn3d_args, validate_hqx_args,
    validate_hue_args, validate_il_args, validate_kerndeint_args, validate_kirsch_args,
    validate_lagfun_args, validate_lenscorrection_args, validate_loop_args, validate_lumakey_args,
    validate_lutyuv_args, validate_minterpolate_args, validate_monochrome_args,
    validate_mpdecimate_args, validate_negate_args, validate_nlmeans_args, validate_owdenoise_args,
    validate_perspective_args, validate_phase_args, validate_photosensitivity_args,
    validate_pixelize_args, validate_prewitt_args, validate_pseudocolor_args, validate_pullup_args,
    validate_removegrain_args, validate_reverse_args, validate_roberts_args, validate_sab_args,
    validate_scharr_args, validate_selectivecolor_args, validate_separatefields_args,
    validate_shuffleframes_args, validate_shuffleplanes_args, validate_smartblur_args,
    validate_sobel_args, validate_stereo3d_args, validate_super2xsai_args, validate_tblend_args,
    validate_telecine_args, validate_thumbnail_args, validate_tile_args, validate_tinterlace_args,
    validate_tmix_args, validate_tonemap_args, validate_unsharp_args, validate_untile_args,
    validate_vaguedenoiser_args, validate_vibrance_args, validate_vignette_args,
    validate_w3fdif_args, validate_weave_args, validate_xbr_args, validate_yadif_args,
    validate_yaepblur_args, validate_zscale_args, vibrance_frame, vignette_frame, w3fdif_flush,
    w3fdif_push_frame, weave_flush, weave_push_frame, xbr_frame, yadif_flush, yadif_push_frame,
    yaepblur_frame, zscale_frame,
};
use lossless::{
    Codec, CropRect, Frame, ScaleSize, Sws, convert_pix_fmt_frame, flip_view,
    horizontal_copy_frame, parse_pix_fmt, scale_convert_frame, scale_frame, validate_scale,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::ptr;

const AGAIN: i32 = -libc::EAGAIN;

#[derive(Serialize, Debug)]
pub struct DecodeStats {
    pub backend: &'static str,
    pub video_frames: u64,
    pub width: u32,
    pub height: u32,
    pub pixel_format: String,
}

#[derive(Clone, Debug, Default)]
pub struct DecodeTransform {
    pub crop: Option<CropRect>,
    pub vertical_flip: bool,
    pub horizontal_flip: bool,
    /// Exact output size after crop/flips/transpose/rotate/pad. Uses libswscale neighbor
    /// sampling so fair pairs match FFmpeg `scale=W:H:flags=neighbor`.
    pub scale: Option<ScaleSize>,
    /// FFmpeg `epx=` pixel-art upscale after scale and before burn/overlay (e.g. `n=2`).
    pub epx: Option<String>,
    /// FFmpeg-compatible `transpose=` after crop/flips and before rotate/pad/scale.
    pub transpose: Option<TransposeMode>,
    /// FFmpeg-compatible `rotate=` after transpose and before pad/scale.
    pub rotate: Option<RotateAngle>,
    /// FFmpeg-compatible `pad=W:H:X:Y:black` after rotate and before scale.
    pub pad: Option<PadRect>,
    /// External text subtitle file burned via libavfilter `subtitles=` after geometry.
    pub burn_subs: Option<PathBuf>,
    /// External video composited via `movie=` + `overlay=` after burn-in.
    pub overlay: Option<crate::OverlaySpec>,
    /// FFmpeg `yadif=` option string (e.g. `mode=0`); empty uses defaults.
    pub yadif: Option<String>,
    /// FFmpeg `bwdif=` option string (e.g. `mode=0`); empty uses defaults.
    pub bwdif: Option<String>,
    /// FFmpeg `w3fdif=` option string (e.g. `mode=0`); empty uses defaults.
    pub w3fdif: Option<String>,
    /// FFmpeg `tblend=` option string (e.g. `all_mode=average`).
    pub tblend: Option<String>,
    /// FFmpeg `tmix=` option string (e.g. `frames=3`).
    pub tmix: Option<String>,
    /// FFmpeg `hqdn3d=` option string (e.g. `4:3:6:4.5`); empty uses defaults.
    pub hqdn3d: Option<String>,
    /// FFmpeg `gblur=` option string (e.g. `sigma=1.5:steps=1`); empty uses defaults.
    pub gblur: Option<String>,
    /// FFmpeg `eq=` option string (e.g. `brightness=0.06:contrast=1.2`); empty uses defaults.
    pub eq: Option<String>,
    /// FFmpeg `unsharp=` option string (e.g. `5:5:1.0:5:5:0.0`); empty uses defaults.
    pub unsharp: Option<String>,
    /// FFmpeg `hue=` option string (e.g. `h=90:s=1.2`); empty uses defaults.
    pub hue: Option<String>,
    /// FFmpeg `avgblur=` option string (e.g. `sizeX=5:sizeY=5`); empty uses defaults.
    pub avgblur: Option<String>,
    /// FFmpeg `boxblur=` option string (e.g. `2:1`); empty uses defaults.
    pub boxblur: Option<String>,
    /// FFmpeg `negate` / `negate=1`; empty/`0` = components only.
    pub negate: Option<String>,
    /// FFmpeg `edgedetect=` option string (e.g. `mode=colormix`); empty uses defaults.
    pub edgedetect: Option<String>,
    /// FFmpeg `sobel=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub sobel: Option<String>,
    /// FFmpeg `prewitt=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub prewitt: Option<String>,
    /// FFmpeg `roberts=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub roberts: Option<String>,
    /// FFmpeg `kirsch=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub kirsch: Option<String>,
    /// FFmpeg `scharr=` option string (e.g. `scale=2:delta=10`); empty uses defaults.
    pub scharr: Option<String>,
    /// FFmpeg `atadenoise=` option string (e.g. `0a=0.02:0b=0.04`); empty uses defaults.
    pub atadenoise: Option<String>,
    /// FFmpeg `owdenoise=` option string (e.g. `depth=8:luma_strength=1.0`); empty uses defaults.
    pub owdenoise: Option<String>,
    /// FFmpeg `vaguedenoiser=` option string (e.g. `threshold=3`); empty uses defaults.
    pub vaguedenoiser: Option<String>,
    /// FFmpeg `nlmeans=` option string (e.g. `s=1.0`); empty uses defaults.
    pub nlmeans: Option<String>,
    /// FFmpeg `bm3d=` option string (e.g. `sigma=3`); empty uses defaults.
    pub bm3d: Option<String>,
    /// FFmpeg `dctdnoiz=` option string (e.g. `s=3`); empty uses defaults.
    pub dctdnoiz: Option<String>,
    /// FFmpeg `fftdnoiz=` option string (e.g. `sigma=1`); empty uses defaults.
    pub fftdnoiz: Option<String>,
    /// FFmpeg `smartblur=` option string (e.g. `lr=1.5:ls=-0.5`); empty uses defaults.
    pub smartblur: Option<String>,
    /// FFmpeg `sab=` option string (e.g. `lr=2:cr=2`); empty uses defaults.
    pub sab: Option<String>,
    /// FFmpeg `bilateral=` option string (e.g. `sigmaS=0.1:sigmaR=0.1`); empty uses defaults.
    pub bilateral: Option<String>,
    /// FFmpeg `cas=` option string (e.g. `strength=0.5`); empty uses defaults.
    pub cas: Option<String>,
    /// FFmpeg `vignette=` option string (e.g. `angle=PI/4`); empty uses defaults.
    pub vignette: Option<String>,
    /// FFmpeg `curves=` option string (e.g. `preset=vintage`); empty uses defaults.
    pub curves: Option<String>,
    /// FFmpeg `colorbalance=` option string (e.g. `rs=.1:gs=.05:bs=-.1`); empty uses defaults.
    pub colorbalance: Option<String>,
    /// FFmpeg `colorlevels=` option string (e.g. `rimin=0.1:gimin=0.1:bimin=0.1`); empty uses defaults.
    pub colorlevels: Option<String>,
    /// FFmpeg `colorchannelmixer=` option string (e.g. `rr=1.1:gg=0.9:bb=1.0`); empty uses defaults.
    pub colorchannelmixer: Option<String>,
    /// FFmpeg `deflicker=` option string (e.g. `mode=am:size=5`); empty uses defaults.
    pub deflicker: Option<String>,
    /// FFmpeg `photosensitivity=` option string (e.g. `f=5`); empty uses defaults.
    pub photosensitivity: Option<String>,
    /// FFmpeg `monochrome=` option string (e.g. `cb=0.2:cr=-0.1`); empty uses defaults.
    pub monochrome: Option<String>,
    /// FFmpeg `grayworld` / `grayworld=` (white balance); empty/`0` uses defaults.
    pub grayworld: Option<String>,
    /// FFmpeg `drawbox=` option string (e.g. `x=10:y=10:w=40:h=20:color=red`); empty uses defaults.
    pub drawbox: Option<String>,
    /// FFmpeg `drawgrid=` option string (e.g. `w=16:h=16:color=white`); empty uses defaults.
    pub drawgrid: Option<String>,
    /// FFmpeg `lagfun=` option string (e.g. `decay=0.95`); empty uses defaults.
    pub lagfun: Option<String>,
    /// FFmpeg `amplify=` option string (e.g. `radius=2:factor=2`); empty uses defaults.
    pub amplify: Option<String>,
    /// FFmpeg `bitplanenoise=` option string (e.g. `bitplane=1:filter=1`); empty uses defaults.
    pub bitplanenoise: Option<String>,
    /// FFmpeg `deband=` option string (e.g. `1thr=0.02`); empty uses defaults.
    pub deband: Option<String>,
    /// FFmpeg `gradfun=` option string (e.g. `strength=1.2`); empty uses defaults.
    pub gradfun: Option<String>,
    /// FFmpeg `lenscorrection=` option string (e.g. `k1=-0.1`); empty uses defaults.
    pub lenscorrection: Option<String>,
    /// FFmpeg `pixelize=` option string (e.g. `width=8:height=8`); empty uses defaults.
    pub pixelize: Option<String>,
    /// FFmpeg `removegrain=` option string (e.g. `m0=1`); empty uses defaults.
    pub removegrain: Option<String>,
    /// FFmpeg `yaepblur=` option string (e.g. `r=3`); empty uses defaults.
    pub yaepblur: Option<String>,
    /// FFmpeg `vibrance=` option string (e.g. `intensity=0.3:rbal=1`); empty uses defaults.
    pub vibrance: Option<String>,
    /// FFmpeg `dilation=` option string (e.g. `threshold0=10`); empty uses defaults.
    pub dilation: Option<String>,
    /// FFmpeg `erosion=` option string (e.g. `threshold0=10`); empty uses defaults.
    pub erosion: Option<String>,
    /// FFmpeg `colorize=` option string (e.g. `hue=120:saturation=0.5`); empty uses defaults.
    pub colorize: Option<String>,
    /// FFmpeg `exposure=` option string (e.g. `exposure=0.5`); empty uses defaults.
    pub exposure: Option<String>,
    /// FFmpeg `chromashift=` option string (e.g. `cbh=4`); empty uses defaults.
    pub chromashift: Option<String>,
    /// FFmpeg `colorcontrast=` option string (e.g. `rc=0.1:gm=0.1:by=0.1`); empty uses defaults.
    pub colorcontrast: Option<String>,
    /// FFmpeg `colorcorrect=` option string (e.g. `rl=0.1:bl=-0.1`); empty uses defaults.
    pub colorcorrect: Option<String>,
    /// FFmpeg `histeq=` option string (e.g. `strength=0.2`); empty uses defaults.
    pub histeq: Option<String>,
    /// FFmpeg `shuffleplanes=` option string (e.g. `map0=0:map1=2:map2=1`); empty uses defaults.
    pub shuffleplanes: Option<String>,
    /// FFmpeg `lutyuv=` option string (e.g. `y=val*0.8`); empty uses defaults.
    pub lutyuv: Option<String>,
    /// FFmpeg `colorhold=` option string (e.g. `similarity=0.2:blend=0.1`); empty uses defaults.
    pub colorhold: Option<String>,
    /// FFmpeg `fade=` option string (e.g. `t=in:s=0:n=4`); empty uses defaults.
    pub fade: Option<String>,
    /// FFmpeg `perspective=` option string; empty uses defaults.
    pub perspective: Option<String>,
    /// FFmpeg `lumakey=` option string (e.g. `threshold=0.1:tolerance=0.1:softness=0.1`); empty uses defaults.
    pub lumakey: Option<String>,
    /// FFmpeg `chromakey=` option string (e.g. `similarity=0.3:blend=0.1`); empty uses defaults.
    pub chromakey: Option<String>,
    /// FFmpeg `colorkey=` option string (e.g. `color=black:similarity=0.1:blend=0.1`); empty uses defaults.
    pub colorkey: Option<String>,
    /// FFmpeg `despill=` option string (e.g. `type=green:mix=0.5`); empty uses defaults.
    pub despill: Option<String>,
    /// FFmpeg `selectivecolor=` option string (e.g. `reds=0.2 0 0 0`); empty uses defaults.
    pub selectivecolor: Option<String>,
    /// FFmpeg `stereo3d=` option string (e.g. `sbsl:abl`); empty uses defaults.
    pub stereo3d: Option<String>,
    /// FFmpeg `field=` option string (e.g. `bottom`); empty uses defaults.
    pub field: Option<String>,
    /// FFmpeg `hqx=` pixel-art upscale after field (e.g. `n=2`).
    pub hqx: Option<String>,
    /// FFmpeg `xbr=` pixel-art upscale after hqx (e.g. `n=2`).
    pub xbr: Option<String>,
    /// FFmpeg `il=` interleave/deinterleave lines (e.g. `l=d:c=d`); empty uses defaults.
    pub il: Option<String>,
    /// FFmpeg `super2xsai` 2× pixel-art upscale after il; empty uses defaults.
    pub super2xsai: Option<String>,
    /// FFmpeg `kerndeint=` kernel deinterlace after super2xsai (e.g. `thresh=10`); empty uses defaults.
    pub kerndeint: Option<String>,
    /// FFmpeg `phase=` field phase shift after kerndeint (e.g. `mode=t`); empty uses defaults.
    pub phase: Option<String>,
    /// FFmpeg `estdif=` edge-slope deinterlace after phase (e.g. `mode=frame`); empty uses defaults.
    pub estdif: Option<String>,
    /// FFmpeg `tinterlace=` temporal field interlace after estdif (e.g. `mode=merge`); empty uses defaults.
    pub tinterlace: Option<String>,
    /// FFmpeg `separatefields` splits each frame into two fields after tinterlace; empty uses defaults.
    pub separatefields: Option<String>,
    /// FFmpeg `weave` joins successive fields into frames after separatefields; empty uses defaults.
    pub weave: Option<String>,
    /// FFmpeg `doubleweave` weaves each input field into a frame after weave; empty uses defaults.
    pub doubleweave: Option<String>,
    /// FFmpeg `framepack=` packs consecutive frames after doubleweave (e.g. `format=sbs`); empty uses defaults.
    pub framepack: Option<String>,
    /// FFmpeg `telecine=` applies a pulldown pattern after framepack (e.g. `pattern=23`); empty uses defaults.
    pub telecine: Option<String>,
    /// FFmpeg `pullup=` inverse telecine after telecine (e.g. `jl=0:jr=0`); empty uses defaults.
    pub pullup: Option<String>,
    /// FFmpeg `decimate=` drops one frame per cycle after pullup (e.g. `cycle=5`); empty uses defaults.
    pub decimate: Option<String>,
    /// FFmpeg `mpdecimate=` drops near-duplicate frames after decimate; empty uses defaults.
    pub mpdecimate: Option<String>,
    /// FFmpeg `framestep=` keeps one frame every N after mpdecimate (e.g. `2`, `step=2`); empty uses defaults.
    pub framestep: Option<String>,
    /// FFmpeg `tile=` packs consecutive frames into a grid after framestep (e.g. `2x2`, `layout=2x2`); empty uses defaults.
    pub tile: Option<String>,
    /// FFmpeg `untile=` splits each frame into a grid of frames after tile (e.g. `2x2`, `layout=2x2`); empty uses defaults.
    pub untile: Option<String>,
    /// FFmpeg `shuffleframes=` reorders frames after untile (e.g. `2 1 0`, `mapping=2|1|0`); empty uses defaults.
    pub shuffleframes: Option<String>,
    /// FFmpeg `reverse` emits frames in reverse order after shuffleframes; empty uses defaults.
    pub reverse: Option<String>,
    /// FFmpeg `loop=` repeats a short frame window after reverse (e.g. `loop=1:size=2:start=0`); empty uses defaults.
    pub r#loop: Option<String>,
    /// FFmpeg `thumbnail=` picks the most representative frame from every batch of N after loop (e.g. `n=3`, `3`); empty uses defaults.
    pub thumbnail: Option<String>,
    /// FFmpeg `freezedetect=` detects frozen input after thumbnail (e.g. `n=0.001:d=0.1`); empty uses defaults.
    pub freezedetect: Option<String>,
    /// FFmpeg `pseudocolor=` option string (e.g. `preset=magma`); empty uses defaults.
    pub pseudocolor: Option<String>,
    /// FFmpeg `minterpolate=` option string (e.g. `mi_mode=blend:fps=50`); empty uses defaults.
    pub minterpolate: Option<String>,
    /// FFmpeg `fps=` rate string (e.g. `25`, `12`, `30000/1001`).
    pub fps: Option<String>,
    /// FFmpeg `colorspace=` option string (e.g. `iall=bt470bg:all=bt709`).
    pub colorspace: Option<String>,
    /// FFmpeg `zscale=` option string (e.g. `matrixin=bt470bg:matrix=bt709`).
    pub zscale: Option<String>,
    /// FFmpeg `tonemap=` option string (e.g. `tonemap=hable`).
    pub tonemap: Option<String>,
    /// Convert to this pixel format after geometry (FFmpeg `format=`).
    pub pix_fmt: Option<String>,
    /// Half-open presentation interval in microseconds from container start.
    pub interval: Option<(i64, i64)>,
}

/// Software decode of the first video stream; frames are dropped after optional
/// crop/hflip/vflip (same view/copy contracts as lossless export).
pub fn decode_video(source: &Path) -> Result<DecodeStats> {
    decode_video_transformed(source, DecodeTransform::default())
}

pub fn decode_video_transformed(source: &Path, transform: DecodeTransform) -> Result<DecodeStats> {
    let mut input = Input::open_fast(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("input has no video stream")?;
    let (interval, seek_target_us) = if let Some((from, to)) = transform.interval {
        if from < 0 || to <= from {
            return Err("decode interval requires 0 <= from < to".into());
        }
        // SAFETY: The selected stream and input context remain live.
        let (tb, origin) = unsafe {
            let tb = (*input.streams()[video]).time_base;
            let origin = if (*input.0).start_time == NOPTS {
                0
            } else {
                (*input.0).start_time
            };
            (tb, origin)
        };
        let ticks = |time: i64| -> Result<i64> {
            let us = origin
                .checked_add(time)
                .ok_or("interval timestamp overflow")?;
            let numerator = i128::from(us) * i128::from(tb.den);
            let denominator = 1_000_000i128 * i128::from(tb.num);
            if denominator <= 0 || tb.den <= 0 || numerator % denominator != 0 {
                return Err("interval boundary is not exact in video time base".into());
            }
            i64::try_from(numerator / denominator).map_err(|_| "interval timestamp overflow".into())
        };
        (
            Some((ticks(from)?, ticks(to)?)),
            Some(
                origin
                    .checked_add(from)
                    .ok_or("interval timestamp overflow")?,
            ),
        )
    } else {
        (None, None)
    };
    if let Some(start) = seek_target_us {
        // Input-side seek matches FFmpeg's fair-pair behavior and occurs before
        // decoder allocation, so no empty decoder state needs flushing.
        check(
            unsafe { avformat_seek_file(input.0, -1, i64::MIN, start, start, 0) },
            "seek before decode interval",
        )?;
    }
    let (decoder, width, height, crop, mut pixel_format, mut format_validated, no_reorder) = unsafe {
        let s = &*input.streams()[video];
        let p = &*s.codecpar;
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
        let width = (*decoder.0).width.max(0) as i32;
        let height = (*decoder.0).height.max(0) as i32;
        let crop = transform.crop.unwrap_or(CropRect {
            x: 0,
            y: 0,
            width: width.max(0) as usize,
            height: height.max(0) as usize,
        });
        let descriptor = av_pix_fmt_desc_get(format);
        if crop.width == 0
            || crop.height == 0
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|v| v > width.max(0) as usize)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|v| v > height.max(0) as usize)
        {
            return Err("crop must be bounded".into());
        }
        let format_validated = if descriptor.is_null() {
            false
        } else {
            let sx = 1usize << (*descriptor).log2_chroma_w;
            let sy = 1usize << (*descriptor).log2_chroma_h;
            if !crop.x.is_multiple_of(sx) || !crop.y.is_multiple_of(sy) {
                return Err("crop origin must be chroma-aligned".into());
            }
            true
        };
        (
            decoder,
            crop.width as u32,
            crop.height as u32,
            crop,
            if format_validated {
                string(av_get_pix_fmt_name(format))
            } else {
                String::new()
            },
            format_validated,
            p.video_delay == 0,
        )
    };
    let (out_w, out_h) = {
        let (mut w, mut h) = if let Some(mode) = transform.transpose {
            mode.size(width, height)
        } else {
            (width, height)
        };
        if let Some(angle) = transform.rotate {
            (w, h) = angle.size(w, h);
        }
        if let Some(pad) = transform.pad {
            pad.validate(w, h)?;
            w = pad.width;
            h = pad.height;
        }
        let (mut ow, mut oh) = if let Some(scale) = transform.scale {
            validate_scale(scale)?;
            (scale.width, scale.height)
        } else {
            (w, h)
        };
        if let Some(ref args) = transform.epx {
            validate_epx_args(args)?;
            (ow, oh) = filter::epx_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.stereo3d {
            validate_stereo3d_args(args)?;
            (ow, oh) = filter::stereo3d_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.field {
            validate_field_args(args)?;
            (ow, oh) = filter::field_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.hqx {
            validate_hqx_args(args)?;
            (ow, oh) = filter::hqx_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.xbr {
            validate_xbr_args(args)?;
            (ow, oh) = filter::xbr_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.super2xsai {
            validate_super2xsai_args(args)?;
            (ow, oh) = filter::super2xsai_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.separatefields {
            validate_separatefields_args(args)?;
            (ow, oh) = filter::separatefields_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.weave {
            validate_weave_args(args)?;
            (ow, oh) = filter::weave_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.doubleweave {
            validate_doubleweave_args(args)?;
            (ow, oh) = filter::doubleweave_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.framepack {
            validate_framepack_args(args)?;
            (ow, oh) = filter::framepack_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.tile {
            validate_tile_args(args)?;
            (ow, oh) = filter::tile_output_size(ow, oh, args)?;
        }
        if let Some(ref args) = transform.untile {
            validate_untile_args(args)?;
            (ow, oh) = filter::untile_output_size(ow, oh, args)?;
        }
        (ow, oh)
    };
    let mut packet = Packet::new()?;
    let mut frame = Frame::new()?;
    let transformed = Frame::new()?;
    let transposed = Frame::new()?;
    let rotated = Frame::new()?;
    let padded = Frame::new()?;
    let scaled = Frame::new()?;
    let mut epxed = if transform.epx.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut burned = if transform.burn_subs.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut overlaid = if transform.overlay.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut deinterlaced = if transform.yadif.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut bwdif_out = if transform.bwdif.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut w3fdif_out = if transform.w3fdif.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut tblended = if transform.tblend.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut tmixed = if transform.tmix.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut denoised = if transform.hqdn3d.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut blurred = if transform.gblur.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut equalized = if transform.eq.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut sharpened = if transform.unsharp.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut hued = if transform.hue.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut avgblurred = if transform.avgblur.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut boxblurred = if transform.boxblur.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut negated = if transform.negate.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut edged = if transform.edgedetect.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut sobeled = if transform.sobel.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut prewitted = if transform.prewitt.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut robertsed = if transform.roberts.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut kirsched = if transform.kirsch.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut scharred = if transform.scharr.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut atdenoised = if transform.atadenoise.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut owdenoised = if transform.owdenoise.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut vaguedenoised = if transform.vaguedenoiser.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut nldenoised = if transform.nlmeans.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut bm3ded = if transform.bm3d.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut dctdnoized = if transform.dctdnoiz.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut fftdnoized = if transform.fftdnoiz.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut sabbed = if transform.sab.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut smartblurred = if transform.smartblur.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut bilateraled = if transform.bilateral.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut cased = if transform.cas.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut vignetted = if transform.vignette.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut curved = if transform.curves.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut balanced = if transform.colorbalance.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut leveled = if transform.colorlevels.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut channelmixed = if transform.colorchannelmixer.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut deflickered = if transform.deflicker.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut photosensitized = if transform.photosensitivity.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut monochromed = if transform.monochrome.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut grayworlded = if transform.grayworld.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut drawboxed = if transform.drawbox.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut drawgridd = if transform.drawgrid.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut lagfuned = if transform.lagfun.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut amplified = if transform.amplify.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut bitplanenoised = if transform.bitplanenoise.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut debanded = if transform.deband.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut gradfuned = if transform.gradfun.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut lenscorrected = if transform.lenscorrection.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut pixelized = if transform.pixelize.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut removegrained = if transform.removegrain.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut yaepblurred = if transform.yaepblur.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut vibranced = if transform.vibrance.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut dilated = if transform.dilation.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut eroded = if transform.erosion.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut colorized = if transform.colorize.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut exposured = if transform.exposure.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut chromashifted = if transform.chromashift.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut colorcontrasted = if transform.colorcontrast.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut colorcorrected = if transform.colorcorrect.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut histeqed = if transform.histeq.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut shuffleplaned = if transform.shuffleplanes.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut lutyuved = if transform.lutyuv.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut colorholded = if transform.colorhold.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut faded = if transform.fade.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut perspectived = if transform.perspective.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut lumakeyed = if transform.lumakey.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut chromakeyed = if transform.chromakey.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut colorkeyed = if transform.colorkey.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut despilled = if transform.despill.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut selectivecolored = if transform.selectivecolor.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut stereo3ded = if transform.stereo3d.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut fielded = if transform.field.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut hqxd = if transform.hqx.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut xbrd = if transform.xbr.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut ild = if transform.il.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut super2xsaid = if transform.super2xsai.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut kerndeintd = if transform.kerndeint.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut phased = if transform.phase.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut estdifd = if transform.estdif.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut tinterlaced = if transform.tinterlace.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut separatefieldsd = if transform.separatefields.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut weaved = if transform.weave.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut doubleweaved = if transform.doubleweave.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut framepacked = if transform.framepack.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut telecined = if transform.telecine.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut pulledup = if transform.pullup.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut decimated = if transform.decimate.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut mpdecimated = if transform.mpdecimate.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut framestepped = if transform.framestep.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut tiled = if transform.tile.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut untiled = if transform.untile.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut shuffled = if transform.shuffleframes.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut reversed = if transform.reverse.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut looped = if transform.r#loop.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut thumbnailed = if transform.thumbnail.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut freezedetectd = if transform.freezedetect.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut pseudocolored = if transform.pseudocolor.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut temporal_scratch = if transform.minterpolate.is_some() || transform.fps.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut fps_out = if transform.fps.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut colorspaced = if transform.colorspace.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut zscaled = if transform.zscale.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let mut tonemapped = if transform.tonemap.is_some() {
        Some(Frame::new()?)
    } else {
        None
    };
    let converted = Frame::new()?;
    let mut sws: Option<Sws> = None;
    let mut fmt_sws: Option<Sws> = None;
    let mut transpose: Option<FilterGraph> = None;
    let mut rotate_graph: Option<FilterGraph> = None;
    let mut pad_graph: Option<FilterGraph> = None;
    let mut epx_graph: Option<FilterGraph> = None;
    let mut burn_graph: Option<FilterGraph> = None;
    let mut overlay_graph: Option<OverlayGraph> = None;
    let mut yadif_graph: Option<FilterGraph> = None;
    let mut bwdif_graph: Option<FilterGraph> = None;
    let mut w3fdif_graph: Option<FilterGraph> = None;
    let mut tblend_graph: Option<FilterGraph> = None;
    let mut tmix_graph: Option<FilterGraph> = None;
    let mut hqdn3d_graph: Option<FilterGraph> = None;
    let mut gblur_graph: Option<FilterGraph> = None;
    let mut eq_graph: Option<FilterGraph> = None;
    let mut unsharp_graph: Option<FilterGraph> = None;
    let mut hue_graph: Option<FilterGraph> = None;
    let mut avgblur_graph: Option<FilterGraph> = None;
    let mut boxblur_graph: Option<FilterGraph> = None;
    let mut negate_graph: Option<FilterGraph> = None;
    let mut edgedetect_graph: Option<FilterGraph> = None;
    let mut sobel_graph: Option<FilterGraph> = None;
    let mut prewitt_graph: Option<FilterGraph> = None;
    let mut roberts_graph: Option<FilterGraph> = None;
    let mut kirsch_graph: Option<FilterGraph> = None;
    let mut scharr_graph: Option<FilterGraph> = None;
    let mut atadenoise_graph: Option<FilterGraph> = None;
    let mut owdenoise_graph: Option<FilterGraph> = None;
    let mut vaguedenoiser_graph: Option<FilterGraph> = None;
    let mut nlmeans_graph: Option<FilterGraph> = None;
    let mut bm3d_graph: Option<FilterGraph> = None;
    let mut dctdnoiz_graph: Option<FilterGraph> = None;
    let mut fftdnoiz_graph: Option<FilterGraph> = None;
    let mut smartblur_graph: Option<FilterGraph> = None;
    let mut sab_graph: Option<FilterGraph> = None;
    let mut bilateral_graph: Option<FilterGraph> = None;
    let mut cas_graph: Option<FilterGraph> = None;
    let mut vignette_graph: Option<FilterGraph> = None;
    let mut curves_graph: Option<FilterGraph> = None;
    let mut colorbalance_graph: Option<FilterGraph> = None;
    let mut colorlevels_graph: Option<FilterGraph> = None;
    let mut colorchannelmixer_graph: Option<FilterGraph> = None;
    let mut deflicker_graph: Option<FilterGraph> = None;
    let mut photosensitivity_graph: Option<FilterGraph> = None;
    let mut monochrome_graph: Option<FilterGraph> = None;
    let mut grayworld_graph: Option<FilterGraph> = None;
    let mut drawbox_graph: Option<FilterGraph> = None;
    let mut drawgrid_graph: Option<FilterGraph> = None;
    let mut lagfun_graph: Option<FilterGraph> = None;
    let mut amplify_graph: Option<FilterGraph> = None;
    let mut bitplanenoise_graph: Option<FilterGraph> = None;
    let mut deband_graph: Option<FilterGraph> = None;
    let mut gradfun_graph: Option<FilterGraph> = None;
    let mut lenscorrection_graph: Option<FilterGraph> = None;
    let mut pixelize_graph: Option<FilterGraph> = None;
    let mut removegrain_graph: Option<FilterGraph> = None;
    let mut yaepblur_graph: Option<FilterGraph> = None;
    let mut vibrance_graph: Option<FilterGraph> = None;
    let mut dilation_graph: Option<FilterGraph> = None;
    let mut erosion_graph: Option<FilterGraph> = None;
    let mut colorize_graph: Option<FilterGraph> = None;
    let mut exposure_graph: Option<FilterGraph> = None;
    let mut chromashift_graph: Option<FilterGraph> = None;
    let mut colorcontrast_graph: Option<FilterGraph> = None;
    let mut colorcorrect_graph: Option<FilterGraph> = None;
    let mut histeq_graph: Option<FilterGraph> = None;
    let mut shuffleplanes_graph: Option<FilterGraph> = None;
    let mut lutyuv_graph: Option<FilterGraph> = None;
    let mut colorhold_graph: Option<FilterGraph> = None;
    let mut fade_graph: Option<FilterGraph> = None;
    let mut fade_push_mode = false;
    let mut perspective_graph: Option<FilterGraph> = None;
    let mut lumakey_graph: Option<FilterGraph> = None;
    let mut chromakey_graph: Option<FilterGraph> = None;
    let mut colorkey_graph: Option<FilterGraph> = None;
    let mut despill_graph: Option<FilterGraph> = None;
    let mut selectivecolor_graph: Option<FilterGraph> = None;
    let mut stereo3d_graph: Option<FilterGraph> = None;
    let mut field_graph: Option<FilterGraph> = None;
    let mut hqx_graph: Option<FilterGraph> = None;
    let mut xbr_graph: Option<FilterGraph> = None;
    let mut il_graph: Option<FilterGraph> = None;
    let mut super2xsai_graph: Option<FilterGraph> = None;
    let mut kerndeint_graph: Option<FilterGraph> = None;
    let mut phase_graph: Option<FilterGraph> = None;
    let mut phase_push_mode = false;
    let mut estdif_graph: Option<FilterGraph> = None;
    let mut tinterlace_graph: Option<FilterGraph> = None;
    let mut separatefields_graph: Option<FilterGraph> = None;
    let mut weave_graph: Option<FilterGraph> = None;
    let mut doubleweave_graph: Option<FilterGraph> = None;
    let mut framepack_graph: Option<FramepackGraph> = None;
    let mut telecine_graph: Option<FilterGraph> = None;
    let mut pullup_graph: Option<FilterGraph> = None;
    let mut decimate_graph: Option<FilterGraph> = None;
    let mut mpdecimate_graph: Option<FilterGraph> = None;
    let mut framestep_graph: Option<FilterGraph> = None;
    let mut tile_graph: Option<FilterGraph> = None;
    let mut untile_graph: Option<FilterGraph> = None;
    let mut shuffleframes_graph: Option<FilterGraph> = None;
    let mut reverse_graph: Option<FilterGraph> = None;
    let mut loop_graph: Option<FilterGraph> = None;
    let mut thumbnail_graph: Option<FilterGraph> = None;
    let mut freezedetect_graph: Option<FilterGraph> = None;
    let mut pseudocolor_graph: Option<FilterGraph> = None;
    let mut minterpolate_graph: Option<FilterGraph> = None;
    let mut fps_graph: Option<FilterGraph> = None;
    let mut colorspace_graph: Option<FilterGraph> = None;
    let mut zscale_graph: Option<FilterGraph> = None;
    let mut tonemap_graph: Option<FilterGraph> = None;
    let target_pix_fmt = transform
        .pix_fmt
        .as_deref()
        .map(parse_pix_fmt)
        .transpose()?;
    if let Some(ref args) = transform.yadif {
        validate_yadif_args(args)?;
    }
    if let Some(ref args) = transform.bwdif {
        validate_bwdif_args(args)?;
    }
    if let Some(ref args) = transform.w3fdif {
        validate_w3fdif_args(args)?;
    }
    if let Some(ref args) = transform.tblend {
        validate_tblend_args(args)?;
    }
    if let Some(ref args) = transform.tmix {
        validate_tmix_args(args)?;
    }
    if let Some(ref args) = transform.hqdn3d {
        validate_hqdn3d_args(args)?;
    }
    if let Some(ref args) = transform.gblur {
        validate_gblur_args(args)?;
    }
    if let Some(ref args) = transform.eq {
        validate_eq_args(args)?;
    }
    if let Some(ref args) = transform.unsharp {
        validate_unsharp_args(args)?;
    }
    if let Some(ref args) = transform.hue {
        validate_hue_args(args)?;
    }
    if let Some(ref args) = transform.avgblur {
        validate_avgblur_args(args)?;
    }
    if let Some(ref args) = transform.boxblur {
        validate_boxblur_args(args)?;
    }
    if let Some(ref args) = transform.negate {
        validate_negate_args(args)?;
    }
    if let Some(ref args) = transform.edgedetect {
        validate_edgedetect_args(args)?;
    }
    if let Some(ref args) = transform.sobel {
        validate_sobel_args(args)?;
    }
    if let Some(ref args) = transform.prewitt {
        validate_prewitt_args(args)?;
    }
    if let Some(ref args) = transform.roberts {
        validate_roberts_args(args)?;
    }
    if let Some(ref args) = transform.kirsch {
        validate_kirsch_args(args)?;
    }
    if let Some(ref args) = transform.scharr {
        validate_scharr_args(args)?;
    }
    if let Some(ref args) = transform.atadenoise {
        validate_atadenoise_args(args)?;
    }
    if let Some(ref args) = transform.owdenoise {
        validate_owdenoise_args(args)?;
    }
    if let Some(ref args) = transform.vaguedenoiser {
        validate_vaguedenoiser_args(args)?;
    }
    if let Some(ref args) = transform.nlmeans {
        validate_nlmeans_args(args)?;
    }
    if let Some(ref args) = transform.bm3d {
        validate_bm3d_args(args)?;
    }
    if let Some(ref args) = transform.dctdnoiz {
        validate_dctdnoiz_args(args)?;
    }
    if let Some(ref args) = transform.fftdnoiz {
        validate_fftdnoiz_args(args)?;
    }
    if let Some(ref args) = transform.smartblur {
        validate_smartblur_args(args)?;
    }
    if let Some(ref args) = transform.sab {
        validate_sab_args(args)?;
    }
    if let Some(ref args) = transform.bilateral {
        validate_bilateral_args(args)?;
    }
    if let Some(ref args) = transform.cas {
        validate_cas_args(args)?;
    }
    if let Some(ref args) = transform.epx {
        validate_epx_args(args)?;
    }
    if let Some(ref args) = transform.vignette {
        validate_vignette_args(args)?;
    }
    if let Some(ref args) = transform.curves {
        validate_curves_args(args)?;
    }
    if let Some(ref args) = transform.colorbalance {
        validate_colorbalance_args(args)?;
    }
    if let Some(ref args) = transform.colorlevels {
        validate_colorlevels_args(args)?;
    }
    if let Some(ref args) = transform.colorchannelmixer {
        validate_colorchannelmixer_args(args)?;
    }
    if let Some(ref args) = transform.deflicker {
        validate_deflicker_args(args)?;
    }
    if let Some(ref args) = transform.photosensitivity {
        validate_photosensitivity_args(args)?;
    }
    if let Some(ref args) = transform.monochrome {
        validate_monochrome_args(args)?;
    }
    if let Some(ref args) = transform.grayworld {
        validate_grayworld_args(args)?;
    }
    if let Some(ref args) = transform.drawbox {
        validate_drawbox_args(args)?;
    }
    if let Some(ref args) = transform.drawgrid {
        validate_drawgrid_args(args)?;
    }
    if let Some(ref args) = transform.lagfun {
        validate_lagfun_args(args)?;
    }
    if let Some(ref args) = transform.amplify {
        validate_amplify_args(args)?;
    }
    if let Some(ref args) = transform.bitplanenoise {
        validate_bitplanenoise_args(args)?;
    }
    if let Some(ref args) = transform.deband {
        validate_deband_args(args)?;
    }
    if let Some(ref args) = transform.gradfun {
        validate_gradfun_args(args)?;
    }
    if let Some(ref args) = transform.lenscorrection {
        validate_lenscorrection_args(args)?;
    }
    if let Some(ref args) = transform.pixelize {
        validate_pixelize_args(args)?;
    }
    if let Some(ref args) = transform.removegrain {
        validate_removegrain_args(args)?;
    }
    if let Some(ref args) = transform.yaepblur {
        validate_yaepblur_args(args)?;
    }
    if let Some(ref args) = transform.vibrance {
        validate_vibrance_args(args)?;
    }
    if let Some(ref args) = transform.dilation {
        validate_dilation_args(args)?;
    }
    if let Some(ref args) = transform.erosion {
        validate_erosion_args(args)?;
    }
    if let Some(ref args) = transform.colorize {
        validate_colorize_args(args)?;
    }
    if let Some(ref args) = transform.exposure {
        validate_exposure_args(args)?;
    }
    if let Some(ref args) = transform.chromashift {
        validate_chromashift_args(args)?;
    }
    if let Some(ref args) = transform.colorcontrast {
        validate_colorcontrast_args(args)?;
    }
    if let Some(ref args) = transform.colorcorrect {
        validate_colorcorrect_args(args)?;
    }
    if let Some(ref args) = transform.histeq {
        validate_histeq_args(args)?;
    }
    if let Some(ref args) = transform.shuffleplanes {
        validate_shuffleplanes_args(args)?;
    }
    if let Some(ref args) = transform.lutyuv {
        validate_lutyuv_args(args)?;
    }
    if let Some(ref args) = transform.colorhold {
        validate_colorhold_args(args)?;
    }
    if let Some(ref args) = transform.fade {
        validate_fade_args(args)?;
    }
    if let Some(ref args) = transform.perspective {
        validate_perspective_args(args)?;
    }
    if let Some(ref args) = transform.lumakey {
        validate_lumakey_args(args)?;
    }
    if let Some(ref args) = transform.chromakey {
        validate_chromakey_args(args)?;
    }
    if let Some(ref args) = transform.colorkey {
        validate_colorkey_args(args)?;
    }
    if let Some(ref args) = transform.despill {
        validate_despill_args(args)?;
    }
    if let Some(ref args) = transform.selectivecolor {
        validate_selectivecolor_args(args)?;
    }
    if let Some(ref args) = transform.stereo3d {
        validate_stereo3d_args(args)?;
    }
    if let Some(ref args) = transform.field {
        validate_field_args(args)?;
    }
    if let Some(ref args) = transform.hqx {
        validate_hqx_args(args)?;
    }
    if let Some(ref args) = transform.xbr {
        validate_xbr_args(args)?;
    }
    if let Some(ref args) = transform.il {
        validate_il_args(args)?;
    }
    if let Some(ref args) = transform.super2xsai {
        validate_super2xsai_args(args)?;
    }
    if let Some(ref args) = transform.kerndeint {
        validate_kerndeint_args(args)?;
    }
    if let Some(ref args) = transform.phase {
        validate_phase_args(args)?;
    }
    if let Some(ref args) = transform.estdif {
        validate_estdif_args(args)?;
    }
    if let Some(ref args) = transform.tinterlace {
        validate_tinterlace_args(args)?;
    }
    if let Some(ref args) = transform.separatefields {
        validate_separatefields_args(args)?;
    }
    if let Some(ref args) = transform.weave {
        validate_weave_args(args)?;
    }
    if let Some(ref args) = transform.doubleweave {
        validate_doubleweave_args(args)?;
    }
    if let Some(ref args) = transform.framepack {
        validate_framepack_args(args)?;
    }
    if let Some(ref args) = transform.telecine {
        validate_telecine_args(args)?;
    }
    if let Some(ref args) = transform.pullup {
        validate_pullup_args(args)?;
    }
    if let Some(ref args) = transform.decimate {
        validate_decimate_args(args)?;
    }
    if let Some(ref args) = transform.mpdecimate {
        validate_mpdecimate_args(args)?;
    }
    if let Some(ref args) = transform.framestep {
        validate_framestep_args(args)?;
    }
    if let Some(ref args) = transform.tile {
        validate_tile_args(args)?;
    }
    if let Some(ref args) = transform.untile {
        validate_untile_args(args)?;
    }
    if let Some(ref args) = transform.shuffleframes {
        validate_shuffleframes_args(args)?;
    }
    if let Some(ref args) = transform.reverse {
        validate_reverse_args(args)?;
    }
    if let Some(ref args) = transform.r#loop {
        validate_loop_args(args)?;
    }
    if let Some(ref args) = transform.thumbnail {
        validate_thumbnail_args(args)?;
    }
    if let Some(ref args) = transform.freezedetect {
        validate_freezedetect_args(args)?;
    }
    if let Some(ref args) = transform.pseudocolor {
        validate_pseudocolor_args(args)?;
    }
    if let Some(ref args) = transform.minterpolate {
        validate_minterpolate_args(args)?;
    }
    if let Some(ref args) = transform.fps {
        validate_fps_args(args)?;
    }
    if let Some(ref args) = transform.colorspace {
        validate_colorspace_args(args)?;
    }
    if let Some(ref args) = transform.zscale {
        validate_zscale_args(args)?;
        if transform.pix_fmt.is_none() {
            return Err("--zscale requires --pix-fmt".into());
        }
    }
    if let Some(ref args) = transform.tonemap {
        validate_tonemap_args(args)?;
        if transform.pix_fmt.is_none() {
            return Err("--tonemap requires --pix-fmt".into());
        }
    }
    let mut video_frames = 0u64;
    let mut finished = false;
    {
        let mut handle = |frame: &mut Frame| -> Result<bool> {
            unsafe {
                let f = &mut *frame.0;
                if !format_validated {
                    let descriptor = av_pix_fmt_desc_get(f.format);
                    if descriptor.is_null() {
                        return Err("unknown decoded pixel format".into());
                    }
                    let sx = 1usize << (*descriptor).log2_chroma_w;
                    let sy = 1usize << (*descriptor).log2_chroma_h;
                    if !crop.x.is_multiple_of(sx) || !crop.y.is_multiple_of(sy) {
                        return Err("crop origin must be chroma-aligned".into());
                    }
                    pixel_format = string(av_get_pix_fmt_name(f.format));
                    format_validated = true;
                }
                if f.width <= 0
                    || f.height <= 0
                    || crop
                        .x
                        .checked_add(crop.width)
                        .is_none_or(|right| right > f.width as usize)
                    || crop
                        .y
                        .checked_add(crop.height)
                        .is_none_or(|bottom| bottom > f.height as usize)
                {
                    return Err("dynamic frame geometry is not compatible with crop".into());
                }
                if f.pts == NOPTS {
                    f.pts = f.best_effort_timestamp;
                }
                if f.time_base.num <= 0 || f.time_base.den <= 0 {
                    f.time_base = (*decoder.0).pkt_timebase;
                }
                if let Some((start, end)) = interval {
                    let pts = f.pts;
                    if pts != NOPTS && pts >= end {
                        av_frame_unref(frame.0);
                        return Ok(true);
                    }
                    if pts == NOPTS || pts < start {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
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
                        "apply exact decode crop view",
                    )?;
                }
                let output = if transform.horizontal_flip {
                    horizontal_copy_frame(transformed.0, frame.0)?;
                    transformed.0
                } else {
                    frame.0
                };
                if transform.vertical_flip {
                    flip_view(output)?;
                    // Keep the reusable hflip destination in positive-stride form.
                    if output == transformed.0 {
                        flip_view(output)?;
                    }
                }
                // Match FFmpeg vf order: crop → hflip → vflip → transpose → rotate → pad → scale → subtitles → overlay → colorspace → zscale → tonemap → format.
                let output = if let Some(mode) = transform.transpose {
                    transpose_frame(&mut transpose, transposed.0, output, mode)?;
                    transposed.0
                } else {
                    output
                };
                let output = if let Some(angle) = transform.rotate {
                    rotate_frame(&mut rotate_graph, rotated.0, output, angle)?;
                    rotated.0
                } else {
                    output
                };
                let output = if let Some(pad) = transform.pad {
                    pad_frame(&mut pad_graph, padded.0, output, pad)?;
                    padded.0
                } else {
                    output
                };
                let (output, format_done) =
                    if let (true, Some(fmt)) = (transform.scale.is_some(), target_pix_fmt) {
                        if transform.burn_subs.is_none()
                            && transform.overlay.is_none()
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
                            && transform.epx.is_none()
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
                            && transform.tile.is_none()
                            && transform.untile.is_none()
                            && transform.shuffleframes.is_none()
                            && transform.reverse.is_none()
                            && transform.r#loop.is_none()
                            && transform.thumbnail.is_none()
                            && transform.freezedetect.is_none()
                            && transform.pseudocolor.is_none()
                            && transform.minterpolate.is_none()
                            && transform.fps.is_none()
                            && transform.colorspace.is_none()
                            && transform.zscale.is_none()
                            && transform.tonemap.is_none()
                        {
                            scale_convert_frame(
                                &mut sws,
                                scaled.0,
                                output,
                                out_w as i32,
                                out_h as i32,
                                fmt,
                            )?;
                            (scaled.0, true)
                        } else {
                            scale_frame(&mut sws, scaled.0, output, out_w as i32, out_h as i32)?;
                            (scaled.0, false)
                        }
                    } else if transform.scale.is_some() {
                        scale_frame(&mut sws, scaled.0, output, out_w as i32, out_h as i32)?;
                        (scaled.0, false)
                    } else {
                        (output, false)
                    };
                let output = if let Some(args) = transform.epx.as_deref() {
                    let dst = epxed.as_mut().ok_or("epx frame missing")?;
                    epx_frame(&mut epx_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(path) = transform.burn_subs.as_deref() {
                    let dst = burned.as_mut().ok_or("burn frame missing")?;
                    subtitles_frame(&mut burn_graph, dst.0, output, path)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(spec) = transform.overlay.as_ref() {
                    let dst = overlaid.as_mut().ok_or("overlay frame missing")?;
                    overlay_frame(
                        &mut overlay_graph,
                        dst.0,
                        output,
                        &spec.path,
                        spec.x,
                        spec.y,
                    )?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.yadif.as_deref() {
                    let dst = deinterlaced.as_mut().ok_or("yadif frame missing")?;
                    let produced = yadif_push_frame(&mut yadif_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.bwdif.as_deref() {
                    let dst = bwdif_out.as_mut().ok_or("bwdif frame missing")?;
                    let produced = bwdif_push_frame(&mut bwdif_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.w3fdif.as_deref() {
                    let dst = w3fdif_out.as_mut().ok_or("w3fdif frame missing")?;
                    let produced = w3fdif_push_frame(&mut w3fdif_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.tblend.as_deref() {
                    let dst = tblended.as_mut().ok_or("tblend frame missing")?;
                    let produced = tblend_frame(&mut tblend_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.tmix.as_deref() {
                    let dst = tmixed.as_mut().ok_or("tmix frame missing")?;
                    let produced = tmix_push_frame(&mut tmix_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.hqdn3d.as_deref() {
                    let dst = denoised.as_mut().ok_or("hqdn3d frame missing")?;
                    hqdn3d_frame(&mut hqdn3d_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.gblur.as_deref() {
                    let dst = blurred.as_mut().ok_or("gblur frame missing")?;
                    gblur_frame(&mut gblur_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.eq.as_deref() {
                    let dst = equalized.as_mut().ok_or("eq frame missing")?;
                    eq_frame(&mut eq_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.unsharp.as_deref() {
                    let dst = sharpened.as_mut().ok_or("unsharp frame missing")?;
                    unsharp_frame(&mut unsharp_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.hue.as_deref() {
                    let dst = hued.as_mut().ok_or("hue frame missing")?;
                    hue_frame(&mut hue_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.avgblur.as_deref() {
                    let dst = avgblurred.as_mut().ok_or("avgblur frame missing")?;
                    avgblur_frame(&mut avgblur_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.boxblur.as_deref() {
                    let dst = boxblurred.as_mut().ok_or("boxblur frame missing")?;
                    boxblur_frame(&mut boxblur_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.negate.as_deref() {
                    let dst = negated.as_mut().ok_or("negate frame missing")?;
                    negate_frame(&mut negate_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.edgedetect.as_deref() {
                    let dst = edged.as_mut().ok_or("edgedetect frame missing")?;
                    edgedetect_frame(&mut edgedetect_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if args.contains("mode=colormix") {
                        let src_fmt = (*output).format;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            converted.0
                        } else {
                            out
                        }
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.sobel.as_deref() {
                    let dst = sobeled.as_mut().ok_or("sobel frame missing")?;
                    sobel_frame(&mut sobel_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.prewitt.as_deref() {
                    let dst = prewitted.as_mut().ok_or("prewitt frame missing")?;
                    prewitt_frame(&mut prewitt_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.roberts.as_deref() {
                    let dst = robertsed.as_mut().ok_or("roberts frame missing")?;
                    roberts_frame(&mut roberts_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.kirsch.as_deref() {
                    let dst = kirsched.as_mut().ok_or("kirsch frame missing")?;
                    kirsch_frame(&mut kirsch_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.scharr.as_deref() {
                    let dst = scharred.as_mut().ok_or("scharr frame missing")?;
                    scharr_frame(&mut scharr_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.atadenoise.as_deref() {
                    let dst = atdenoised.as_mut().ok_or("atadenoise frame missing")?;
                    let produced =
                        atadenoise_push_frame(&mut atadenoise_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.owdenoise.as_deref() {
                    let dst = owdenoised.as_mut().ok_or("owdenoise frame missing")?;
                    owdenoise_frame(&mut owdenoise_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.vaguedenoiser.as_deref() {
                    let dst = vaguedenoised
                        .as_mut()
                        .ok_or("vaguedenoiser frame missing")?;
                    vaguedenoiser_frame(&mut vaguedenoiser_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.nlmeans.as_deref() {
                    let dst = nldenoised.as_mut().ok_or("nlmeans frame missing")?;
                    nlmeans_frame(&mut nlmeans_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.bm3d.as_deref() {
                    let dst = bm3ded.as_mut().ok_or("bm3d frame missing")?;
                    bm3d_frame(&mut bm3d_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.dctdnoiz.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = dctdnoized.as_mut().ok_or("dctdnoiz frame missing")?;
                    dctdnoiz_frame(&mut dctdnoiz_graph, dst.0, output, args)?;
                    let out = dst.0;
                    // dctdnoiz materializes rgb24; fair-pair reverts via libswscale like photosensitivity.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.fftdnoiz.as_deref() {
                    let dst = fftdnoized.as_mut().ok_or("fftdnoiz frame missing")?;
                    fftdnoiz_frame(&mut fftdnoiz_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.smartblur.as_deref() {
                    let dst = smartblurred.as_mut().ok_or("smartblur frame missing")?;
                    smartblur_frame(&mut smartblur_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.sab.as_deref() {
                    let dst = sabbed.as_mut().ok_or("sab frame missing")?;
                    sab_frame(&mut sab_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.bilateral.as_deref() {
                    let dst = bilateraled.as_mut().ok_or("bilateral frame missing")?;
                    bilateral_frame(&mut bilateral_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.cas.as_deref() {
                    let dst = cased.as_mut().ok_or("cas frame missing")?;
                    cas_frame(&mut cas_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.vignette.as_deref() {
                    let dst = vignetted.as_mut().ok_or("vignette frame missing")?;
                    vignette_frame(&mut vignette_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.curves.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = curved.as_mut().ok_or("curves frame missing")?;
                    curves_frame(&mut curves_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorbalance.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = balanced.as_mut().ok_or("colorbalance frame missing")?;
                    colorbalance_frame(&mut colorbalance_graph, dst.0, output, args)?;
                    let out = dst.0;
                    // colorbalance materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorlevels.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = leveled.as_mut().ok_or("colorlevels frame missing")?;
                    colorlevels_frame(&mut colorlevels_graph, dst.0, output, args)?;
                    let out = dst.0;
                    // colorlevels materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorchannelmixer.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = channelmixed
                        .as_mut()
                        .ok_or("colorchannelmixer frame missing")?;
                    colorchannelmixer_frame(&mut colorchannelmixer_graph, dst.0, output, args)?;
                    let out = dst.0;
                    // colorchannelmixer materializes bgr0; fair-pair reverts via libswscale like `--pix-fmt`.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.deflicker.as_deref() {
                    let dst = deflickered.as_mut().ok_or("deflicker frame missing")?;
                    let produced = deflicker_push_frame(&mut deflicker_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.photosensitivity.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = photosensitized
                        .as_mut()
                        .ok_or("photosensitivity frame missing")?;
                    photosensitivity_frame(&mut photosensitivity_graph, dst.0, output, args)?;
                    let out = dst.0;
                    // photosensitivity materializes rgb24; fair-pair reverts via libswscale like colorbalance.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.monochrome.as_deref() {
                    let dst = monochromed.as_mut().ok_or("monochrome frame missing")?;
                    monochrome_frame(&mut monochrome_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.grayworld.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = grayworlded.as_mut().ok_or("grayworld frame missing")?;
                    grayworld_frame(&mut grayworld_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.drawbox.as_deref() {
                    let dst = drawboxed.as_mut().ok_or("drawbox frame missing")?;
                    drawbox_frame(&mut drawbox_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.drawgrid.as_deref() {
                    let dst = drawgridd.as_mut().ok_or("drawgrid frame missing")?;
                    drawgrid_frame(&mut drawgrid_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.lagfun.as_deref() {
                    let dst = lagfuned.as_mut().ok_or("lagfun frame missing")?;
                    let produced = lagfun_push_frame(&mut lagfun_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.amplify.as_deref() {
                    let dst = amplified.as_mut().ok_or("amplify frame missing")?;
                    let produced = amplify_push_frame(&mut amplify_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.bitplanenoise.as_deref() {
                    let dst = bitplanenoised
                        .as_mut()
                        .ok_or("bitplanenoise frame missing")?;
                    bitplanenoise_frame(&mut bitplanenoise_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.deband.as_deref() {
                    let dst = debanded.as_mut().ok_or("deband frame missing")?;
                    deband_frame(&mut deband_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.gradfun.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = gradfuned.as_mut().ok_or("gradfun frame missing")?;
                    gradfun_frame(&mut gradfun_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.lenscorrection.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = lenscorrected
                        .as_mut()
                        .ok_or("lenscorrection frame missing")?;
                    lenscorrection_frame(&mut lenscorrection_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.pixelize.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = pixelized.as_mut().ok_or("pixelize frame missing")?;
                    pixelize_frame(&mut pixelize_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.removegrain.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = removegrained.as_mut().ok_or("removegrain frame missing")?;
                    removegrain_frame(&mut removegrain_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.yaepblur.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = yaepblurred.as_mut().ok_or("yaepblur frame missing")?;
                    yaepblur_frame(&mut yaepblur_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.vibrance.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = vibranced.as_mut().ok_or("vibrance frame missing")?;
                    vibrance_frame(&mut vibrance_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.dilation.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = dilated.as_mut().ok_or("dilation frame missing")?;
                    dilation_frame(&mut dilation_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.erosion.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = eroded.as_mut().ok_or("erosion frame missing")?;
                    erosion_frame(&mut erosion_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorize.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = colorized.as_mut().ok_or("colorize frame missing")?;
                    colorize_frame(&mut colorize_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.exposure.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = exposured.as_mut().ok_or("exposure frame missing")?;
                    exposure_frame(&mut exposure_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.chromashift.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = chromashifted.as_mut().ok_or("chromashift frame missing")?;
                    chromashift_frame(&mut chromashift_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorcontrast.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = colorcontrasted
                        .as_mut()
                        .ok_or("colorcontrast frame missing")?;
                    colorcontrast_frame(&mut colorcontrast_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorcorrect.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = colorcorrected
                        .as_mut()
                        .ok_or("colorcorrect frame missing")?;
                    colorcorrect_frame(&mut colorcorrect_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.histeq.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = histeqed.as_mut().ok_or("histeq frame missing")?;
                    histeq_frame(&mut histeq_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.shuffleplanes.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = shuffleplaned
                        .as_mut()
                        .ok_or("shuffleplanes frame missing")?;
                    shuffleplanes_frame(&mut shuffleplanes_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.lutyuv.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = lutyuved.as_mut().ok_or("lutyuv frame missing")?;
                    lutyuv_frame(&mut lutyuv_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorhold.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = colorholded.as_mut().ok_or("colorhold frame missing")?;
                    colorhold_frame(&mut colorhold_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.fade.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = faded.as_mut().ok_or("fade frame missing")?;
                    let produced = fade_apply_frame(
                        &mut fade_graph,
                        dst.0,
                        output,
                        args,
                        &mut fade_push_mode,
                    )?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.perspective.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = perspectived.as_mut().ok_or("perspective frame missing")?;
                    perspective_frame(&mut perspective_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.lumakey.as_deref() {
                    let dst = lumakeyed.as_mut().ok_or("lumakey frame missing")?;
                    lumakey_frame(&mut lumakey_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.chromakey.as_deref() {
                    let dst = chromakeyed.as_mut().ok_or("chromakey frame missing")?;
                    chromakey_frame(&mut chromakey_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorkey.as_deref() {
                    let dst = colorkeyed.as_mut().ok_or("colorkey frame missing")?;
                    colorkey_frame(&mut colorkey_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.despill.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = despilled.as_mut().ok_or("despill frame missing")?;
                    despill_frame(&mut despill_graph, dst.0, output, args)?;
                    let out = dst.0;
                    // despill materializes gbrp/rgb; fair-pair reverts via libswscale like colorbalance.
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.selectivecolor.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = selectivecolored
                        .as_mut()
                        .ok_or("selectivecolor frame missing")?;
                    selectivecolor_frame(&mut selectivecolor_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.stereo3d.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = stereo3ded.as_mut().ok_or("stereo3d frame missing")?;
                    stereo3d_frame(&mut stereo3d_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.field.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = fielded.as_mut().ok_or("field frame missing")?;
                    field_frame(&mut field_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.hqx.as_deref() {
                    let dst = hqxd.as_mut().ok_or("hqx frame missing")?;
                    hqx_frame(&mut hqx_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.xbr.as_deref() {
                    let dst = xbrd.as_mut().ok_or("xbr frame missing")?;
                    xbr_frame(&mut xbr_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.il.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = ild.as_mut().ok_or("il frame missing")?;
                    il_frame(&mut il_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.super2xsai.as_deref() {
                    let dst = super2xsaid.as_mut().ok_or("super2xsai frame missing")?;
                    super2xsai_frame(&mut super2xsai_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let output = if let Some(args) = transform.kerndeint.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = kerndeintd.as_mut().ok_or("kerndeint frame missing")?;
                    kerndeint_frame(&mut kerndeint_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.phase.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = phased.as_mut().ok_or("phase frame missing")?;
                    let produced = phase_apply_frame(
                        &mut phase_graph,
                        dst.0,
                        output,
                        args,
                        &mut phase_push_mode,
                    )?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.estdif.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = estdifd.as_mut().ok_or("estdif frame missing")?;
                    let produced = estdif_push_frame(&mut estdif_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.tinterlace.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = tinterlaced.as_mut().ok_or("tinterlace frame missing")?;
                    let produced =
                        tinterlace_push_frame(&mut tinterlace_graph, dst.0, output, args)?;
                    if !produced {
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                if let Some(args) = transform.separatefields.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = separatefieldsd
                        .as_mut()
                        .ok_or("separatefields frame missing")?;
                    let mut emitted = 0u64;
                    separatefields_push_frame(
                        &mut separatefields_graph,
                        dst.0,
                        output,
                        args,
                        |field| {
                            let mut out = field;
                            if (*out).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                                out = converted.0;
                            }
                            let mut doubleweave_field = |mut out: *mut AVFrame| -> Result<()> {
                                let out = if let Some(args) = transform.freezedetect.as_deref() {
                                    let src_fmt = (*out).format;
                                    let dst = freezedetectd
                                        .as_mut()
                                        .ok_or("freezedetect frame missing")?;
                                    freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                                    let o = dst.0;
                                    if (*o).format != src_fmt {
                                        convert_pix_fmt_frame(
                                            &mut fmt_sws,
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
                                let out = if let Some(args) = transform.pseudocolor.as_deref() {
                                    let src_fmt = (*out).format;
                                    let dst = pseudocolored
                                        .as_mut()
                                        .ok_or("pseudocolor frame missing")?;
                                    pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                                    let o = dst.0;
                                    if (*o).format != src_fmt {
                                        convert_pix_fmt_frame(
                                            &mut fmt_sws,
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
                                let out = if let Some(args) = transform.colorspace.as_deref() {
                                    let dst =
                                        colorspaced.as_mut().ok_or("colorspace frame missing")?;
                                    colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                                    dst.0
                                } else {
                                    out
                                };
                                let (out, format_done) =
                                    if let Some(args) = transform.zscale.as_deref() {
                                        let fmt =
                                            target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                                        let name = string(av_get_pix_fmt_name(fmt));
                                        if name.is_empty() {
                                            return Err("unknown zscale output pixel format".into());
                                        }
                                        let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                                        zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                                        (dst.0, true)
                                    } else {
                                        (out, false)
                                    };
                                let (out, format_done) = if let Some(args) =
                                    transform.tonemap.as_deref()
                                {
                                    let fmt =
                                        target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                                    let name = string(av_get_pix_fmt_name(fmt));
                                    if name.is_empty() {
                                        return Err("unknown tonemap output pixel format".into());
                                    }
                                    let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                                    tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                                    (dst.0, format_done)
                                } else {
                                    (out, format_done)
                                };
                                let out = if let Some(fmt) = target_pix_fmt {
                                    if !format_done && (*out).format != fmt {
                                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                        converted.0
                                    } else {
                                        out
                                    }
                                } else {
                                    out
                                };
                                if transform.minterpolate.is_some() || transform.fps.is_some() {
                                    let scratch = temporal_scratch
                                        .as_mut()
                                        .ok_or("temporal frame missing")?;
                                    let fps_dst =
                                        fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                                    temporal_push_frame(
                                        &mut minterpolate_graph,
                                        scratch.0,
                                        transform.minterpolate.as_deref(),
                                        &mut fps_graph,
                                        fps_dst,
                                        transform.fps.as_deref(),
                                        out,
                                        |_| {
                                            emitted += 1;
                                            Ok(())
                                        },
                                    )?;
                                } else {
                                    emitted += 1;
                                }
                                Ok(())
                            };
                            let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                                if transform.thumbnail.is_some() {
                                    let thumb_dst =
                                        thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                                    push_thumbnail_or_emit(
                                        &mut thumbnail_graph,
                                        thumb_dst.0,
                                        out,
                                        transform.thumbnail.as_deref(),
                                        |f| doubleweave_field(f),
                                    )
                                } else {
                                    doubleweave_field(out)
                                }
                            };
                            let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                                if transform.r#loop.is_some() {
                                    let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                                    push_loop_or_emit(
                                        &mut loop_graph,
                                        loop_dst.0,
                                        out,
                                        transform.r#loop.as_deref(),
                                        |f| apply_thumbnail(f),
                                    )
                                } else {
                                    apply_thumbnail(out)
                                }
                            };
                            let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                                if transform.reverse.is_some() {
                                    let rev_dst =
                                        reversed.as_mut().ok_or("reverse frame missing")?;
                                    push_reverse_or_emit(
                                        &mut reverse_graph,
                                        rev_dst.0,
                                        out,
                                        transform.reverse.as_deref(),
                                        |f| apply_loop(f),
                                    )
                                } else {
                                    apply_loop(out)
                                }
                            };
                            let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                                if transform.shuffleframes.is_some() {
                                    let sf_dst =
                                        shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                                    push_shuffleframes_or_emit(
                                        &mut shuffleframes_graph,
                                        sf_dst.0,
                                        out,
                                        transform.shuffleframes.as_deref(),
                                        |f| apply_reverse(f),
                                    )
                                } else {
                                    apply_reverse(out)
                                }
                            };
                            let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                                if transform.untile.is_some() {
                                    let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                                    push_untile_or_emit(
                                        &mut untile_graph,
                                        u_dst.0,
                                        out,
                                        transform.untile.as_deref(),
                                        |f| apply_shuffleframes(f),
                                    )
                                } else {
                                    apply_shuffleframes(out)
                                }
                            };
                            let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                                if transform.tile.is_some() {
                                    let t_dst = tiled.as_mut().ok_or("tile frame missing")?;
                                    push_tile_or_emit(
                                        &mut tile_graph,
                                        t_dst.0,
                                        out,
                                        transform.tile.as_deref(),
                                        |f| apply_untile(f),
                                    )
                                } else {
                                    apply_untile(out)
                                }
                            };

                            let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                                if transform.framestep.is_some() {
                                    let fs_dst =
                                        framestepped.as_mut().ok_or("framestep frame missing")?;
                                    push_framestep_or_emit(
                                        &mut framestep_graph,
                                        fs_dst.0,
                                        out,
                                        transform.framestep.as_deref(),
                                        |f| apply_tile(f),
                                    )
                                } else {
                                    apply_tile(out)
                                }
                            };
                            let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                                if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                                    let mpd_dst =
                                        mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                                    mpdecimate_push_frame(
                                        &mut mpdecimate_graph,
                                        mpd_dst.0,
                                        out,
                                        mpd_args,
                                        |mpd| apply_framestep(mpd),
                                    )
                                } else {
                                    apply_framestep(out)
                                }
                            };
                            let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                                if let Some(dc_args) = transform.decimate.as_deref() {
                                    let dc_dst =
                                        decimated.as_mut().ok_or("decimate frame missing")?;
                                    decimate_push_frame(
                                        &mut decimate_graph,
                                        dc_dst.0,
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
                                if let Some(pu_args) = transform.pullup.as_deref() {
                                    let pu_dst = pulledup.as_mut().ok_or("pullup frame missing")?;
                                    pullup_push_frame(
                                        &mut pullup_graph,
                                        pu_dst.0,
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
                                if let Some(tc_args) = transform.telecine.as_deref() {
                                    let tc_dst =
                                        telecined.as_mut().ok_or("telecine frame missing")?;
                                    telecine_push_frame(
                                        &mut telecine_graph,
                                        tc_dst.0,
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
                                if let Some(fp_args) = transform.framepack.as_deref() {
                                    let fp_dst =
                                        framepacked.as_mut().ok_or("framepack frame missing")?;
                                    framepack_push_frame(
                                        &mut framepack_graph,
                                        fp_dst.0,
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
                                if let Some(dw_args) = transform.doubleweave.as_deref() {
                                    let dw_dst =
                                        doubleweaved.as_mut().ok_or("doubleweave frame missing")?;
                                    doubleweave_push_frame(
                                        &mut doubleweave_graph,
                                        dw_dst.0,
                                        out,
                                        dw_args,
                                        |doubled| apply_framepack(doubled),
                                    )?;
                                } else {
                                    apply_framepack(out)?;
                                }
                                Ok(())
                            };
                            if let Some(weave_args) = transform.weave.as_deref() {
                                let weave_dst = weaved.as_mut().ok_or("weave frame missing")?;
                                weave_push_frame(
                                    &mut weave_graph,
                                    weave_dst.0,
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
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.weave.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = weaved.as_mut().ok_or("weave frame missing")?;
                    let mut emitted = 0u64;
                    weave_push_frame(&mut weave_graph, dst.0, output, args, |woven| {
                        let mut out = woven;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let mut finish = |mut out: *mut AVFrame| -> Result<()> {
                            let out = if let Some(args) = transform.freezedetect.as_deref() {
                                let src_fmt = (*out).format;
                                let dst =
                                    freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                                freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                                let o = dst.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = transform.pseudocolor.as_deref() {
                                let src_fmt = (*out).format;
                                let dst =
                                    pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                                pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                                let o = dst.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = transform.colorspace.as_deref() {
                                let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                                colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                                dst.0
                            } else {
                                out
                            };
                            let (out, format_done) = if let Some(args) = transform.zscale.as_deref()
                            {
                                let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown zscale output pixel format".into());
                                }
                                let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                                zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                                (dst.0, true)
                            } else {
                                (out, false)
                            };
                            let (out, format_done) = if let Some(args) =
                                transform.tonemap.as_deref()
                            {
                                let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown tonemap output pixel format".into());
                                }
                                let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                                tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                                (dst.0, format_done)
                            } else {
                                (out, format_done)
                            };
                            let out = if let Some(fmt) = target_pix_fmt {
                                if !format_done && (*out).format != fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                    converted.0
                                } else {
                                    out
                                }
                            } else {
                                out
                            };
                            if transform.minterpolate.is_some() || transform.fps.is_some() {
                                let scratch =
                                    temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                                let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                                temporal_push_frame(
                                    &mut minterpolate_graph,
                                    scratch.0,
                                    transform.minterpolate.as_deref(),
                                    &mut fps_graph,
                                    fps_dst,
                                    transform.fps.as_deref(),
                                    out,
                                    |_| {
                                        emitted += 1;
                                        Ok(())
                                    },
                                )?;
                            } else {
                                emitted += 1;
                            }
                            Ok(())
                        };
                        let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                            if transform.thumbnail.is_some() {
                                let thumb_dst =
                                    thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                                push_thumbnail_or_emit(
                                    &mut thumbnail_graph,
                                    thumb_dst.0,
                                    out,
                                    transform.thumbnail.as_deref(),
                                    |f| finish(f),
                                )
                            } else {
                                finish(out)
                            }
                        };
                        let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                            if transform.r#loop.is_some() {
                                let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                                push_loop_or_emit(
                                    &mut loop_graph,
                                    loop_dst.0,
                                    out,
                                    transform.r#loop.as_deref(),
                                    |f| apply_thumbnail(f),
                                )
                            } else {
                                apply_thumbnail(out)
                            }
                        };
                        let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                            if transform.reverse.is_some() {
                                let rev_dst = reversed.as_mut().ok_or("reverse frame missing")?;
                                push_reverse_or_emit(
                                    &mut reverse_graph,
                                    rev_dst.0,
                                    out,
                                    transform.reverse.as_deref(),
                                    |f| apply_loop(f),
                                )
                            } else {
                                apply_loop(out)
                            }
                        };
                        let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                            if transform.shuffleframes.is_some() {
                                let sf_dst =
                                    shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                                push_shuffleframes_or_emit(
                                    &mut shuffleframes_graph,
                                    sf_dst.0,
                                    out,
                                    transform.shuffleframes.as_deref(),
                                    |f| apply_reverse(f),
                                )
                            } else {
                                apply_reverse(out)
                            }
                        };
                        let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                            if transform.untile.is_some() {
                                let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                                push_untile_or_emit(
                                    &mut untile_graph,
                                    u_dst.0,
                                    out,
                                    transform.untile.as_deref(),
                                    |f| apply_shuffleframes(f),
                                )
                            } else {
                                apply_shuffleframes(out)
                            }
                        };
                        let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                            if transform.tile.is_some() {
                                let t_dst = tiled.as_mut().ok_or("tile frame missing")?;
                                push_tile_or_emit(
                                    &mut tile_graph,
                                    t_dst.0,
                                    out,
                                    transform.tile.as_deref(),
                                    |f| apply_untile(f),
                                )
                            } else {
                                apply_untile(out)
                            }
                        };

                        let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                            if transform.framestep.is_some() {
                                let fs_dst =
                                    framestepped.as_mut().ok_or("framestep frame missing")?;
                                push_framestep_or_emit(
                                    &mut framestep_graph,
                                    fs_dst.0,
                                    out,
                                    transform.framestep.as_deref(),
                                    |f| apply_tile(f),
                                )
                            } else {
                                apply_tile(out)
                            }
                        };
                        let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                                let mpd_dst =
                                    mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                                mpdecimate_push_frame(
                                    &mut mpdecimate_graph,
                                    mpd_dst.0,
                                    out,
                                    mpd_args,
                                    |mpd| apply_framestep(mpd),
                                )
                            } else {
                                apply_framestep(out)
                            }
                        };
                        let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(dc_args) = transform.decimate.as_deref() {
                                let dc_dst = decimated.as_mut().ok_or("decimate frame missing")?;
                                decimate_push_frame(
                                    &mut decimate_graph,
                                    dc_dst.0,
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
                            if let Some(pu_args) = transform.pullup.as_deref() {
                                let pu_dst = pulledup.as_mut().ok_or("pullup frame missing")?;
                                pullup_push_frame(
                                    &mut pullup_graph,
                                    pu_dst.0,
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
                            if let Some(tc_args) = transform.telecine.as_deref() {
                                let tc_dst = telecined.as_mut().ok_or("telecine frame missing")?;
                                telecine_push_frame(
                                    &mut telecine_graph,
                                    tc_dst.0,
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
                            if let Some(fp_args) = transform.framepack.as_deref() {
                                let fp_dst =
                                    framepacked.as_mut().ok_or("framepack frame missing")?;
                                framepack_push_frame(
                                    &mut framepack_graph,
                                    fp_dst.0,
                                    out,
                                    fp_args,
                                    |packed| apply_telecine(packed),
                                )?;
                            } else {
                                apply_telecine(out)?;
                            }
                            Ok(())
                        };
                        if let Some(dw_args) = transform.doubleweave.as_deref() {
                            let dw_dst =
                                doubleweaved.as_mut().ok_or("doubleweave frame missing")?;
                            doubleweave_push_frame(
                                &mut doubleweave_graph,
                                dw_dst.0,
                                out,
                                dw_args,
                                |doubled| apply_framepack(doubled),
                            )?;
                        } else {
                            apply_framepack(out)?;
                        }
                        Ok(())
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.doubleweave.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = doubleweaved.as_mut().ok_or("doubleweave frame missing")?;
                    let mut emitted = 0u64;
                    doubleweave_push_frame(
                        &mut doubleweave_graph,
                        dst.0,
                        output,
                        args,
                        |doubled| {
                            let mut out = doubled;
                            if (*out).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                                out = converted.0;
                            }
                            let mut finish = |mut out: *mut AVFrame| -> Result<()> {
                                let out = if let Some(args) = transform.freezedetect.as_deref() {
                                    let src_fmt = (*out).format;
                                    let dst = freezedetectd
                                        .as_mut()
                                        .ok_or("freezedetect frame missing")?;
                                    freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                                    let o = dst.0;
                                    if (*o).format != src_fmt {
                                        convert_pix_fmt_frame(
                                            &mut fmt_sws,
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
                                let out = if let Some(args) = transform.pseudocolor.as_deref() {
                                    let src_fmt = (*out).format;
                                    let dst = pseudocolored
                                        .as_mut()
                                        .ok_or("pseudocolor frame missing")?;
                                    pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                                    let o = dst.0;
                                    if (*o).format != src_fmt {
                                        convert_pix_fmt_frame(
                                            &mut fmt_sws,
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
                                let out = if let Some(args) = transform.colorspace.as_deref() {
                                    let dst =
                                        colorspaced.as_mut().ok_or("colorspace frame missing")?;
                                    colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                                    dst.0
                                } else {
                                    out
                                };
                                let (out, format_done) =
                                    if let Some(args) = transform.zscale.as_deref() {
                                        let fmt =
                                            target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                                        let name = string(av_get_pix_fmt_name(fmt));
                                        if name.is_empty() {
                                            return Err("unknown zscale output pixel format".into());
                                        }
                                        let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                                        zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                                        (dst.0, true)
                                    } else {
                                        (out, false)
                                    };
                                let (out, format_done) = if let Some(args) =
                                    transform.tonemap.as_deref()
                                {
                                    let fmt =
                                        target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                                    let name = string(av_get_pix_fmt_name(fmt));
                                    if name.is_empty() {
                                        return Err("unknown tonemap output pixel format".into());
                                    }
                                    let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                                    tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                                    (dst.0, format_done)
                                } else {
                                    (out, format_done)
                                };
                                let out = if let Some(fmt) = target_pix_fmt {
                                    if !format_done && (*out).format != fmt {
                                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                        converted.0
                                    } else {
                                        out
                                    }
                                } else {
                                    out
                                };
                                if transform.minterpolate.is_some() || transform.fps.is_some() {
                                    let scratch = temporal_scratch
                                        .as_mut()
                                        .ok_or("temporal frame missing")?;
                                    let fps_dst =
                                        fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                                    temporal_push_frame(
                                        &mut minterpolate_graph,
                                        scratch.0,
                                        transform.minterpolate.as_deref(),
                                        &mut fps_graph,
                                        fps_dst,
                                        transform.fps.as_deref(),
                                        out,
                                        |_| {
                                            emitted += 1;
                                            Ok(())
                                        },
                                    )?;
                                } else {
                                    emitted += 1;
                                }
                                Ok(())
                            };
                            let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                                if transform.thumbnail.is_some() {
                                    let thumb_dst =
                                        thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                                    push_thumbnail_or_emit(
                                        &mut thumbnail_graph,
                                        thumb_dst.0,
                                        out,
                                        transform.thumbnail.as_deref(),
                                        |f| finish(f),
                                    )
                                } else {
                                    finish(out)
                                }
                            };
                            let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                                if transform.r#loop.is_some() {
                                    let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                                    push_loop_or_emit(
                                        &mut loop_graph,
                                        loop_dst.0,
                                        out,
                                        transform.r#loop.as_deref(),
                                        |f| apply_thumbnail(f),
                                    )
                                } else {
                                    apply_thumbnail(out)
                                }
                            };
                            let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                                if transform.reverse.is_some() {
                                    let rev_dst =
                                        reversed.as_mut().ok_or("reverse frame missing")?;
                                    push_reverse_or_emit(
                                        &mut reverse_graph,
                                        rev_dst.0,
                                        out,
                                        transform.reverse.as_deref(),
                                        |f| apply_loop(f),
                                    )
                                } else {
                                    apply_loop(out)
                                }
                            };
                            let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                                if transform.shuffleframes.is_some() {
                                    let sf_dst =
                                        shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                                    push_shuffleframes_or_emit(
                                        &mut shuffleframes_graph,
                                        sf_dst.0,
                                        out,
                                        transform.shuffleframes.as_deref(),
                                        |f| apply_reverse(f),
                                    )
                                } else {
                                    apply_reverse(out)
                                }
                            };
                            let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                                if transform.untile.is_some() {
                                    let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                                    push_untile_or_emit(
                                        &mut untile_graph,
                                        u_dst.0,
                                        out,
                                        transform.untile.as_deref(),
                                        |f| apply_shuffleframes(f),
                                    )
                                } else {
                                    apply_shuffleframes(out)
                                }
                            };
                            let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                                if transform.tile.is_some() {
                                    let t_dst = tiled.as_mut().ok_or("tile frame missing")?;
                                    push_tile_or_emit(
                                        &mut tile_graph,
                                        t_dst.0,
                                        out,
                                        transform.tile.as_deref(),
                                        |f| apply_untile(f),
                                    )
                                } else {
                                    apply_untile(out)
                                }
                            };

                            let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                                if transform.framestep.is_some() {
                                    let fs_dst =
                                        framestepped.as_mut().ok_or("framestep frame missing")?;
                                    push_framestep_or_emit(
                                        &mut framestep_graph,
                                        fs_dst.0,
                                        out,
                                        transform.framestep.as_deref(),
                                        |f| apply_tile(f),
                                    )
                                } else {
                                    apply_tile(out)
                                }
                            };
                            let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                                if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                                    let mpd_dst =
                                        mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                                    mpdecimate_push_frame(
                                        &mut mpdecimate_graph,
                                        mpd_dst.0,
                                        out,
                                        mpd_args,
                                        |mpd| apply_framestep(mpd),
                                    )
                                } else {
                                    apply_framestep(out)
                                }
                            };
                            let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                                if let Some(dc_args) = transform.decimate.as_deref() {
                                    let dc_dst =
                                        decimated.as_mut().ok_or("decimate frame missing")?;
                                    decimate_push_frame(
                                        &mut decimate_graph,
                                        dc_dst.0,
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
                                if let Some(pu_args) = transform.pullup.as_deref() {
                                    let pu_dst = pulledup.as_mut().ok_or("pullup frame missing")?;
                                    pullup_push_frame(
                                        &mut pullup_graph,
                                        pu_dst.0,
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
                                if let Some(tc_args) = transform.telecine.as_deref() {
                                    let tc_dst =
                                        telecined.as_mut().ok_or("telecine frame missing")?;
                                    telecine_push_frame(
                                        &mut telecine_graph,
                                        tc_dst.0,
                                        out,
                                        tc_args,
                                        |tc| apply_pullup(tc),
                                    )?;
                                } else {
                                    apply_pullup(out)?;
                                }
                                Ok(())
                            };
                            if let Some(fp_args) = transform.framepack.as_deref() {
                                let fp_dst =
                                    framepacked.as_mut().ok_or("framepack frame missing")?;
                                framepack_push_frame(
                                    &mut framepack_graph,
                                    fp_dst.0,
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
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.framepack.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = framepacked.as_mut().ok_or("framepack frame missing")?;
                    let mut emitted = 0u64;
                    let mut finish = |mut out: *mut AVFrame| -> Result<()> {
                        let out = if let Some(args) = transform.freezedetect.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                            freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.pseudocolor.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                            pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.colorspace.as_deref() {
                            let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                            colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                            dst.0
                        } else {
                            out
                        };
                        let (out, format_done) = if let Some(args) = transform.zscale.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                            zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                            (dst.0, true)
                        } else {
                            (out, false)
                        };
                        let (out, format_done) = if let Some(args) = transform.tonemap.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                            tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                            (dst.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = target_pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if transform.minterpolate.is_some() || transform.fps.is_some() {
                            let scratch =
                                temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                            let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                            temporal_push_frame(
                                &mut minterpolate_graph,
                                scratch.0,
                                transform.minterpolate.as_deref(),
                                &mut fps_graph,
                                fps_dst,
                                transform.fps.as_deref(),
                                out,
                                |_| {
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            emitted += 1;
                        }
                        Ok(())
                    };
                    let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                        if transform.thumbnail.is_some() {
                            let thumb_dst =
                                thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                            push_thumbnail_or_emit(
                                &mut thumbnail_graph,
                                thumb_dst.0,
                                out,
                                transform.thumbnail.as_deref(),
                                |f| finish(f),
                            )
                        } else {
                            finish(out)
                        }
                    };
                    let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                        if transform.r#loop.is_some() {
                            let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                            push_loop_or_emit(
                                &mut loop_graph,
                                loop_dst.0,
                                out,
                                transform.r#loop.as_deref(),
                                |f| apply_thumbnail(f),
                            )
                        } else {
                            apply_thumbnail(out)
                        }
                    };
                    let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                        if transform.reverse.is_some() {
                            let rev_dst = reversed.as_mut().ok_or("reverse frame missing")?;
                            push_reverse_or_emit(
                                &mut reverse_graph,
                                rev_dst.0,
                                out,
                                transform.reverse.as_deref(),
                                |f| apply_loop(f),
                            )
                        } else {
                            apply_loop(out)
                        }
                    };
                    let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                        if transform.shuffleframes.is_some() {
                            let sf_dst = shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                            push_shuffleframes_or_emit(
                                &mut shuffleframes_graph,
                                sf_dst.0,
                                out,
                                transform.shuffleframes.as_deref(),
                                |f| apply_reverse(f),
                            )
                        } else {
                            apply_reverse(out)
                        }
                    };
                    let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                        if transform.untile.is_some() {
                            let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                            push_untile_or_emit(
                                &mut untile_graph,
                                u_dst.0,
                                out,
                                transform.untile.as_deref(),
                                |f| apply_shuffleframes(f),
                            )
                        } else {
                            apply_shuffleframes(out)
                        }
                    };
                    let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                        if transform.tile.is_some() {
                            let t_dst = tiled.as_mut().ok_or("tile frame missing")?;
                            push_tile_or_emit(
                                &mut tile_graph,
                                t_dst.0,
                                out,
                                transform.tile.as_deref(),
                                |f| apply_untile(f),
                            )
                        } else {
                            apply_untile(out)
                        }
                    };

                    let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                        if transform.framestep.is_some() {
                            let fs_dst = framestepped.as_mut().ok_or("framestep frame missing")?;
                            push_framestep_or_emit(
                                &mut framestep_graph,
                                fs_dst.0,
                                out,
                                transform.framestep.as_deref(),
                                |f| apply_tile(f),
                            )
                        } else {
                            apply_tile(out)
                        }
                    };
                    let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                            let mpd_dst = mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                            mpdecimate_push_frame(
                                &mut mpdecimate_graph,
                                mpd_dst.0,
                                out,
                                mpd_args,
                                |mpd| apply_framestep(mpd),
                            )
                        } else {
                            apply_framestep(out)
                        }
                    };
                    let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(dc_args) = transform.decimate.as_deref() {
                            let dc_dst = decimated.as_mut().ok_or("decimate frame missing")?;
                            decimate_push_frame(
                                &mut decimate_graph,
                                dc_dst.0,
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
                        if let Some(pu_args) = transform.pullup.as_deref() {
                            let pu_dst = pulledup.as_mut().ok_or("pullup frame missing")?;
                            pullup_push_frame(&mut pullup_graph, pu_dst.0, out, pu_args, |pu| {
                                apply_decimate(pu)
                            })?;
                        } else {
                            apply_decimate(out)?;
                        }
                        Ok(())
                    };
                    let mut apply_telecine = |out: *mut AVFrame| -> Result<()> {
                        if let Some(tc_args) = transform.telecine.as_deref() {
                            let tc_dst = telecined.as_mut().ok_or("telecine frame missing")?;
                            telecine_push_frame(
                                &mut telecine_graph,
                                tc_dst.0,
                                out,
                                tc_args,
                                |tc| apply_pullup(tc),
                            )?;
                        } else {
                            apply_pullup(out)?;
                        }
                        Ok(())
                    };
                    framepack_push_frame(&mut framepack_graph, dst.0, output, args, |packed| {
                        apply_telecine(packed)
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.telecine.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = telecined.as_mut().ok_or("telecine frame missing")?;
                    let mut emitted = 0u64;
                    telecine_push_frame(&mut telecine_graph, dst.0, output, args, |tc| {
                        let mut after_decimate = |mut out: *mut AVFrame| -> Result<()> {
                            if (*out).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                                out = converted.0;
                            }
                            let out = if let Some(args) = transform.freezedetect.as_deref() {
                                let src_fmt = (*out).format;
                                let dst =
                                    freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                                freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                                let o = dst.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = transform.pseudocolor.as_deref() {
                                let src_fmt = (*out).format;
                                let dst =
                                    pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                                pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                                let o = dst.0;
                                if (*o).format != src_fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                    converted.0
                                } else {
                                    o
                                }
                            } else {
                                out
                            };
                            let out = if let Some(args) = transform.colorspace.as_deref() {
                                let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                                colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                                dst.0
                            } else {
                                out
                            };
                            let (out, format_done) = if let Some(args) = transform.zscale.as_deref()
                            {
                                let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown zscale output pixel format".into());
                                }
                                let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                                zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                                (dst.0, true)
                            } else {
                                (out, false)
                            };
                            let (out, format_done) = if let Some(args) =
                                transform.tonemap.as_deref()
                            {
                                let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                                let name = string(av_get_pix_fmt_name(fmt));
                                if name.is_empty() {
                                    return Err("unknown tonemap output pixel format".into());
                                }
                                let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                                tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                                (dst.0, format_done)
                            } else {
                                (out, format_done)
                            };
                            let out = if let Some(fmt) = target_pix_fmt {
                                if !format_done && (*out).format != fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                    converted.0
                                } else {
                                    out
                                }
                            } else {
                                out
                            };
                            if transform.minterpolate.is_some() || transform.fps.is_some() {
                                let scratch =
                                    temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                                let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                                temporal_push_frame(
                                    &mut minterpolate_graph,
                                    scratch.0,
                                    transform.minterpolate.as_deref(),
                                    &mut fps_graph,
                                    fps_dst,
                                    transform.fps.as_deref(),
                                    out,
                                    |_| {
                                        emitted += 1;
                                        Ok(())
                                    },
                                )?;
                            } else {
                                emitted += 1;
                            }
                            Ok(())
                        };
                        let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                            if transform.thumbnail.is_some() {
                                let thumb_dst =
                                    thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                                push_thumbnail_or_emit(
                                    &mut thumbnail_graph,
                                    thumb_dst.0,
                                    out,
                                    transform.thumbnail.as_deref(),
                                    |f| after_decimate(f),
                                )
                            } else {
                                after_decimate(out)
                            }
                        };
                        let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                            if transform.r#loop.is_some() {
                                let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                                push_loop_or_emit(
                                    &mut loop_graph,
                                    loop_dst.0,
                                    out,
                                    transform.r#loop.as_deref(),
                                    |f| apply_thumbnail(f),
                                )
                            } else {
                                apply_thumbnail(out)
                            }
                        };
                        let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                            if transform.reverse.is_some() {
                                let rev_dst = reversed.as_mut().ok_or("reverse frame missing")?;
                                push_reverse_or_emit(
                                    &mut reverse_graph,
                                    rev_dst.0,
                                    out,
                                    transform.reverse.as_deref(),
                                    |f| apply_loop(f),
                                )
                            } else {
                                apply_loop(out)
                            }
                        };
                        let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                            if transform.shuffleframes.is_some() {
                                let sf_dst =
                                    shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                                push_shuffleframes_or_emit(
                                    &mut shuffleframes_graph,
                                    sf_dst.0,
                                    out,
                                    transform.shuffleframes.as_deref(),
                                    |f| apply_reverse(f),
                                )
                            } else {
                                apply_reverse(out)
                            }
                        };
                        let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                            if transform.untile.is_some() {
                                let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                                push_untile_or_emit(
                                    &mut untile_graph,
                                    u_dst.0,
                                    out,
                                    transform.untile.as_deref(),
                                    |f| apply_shuffleframes(f),
                                )
                            } else {
                                apply_shuffleframes(out)
                            }
                        };
                        let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                            if transform.tile.is_some() {
                                let t_dst = tiled.as_mut().ok_or("tile frame missing")?;
                                push_tile_or_emit(
                                    &mut tile_graph,
                                    t_dst.0,
                                    out,
                                    transform.tile.as_deref(),
                                    |f| apply_untile(f),
                                )
                            } else {
                                apply_untile(out)
                            }
                        };

                        let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                            if transform.framestep.is_some() {
                                let fs_dst =
                                    framestepped.as_mut().ok_or("framestep frame missing")?;
                                push_framestep_or_emit(
                                    &mut framestep_graph,
                                    fs_dst.0,
                                    out,
                                    transform.framestep.as_deref(),
                                    |f| apply_tile(f),
                                )
                            } else {
                                apply_tile(out)
                            }
                        };
                        let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                                let mpd_dst =
                                    mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                                mpdecimate_push_frame(
                                    &mut mpdecimate_graph,
                                    mpd_dst.0,
                                    out,
                                    mpd_args,
                                    |mpd| apply_framestep(mpd),
                                )
                            } else {
                                apply_framestep(out)
                            }
                        };
                        let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                            if let Some(dc_args) = transform.decimate.as_deref() {
                                let dc_dst = decimated.as_mut().ok_or("decimate frame missing")?;
                                decimate_push_frame(
                                    &mut decimate_graph,
                                    dc_dst.0,
                                    out,
                                    dc_args,
                                    |dc| apply_mpdecimate(dc),
                                )
                            } else {
                                apply_mpdecimate(out)
                            }
                        };
                        if let Some(pu_args) = transform.pullup.as_deref() {
                            let pu_dst = pulledup.as_mut().ok_or("pullup frame missing")?;
                            pullup_push_frame(&mut pullup_graph, pu_dst.0, tc, pu_args, |pu| {
                                apply_decimate(pu)
                            })
                        } else {
                            apply_decimate(tc)
                        }
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.pullup.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = pulledup.as_mut().ok_or("pullup frame missing")?;
                    let mut emitted = 0u64;
                    let mut after_decimate = |mut out: *mut AVFrame| -> Result<()> {
                        let mut out = out;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let out = if let Some(args) = transform.freezedetect.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                            freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.pseudocolor.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                            pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.colorspace.as_deref() {
                            let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                            colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                            dst.0
                        } else {
                            out
                        };
                        let (out, format_done) = if let Some(args) = transform.zscale.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                            zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                            (dst.0, true)
                        } else {
                            (out, false)
                        };
                        let (out, format_done) = if let Some(args) = transform.tonemap.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                            tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                            (dst.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = target_pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if transform.minterpolate.is_some() || transform.fps.is_some() {
                            let scratch =
                                temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                            let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                            temporal_push_frame(
                                &mut minterpolate_graph,
                                scratch.0,
                                transform.minterpolate.as_deref(),
                                &mut fps_graph,
                                fps_dst,
                                transform.fps.as_deref(),
                                out,
                                |_| {
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            emitted += 1;
                        }
                        Ok(())
                    };
                    let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                        if transform.thumbnail.is_some() {
                            let thumb_dst =
                                thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                            push_thumbnail_or_emit(
                                &mut thumbnail_graph,
                                thumb_dst.0,
                                out,
                                transform.thumbnail.as_deref(),
                                |f| after_decimate(f),
                            )
                        } else {
                            after_decimate(out)
                        }
                    };
                    let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                        if transform.r#loop.is_some() {
                            let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                            push_loop_or_emit(
                                &mut loop_graph,
                                loop_dst.0,
                                out,
                                transform.r#loop.as_deref(),
                                |f| apply_thumbnail(f),
                            )
                        } else {
                            apply_thumbnail(out)
                        }
                    };
                    let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                        if transform.reverse.is_some() {
                            let rev_dst = reversed.as_mut().ok_or("reverse frame missing")?;
                            push_reverse_or_emit(
                                &mut reverse_graph,
                                rev_dst.0,
                                out,
                                transform.reverse.as_deref(),
                                |f| apply_loop(f),
                            )
                        } else {
                            apply_loop(out)
                        }
                    };
                    let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                        if transform.shuffleframes.is_some() {
                            let sf_dst = shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                            push_shuffleframes_or_emit(
                                &mut shuffleframes_graph,
                                sf_dst.0,
                                out,
                                transform.shuffleframes.as_deref(),
                                |f| apply_reverse(f),
                            )
                        } else {
                            apply_reverse(out)
                        }
                    };
                    let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                        if transform.untile.is_some() {
                            let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                            push_untile_or_emit(
                                &mut untile_graph,
                                u_dst.0,
                                out,
                                transform.untile.as_deref(),
                                |f| apply_shuffleframes(f),
                            )
                        } else {
                            apply_shuffleframes(out)
                        }
                    };
                    let mut apply_tile = |out: *mut AVFrame| -> Result<()> {
                        if transform.tile.is_some() {
                            let t_dst = tiled.as_mut().ok_or("tile frame missing")?;
                            push_tile_or_emit(
                                &mut tile_graph,
                                t_dst.0,
                                out,
                                transform.tile.as_deref(),
                                |f| apply_untile(f),
                            )
                        } else {
                            apply_untile(out)
                        }
                    };

                    let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                        if transform.framestep.is_some() {
                            let fs_dst = framestepped.as_mut().ok_or("framestep frame missing")?;
                            push_framestep_or_emit(
                                &mut framestep_graph,
                                fs_dst.0,
                                out,
                                transform.framestep.as_deref(),
                                |f| apply_tile(f),
                            )
                        } else {
                            apply_tile(out)
                        }
                    };
                    let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                            let mpd_dst = mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                            mpdecimate_push_frame(
                                &mut mpdecimate_graph,
                                mpd_dst.0,
                                out,
                                mpd_args,
                                |mpd| apply_framestep(mpd),
                            )
                        } else {
                            apply_framestep(out)
                        }
                    };
                    let mut apply_decimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(dc_args) = transform.decimate.as_deref() {
                            let dc_dst = decimated.as_mut().ok_or("decimate frame missing")?;
                            decimate_push_frame(&mut decimate_graph, dc_dst.0, out, dc_args, |dc| {
                                apply_mpdecimate(dc)
                            })
                        } else {
                            apply_mpdecimate(out)
                        }
                    };
                    pullup_push_frame(&mut pullup_graph, dst.0, output, args, |pu| {
                        apply_decimate(pu)
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.decimate.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = decimated.as_mut().ok_or("decimate frame missing")?;
                    let mut emitted = 0u64;
                    let mut after_tile = |mut out: *mut AVFrame| -> Result<()> {
                        let mut out = out;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let out = if let Some(args) = transform.freezedetect.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                            freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.pseudocolor.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                            pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.colorspace.as_deref() {
                            let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                            colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                            dst.0
                        } else {
                            out
                        };
                        let (out, format_done) = if let Some(args) = transform.zscale.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                            zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                            (dst.0, true)
                        } else {
                            (out, false)
                        };
                        let (out, format_done) = if let Some(args) = transform.tonemap.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                            tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                            (dst.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = target_pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if transform.minterpolate.is_some() || transform.fps.is_some() {
                            let scratch =
                                temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                            let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                            temporal_push_frame(
                                &mut minterpolate_graph,
                                scratch.0,
                                transform.minterpolate.as_deref(),
                                &mut fps_graph,
                                fps_dst,
                                transform.fps.as_deref(),
                                out,
                                |_| {
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            emitted += 1;
                        }
                        Ok(())
                    };
                    let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                        if transform.thumbnail.is_some() {
                            let thumb_dst =
                                thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                            push_thumbnail_or_emit(
                                &mut thumbnail_graph,
                                thumb_dst.0,
                                out,
                                transform.thumbnail.as_deref(),
                                |f| after_tile(f),
                            )
                        } else {
                            after_tile(out)
                        }
                    };
                    let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                        if transform.r#loop.is_some() {
                            let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                            push_loop_or_emit(
                                &mut loop_graph,
                                loop_dst.0,
                                out,
                                transform.r#loop.as_deref(),
                                |f| apply_thumbnail(f),
                            )
                        } else {
                            apply_thumbnail(out)
                        }
                    };
                    let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                        if transform.reverse.is_some() {
                            let rev_dst = reversed.as_mut().ok_or("reverse frame missing")?;
                            push_reverse_or_emit(
                                &mut reverse_graph,
                                rev_dst.0,
                                out,
                                transform.reverse.as_deref(),
                                |f| apply_loop(f),
                            )
                        } else {
                            apply_loop(out)
                        }
                    };
                    let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                        if transform.shuffleframes.is_some() {
                            let sf_dst = shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                            push_shuffleframes_or_emit(
                                &mut shuffleframes_graph,
                                sf_dst.0,
                                out,
                                transform.shuffleframes.as_deref(),
                                |f| apply_reverse(f),
                            )
                        } else {
                            apply_reverse(out)
                        }
                    };
                    let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                        if transform.untile.is_some() {
                            let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                            push_untile_or_emit(
                                &mut untile_graph,
                                u_dst.0,
                                out,
                                transform.untile.as_deref(),
                                |f| apply_shuffleframes(f),
                            )
                        } else {
                            apply_shuffleframes(out)
                        }
                    };
                    let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                        if transform.framestep.is_some() {
                            let fs_dst = framestepped.as_mut().ok_or("framestep frame missing")?;
                            push_framestep_or_emit(
                                &mut framestep_graph,
                                fs_dst.0,
                                out,
                                transform.framestep.as_deref(),
                                |f| apply_untile(f),
                            )
                        } else {
                            apply_untile(out)
                        }
                    };
                    let mut apply_mpdecimate = |out: *mut AVFrame| -> Result<()> {
                        if let Some(mpd_args) = transform.mpdecimate.as_deref() {
                            let mpd_dst = mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                            mpdecimate_push_frame(
                                &mut mpdecimate_graph,
                                mpd_dst.0,
                                out,
                                mpd_args,
                                |mpd| apply_framestep(mpd),
                            )
                        } else {
                            apply_framestep(out)
                        }
                    };
                    decimate_push_frame(&mut decimate_graph, dst.0, output, args, |dc| {
                        apply_mpdecimate(dc)
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.mpdecimate.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
                    let mut emitted = 0u64;
                    let mut after_tile = |mut out: *mut AVFrame| -> Result<()> {
                        let mut out = out;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let out = if let Some(args) = transform.freezedetect.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                            freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.pseudocolor.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                            pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.colorspace.as_deref() {
                            let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                            colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                            dst.0
                        } else {
                            out
                        };
                        let (out, format_done) = if let Some(args) = transform.zscale.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                            zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                            (dst.0, true)
                        } else {
                            (out, false)
                        };
                        let (out, format_done) = if let Some(args) = transform.tonemap.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                            tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                            (dst.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = target_pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if transform.minterpolate.is_some() || transform.fps.is_some() {
                            let scratch =
                                temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                            let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                            temporal_push_frame(
                                &mut minterpolate_graph,
                                scratch.0,
                                transform.minterpolate.as_deref(),
                                &mut fps_graph,
                                fps_dst,
                                transform.fps.as_deref(),
                                out,
                                |_| {
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
                            emitted += 1;
                        }
                        Ok(())
                    };
                    let mut apply_thumbnail = |out: *mut AVFrame| -> Result<()> {
                        if transform.thumbnail.is_some() {
                            let thumb_dst =
                                thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                            push_thumbnail_or_emit(
                                &mut thumbnail_graph,
                                thumb_dst.0,
                                out,
                                transform.thumbnail.as_deref(),
                                |f| after_tile(f),
                            )
                        } else {
                            after_tile(out)
                        }
                    };
                    let mut apply_loop = |out: *mut AVFrame| -> Result<()> {
                        if transform.r#loop.is_some() {
                            let loop_dst = looped.as_mut().ok_or("loop frame missing")?;
                            push_loop_or_emit(
                                &mut loop_graph,
                                loop_dst.0,
                                out,
                                transform.r#loop.as_deref(),
                                |f| apply_thumbnail(f),
                            )
                        } else {
                            apply_thumbnail(out)
                        }
                    };
                    let mut apply_reverse = |out: *mut AVFrame| -> Result<()> {
                        if transform.reverse.is_some() {
                            let rev_dst = reversed.as_mut().ok_or("reverse frame missing")?;
                            push_reverse_or_emit(
                                &mut reverse_graph,
                                rev_dst.0,
                                out,
                                transform.reverse.as_deref(),
                                |f| apply_loop(f),
                            )
                        } else {
                            apply_loop(out)
                        }
                    };
                    let mut apply_shuffleframes = |out: *mut AVFrame| -> Result<()> {
                        if transform.shuffleframes.is_some() {
                            let sf_dst = shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                            push_shuffleframes_or_emit(
                                &mut shuffleframes_graph,
                                sf_dst.0,
                                out,
                                transform.shuffleframes.as_deref(),
                                |f| apply_reverse(f),
                            )
                        } else {
                            apply_reverse(out)
                        }
                    };
                    let mut apply_untile = |out: *mut AVFrame| -> Result<()> {
                        if transform.untile.is_some() {
                            let u_dst = untiled.as_mut().ok_or("untile frame missing")?;
                            push_untile_or_emit(
                                &mut untile_graph,
                                u_dst.0,
                                out,
                                transform.untile.as_deref(),
                                |f| apply_shuffleframes(f),
                            )
                        } else {
                            apply_shuffleframes(out)
                        }
                    };
                    let mut apply_framestep = |out: *mut AVFrame| -> Result<()> {
                        if transform.framestep.is_some() {
                            let fs_dst = framestepped.as_mut().ok_or("framestep frame missing")?;
                            push_framestep_or_emit(
                                &mut framestep_graph,
                                fs_dst.0,
                                out,
                                transform.framestep.as_deref(),
                                |f| apply_untile(f),
                            )
                        } else {
                            apply_untile(out)
                        }
                    };
                    mpdecimate_push_frame(&mut mpdecimate_graph, dst.0, output, args, |out| {
                        apply_framestep(out)
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.reverse.as_deref() {
                    if transform.shuffleframes.is_none() {
                        let src_fmt = (*output).format;
                        let dst = reversed.as_mut().ok_or("reverse frame missing")?;
                        let mut emitted = 0u64;
                        reverse_push_frame(
                            &mut reverse_graph,
                            dst.0,
                            output,
                            args,
                            |out| unsafe {
                                push_loop_or_emit(
                                    &mut loop_graph,
                                    looped.as_mut().ok_or("loop frame missing")?.0,
                                    out,
                                    transform.r#loop.as_deref(),
                                    |out| unsafe {
                                        push_thumbnail_or_emit(
                                            &mut thumbnail_graph,
                                            thumbnailed
                                                .as_mut()
                                                .ok_or("thumbnail frame missing")?
                                                .0,
                                            out,
                                            transform.thumbnail.as_deref(),
                                            |mut out| {
                                                let mut out = out;
                                                if (*out).format != src_fmt {
                                                    convert_pix_fmt_frame(
                                                        &mut fmt_sws,
                                                        converted.0,
                                                        out,
                                                        src_fmt,
                                                    )?;
                                                    out = converted.0;
                                                }
                                                if transform.minterpolate.is_some()
                                                    || transform.fps.is_some()
                                                {
                                                    let scratch = temporal_scratch
                                                        .as_mut()
                                                        .ok_or("temporal frame missing")?;
                                                    let fps_dst = fps_out
                                                        .as_mut()
                                                        .map(|f| f.0)
                                                        .unwrap_or(scratch.0);
                                                    temporal_push_frame(
                                                        &mut minterpolate_graph,
                                                        scratch.0,
                                                        transform.minterpolate.as_deref(),
                                                        &mut fps_graph,
                                                        fps_dst,
                                                        transform.fps.as_deref(),
                                                        out,
                                                        |_| {
                                                            emitted += 1;
                                                            Ok(())
                                                        },
                                                    )?;
                                                } else {
                                                    emitted += 1;
                                                }
                                                Ok(())
                                            },
                                        )
                                    },
                                )
                            },
                        )?;
                        video_frames += emitted;
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                }
                if let Some(args) = transform.r#loop.as_deref() {
                    if transform.shuffleframes.is_none() && transform.reverse.is_none() {
                        let src_fmt = (*output).format;
                        let dst = looped.as_mut().ok_or("loop frame missing")?;
                        let mut emitted = 0u64;
                        loop_push_frame(&mut loop_graph, dst.0, output, args, |out| unsafe {
                            push_thumbnail_or_emit(
                                &mut thumbnail_graph,
                                thumbnailed.as_mut().ok_or("thumbnail frame missing")?.0,
                                out,
                                transform.thumbnail.as_deref(),
                                |mut out| {
                                    let mut out = out;
                                    if (*out).format != src_fmt {
                                        convert_pix_fmt_frame(
                                            &mut fmt_sws,
                                            converted.0,
                                            out,
                                            src_fmt,
                                        )?;
                                        out = converted.0;
                                    }
                                    if transform.minterpolate.is_some() || transform.fps.is_some() {
                                        let scratch = temporal_scratch
                                            .as_mut()
                                            .ok_or("temporal frame missing")?;
                                        let fps_dst =
                                            fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                                        temporal_push_frame(
                                            &mut minterpolate_graph,
                                            scratch.0,
                                            transform.minterpolate.as_deref(),
                                            &mut fps_graph,
                                            fps_dst,
                                            transform.fps.as_deref(),
                                            out,
                                            |_| {
                                                emitted += 1;
                                                Ok(())
                                            },
                                        )?;
                                    } else {
                                        emitted += 1;
                                    }
                                    Ok(())
                                },
                            )
                        })?;
                        video_frames += emitted;
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                }
                if let Some(args) = transform.thumbnail.as_deref() {
                    if transform.shuffleframes.is_none()
                        && transform.reverse.is_none()
                        && transform.r#loop.is_none()
                    {
                        let src_fmt = (*output).format;
                        let dst = thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
                        let mut emitted = 0u64;
                        thumbnail_push_frame(
                            &mut thumbnail_graph,
                            dst.0,
                            output,
                            args,
                            |mut out| {
                                let mut out = out;
                                if (*out).format != src_fmt {
                                    convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                                    out = converted.0;
                                }
                                if transform.minterpolate.is_some() || transform.fps.is_some() {
                                    let scratch = temporal_scratch
                                        .as_mut()
                                        .ok_or("temporal frame missing")?;
                                    let fps_dst =
                                        fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                                    temporal_push_frame(
                                        &mut minterpolate_graph,
                                        scratch.0,
                                        transform.minterpolate.as_deref(),
                                        &mut fps_graph,
                                        fps_dst,
                                        transform.fps.as_deref(),
                                        out,
                                        |_| {
                                            emitted += 1;
                                            Ok(())
                                        },
                                    )?;
                                } else {
                                    emitted += 1;
                                }
                                Ok(())
                            },
                        )?;
                        video_frames += emitted;
                        av_frame_unref(frame.0);
                        return Ok(false);
                    }
                }
                if let Some(args) = transform.shuffleframes.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = shuffled.as_mut().ok_or("shuffleframes frame missing")?;
                    let mut emitted = 0u64;
                    shuffleframes_push_frame(
                        &mut shuffleframes_graph,
                        dst.0,
                        output,
                        args,
                        |out| unsafe {
                            push_reverse_or_emit(
                                &mut reverse_graph,
                                reversed.as_mut().ok_or("reverse frame missing")?.0,
                                out,
                                transform.reverse.as_deref(),
                                |out| unsafe {
                                    push_loop_or_emit(
                                        &mut loop_graph,
                                        looped.as_mut().ok_or("loop frame missing")?.0,
                                        out,
                                        transform.r#loop.as_deref(),
                                        |out| unsafe {
                                            push_thumbnail_or_emit(
                                                &mut thumbnail_graph,
                                                thumbnailed
                                                    .as_mut()
                                                    .ok_or("thumbnail frame missing")?
                                                    .0,
                                                out,
                                                transform.thumbnail.as_deref(),
                                                |mut out| {
                                                    let mut out = out;
                                                    if (*out).format != src_fmt {
                                                        convert_pix_fmt_frame(
                                                            &mut fmt_sws,
                                                            converted.0,
                                                            out,
                                                            src_fmt,
                                                        )?;
                                                        out = converted.0;
                                                    }
                                                    if transform.minterpolate.is_some()
                                                        || transform.fps.is_some()
                                                    {
                                                        let scratch = temporal_scratch
                                                            .as_mut()
                                                            .ok_or("temporal frame missing")?;
                                                        let fps_dst = fps_out
                                                            .as_mut()
                                                            .map(|f| f.0)
                                                            .unwrap_or(scratch.0);
                                                        temporal_push_frame(
                                                            &mut minterpolate_graph,
                                                            scratch.0,
                                                            transform.minterpolate.as_deref(),
                                                            &mut fps_graph,
                                                            fps_dst,
                                                            transform.fps.as_deref(),
                                                            out,
                                                            |_| {
                                                                emitted += 1;
                                                                Ok(())
                                                            },
                                                        )?;
                                                    } else {
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
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.tile.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = tiled.as_mut().ok_or("tile frame missing")?;
                    let mut emitted = 0u64;
                    tile_push_frame(&mut tile_graph, dst.0, output, args, |out| {
                        push_untile_or_emit(
                            &mut untile_graph,
                            untiled.as_mut().ok_or("untile frame missing")?.0,
                            out,
                            transform.untile.as_deref(),
                            |out| {
                                push_shuffleframes_or_emit(
                                    &mut shuffleframes_graph,
                                    shuffled.as_mut().ok_or("shuffleframes frame missing")?.0,
                                    out,
                                    transform.shuffleframes.as_deref(),
                                    |out| {
                                        push_reverse_or_emit(
                                            &mut reverse_graph,
                                            reversed.as_mut().ok_or("reverse frame missing")?.0,
                                            out,
                                            transform.reverse.as_deref(),
                                            |out| unsafe {
                                                push_loop_or_emit(
                                                    &mut loop_graph,
                                                    looped.as_mut().ok_or("loop frame missing")?.0,
                                                    out,
                                                    transform.r#loop.as_deref(),
                                                    |out| unsafe {
                                                        push_thumbnail_or_emit(
                                                            &mut thumbnail_graph,
                                                            thumbnailed
                                                                .as_mut()
                                                                .ok_or("thumbnail frame missing")?
                                                                .0,
                                                            out,
                                                            transform.thumbnail.as_deref(),
                                                            |mut out| {
                                                                let mut out = out;
                                                                if (*out).format != src_fmt {
                                                                    convert_pix_fmt_frame(
                                                                        &mut fmt_sws,
                                                                        converted.0,
                                                                        out,
                                                                        src_fmt,
                                                                    )?;
                                                                    out = converted.0;
                                                                }
                                                                let out = if let Some(args) =
                                                                    transform
                                                                        .freezedetect
                                                                        .as_deref()
                                                                {
                                                                    let src_fmt = (*out).format;
                                                                    let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                                                                    freezedetect_frame(
                                                                        &mut freezedetect_graph,
                                                                        dst.0,
                                                                        out,
                                                                        args,
                                                                    )?;
                                                                    let o = dst.0;
                                                                    if (*o).format != src_fmt {
                                                                        convert_pix_fmt_frame(
                                                                            &mut fmt_sws,
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
                                                                let out = if let Some(args) =
                                                                    transform.pseudocolor.as_deref()
                                                                {
                                                                    let src_fmt = (*out).format;
                                                                    let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                                                                    pseudocolor_frame(
                                                                        &mut pseudocolor_graph,
                                                                        dst.0,
                                                                        out,
                                                                        args,
                                                                    )?;
                                                                    let o = dst.0;
                                                                    if (*o).format != src_fmt {
                                                                        convert_pix_fmt_frame(
                                                                            &mut fmt_sws,
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
                                                                let out = if let Some(args) =
                                                                    transform.colorspace.as_deref()
                                                                {
                                                                    let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                                                                    colorspace_frame(
                                                                        &mut colorspace_graph,
                                                                        dst.0,
                                                                        out,
                                                                        args,
                                                                    )?;
                                                                    dst.0
                                                                } else {
                                                                    out
                                                                };
                                                                let (out, format_done) =
                                                                    if let Some(args) =
                                                                        transform.zscale.as_deref()
                                                                    {
                                                                        let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                                                                        let name = string(
                                                                            av_get_pix_fmt_name(
                                                                                fmt,
                                                                            ),
                                                                        );
                                                                        if name.is_empty() {
                                                                            return Err("unknown zscale output pixel format".into());
                                                                        }
                                                                        let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                                                                        zscale_frame(
                                                                            &mut zscale_graph,
                                                                            dst.0,
                                                                            out,
                                                                            args,
                                                                            &name,
                                                                        )?;
                                                                        (dst.0, true)
                                                                    } else {
                                                                        (out, false)
                                                                    };
                                                                let (out, format_done) =
                                                                    if let Some(args) =
                                                                        transform.tonemap.as_deref()
                                                                    {
                                                                        let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                                                                        let name = string(
                                                                            av_get_pix_fmt_name(
                                                                                fmt,
                                                                            ),
                                                                        );
                                                                        if name.is_empty() {
                                                                            return Err("unknown tonemap output pixel format".into());
                                                                        }
                                                                        let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                                                                        tonemap_frame(
                                                                            &mut tonemap_graph,
                                                                            dst.0,
                                                                            out,
                                                                            args,
                                                                            &name,
                                                                        )?;
                                                                        (dst.0, format_done)
                                                                    } else {
                                                                        (out, format_done)
                                                                    };
                                                                let out = if let Some(fmt) =
                                                                    target_pix_fmt
                                                                {
                                                                    if !format_done
                                                                        && (*out).format != fmt
                                                                    {
                                                                        convert_pix_fmt_frame(
                                                                            &mut fmt_sws,
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
                                                                if transform.minterpolate.is_some()
                                                                    || transform.fps.is_some()
                                                                {
                                                                    let scratch =
                                temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                                                                    let fps_dst = fps_out
                                                                        .as_mut()
                                                                        .map(|f| f.0)
                                                                        .unwrap_or(scratch.0);
                                                                    temporal_push_frame(
                                                                        &mut minterpolate_graph,
                                                                        scratch.0,
                                                                        transform
                                                                            .minterpolate
                                                                            .as_deref(),
                                                                        &mut fps_graph,
                                                                        fps_dst,
                                                                        transform.fps.as_deref(),
                                                                        out,
                                                                        |_| {
                                                                            emitted += 1;
                                                                            Ok(())
                                                                        },
                                                                    )?;
                                                                } else {
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
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.untile.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = untiled.as_mut().ok_or("untile frame missing")?;
                    let mut emitted = 0u64;
                    untile_push_frame(&mut untile_graph, dst.0, output, args, |out| unsafe {
                        push_shuffleframes_or_emit(
                            &mut shuffleframes_graph,
                            shuffled.as_mut().ok_or("shuffleframes frame missing")?.0,
                            out,
                            transform.shuffleframes.as_deref(),
                            |out| {
                                push_reverse_or_emit(
                                    &mut reverse_graph,
                                    reversed.as_mut().ok_or("reverse frame missing")?.0,
                                    out,
                                    transform.reverse.as_deref(),
                                    |out| unsafe {
                                        push_loop_or_emit(
                                            &mut loop_graph,
                                            looped.as_mut().ok_or("loop frame missing")?.0,
                                            out,
                                            transform.r#loop.as_deref(),
                                            |out| unsafe {
                                                push_thumbnail_or_emit(
                                                    &mut thumbnail_graph,
                                                    thumbnailed
                                                        .as_mut()
                                                        .ok_or("thumbnail frame missing")?
                                                        .0,
                                                    out,
                                                    transform.thumbnail.as_deref(),
                                                    |mut out| {
                                                        let mut out = out;
                                                        if (*out).format != src_fmt {
                                                            convert_pix_fmt_frame(
                                                                &mut fmt_sws,
                                                                converted.0,
                                                                out,
                                                                src_fmt,
                                                            )?;
                                                            out = converted.0;
                                                        }
                                                        let out = if let Some(args) =
                                                            transform.freezedetect.as_deref()
                                                        {
                                                            let src_fmt = (*out).format;
                                                            let dst =
                                                                freezedetectd.as_mut().ok_or(
                                                                    "freezedetect frame missing",
                                                                )?;
                                                            freezedetect_frame(
                                                                &mut freezedetect_graph,
                                                                dst.0,
                                                                out,
                                                                args,
                                                            )?;
                                                            let o = dst.0;
                                                            if (*o).format != src_fmt {
                                                                convert_pix_fmt_frame(
                                                                    &mut fmt_sws,
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
                                                        let out = if let Some(args) =
                                                            transform.pseudocolor.as_deref()
                                                        {
                                                            let src_fmt = (*out).format;
                                                            let dst =
                                                                pseudocolored.as_mut().ok_or(
                                                                    "pseudocolor frame missing",
                                                                )?;
                                                            pseudocolor_frame(
                                                                &mut pseudocolor_graph,
                                                                dst.0,
                                                                out,
                                                                args,
                                                            )?;
                                                            let o = dst.0;
                                                            if (*o).format != src_fmt {
                                                                convert_pix_fmt_frame(
                                                                    &mut fmt_sws,
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
                                                        let out = if let Some(args) =
                                                            transform.colorspace.as_deref()
                                                        {
                                                            let dst = colorspaced.as_mut().ok_or(
                                                                "colorspace frame missing",
                                                            )?;
                                                            colorspace_frame(
                                                                &mut colorspace_graph,
                                                                dst.0,
                                                                out,
                                                                args,
                                                            )?;
                                                            dst.0
                                                        } else {
                                                            out
                                                        };
                                                        let (out, format_done) = if let Some(args) =
                                                            transform.zscale.as_deref()
                                                        {
                                                            let fmt = target_pix_fmt.ok_or(
                                                                "--zscale requires --pix-fmt",
                                                            )?;
                                                            let name =
                                                                string(av_get_pix_fmt_name(fmt));
                                                            if name.is_empty() {
                                                                return Err("unknown zscale output pixel format".into());
                                                            }
                                                            let dst = zscaled
                                                                .as_mut()
                                                                .ok_or("zscale frame missing")?;
                                                            zscale_frame(
                                                                &mut zscale_graph,
                                                                dst.0,
                                                                out,
                                                                args,
                                                                &name,
                                                            )?;
                                                            (dst.0, true)
                                                        } else {
                                                            (out, false)
                                                        };
                                                        let (out, format_done) = if let Some(args) =
                                                            transform.tonemap.as_deref()
                                                        {
                                                            let fmt = target_pix_fmt.ok_or(
                                                                "--tonemap requires --pix-fmt",
                                                            )?;
                                                            let name =
                                                                string(av_get_pix_fmt_name(fmt));
                                                            if name.is_empty() {
                                                                return Err("unknown tonemap output pixel format".into());
                                                            }
                                                            let dst = tonemapped
                                                                .as_mut()
                                                                .ok_or("tonemap frame missing")?;
                                                            tonemap_frame(
                                                                &mut tonemap_graph,
                                                                dst.0,
                                                                out,
                                                                args,
                                                                &name,
                                                            )?;
                                                            (dst.0, format_done)
                                                        } else {
                                                            (out, format_done)
                                                        };
                                                        let out = if let Some(fmt) = target_pix_fmt
                                                        {
                                                            if !format_done && (*out).format != fmt
                                                            {
                                                                convert_pix_fmt_frame(
                                                                    &mut fmt_sws,
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
                                                        if transform.minterpolate.is_some()
                                                            || transform.fps.is_some()
                                                        {
                                                            let scratch = temporal_scratch
                                                                .as_mut()
                                                                .ok_or("temporal frame missing")?;
                                                            let fps_dst = fps_out
                                                                .as_mut()
                                                                .map(|f| f.0)
                                                                .unwrap_or(scratch.0);
                                                            temporal_push_frame(
                                                                &mut minterpolate_graph,
                                                                scratch.0,
                                                                transform.minterpolate.as_deref(),
                                                                &mut fps_graph,
                                                                fps_dst,
                                                                transform.fps.as_deref(),
                                                                out,
                                                                |_| {
                                                                    emitted += 1;
                                                                    Ok(())
                                                                },
                                                            )?;
                                                        } else {
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
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                if let Some(args) = transform.framestep.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = framestepped.as_mut().ok_or("framestep frame missing")?;
                    let mut emitted = 0u64;
                    framestep_push_frame(&mut framestep_graph, dst.0, output, args, |out| {
                        push_tile_or_emit(
                            &mut tile_graph,
                            tiled.as_mut().ok_or("tile frame missing")?.0,
                            out,
                            transform.tile.as_deref(),
                            |out| {
                                push_untile_or_emit(
                                    &mut untile_graph,
                                    untiled.as_mut().ok_or("untile frame missing")?.0,
                                    out,
                                    transform.untile.as_deref(),
                                    |out| {
                                        push_shuffleframes_or_emit(
                                            &mut shuffleframes_graph,
                                            shuffled
                                                .as_mut()
                                                .ok_or("shuffleframes frame missing")?
                                                .0,
                                            out,
                                            transform.shuffleframes.as_deref(),
                                            |out| {
                                                push_reverse_or_emit(
                                                    &mut reverse_graph,
                                                    reversed
                                                        .as_mut()
                                                        .ok_or("reverse frame missing")?
                                                        .0,
                                                    out,
                                                    transform.reverse.as_deref(),
                                                    |out| unsafe {
                                                        push_loop_or_emit(
                                                            &mut loop_graph,
                                                            looped
                                                                .as_mut()
                                                                .ok_or("loop frame missing")?
                                                                .0,
                                                            out,
                                                            transform.r#loop.as_deref(),
                                                            |out| unsafe {
                                                                push_thumbnail_or_emit(
                                                            &mut thumbnail_graph,
                                                            thumbnailed.as_mut().ok_or("thumbnail frame missing")?.0,
                                                            out,
                                                            transform.thumbnail.as_deref(),
                                                            |mut out| {
                        let mut out = out;
                        if (*out).format != src_fmt {
                            convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                            out = converted.0;
                        }
                        let out = if let Some(args) = transform.freezedetect.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                            freezedetect_frame(&mut freezedetect_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.pseudocolor.as_deref() {
                            let src_fmt = (*out).format;
                            let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                            pseudocolor_frame(&mut pseudocolor_graph, dst.0, out, args)?;
                            let o = dst.0;
                            if (*o).format != src_fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, o, src_fmt)?;
                                converted.0
                            } else {
                                o
                            }
                        } else {
                            out
                        };
                        let out = if let Some(args) = transform.colorspace.as_deref() {
                            let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                            colorspace_frame(&mut colorspace_graph, dst.0, out, args)?;
                            dst.0
                        } else {
                            out
                        };
                        let (out, format_done) = if let Some(args) = transform.zscale.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown zscale output pixel format".into());
                            }
                            let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                            zscale_frame(&mut zscale_graph, dst.0, out, args, &name)?;
                            (dst.0, true)
                        } else {
                            (out, false)
                        };
                        let (out, format_done) = if let Some(args) = transform.tonemap.as_deref() {
                            let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                            let name = string(av_get_pix_fmt_name(fmt));
                            if name.is_empty() {
                                return Err("unknown tonemap output pixel format".into());
                            }
                            let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                            tonemap_frame(&mut tonemap_graph, dst.0, out, args, &name)?;
                            (dst.0, format_done)
                        } else {
                            (out, format_done)
                        };
                        let out = if let Some(fmt) = target_pix_fmt {
                            if !format_done && (*out).format != fmt {
                                convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, fmt)?;
                                converted.0
                            } else {
                                out
                            }
                        } else {
                            out
                        };
                        if transform.minterpolate.is_some() || transform.fps.is_some() {
                            let scratch =
                                temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                            let fps_dst = fps_out
                                .as_mut()
                                .map(|f| f.0)
                                .unwrap_or(scratch.0);
                            temporal_push_frame(
                                &mut minterpolate_graph,
                                scratch.0,
                                transform.minterpolate.as_deref(),
                                &mut fps_graph,
                                fps_dst,
                                transform.fps.as_deref(),
                                out,
                                |_| {
                                    emitted += 1;
                                    Ok(())
                                },
                            )?;
                        } else {
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
                            },
                        )
                    })?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
                let output = if let Some(args) = transform.freezedetect.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = freezedetectd.as_mut().ok_or("freezedetect frame missing")?;
                    freezedetect_frame(&mut freezedetect_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.pseudocolor.as_deref() {
                    let src_fmt = (*output).format;
                    let dst = pseudocolored.as_mut().ok_or("pseudocolor frame missing")?;
                    pseudocolor_frame(&mut pseudocolor_graph, dst.0, output, args)?;
                    let out = dst.0;
                    if (*out).format != src_fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, out, src_fmt)?;
                        converted.0
                    } else {
                        out
                    }
                } else {
                    output
                };
                let output = if let Some(args) = transform.colorspace.as_deref() {
                    let dst = colorspaced.as_mut().ok_or("colorspace frame missing")?;
                    colorspace_frame(&mut colorspace_graph, dst.0, output, args)?;
                    dst.0
                } else {
                    output
                };
                let (output, format_done) = if let Some(args) = transform.zscale.as_deref() {
                    let fmt = target_pix_fmt.ok_or("--zscale requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown zscale output pixel format".into());
                    }
                    let dst = zscaled.as_mut().ok_or("zscale frame missing")?;
                    zscale_frame(&mut zscale_graph, dst.0, output, args, &name)?;
                    (dst.0, true)
                } else {
                    (output, format_done)
                };
                let (output, format_done) = if let Some(args) = transform.tonemap.as_deref() {
                    let fmt = target_pix_fmt.ok_or("--tonemap requires --pix-fmt")?;
                    let name = string(av_get_pix_fmt_name(fmt));
                    if name.is_empty() {
                        return Err("unknown tonemap output pixel format".into());
                    }
                    let dst = tonemapped.as_mut().ok_or("tonemap frame missing")?;
                    tonemap_frame(&mut tonemap_graph, dst.0, output, args, &name)?;
                    (dst.0, true)
                } else {
                    (output, format_done)
                };
                let output = if let Some(fmt) = target_pix_fmt {
                    if !format_done && (*output).format != fmt {
                        convert_pix_fmt_frame(&mut fmt_sws, converted.0, output, fmt)?;
                        converted.0
                    } else {
                        output
                    }
                } else {
                    output
                };
                if let Some(fmt) = target_pix_fmt {
                    pixel_format = string(av_get_pix_fmt_name(fmt));
                }
                if transform.minterpolate.is_some() || transform.fps.is_some() {
                    let scratch = temporal_scratch.as_mut().ok_or("temporal frame missing")?;
                    let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
                    let mut emitted = 0u64;
                    temporal_push_frame(
                        &mut minterpolate_graph,
                        scratch.0,
                        transform.minterpolate.as_deref(),
                        &mut fps_graph,
                        fps_dst,
                        transform.fps.as_deref(),
                        output,
                        |_| {
                            emitted += 1;
                            Ok(())
                        },
                    )?;
                    video_frames += emitted;
                    av_frame_unref(frame.0);
                    return Ok(false);
                }
            }
            video_frames += 1;
            unsafe { av_frame_unref(frame.0) };
            Ok(false)
        };
        'packets: while packet.read(&mut input)? {
            if unsafe { (*packet.0).stream_index } != video as i32 {
                continue;
            }
            if no_reorder
                && let Some((_, end)) = interval
                && unsafe { (*packet.0).pts } != NOPTS
                && unsafe { (*packet.0).pts } >= end
            {
                finished = true;
                break;
            }
            check(
                unsafe { avcodec_send_packet(decoder.0, packet.0) },
                "send compressed video packet",
            )?;
            loop {
                let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
                if code == AGAIN || code == EOF {
                    break;
                }
                check(code, "receive decoded frame")?;
                if handle(&mut frame)? {
                    finished = true;
                    break 'packets;
                }
            }
        }
        if !finished {
            check(
                unsafe { avcodec_send_packet(decoder.0, ptr::null_mut()) },
                "flush decoder",
            )?;
            loop {
                let code = unsafe { avcodec_receive_frame(decoder.0, frame.0) };
                if code == AGAIN || code == EOF {
                    break;
                }
                check(code, "flush receive")?;
                if handle(&mut frame)? {
                    break;
                }
            }
        }
    } // drop handle so temporal filter graphs can be flushed
    if transform.yadif.is_some() {
        let dst = deinterlaced.as_mut().ok_or("yadif frame missing")?;
        unsafe {
            yadif_flush(&mut yadif_graph, dst.0, |mut send| {
                if let Some(args) = transform.bwdif.as_deref() {
                    let bwdif_dst = bwdif_out.as_mut().ok_or("bwdif frame missing")?;
                    let produced = bwdif_push_frame(&mut bwdif_graph, bwdif_dst.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                    send = bwdif_dst.0;
                }
                if let Some(args) = transform.w3fdif.as_deref() {
                    let w3fdif_dst = w3fdif_out.as_mut().ok_or("w3fdif frame missing")?;
                    let produced = w3fdif_push_frame(&mut w3fdif_graph, w3fdif_dst.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                }
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.bwdif.is_some() {
        let dst = bwdif_out.as_mut().ok_or("bwdif frame missing")?;
        unsafe {
            bwdif_flush(&mut bwdif_graph, dst.0, |send| {
                if let Some(args) = transform.w3fdif.as_deref() {
                    let w3fdif_dst = w3fdif_out.as_mut().ok_or("w3fdif frame missing")?;
                    let produced = w3fdif_push_frame(&mut w3fdif_graph, w3fdif_dst.0, send, args)?;
                    if !produced {
                        return Ok(());
                    }
                }
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.w3fdif.is_some() {
        let dst = w3fdif_out.as_mut().ok_or("w3fdif frame missing")?;
        unsafe {
            w3fdif_flush(&mut w3fdif_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.atadenoise.is_some() {
        let dst = atdenoised.as_mut().ok_or("atadenoise frame missing")?;
        unsafe {
            atadenoise_flush(&mut atadenoise_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.deflicker.is_some() {
        let dst = deflickered.as_mut().ok_or("deflicker frame missing")?;
        unsafe {
            deflicker_flush(&mut deflicker_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.tmix.is_some() {
        let dst = tmixed.as_mut().ok_or("tmix frame missing")?;
        unsafe {
            tmix_flush(&mut tmix_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.lagfun.is_some() {
        let dst = lagfuned.as_mut().ok_or("lagfun frame missing")?;
        unsafe {
            lagfun_flush(&mut lagfun_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.fade.is_some() && fade_push_mode {
        let dst = faded.as_mut().ok_or("fade frame missing")?;
        unsafe {
            fade_flush(&mut fade_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.phase.is_some() && phase_push_mode {
        let dst = phased.as_mut().ok_or("phase frame missing")?;
        unsafe {
            phase_flush(&mut phase_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.estdif.is_some() {
        let dst = estdifd.as_mut().ok_or("estdif frame missing")?;
        unsafe {
            estdif_flush(&mut estdif_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.tinterlace.is_some() {
        let dst = tinterlaced.as_mut().ok_or("tinterlace frame missing")?;
        unsafe {
            tinterlace_flush(&mut tinterlace_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.separatefields.is_some() {
        let dst = separatefieldsd
            .as_mut()
            .ok_or("separatefields frame missing")?;
        unsafe {
            separatefields_flush(&mut separatefields_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.weave.is_some() {
        let dst = weaved.as_mut().ok_or("weave frame missing")?;
        unsafe {
            weave_flush(&mut weave_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.doubleweave.is_some() {
        let dst = doubleweaved.as_mut().ok_or("doubleweave frame missing")?;
        unsafe {
            doubleweave_flush(&mut doubleweave_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.framepack.is_some() {
        let dst = framepacked.as_mut().ok_or("framepack frame missing")?;
        unsafe {
            framepack_flush(&mut framepack_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.telecine.is_some() {
        let dst = telecined.as_mut().ok_or("telecine frame missing")?;
        unsafe {
            telecine_flush(&mut telecine_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.pullup.is_some() {
        let dst = pulledup.as_mut().ok_or("pullup frame missing")?;
        unsafe {
            pullup_flush(&mut pullup_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.decimate.is_some() {
        let dst = decimated.as_mut().ok_or("decimate frame missing")?;
        unsafe {
            decimate_flush(&mut decimate_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.mpdecimate.is_some() {
        let dst = mpdecimated.as_mut().ok_or("mpdecimate frame missing")?;
        unsafe {
            mpdecimate_flush(&mut mpdecimate_graph, dst.0, |out| {
                if transform.framestep.is_some() {
                    let fs_dst = framestepped.as_mut().ok_or("framestep frame missing")?;
                    push_framestep_or_emit(
                        &mut framestep_graph,
                        fs_dst.0,
                        out,
                        transform.framestep.as_deref(),
                        |out| {
                            push_tile_or_emit(
                                &mut tile_graph,
                                tiled.as_mut().ok_or("tile frame missing")?.0,
                                out,
                                transform.tile.as_deref(),
                                |out| {
                                    push_untile_or_emit(
                                        &mut untile_graph,
                                        untiled.as_mut().ok_or("untile frame missing")?.0,
                                        out,
                                        transform.untile.as_deref(),
                                        |out| {
                                            push_shuffleframes_or_emit(
                                                &mut shuffleframes_graph,
                                                shuffled
                                                    .as_mut()
                                                    .ok_or("shuffleframes frame missing")?
                                                    .0,
                                                out,
                                                transform.shuffleframes.as_deref(),
                                                |_| {
                                                    video_frames += 1;
                                                    Ok(())
                                                },
                                            )
                                        },
                                    )
                                },
                            )
                        },
                    )
                } else {
                    push_tile_or_emit(
                        &mut tile_graph,
                        tiled.as_mut().ok_or("tile frame missing")?.0,
                        out,
                        transform.tile.as_deref(),
                        |out| {
                            push_untile_or_emit(
                                &mut untile_graph,
                                untiled.as_mut().ok_or("untile frame missing")?.0,
                                out,
                                transform.untile.as_deref(),
                                |out| {
                                    push_shuffleframes_or_emit(
                                        &mut shuffleframes_graph,
                                        shuffled.as_mut().ok_or("shuffleframes frame missing")?.0,
                                        out,
                                        transform.shuffleframes.as_deref(),
                                        |out| {
                                            push_reverse_or_emit(
                                                &mut reverse_graph,
                                                reversed.as_mut().ok_or("reverse frame missing")?.0,
                                                out,
                                                transform.reverse.as_deref(),
                                                |out| unsafe {
                                                    push_loop_or_emit(
                                                        &mut loop_graph,
                                                        looped
                                                            .as_mut()
                                                            .ok_or("loop frame missing")?
                                                            .0,
                                                        out,
                                                        transform.r#loop.as_deref(),
                                                        |out| unsafe {
                                                            push_thumbnail_or_emit(
                                                                &mut thumbnail_graph,
                                                                thumbnailed
                                                                    .as_mut()
                                                                    .ok_or(
                                                                        "thumbnail frame missing",
                                                                    )?
                                                                    .0,
                                                                out,
                                                                transform.thumbnail.as_deref(),
                                                                |_| {
                                                                    video_frames += 1;
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
                        },
                    )
                }
            })?;
        }
    }
    if transform.framestep.is_some() && transform.mpdecimate.is_none() {
        let dst = framestepped.as_mut().ok_or("framestep frame missing")?;
        unsafe {
            framestep_flush(&mut framestep_graph, dst.0, |out| {
                push_tile_or_emit(
                    &mut tile_graph,
                    tiled.as_mut().ok_or("tile frame missing")?.0,
                    out,
                    transform.tile.as_deref(),
                    |out| {
                        push_untile_or_emit(
                            &mut untile_graph,
                            untiled.as_mut().ok_or("untile frame missing")?.0,
                            out,
                            transform.untile.as_deref(),
                            |out| {
                                push_shuffleframes_or_emit(
                                    &mut shuffleframes_graph,
                                    shuffled.as_mut().ok_or("shuffleframes frame missing")?.0,
                                    out,
                                    transform.shuffleframes.as_deref(),
                                    |out| {
                                        push_reverse_or_emit(
                                            &mut reverse_graph,
                                            reversed.as_mut().ok_or("reverse frame missing")?.0,
                                            out,
                                            transform.reverse.as_deref(),
                                            |out| unsafe {
                                                push_loop_or_emit(
                                                    &mut loop_graph,
                                                    looped.as_mut().ok_or("loop frame missing")?.0,
                                                    out,
                                                    transform.r#loop.as_deref(),
                                                    |out| unsafe {
                                                        push_thumbnail_or_emit(
                                                            &mut thumbnail_graph,
                                                            thumbnailed
                                                                .as_mut()
                                                                .ok_or("thumbnail frame missing")?
                                                                .0,
                                                            out,
                                                            transform.thumbnail.as_deref(),
                                                            |_| {
                                                                video_frames += 1;
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
                    },
                )
            })?;
        }
    }
    if transform.tile.is_some() && transform.framestep.is_none() && transform.mpdecimate.is_none() {
        let dst = tiled.as_mut().ok_or("tile frame missing")?;
        unsafe {
            tile_flush(&mut tile_graph, dst.0, |out| {
                push_untile_or_emit(
                    &mut untile_graph,
                    untiled.as_mut().ok_or("untile frame missing")?.0,
                    out,
                    transform.untile.as_deref(),
                    |out| {
                        push_shuffleframes_or_emit(
                            &mut shuffleframes_graph,
                            shuffled.as_mut().ok_or("shuffleframes frame missing")?.0,
                            out,
                            transform.shuffleframes.as_deref(),
                            |out| {
                                push_reverse_or_emit(
                                    &mut reverse_graph,
                                    reversed.as_mut().ok_or("reverse frame missing")?.0,
                                    out,
                                    transform.reverse.as_deref(),
                                    |out| unsafe {
                                        push_loop_or_emit(
                                            &mut loop_graph,
                                            looped.as_mut().ok_or("loop frame missing")?.0,
                                            out,
                                            transform.r#loop.as_deref(),
                                            |out| unsafe {
                                                push_thumbnail_or_emit(
                                                    &mut thumbnail_graph,
                                                    thumbnailed
                                                        .as_mut()
                                                        .ok_or("thumbnail frame missing")?
                                                        .0,
                                                    out,
                                                    transform.thumbnail.as_deref(),
                                                    |_| {
                                                        video_frames += 1;
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
        }
    }
    if transform.untile.is_some()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.mpdecimate.is_none()
    {
        let dst = untiled.as_mut().ok_or("untile frame missing")?;
        unsafe {
            untile_flush(&mut untile_graph, dst.0, |out| {
                push_shuffleframes_or_emit(
                    &mut shuffleframes_graph,
                    shuffled.as_mut().ok_or("shuffleframes frame missing")?.0,
                    out,
                    transform.shuffleframes.as_deref(),
                    |out| {
                        push_reverse_or_emit(
                            &mut reverse_graph,
                            reversed.as_mut().ok_or("reverse frame missing")?.0,
                            out,
                            transform.reverse.as_deref(),
                            |out| unsafe {
                                push_loop_or_emit(
                                    &mut loop_graph,
                                    looped.as_mut().ok_or("loop frame missing")?.0,
                                    out,
                                    transform.r#loop.as_deref(),
                                    |out| unsafe {
                                        push_thumbnail_or_emit(
                                            &mut thumbnail_graph,
                                            thumbnailed
                                                .as_mut()
                                                .ok_or("thumbnail frame missing")?
                                                .0,
                                            out,
                                            transform.thumbnail.as_deref(),
                                            |_| {
                                                video_frames += 1;
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
        }
    }
    if transform.shuffleframes.is_some()
        && transform.untile.is_none()
        && transform.tile.is_none()
        && transform.framestep.is_none()
        && transform.mpdecimate.is_none()
    {
        let dst = shuffled.as_mut().ok_or("shuffleframes frame missing")?;
        unsafe {
            shuffleframes_flush(&mut shuffleframes_graph, dst.0, |out| {
                push_reverse_or_emit(
                    &mut reverse_graph,
                    reversed.as_mut().ok_or("reverse frame missing")?.0,
                    out,
                    transform.reverse.as_deref(),
                    |out| unsafe {
                        push_loop_or_emit(
                            &mut loop_graph,
                            looped.as_mut().ok_or("loop frame missing")?.0,
                            out,
                            transform.r#loop.as_deref(),
                            |out| unsafe {
                                push_thumbnail_or_emit(
                                    &mut thumbnail_graph,
                                    thumbnailed.as_mut().ok_or("thumbnail frame missing")?.0,
                                    out,
                                    transform.thumbnail.as_deref(),
                                    |_| {
                                        video_frames += 1;
                                        Ok(())
                                    },
                                )
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
        && transform.mpdecimate.is_none()
    {
        let dst = reversed.as_mut().ok_or("reverse frame missing")?;
        unsafe {
            reverse_flush(&mut reverse_graph, dst.0, |out| unsafe {
                push_loop_or_emit(
                    &mut loop_graph,
                    looped.as_mut().ok_or("loop frame missing")?.0,
                    out,
                    transform.r#loop.as_deref(),
                    |out| unsafe {
                        push_thumbnail_or_emit(
                            &mut thumbnail_graph,
                            thumbnailed.as_mut().ok_or("thumbnail frame missing")?.0,
                            out,
                            transform.thumbnail.as_deref(),
                            |_| {
                                video_frames += 1;
                                Ok(())
                            },
                        )
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
        && transform.mpdecimate.is_none()
    {
        let dst = looped.as_mut().ok_or("loop frame missing")?;
        unsafe {
            loop_flush(&mut loop_graph, dst.0, |out| unsafe {
                push_thumbnail_or_emit(
                    &mut thumbnail_graph,
                    thumbnailed.as_mut().ok_or("thumbnail frame missing")?.0,
                    out,
                    transform.thumbnail.as_deref(),
                    |_| {
                        video_frames += 1;
                        Ok(())
                    },
                )
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
        && transform.mpdecimate.is_none()
    {
        let dst = thumbnailed.as_mut().ok_or("thumbnail frame missing")?;
        unsafe {
            thumbnail_flush(&mut thumbnail_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.amplify.is_some() {
        let dst = amplified.as_mut().ok_or("amplify frame missing")?;
        unsafe {
            amplify_flush(&mut amplify_graph, dst.0, |_| {
                video_frames += 1;
                Ok(())
            })?;
        }
    }
    if transform.minterpolate.is_some() || transform.fps.is_some() {
        let scratch = temporal_scratch.as_mut().ok_or("temporal frame missing")?;
        let fps_dst = fps_out.as_mut().map(|f| f.0).unwrap_or(scratch.0);
        unsafe {
            temporal_flush_frames(
                &mut minterpolate_graph,
                scratch.0,
                transform.minterpolate.as_deref(),
                &mut fps_graph,
                fps_dst,
                transform.fps.as_deref(),
                |_| {
                    video_frames += 1;
                    Ok(())
                },
            )?;
        }
    }
    Ok(DecodeStats {
        backend: "native libavcodec decode",
        video_frames,
        width: out_w,
        height: out_h,
        pixel_format,
    })
}

#[cfg(feature = "cuda-hw")]
pub fn decode_video_cuda(source: &Path, device: usize) -> Result<DecodeStats> {
    let (video_frames, width, height) = crate::hw_cuda::hw_decode_only(source, device)?;
    Ok(DecodeStats {
        backend: "cuda-nvdec",
        video_frames,
        width,
        height,
        pixel_format: "cuda/nv12".into(),
    })
}
