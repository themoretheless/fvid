//! Dry-run job plans: copy/reencode/materialization before execution.
use super::*;
use serde::Serialize;

pub use fvid_media_info::{MediaPlan, PlanStep, PlanStream};

fn stream_plans(input: &Input, selected: &[usize]) -> Result<Vec<PlanStream>> {
    let mut out = Vec::with_capacity(selected.len());
    for &index in selected {
        // SAFETY: selection indices are in-range; codecpar owned by Input.
        unsafe {
            let s = &*input.streams()[index];
            let p = &*s.codecpar;
            let disposition = "copy";
            out.push(PlanStream {
                index,
                media_type: string(av_get_media_type_string(p.codec_type)),
                codec: string(avcodec_get_name(p.codec_id)),
                disposition: disposition.into(),
            });
        }
    }
    Ok(out)
}

/// Build the FFmpeg-equivalent `-vf` chain for a lossless transform.
fn video_graph(transform: &LosslessTransform) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(crop) = transform.crop {
        parts.push(format!(
            "crop={}:{}:{}:{}:exact=1",
            crop.width, crop.height, crop.x, crop.y
        ));
    }
    if transform.horizontal_flip {
        parts.push("hflip".into());
    }
    if transform.vertical_flip {
        parts.push("vflip".into());
    }
    if let Some(mode) = transform.transpose {
        parts.push(format!("transpose={}", mode.as_str()));
    }
    if let Some(angle) = transform.rotate {
        // ow/oh depend on prior geometry; plan emits the angle form and notes
        // that executed paths pin numeric rotw/roth sizes.
        parts.push(format!("rotate=a={}*PI/180:c=black", angle.degrees));
    }
    if let Some(pad) = transform.pad {
        parts.push(format!(
            "pad={}:{}:{}:{}:black",
            pad.width, pad.height, pad.x, pad.y
        ));
    }
    let scale = transform.scale;
    let fmt = transform.pix_fmt.as_deref();
    let burn = transform.burn_subs.is_some();
    let overlay = transform.overlay.as_ref();
    let yadif = transform.yadif.as_deref();
    let bwdif = transform.bwdif.as_deref();
    let w3fdif = transform.w3fdif.as_deref();
    let tblend = transform.tblend.as_deref();
    let tmix = transform.tmix.as_deref();
    let hqdn3d = transform.hqdn3d.as_deref();
    let gblur = transform.gblur.as_deref();
    let eq = transform.eq.as_deref();
    let unsharp = transform.unsharp.as_deref();
    let hue = transform.hue.as_deref();
    let avgblur = transform.avgblur.as_deref();
    let boxblur = transform.boxblur.as_deref();
    let negate = transform.negate.as_deref();
    let edgedetect = transform.edgedetect.as_deref();
    let sobel = transform.sobel.as_deref();
    let prewitt = transform.prewitt.as_deref();
    let roberts = transform.roberts.as_deref();
    let kirsch = transform.kirsch.as_deref();
    let scharr = transform.scharr.as_deref();
    let atadenoise = transform.atadenoise.as_deref();
    let owdenoise = transform.owdenoise.as_deref();
    let vaguedenoiser = transform.vaguedenoiser.as_deref();
    let nlmeans = transform.nlmeans.as_deref();
    let bm3d = transform.bm3d.as_deref();
    let dctdnoiz = transform.dctdnoiz.as_deref();
    let fftdnoiz = transform.fftdnoiz.as_deref();
    let smartblur = transform.smartblur.as_deref();
    let sab = transform.sab.as_deref();
    let bilateral = transform.bilateral.as_deref();
    let cas = transform.cas.as_deref();
    let epx = transform.epx.as_deref();
    let vignette = transform.vignette.as_deref();
    let curves = transform.curves.as_deref();
    let colorbalance = transform.colorbalance.as_deref();
    let colorlevels = transform.colorlevels.as_deref();
    let colorchannelmixer = transform.colorchannelmixer.as_deref();
    let deflicker = transform.deflicker.as_deref();
    let photosensitivity = transform.photosensitivity.as_deref();
    let monochrome = transform.monochrome.as_deref();
    let grayworld = transform.grayworld.as_deref();
    let drawbox = transform.drawbox.as_deref();
    let drawgrid = transform.drawgrid.as_deref();
    let lagfun = transform.lagfun.as_deref();
    let amplify = transform.amplify.as_deref();
    let bitplanenoise = transform.bitplanenoise.as_deref();
    let deband = transform.deband.as_deref();
    let gradfun = transform.gradfun.as_deref();
    let lenscorrection = transform.lenscorrection.as_deref();
    let pixelize = transform.pixelize.as_deref();
    let removegrain = transform.removegrain.as_deref();
    let yaepblur = transform.yaepblur.as_deref();
    let vibrance = transform.vibrance.as_deref();
    let dilation = transform.dilation.as_deref();
    let erosion = transform.erosion.as_deref();
    let colorize = transform.colorize.as_deref();
    let exposure = transform.exposure.as_deref();
    let chromashift = transform.chromashift.as_deref();
    let colorcontrast = transform.colorcontrast.as_deref();
    let colorcorrect = transform.colorcorrect.as_deref();
    let histeq = transform.histeq.as_deref();
    let shuffleplanes = transform.shuffleplanes.as_deref();
    let lutyuv = transform.lutyuv.as_deref();
    let colorhold = transform.colorhold.as_deref();
    let fade = transform.fade.as_deref();
    let perspective = transform.perspective.as_deref();
    let lumakey = transform.lumakey.as_deref();
    let chromakey = transform.chromakey.as_deref();
    let colorkey = transform.colorkey.as_deref();
    let despill = transform.despill.as_deref();
    let selectivecolor = transform.selectivecolor.as_deref();
    let stereo3d = transform.stereo3d.as_deref();
    let field = transform.field.as_deref();
    let hqx = transform.hqx.as_deref();
    let xbr = transform.xbr.as_deref();
    let il = transform.il.as_deref();
    let super2xsai = transform.super2xsai.as_deref();
    let kerndeint = transform.kerndeint.as_deref();
    let phase = transform.phase.as_deref();
    let estdif = transform.estdif.as_deref();
    let tinterlace = transform.tinterlace.as_deref();
    let separatefields = transform.separatefields.as_deref();
    let weave = transform.weave.as_deref();
    let doubleweave = transform.doubleweave.as_deref();
    let framepack = transform.framepack.as_deref();
    let telecine = transform.telecine.as_deref();
    let pullup = transform.pullup.as_deref();
    let decimate = transform.decimate.as_deref();
    let mpdecimate = transform.mpdecimate.as_deref();
    let framestep = transform.framestep.as_deref();
    let tile = transform.tile.as_deref();
    let untile = transform.untile.as_deref();
    let shuffleframes = transform.shuffleframes.as_deref();
    let reverse = transform.reverse.as_deref();
    let vloop = transform.r#loop.as_deref();
    let vthumbnail = transform.thumbnail.as_deref();
    let vfreezedetect = transform.freezedetect.as_deref();
    let pseudocolor = transform.pseudocolor.as_deref();
    let minterpolate = transform.minterpolate.as_deref();
    let fps = transform.fps.as_deref();
    let colorspace = transform.colorspace.as_deref();
    let zscale = transform.zscale.as_deref();
    let tonemap = transform.tonemap.as_deref();
    match (
        scale,
        fmt,
        burn,
        overlay,
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
        vloop,
        vthumbnail,
        vfreezedetect,
        pseudocolor,
        minterpolate,
        fps,
        colorspace,
        zscale,
        tonemap,
    ) {
        (
            Some(size),
            Some(name),
            false,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ) if epx.is_none() => {
            parts.push(format!(
                "scale={}:{}:flags=neighbor,format={name}",
                size.width, size.height
            ));
        }
        (
            Some(size),
            fmt,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
        ) => {
            parts.push(format!(
                "scale={}:{}:flags=neighbor",
                size.width, size.height
            ));
            if let Some(args) = epx {
                if args.is_empty() {
                    parts.push("epx".into());
                } else {
                    parts.push(format!("epx={args}"));
                }
            }
            if burn {
                parts.push("subtitles=<file>".into());
            }
            if let Some(spec) = overlay {
                parts.push(format!(
                    "movie=<file>[ov];[in][ov]overlay={}:{}",
                    spec.x, spec.y
                ));
            }
            if let Some(args) = yadif {
                if args.is_empty() {
                    parts.push("yadif".into());
                } else {
                    parts.push(format!("yadif={args}"));
                }
            }
            if let Some(args) = bwdif {
                if args.is_empty() {
                    parts.push("bwdif".into());
                } else {
                    parts.push(format!("bwdif={args}"));
                }
            }
            if let Some(args) = w3fdif {
                if args.is_empty() {
                    parts.push("w3fdif".into());
                } else {
                    parts.push(format!("w3fdif={args}"));
                }
            }
            if let Some(args) = tblend {
                parts.push(format!("tblend={args}"));
            }
            if let Some(args) = tmix {
                parts.push(format!("tmix={args}"));
            }
            if let Some(args) = hqdn3d {
                if args.is_empty() {
                    parts.push("hqdn3d".into());
                } else {
                    parts.push(format!("hqdn3d={args}"));
                }
            }
            if let Some(args) = gblur {
                if args.is_empty() {
                    parts.push("gblur".into());
                } else {
                    parts.push(format!("gblur={args}"));
                }
            }
            if let Some(args) = eq {
                if args.is_empty() {
                    parts.push("eq".into());
                } else {
                    parts.push(format!("eq={args}"));
                }
            }
            if let Some(args) = unsharp {
                if args.is_empty() {
                    parts.push("unsharp".into());
                } else {
                    parts.push(format!("unsharp={args}"));
                }
            }
            if let Some(args) = hue {
                if args.is_empty() {
                    parts.push("hue".into());
                } else {
                    parts.push(format!("hue={args}"));
                }
            }
            if let Some(args) = avgblur {
                if args.is_empty() {
                    parts.push("avgblur".into());
                } else {
                    parts.push(format!("avgblur={args}"));
                }
            }
            if let Some(args) = boxblur {
                if args.is_empty() {
                    parts.push("boxblur".into());
                } else {
                    parts.push(format!("boxblur={args}"));
                }
            }
            if let Some(args) = negate {
                if args.is_empty() || args == "0" {
                    parts.push("negate".into());
                } else {
                    parts.push(format!("negate={args}"));
                }
            }
            if let Some(args) = edgedetect {
                if args.is_empty() {
                    parts.push("edgedetect".into());
                } else {
                    parts.push(format!("edgedetect={args}"));
                }
            }
            if let Some(args) = sobel {
                if args.is_empty() {
                    parts.push("sobel".into());
                } else {
                    parts.push(format!("sobel={args}"));
                }
            }
            if let Some(args) = prewitt {
                if args.is_empty() {
                    parts.push("prewitt".into());
                } else {
                    parts.push(format!("prewitt={args}"));
                }
            }
            if let Some(args) = roberts {
                if args.is_empty() {
                    parts.push("roberts".into());
                } else {
                    parts.push(format!("roberts={args}"));
                }
            }
            if let Some(args) = kirsch {
                if args.is_empty() {
                    parts.push("kirsch".into());
                } else {
                    parts.push(format!("kirsch={args}"));
                }
            }
            if let Some(args) = scharr {
                if args.is_empty() {
                    parts.push("scharr".into());
                } else {
                    parts.push(format!("scharr={args}"));
                }
            }
            if let Some(args) = atadenoise {
                if args.is_empty() {
                    parts.push("atadenoise".into());
                } else {
                    parts.push(format!("atadenoise={args}"));
                }
            }
            if let Some(args) = owdenoise {
                if args.is_empty() {
                    parts.push("owdenoise".into());
                } else {
                    parts.push(format!("owdenoise={args}"));
                }
            }
            if let Some(args) = vaguedenoiser {
                if args.is_empty() {
                    parts.push("vaguedenoiser".into());
                } else {
                    parts.push(format!("vaguedenoiser={args}"));
                }
            }
            if let Some(args) = nlmeans {
                if args.is_empty() {
                    parts.push("nlmeans".into());
                } else {
                    parts.push(format!("nlmeans={args}"));
                }
            }
            if let Some(args) = bm3d {
                if args.is_empty() {
                    parts.push("bm3d".into());
                } else {
                    parts.push(format!("bm3d={args}"));
                }
            }
            if let Some(args) = dctdnoiz {
                if args.is_empty() {
                    parts.push("dctdnoiz".into());
                } else {
                    parts.push(format!("dctdnoiz={args}"));
                }
            }
            if let Some(args) = fftdnoiz {
                if args.is_empty() {
                    parts.push("fftdnoiz".into());
                } else {
                    parts.push(format!("fftdnoiz={args}"));
                }
            }
            if let Some(args) = smartblur {
                if args.is_empty() {
                    parts.push("smartblur".into());
                } else {
                    parts.push(format!("smartblur={args}"));
                }
            }
            if let Some(args) = sab {
                if args.is_empty() {
                    parts.push("sab".into());
                } else {
                    parts.push(format!("sab={args}"));
                }
            }
            if let Some(args) = bilateral {
                if args.is_empty() {
                    parts.push("bilateral".into());
                } else {
                    parts.push(format!("bilateral={args}"));
                }
            }
            if let Some(args) = cas {
                if args.is_empty() {
                    parts.push("cas".into());
                } else {
                    parts.push(format!("cas={args}"));
                }
            }
            if let Some(args) = vignette {
                if args.is_empty() {
                    parts.push("vignette".into());
                } else {
                    parts.push(format!("vignette={args}"));
                }
            }
            if let Some(args) = curves {
                if args.is_empty() {
                    parts.push("curves".into());
                } else {
                    parts.push(format!("curves={args}"));
                }
            }
            if let Some(args) = colorbalance {
                if args.is_empty() {
                    parts.push("colorbalance".into());
                } else {
                    parts.push(format!("colorbalance={args}"));
                }
            }
            if let Some(args) = colorlevels {
                if args.is_empty() {
                    parts.push("colorlevels".into());
                } else {
                    parts.push(format!("colorlevels={args}"));
                }
            }
            if let Some(args) = colorchannelmixer {
                if args.is_empty() {
                    parts.push("colorchannelmixer".into());
                } else {
                    parts.push(format!("colorchannelmixer={args}"));
                }
            }
            if let Some(args) = deflicker {
                if args.is_empty() {
                    parts.push("deflicker".into());
                } else {
                    parts.push(format!("deflicker={args}"));
                }
            }
            if let Some(args) = photosensitivity {
                if args.is_empty() {
                    parts.push("photosensitivity".into());
                } else {
                    parts.push(format!("photosensitivity={args}"));
                }
            }
            if let Some(args) = monochrome {
                if args.is_empty() {
                    parts.push("monochrome".into());
                } else {
                    parts.push(format!("monochrome={args}"));
                }
            }
            if let Some(args) = grayworld {
                if args.is_empty() || args == "0" {
                    parts.push("grayworld".into());
                } else {
                    parts.push(format!("grayworld={args}"));
                }
            }
            if let Some(args) = drawbox {
                if args.is_empty() {
                    parts.push("drawbox".into());
                } else {
                    parts.push(format!("drawbox={args}"));
                }
            }
            if let Some(args) = drawgrid {
                if args.is_empty() {
                    parts.push("drawgrid".into());
                } else {
                    parts.push(format!("drawgrid={args}"));
                }
            }
            if let Some(args) = lagfun {
                if args.is_empty() {
                    parts.push("lagfun".into());
                } else {
                    parts.push(format!("lagfun={args}"));
                }
            }
            if let Some(args) = amplify {
                if args.is_empty() {
                    parts.push("amplify".into());
                } else {
                    parts.push(format!("amplify={args}"));
                }
            }
            if let Some(args) = bitplanenoise {
                if args.is_empty() {
                    parts.push("bitplanenoise".into());
                } else {
                    parts.push(format!("bitplanenoise={args}"));
                }
            }
            if let Some(args) = deband {
                if args.is_empty() {
                    parts.push("deband".into());
                } else {
                    parts.push(format!("deband={args}"));
                }
            }
            if let Some(args) = gradfun {
                if args.is_empty() {
                    parts.push("gradfun".into());
                } else {
                    parts.push(format!("gradfun={args}"));
                }
            }
            if let Some(args) = lenscorrection {
                if args.is_empty() {
                    parts.push("lenscorrection".into());
                } else {
                    parts.push(format!("lenscorrection={args}"));
                }
            }
            if let Some(args) = pixelize {
                if args.is_empty() {
                    parts.push("pixelize".into());
                } else {
                    parts.push(format!("pixelize={args}"));
                }
            }
            if let Some(args) = removegrain {
                if args.is_empty() {
                    parts.push("removegrain".into());
                } else {
                    parts.push(format!("removegrain={args}"));
                }
            }
            if let Some(args) = yaepblur {
                if args.is_empty() {
                    parts.push("yaepblur".into());
                } else {
                    parts.push(format!("yaepblur={args}"));
                }
            }
            if let Some(args) = vibrance {
                if args.is_empty() {
                    parts.push("vibrance".into());
                } else {
                    parts.push(format!("vibrance={args}"));
                }
            }
            if let Some(args) = dilation {
                if args.is_empty() {
                    parts.push("dilation".into());
                } else {
                    parts.push(format!("dilation={args}"));
                }
            }
            if let Some(args) = erosion {
                if args.is_empty() {
                    parts.push("erosion".into());
                } else {
                    parts.push(format!("erosion={args}"));
                }
            }
            if let Some(args) = colorize {
                if args.is_empty() {
                    parts.push("colorize".into());
                } else {
                    parts.push(format!("colorize={args}"));
                }
            }
            if let Some(args) = exposure {
                if args.is_empty() {
                    parts.push("exposure".into());
                } else {
                    parts.push(format!("exposure={args}"));
                }
            }
            if let Some(args) = chromashift {
                if args.is_empty() {
                    parts.push("chromashift".into());
                } else {
                    parts.push(format!("chromashift={args}"));
                }
            }
            if let Some(args) = colorcontrast {
                if args.is_empty() {
                    parts.push("colorcontrast".into());
                } else {
                    parts.push(format!("colorcontrast={args}"));
                }
            }
            if let Some(args) = colorcorrect {
                if args.is_empty() {
                    parts.push("colorcorrect".into());
                } else {
                    parts.push(format!("colorcorrect={args}"));
                }
            }
            if let Some(args) = histeq {
                if args.is_empty() {
                    parts.push("histeq".into());
                } else {
                    parts.push(format!("histeq={args}"));
                }
            }
            if let Some(args) = shuffleplanes {
                if args.is_empty() {
                    parts.push("shuffleplanes".into());
                } else {
                    parts.push(format!("shuffleplanes={args}"));
                }
            }
            if let Some(args) = lutyuv {
                if args.is_empty() {
                    parts.push("lutyuv".into());
                } else {
                    parts.push(format!("lutyuv={args}"));
                }
            }
            if let Some(args) = colorhold {
                if args.is_empty() {
                    parts.push("colorhold".into());
                } else {
                    parts.push(format!("colorhold={args}"));
                }
            }
            if let Some(args) = fade {
                if args.is_empty() {
                    parts.push("fade".into());
                } else {
                    parts.push(format!("fade={args}"));
                }
            }
            if let Some(args) = perspective {
                if args.is_empty() {
                    parts.push("perspective".into());
                } else {
                    parts.push(format!("perspective={args}"));
                }
            }
            if let Some(args) = lumakey {
                if args.is_empty() {
                    parts.push("lumakey".into());
                } else {
                    parts.push(format!("lumakey={args}"));
                }
            }
            if let Some(args) = chromakey {
                if args.is_empty() {
                    parts.push("chromakey".into());
                } else {
                    parts.push(format!("chromakey={args}"));
                }
            }
            if let Some(args) = colorkey {
                if args.is_empty() {
                    parts.push("colorkey".into());
                } else {
                    parts.push(format!("colorkey={args}"));
                }
            }
            if let Some(args) = despill {
                if args.is_empty() {
                    parts.push("despill".into());
                } else {
                    parts.push(format!("despill={args}"));
                }
            }
            if let Some(args) = selectivecolor {
                if args.is_empty() {
                    parts.push("selectivecolor".into());
                } else {
                    parts.push(format!("selectivecolor={args}"));
                }
            }
            if let Some(args) = stereo3d {
                if args.is_empty() {
                    parts.push("stereo3d".into());
                } else {
                    parts.push(format!("stereo3d={args}"));
                }
            }
            if let Some(args) = field {
                if args.is_empty() {
                    parts.push("field".into());
                } else {
                    parts.push(format!("field={args}"));
                }
            }
            if let Some(args) = hqx {
                if args.is_empty() {
                    parts.push("hqx".into());
                } else {
                    parts.push(format!("hqx={args}"));
                }
            }
            if let Some(args) = xbr {
                if args.is_empty() {
                    parts.push("xbr".into());
                } else {
                    parts.push(format!("xbr={args}"));
                }
            }
            if let Some(args) = il {
                if args.is_empty() {
                    parts.push("il".into());
                } else {
                    parts.push(format!("il={args}"));
                }
            }
            if let Some(args) = super2xsai {
                if args.is_empty() {
                    parts.push("super2xsai".into());
                } else {
                    parts.push(format!("super2xsai={args}"));
                }
            }
            if let Some(args) = kerndeint {
                if args.is_empty() {
                    parts.push("kerndeint".into());
                } else {
                    parts.push(format!("kerndeint={args}"));
                }
            }
            if let Some(args) = phase {
                if args.is_empty() {
                    parts.push("phase".into());
                } else {
                    parts.push(format!("phase={args}"));
                }
            }
            if let Some(args) = estdif {
                if args.is_empty() {
                    parts.push("estdif".into());
                } else {
                    parts.push(format!("estdif={args}"));
                }
            }
            if let Some(args) = tinterlace {
                if args.is_empty() {
                    parts.push("tinterlace".into());
                } else {
                    parts.push(format!("tinterlace={args}"));
                }
            }
            if let Some(args) = separatefields {
                if args.is_empty() {
                    parts.push("separatefields".into());
                } else {
                    parts.push(format!("separatefields={args}"));
                }
            }
            if let Some(args) = weave {
                if args.is_empty() {
                    parts.push("weave".into());
                } else {
                    parts.push(format!("weave={args}"));
                }
            }
            if let Some(args) = doubleweave {
                if args.is_empty() {
                    parts.push("doubleweave".into());
                } else {
                    parts.push(format!("doubleweave={args}"));
                }
            }
            if let Some(args) = framepack {
                if args.is_empty() {
                    parts.push("framepack".into());
                } else {
                    parts.push(format!("framepack={args}"));
                }
            }
            if let Some(args) = telecine {
                if args.is_empty() {
                    parts.push("telecine".into());
                } else {
                    parts.push(format!("telecine={args}"));
                }
            }
            if let Some(args) = pullup {
                if args.is_empty() {
                    parts.push("pullup".into());
                } else {
                    parts.push(format!("pullup={args}"));
                }
            }
            if let Some(args) = decimate {
                if args.is_empty() {
                    parts.push("decimate".into());
                } else {
                    parts.push(format!("decimate={args}"));
                }
            }
            if let Some(args) = mpdecimate {
                if args.is_empty() {
                    parts.push("mpdecimate".into());
                } else {
                    parts.push(format!("mpdecimate={args}"));
                }
            }
            if let Some(args) = framestep {
                if args.is_empty() {
                    parts.push("framestep".into());
                } else {
                    parts.push(format!("framestep={args}"));
                }
            }
            if let Some(args) = tile {
                if args.is_empty() {
                    parts.push("tile".into());
                } else {
                    parts.push(format!("tile={args}"));
                }
            }
            if let Some(args) = untile {
                if args.is_empty() {
                    parts.push("untile".into());
                } else {
                    parts.push(format!("untile={args}"));
                }
            }
            if let Some(args) = shuffleframes {
                if args.is_empty() {
                    parts.push("shuffleframes".into());
                } else {
                    parts.push(format!("shuffleframes={args}"));
                }
            }
            if let Some(args) = reverse {
                if args.is_empty() {
                    parts.push("reverse".into());
                } else {
                    parts.push(format!("reverse={args}"));
                }
            }
            if let Some(args) = vloop {
                if args.is_empty() {
                    parts.push("loop".into());
                } else {
                    parts.push(format!("loop={args}"));
                }
            }
            if let Some(args) = vthumbnail {
                if args.is_empty() {
                    parts.push("thumbnail".into());
                } else {
                    parts.push(format!("thumbnail={args}"));
                }
            }
            if let Some(args) = vfreezedetect {
                if args.is_empty() {
                    parts.push("freezedetect".into());
                } else {
                    parts.push(format!("freezedetect={args}"));
                }
            }
            if let Some(args) = pseudocolor {
                if args.is_empty() {
                    parts.push("pseudocolor".into());
                } else {
                    parts.push(format!("pseudocolor={args}"));
                }
            }
            if let Some(args) = colorspace {
                parts.push(format!("colorspace={args}"));
            }
            if let Some(args) = zscale {
                parts.push(format!("zscale={args}"));
            }
            if let Some(args) = tonemap {
                parts.push(format!("tonemap={args}"));
            }
            if let Some(name) = fmt {
                parts.push(format!("format={name}"));
            }
            if let Some(args) = minterpolate {
                if args.is_empty() {
                    parts.push("minterpolate".into());
                } else {
                    parts.push(format!("minterpolate={args}"));
                }
            }
            if let Some(args) = fps {
                parts.push(format!("fps={args}"));
            }
        }
        (
            None,
            fmt,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
            _,
        ) => {
            if let Some(args) = epx {
                if args.is_empty() {
                    parts.push("epx".into());
                } else {
                    parts.push(format!("epx={args}"));
                }
            }
            if burn {
                parts.push("subtitles=<file>".into());
            }
            if let Some(spec) = overlay {
                parts.push(format!(
                    "movie=<file>[ov];[in][ov]overlay={}:{}",
                    spec.x, spec.y
                ));
            }
            if let Some(args) = yadif {
                if args.is_empty() {
                    parts.push("yadif".into());
                } else {
                    parts.push(format!("yadif={args}"));
                }
            }
            if let Some(args) = bwdif {
                if args.is_empty() {
                    parts.push("bwdif".into());
                } else {
                    parts.push(format!("bwdif={args}"));
                }
            }
            if let Some(args) = w3fdif {
                if args.is_empty() {
                    parts.push("w3fdif".into());
                } else {
                    parts.push(format!("w3fdif={args}"));
                }
            }
            if let Some(args) = tblend {
                parts.push(format!("tblend={args}"));
            }
            if let Some(args) = tmix {
                parts.push(format!("tmix={args}"));
            }
            if let Some(args) = hqdn3d {
                if args.is_empty() {
                    parts.push("hqdn3d".into());
                } else {
                    parts.push(format!("hqdn3d={args}"));
                }
            }
            if let Some(args) = gblur {
                if args.is_empty() {
                    parts.push("gblur".into());
                } else {
                    parts.push(format!("gblur={args}"));
                }
            }
            if let Some(args) = eq {
                if args.is_empty() {
                    parts.push("eq".into());
                } else {
                    parts.push(format!("eq={args}"));
                }
            }
            if let Some(args) = unsharp {
                if args.is_empty() {
                    parts.push("unsharp".into());
                } else {
                    parts.push(format!("unsharp={args}"));
                }
            }
            if let Some(args) = hue {
                if args.is_empty() {
                    parts.push("hue".into());
                } else {
                    parts.push(format!("hue={args}"));
                }
            }
            if let Some(args) = avgblur {
                if args.is_empty() {
                    parts.push("avgblur".into());
                } else {
                    parts.push(format!("avgblur={args}"));
                }
            }
            if let Some(args) = boxblur {
                if args.is_empty() {
                    parts.push("boxblur".into());
                } else {
                    parts.push(format!("boxblur={args}"));
                }
            }
            if let Some(args) = negate {
                if args.is_empty() || args == "0" {
                    parts.push("negate".into());
                } else {
                    parts.push(format!("negate={args}"));
                }
            }
            if let Some(args) = edgedetect {
                if args.is_empty() {
                    parts.push("edgedetect".into());
                } else {
                    parts.push(format!("edgedetect={args}"));
                }
            }
            if let Some(args) = sobel {
                if args.is_empty() {
                    parts.push("sobel".into());
                } else {
                    parts.push(format!("sobel={args}"));
                }
            }
            if let Some(args) = prewitt {
                if args.is_empty() {
                    parts.push("prewitt".into());
                } else {
                    parts.push(format!("prewitt={args}"));
                }
            }
            if let Some(args) = roberts {
                if args.is_empty() {
                    parts.push("roberts".into());
                } else {
                    parts.push(format!("roberts={args}"));
                }
            }
            if let Some(args) = kirsch {
                if args.is_empty() {
                    parts.push("kirsch".into());
                } else {
                    parts.push(format!("kirsch={args}"));
                }
            }
            if let Some(args) = scharr {
                if args.is_empty() {
                    parts.push("scharr".into());
                } else {
                    parts.push(format!("scharr={args}"));
                }
            }
            if let Some(args) = atadenoise {
                if args.is_empty() {
                    parts.push("atadenoise".into());
                } else {
                    parts.push(format!("atadenoise={args}"));
                }
            }
            if let Some(args) = owdenoise {
                if args.is_empty() {
                    parts.push("owdenoise".into());
                } else {
                    parts.push(format!("owdenoise={args}"));
                }
            }
            if let Some(args) = vaguedenoiser {
                if args.is_empty() {
                    parts.push("vaguedenoiser".into());
                } else {
                    parts.push(format!("vaguedenoiser={args}"));
                }
            }
            if let Some(args) = nlmeans {
                if args.is_empty() {
                    parts.push("nlmeans".into());
                } else {
                    parts.push(format!("nlmeans={args}"));
                }
            }
            if let Some(args) = bm3d {
                if args.is_empty() {
                    parts.push("bm3d".into());
                } else {
                    parts.push(format!("bm3d={args}"));
                }
            }
            if let Some(args) = dctdnoiz {
                if args.is_empty() {
                    parts.push("dctdnoiz".into());
                } else {
                    parts.push(format!("dctdnoiz={args}"));
                }
            }
            if let Some(args) = fftdnoiz {
                if args.is_empty() {
                    parts.push("fftdnoiz".into());
                } else {
                    parts.push(format!("fftdnoiz={args}"));
                }
            }
            if let Some(args) = smartblur {
                if args.is_empty() {
                    parts.push("smartblur".into());
                } else {
                    parts.push(format!("smartblur={args}"));
                }
            }
            if let Some(args) = sab {
                if args.is_empty() {
                    parts.push("sab".into());
                } else {
                    parts.push(format!("sab={args}"));
                }
            }
            if let Some(args) = bilateral {
                if args.is_empty() {
                    parts.push("bilateral".into());
                } else {
                    parts.push(format!("bilateral={args}"));
                }
            }
            if let Some(args) = cas {
                if args.is_empty() {
                    parts.push("cas".into());
                } else {
                    parts.push(format!("cas={args}"));
                }
            }
            if let Some(args) = vignette {
                if args.is_empty() {
                    parts.push("vignette".into());
                } else {
                    parts.push(format!("vignette={args}"));
                }
            }
            if let Some(args) = curves {
                if args.is_empty() {
                    parts.push("curves".into());
                } else {
                    parts.push(format!("curves={args}"));
                }
            }
            if let Some(args) = colorbalance {
                if args.is_empty() {
                    parts.push("colorbalance".into());
                } else {
                    parts.push(format!("colorbalance={args}"));
                }
            }
            if let Some(args) = colorlevels {
                if args.is_empty() {
                    parts.push("colorlevels".into());
                } else {
                    parts.push(format!("colorlevels={args}"));
                }
            }
            if let Some(args) = colorchannelmixer {
                if args.is_empty() {
                    parts.push("colorchannelmixer".into());
                } else {
                    parts.push(format!("colorchannelmixer={args}"));
                }
            }
            if let Some(args) = deflicker {
                if args.is_empty() {
                    parts.push("deflicker".into());
                } else {
                    parts.push(format!("deflicker={args}"));
                }
            }
            if let Some(args) = photosensitivity {
                if args.is_empty() {
                    parts.push("photosensitivity".into());
                } else {
                    parts.push(format!("photosensitivity={args}"));
                }
            }
            if let Some(args) = monochrome {
                if args.is_empty() {
                    parts.push("monochrome".into());
                } else {
                    parts.push(format!("monochrome={args}"));
                }
            }
            if let Some(args) = grayworld {
                if args.is_empty() || args == "0" {
                    parts.push("grayworld".into());
                } else {
                    parts.push(format!("grayworld={args}"));
                }
            }
            if let Some(args) = drawbox {
                if args.is_empty() {
                    parts.push("drawbox".into());
                } else {
                    parts.push(format!("drawbox={args}"));
                }
            }
            if let Some(args) = drawgrid {
                if args.is_empty() {
                    parts.push("drawgrid".into());
                } else {
                    parts.push(format!("drawgrid={args}"));
                }
            }
            if let Some(args) = lagfun {
                if args.is_empty() {
                    parts.push("lagfun".into());
                } else {
                    parts.push(format!("lagfun={args}"));
                }
            }
            if let Some(args) = amplify {
                if args.is_empty() {
                    parts.push("amplify".into());
                } else {
                    parts.push(format!("amplify={args}"));
                }
            }
            if let Some(args) = bitplanenoise {
                if args.is_empty() {
                    parts.push("bitplanenoise".into());
                } else {
                    parts.push(format!("bitplanenoise={args}"));
                }
            }
            if let Some(args) = deband {
                if args.is_empty() {
                    parts.push("deband".into());
                } else {
                    parts.push(format!("deband={args}"));
                }
            }
            if let Some(args) = gradfun {
                if args.is_empty() {
                    parts.push("gradfun".into());
                } else {
                    parts.push(format!("gradfun={args}"));
                }
            }
            if let Some(args) = lenscorrection {
                if args.is_empty() {
                    parts.push("lenscorrection".into());
                } else {
                    parts.push(format!("lenscorrection={args}"));
                }
            }
            if let Some(args) = pixelize {
                if args.is_empty() {
                    parts.push("pixelize".into());
                } else {
                    parts.push(format!("pixelize={args}"));
                }
            }
            if let Some(args) = removegrain {
                if args.is_empty() {
                    parts.push("removegrain".into());
                } else {
                    parts.push(format!("removegrain={args}"));
                }
            }
            if let Some(args) = yaepblur {
                if args.is_empty() {
                    parts.push("yaepblur".into());
                } else {
                    parts.push(format!("yaepblur={args}"));
                }
            }
            if let Some(args) = vibrance {
                if args.is_empty() {
                    parts.push("vibrance".into());
                } else {
                    parts.push(format!("vibrance={args}"));
                }
            }
            if let Some(args) = dilation {
                if args.is_empty() {
                    parts.push("dilation".into());
                } else {
                    parts.push(format!("dilation={args}"));
                }
            }
            if let Some(args) = erosion {
                if args.is_empty() {
                    parts.push("erosion".into());
                } else {
                    parts.push(format!("erosion={args}"));
                }
            }
            if let Some(args) = colorize {
                if args.is_empty() {
                    parts.push("colorize".into());
                } else {
                    parts.push(format!("colorize={args}"));
                }
            }
            if let Some(args) = exposure {
                if args.is_empty() {
                    parts.push("exposure".into());
                } else {
                    parts.push(format!("exposure={args}"));
                }
            }
            if let Some(args) = chromashift {
                if args.is_empty() {
                    parts.push("chromashift".into());
                } else {
                    parts.push(format!("chromashift={args}"));
                }
            }
            if let Some(args) = colorcontrast {
                if args.is_empty() {
                    parts.push("colorcontrast".into());
                } else {
                    parts.push(format!("colorcontrast={args}"));
                }
            }
            if let Some(args) = colorcorrect {
                if args.is_empty() {
                    parts.push("colorcorrect".into());
                } else {
                    parts.push(format!("colorcorrect={args}"));
                }
            }
            if let Some(args) = histeq {
                if args.is_empty() {
                    parts.push("histeq".into());
                } else {
                    parts.push(format!("histeq={args}"));
                }
            }
            if let Some(args) = shuffleplanes {
                if args.is_empty() {
                    parts.push("shuffleplanes".into());
                } else {
                    parts.push(format!("shuffleplanes={args}"));
                }
            }
            if let Some(args) = lutyuv {
                if args.is_empty() {
                    parts.push("lutyuv".into());
                } else {
                    parts.push(format!("lutyuv={args}"));
                }
            }
            if let Some(args) = colorhold {
                if args.is_empty() {
                    parts.push("colorhold".into());
                } else {
                    parts.push(format!("colorhold={args}"));
                }
            }
            if let Some(args) = fade {
                if args.is_empty() {
                    parts.push("fade".into());
                } else {
                    parts.push(format!("fade={args}"));
                }
            }
            if let Some(args) = perspective {
                if args.is_empty() {
                    parts.push("perspective".into());
                } else {
                    parts.push(format!("perspective={args}"));
                }
            }
            if let Some(args) = lumakey {
                if args.is_empty() {
                    parts.push("lumakey".into());
                } else {
                    parts.push(format!("lumakey={args}"));
                }
            }
            if let Some(args) = chromakey {
                if args.is_empty() {
                    parts.push("chromakey".into());
                } else {
                    parts.push(format!("chromakey={args}"));
                }
            }
            if let Some(args) = colorkey {
                if args.is_empty() {
                    parts.push("colorkey".into());
                } else {
                    parts.push(format!("colorkey={args}"));
                }
            }
            if let Some(args) = despill {
                if args.is_empty() {
                    parts.push("despill".into());
                } else {
                    parts.push(format!("despill={args}"));
                }
            }
            if let Some(args) = selectivecolor {
                if args.is_empty() {
                    parts.push("selectivecolor".into());
                } else {
                    parts.push(format!("selectivecolor={args}"));
                }
            }
            if let Some(args) = stereo3d {
                if args.is_empty() {
                    parts.push("stereo3d".into());
                } else {
                    parts.push(format!("stereo3d={args}"));
                }
            }
            if let Some(args) = field {
                if args.is_empty() {
                    parts.push("field".into());
                } else {
                    parts.push(format!("field={args}"));
                }
            }
            if let Some(args) = hqx {
                if args.is_empty() {
                    parts.push("hqx".into());
                } else {
                    parts.push(format!("hqx={args}"));
                }
            }
            if let Some(args) = xbr {
                if args.is_empty() {
                    parts.push("xbr".into());
                } else {
                    parts.push(format!("xbr={args}"));
                }
            }
            if let Some(args) = il {
                if args.is_empty() {
                    parts.push("il".into());
                } else {
                    parts.push(format!("il={args}"));
                }
            }
            if let Some(args) = super2xsai {
                if args.is_empty() {
                    parts.push("super2xsai".into());
                } else {
                    parts.push(format!("super2xsai={args}"));
                }
            }
            if let Some(args) = kerndeint {
                if args.is_empty() {
                    parts.push("kerndeint".into());
                } else {
                    parts.push(format!("kerndeint={args}"));
                }
            }
            if let Some(args) = phase {
                if args.is_empty() {
                    parts.push("phase".into());
                } else {
                    parts.push(format!("phase={args}"));
                }
            }
            if let Some(args) = estdif {
                if args.is_empty() {
                    parts.push("estdif".into());
                } else {
                    parts.push(format!("estdif={args}"));
                }
            }
            if let Some(args) = tinterlace {
                if args.is_empty() {
                    parts.push("tinterlace".into());
                } else {
                    parts.push(format!("tinterlace={args}"));
                }
            }
            if let Some(args) = separatefields {
                if args.is_empty() {
                    parts.push("separatefields".into());
                } else {
                    parts.push(format!("separatefields={args}"));
                }
            }
            if let Some(args) = weave {
                if args.is_empty() {
                    parts.push("weave".into());
                } else {
                    parts.push(format!("weave={args}"));
                }
            }
            if let Some(args) = doubleweave {
                if args.is_empty() {
                    parts.push("doubleweave".into());
                } else {
                    parts.push(format!("doubleweave={args}"));
                }
            }
            if let Some(args) = framepack {
                if args.is_empty() {
                    parts.push("framepack".into());
                } else {
                    parts.push(format!("framepack={args}"));
                }
            }
            if let Some(args) = telecine {
                if args.is_empty() {
                    parts.push("telecine".into());
                } else {
                    parts.push(format!("telecine={args}"));
                }
            }
            if let Some(args) = pullup {
                if args.is_empty() {
                    parts.push("pullup".into());
                } else {
                    parts.push(format!("pullup={args}"));
                }
            }
            if let Some(args) = decimate {
                if args.is_empty() {
                    parts.push("decimate".into());
                } else {
                    parts.push(format!("decimate={args}"));
                }
            }
            if let Some(args) = mpdecimate {
                if args.is_empty() {
                    parts.push("mpdecimate".into());
                } else {
                    parts.push(format!("mpdecimate={args}"));
                }
            }
            if let Some(args) = framestep {
                if args.is_empty() {
                    parts.push("framestep".into());
                } else {
                    parts.push(format!("framestep={args}"));
                }
            }
            if let Some(args) = tile {
                if args.is_empty() {
                    parts.push("tile".into());
                } else {
                    parts.push(format!("tile={args}"));
                }
            }
            if let Some(args) = untile {
                if args.is_empty() {
                    parts.push("untile".into());
                } else {
                    parts.push(format!("untile={args}"));
                }
            }
            if let Some(args) = shuffleframes {
                if args.is_empty() {
                    parts.push("shuffleframes".into());
                } else {
                    parts.push(format!("shuffleframes={args}"));
                }
            }
            if let Some(args) = reverse {
                if args.is_empty() {
                    parts.push("reverse".into());
                } else {
                    parts.push(format!("reverse={args}"));
                }
            }
            if let Some(args) = vloop {
                if args.is_empty() {
                    parts.push("loop".into());
                } else {
                    parts.push(format!("loop={args}"));
                }
            }
            if let Some(args) = vthumbnail {
                if args.is_empty() {
                    parts.push("thumbnail".into());
                } else {
                    parts.push(format!("thumbnail={args}"));
                }
            }
            if let Some(args) = vfreezedetect {
                if args.is_empty() {
                    parts.push("freezedetect".into());
                } else {
                    parts.push(format!("freezedetect={args}"));
                }
            }
            if let Some(args) = pseudocolor {
                if args.is_empty() {
                    parts.push("pseudocolor".into());
                } else {
                    parts.push(format!("pseudocolor={args}"));
                }
            }
            if let Some(args) = colorspace {
                parts.push(format!("colorspace={args}"));
            }
            if let Some(args) = zscale {
                parts.push(format!("zscale={args}"));
            }
            if let Some(args) = tonemap {
                parts.push(format!("tonemap={args}"));
            }
            if let Some(name) = fmt {
                parts.push(format!("format={name}"));
            }
            if let Some(args) = minterpolate {
                if args.is_empty() {
                    parts.push("minterpolate".into());
                } else {
                    parts.push(format!("minterpolate={args}"));
                }
            }
            if let Some(args) = fps {
                parts.push(format!("fps={args}"));
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Plan a remux without opening an output.
pub fn plan_remux(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    if crate::owned_wave_plan::supports(source, options) {
        return crate::owned_wave_plan::plan_remux(source, options);
    }
    let input = Input::open_fast(source)?;
    let selected = selection(&input, options)?;
    let streams = stream_plans(&input, &selected)?;
    Ok(MediaPlan {
        command: "remux".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps: vec![PlanStep {
            action: "copy".into(),
            detail: "stream-copy all selected packets".into(),
        }],
        graph: None,
        notes: vec![
            "no decode/encode".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan lossless/explicit transcode path without executing codecs.
pub fn plan_transcode_lossless(
    source: &Path,
    transform: &LosslessTransform,
    options: &CopyOptions,
    encoder: Option<&str>,
) -> Result<MediaPlan> {
    let input = Input::open(source)?;
    let selected = selection(&input, options)?;
    let videos: Vec<_> = selected
        .iter()
        .copied()
        .filter(|&i| unsafe {
            (*(*input.streams()[i]).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO
        })
        .collect();
    if videos.is_empty() {
        return Err("plan requires at least one selected video stream".into());
    }
    let primary = videos[0];
    let mut streams = stream_plans(&input, &selected)?;
    for s in &mut streams {
        if s.index == primary {
            s.disposition = "primary_video".into();
        } else if s.media_type == "video" {
            s.disposition = if transform.interval.is_some() {
                "secondary_interval_copy".into()
            } else {
                "copy".into()
            };
        } else if s.media_type == "audio" && transform.interval.is_some() {
            // SAFETY: stream index validated against live input.
            let classification =
                unsafe { pcm::classify_interval_audio(&*(*input.streams()[s.index]).codecpar) };
            s.disposition = match classification {
                Ok(pcm::IntervalAudio::Decode) => "decode_pcm".into(),
                Ok(pcm::IntervalAudio::Pcm) => "trim_pcm".into(),
                Err(_) => "unsupported_interval_audio".into(),
            };
        }
    }
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    let graph = video_graph(transform);
    let identity = transform.crop.is_none()
        && !transform.vertical_flip
        && !transform.horizontal_flip
        && transform.scale.is_none()
        && transform.transpose.is_none()
        && transform.rotate.is_none()
        && transform.pad.is_none()
        && transform.burn_subs.is_none()
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
        && transform.pix_fmt.is_none()
        && transform.interval.is_none()
        && !transform.seek
        && encoder.is_none();
    if identity && selected.len() == 1 {
        // SAFETY: primary index validated.
        let is_ffv1 = unsafe {
            (*(*input.streams()[primary]).codecpar).codec_id == AVCodecID_AV_CODEC_ID_FFV1
        };
        if is_ffv1 {
            steps.push(PlanStep {
                action: "copy".into(),
                detail: "FFV1 identity stream-copy (no decode)".into(),
            });
            notes.push("encoder reports ffv1 (stream copy)".into());
            return Ok(MediaPlan {
                command: "transcode-lossless".into(),
                input: source.to_path_buf(),
                inputs: vec![source.to_path_buf()],
                streams,
                steps,
                graph: None,
                notes,
            });
        }
    }
    steps.push(PlanStep {
        action: "decode".into(),
        detail: format!("software decode primary video stream {primary}"),
    });
    if transform.seek {
        steps.push(PlanStep {
            action: "seek".into(),
            detail: "demuxer seek before interval; compressed audio and secondary video use dedicated demuxers".into(),
        });
    }
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep {
            action: "interval".into(),
            detail: format!("presentation window [{from},{to}) µs relative to container start"),
        });
    }
    if transform.crop.is_some() {
        steps.push(PlanStep {
            action: "filter".into(),
            detail: "crop via plane view (no pixel payload copy)".into(),
        });
        notes.push("fvid_crop_payload_copies stays 0 for crop views".into());
    }
    if transform.horizontal_flip {
        steps.push(PlanStep {
            action: "materialize".into(),
            detail: "hflip writes into reusable frame buffers".into(),
        });
    }
    if transform.vertical_flip {
        steps.push(PlanStep {
            action: "filter".into(),
            detail: "vflip via plane pointer/stride view".into(),
        });
    }
    if transform.transpose.is_some() {
        steps.push(PlanStep {
            action: "materialize".into(),
            detail: "transpose via libavfilter".into(),
        });
    }
    if transform.rotate.is_some() {
        steps.push(PlanStep {
            action: "materialize".into(),
            detail: "rotate via libavfilter".into(),
        });
    }
    if transform.pad.is_some() {
        steps.push(PlanStep {
            action: "materialize".into(),
            detail: "pad via libavfilter".into(),
        });
    }
    match (
        transform.scale.is_some(),
        transform.pix_fmt.is_some(),
        transform.burn_subs.is_some(),
        transform.overlay.is_some(),
        transform.yadif.is_some(),
        transform.bwdif.is_some(),
        transform.w3fdif.is_some(),
        transform.tblend.is_some(),
        transform.tmix.is_some(),
        transform.hqdn3d.is_some(),
        transform.gblur.is_some(),
        transform.eq.is_some(),
        transform.unsharp.is_some(),
        transform.hue.is_some(),
        transform.avgblur.is_some(),
        transform.boxblur.is_some(),
        transform.negate.is_some(),
        transform.edgedetect.is_some(),
        transform.sobel.is_some(),
        transform.prewitt.is_some(),
        transform.roberts.is_some(),
        transform.kirsch.is_some(),
        transform.scharr.is_some(),
        transform.atadenoise.is_some(),
        transform.owdenoise.is_some(),
        transform.vaguedenoiser.is_some(),
        transform.nlmeans.is_some(),
        transform.bm3d.is_some(),
        transform.dctdnoiz.is_some(),
        transform.fftdnoiz.is_some(),
        transform.smartblur.is_some(),
        transform.sab.is_some(),
        transform.bilateral.is_some(),
        transform.cas.is_some(),
        transform.vignette.is_some(),
        transform.curves.is_some(),
        transform.colorbalance.is_some(),
        transform.colorlevels.is_some(),
        transform.colorchannelmixer.is_some(),
        transform.deflicker.is_some(),
        transform.photosensitivity.is_some(),
        transform.monochrome.is_some(),
        transform.grayworld.is_some(),
        transform.drawbox.is_some(),
        transform.drawgrid.is_some(),
        transform.lagfun.is_some(),
        transform.amplify.is_some(),
        transform.bitplanenoise.is_some(),
        transform.deband.is_some(),
        transform.gradfun.is_some(),
        transform.lenscorrection.is_some(),
        transform.pixelize.is_some(),
        transform.removegrain.is_some(),
        transform.yaepblur.is_some(),
        transform.vibrance.is_some(),
        transform.dilation.is_some(),
        transform.erosion.is_some(),
        transform.colorize.is_some(),
        transform.exposure.is_some(),
        transform.chromashift.is_some(),
        transform.colorcontrast.is_some(),
        transform.colorcorrect.is_some(),
        transform.histeq.is_some(),
        transform.shuffleplanes.is_some(),
        transform.lutyuv.is_some(),
        transform.colorhold.is_some(),
        transform.fade.is_some(),
        transform.perspective.is_some(),
        transform.lumakey.is_some(),
        transform.chromakey.is_some(),
        transform.colorkey.is_some(),
        transform.despill.is_some(),
        transform.selectivecolor.is_some(),
        transform.stereo3d.is_some(),
        transform.field.is_some(),
        transform.hqx.is_some(),
        transform.xbr.is_some(),
        transform.il.is_some(),
        transform.super2xsai.is_some(),
        transform.kerndeint.is_some(),
        transform.phase.is_some(),
        transform.estdif.is_some(),
        transform.tinterlace.is_some(),
        transform.separatefields.is_some(),
        transform.weave.is_some(),
        transform.doubleweave.is_some(),
        transform.framepack.is_some(),
        transform.telecine.is_some(),
        transform.pullup.is_some(),
        transform.decimate.is_some(),
        transform.mpdecimate.is_some(),
        transform.framestep.is_some(),
        transform.tile.is_some(),
        transform.untile.is_some(),
        transform.shuffleframes.is_some(),
        transform.reverse.is_some(),
        transform.r#loop.is_some(),
        transform.thumbnail.is_some(),
        transform.freezedetect.is_some(),
        transform.pseudocolor.is_some(),
        transform.minterpolate.is_some(),
        transform.fps.is_some(),
        transform.colorspace.is_some(),
        transform.zscale.is_some(),
        transform.tonemap.is_some(),
    ) {
        (
            true,
            true,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
            false,
        ) if transform.epx.is_none() => {
            steps.push(PlanStep {
                action: "materialize".into(),
                detail: "neighbor scale+format fused via libswscale (matches FFmpeg scale=flags=neighbor,format=)".into(),
            });
        }
        (
            scale,
            fmt,
            burn,
            overlay,
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
            vloop,
            vthumbnail,
            vfreezedetect,
            pseudocolor,
            minterpolate,
            fps,
            colorspace,
            zscale,
            tonemap,
        ) => {
            if scale {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "neighbor scale via libswscale".into(),
                });
            }
            if transform.epx.is_some() {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "epx via libavfilter epx=".into(),
                });
            }
            if burn {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "burn-in via libavfilter subtitles=".into(),
                });
            }
            if overlay {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "overlay via libavfilter movie=+overlay=".into(),
                });
            }
            if yadif {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "yadif via libavfilter yadif=".into(),
                });
            }
            if bwdif {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "bwdif via libavfilter bwdif=".into(),
                });
            }
            if w3fdif {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "w3fdif via libavfilter w3fdif=".into(),
                });
            }
            if tblend {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "tblend via libavfilter tblend=".into(),
                });
            }
            if tmix {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "tmix via libavfilter tmix=".into(),
                });
            }
            if hqdn3d {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "hqdn3d via libavfilter hqdn3d=".into(),
                });
            }
            if gblur {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "gblur via libavfilter gblur=".into(),
                });
            }
            if eq {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "eq via libavfilter eq=".into(),
                });
            }
            if unsharp {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "unsharp via libavfilter unsharp=".into(),
                });
            }
            if hue {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "hue via libavfilter hue=".into(),
                });
            }
            if avgblur {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "avgblur via libavfilter avgblur=".into(),
                });
            }
            if boxblur {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "boxblur via libavfilter boxblur=".into(),
                });
            }
            if negate {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "negate via libavfilter negate".into(),
                });
            }
            if edgedetect {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "edgedetect via libavfilter edgedetect=".into(),
                });
            }
            if sobel {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "sobel via libavfilter sobel=".into(),
                });
            }
            if prewitt {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "prewitt via libavfilter prewitt=".into(),
                });
            }
            if roberts {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "roberts via libavfilter roberts=".into(),
                });
            }
            if kirsch {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "kirsch via libavfilter kirsch=".into(),
                });
            }
            if scharr {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "scharr via libavfilter scharr=".into(),
                });
            }
            if atadenoise {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "atadenoise via libavfilter atadenoise=".into(),
                });
            }
            if owdenoise {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "owdenoise via libavfilter owdenoise=".into(),
                });
            }
            if vaguedenoiser {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "vaguedenoiser via libavfilter vaguedenoiser=".into(),
                });
            }
            if nlmeans {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "nlmeans via libavfilter nlmeans=".into(),
                });
            }
            if bm3d {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "bm3d via libavfilter bm3d=".into(),
                });
            }
            if dctdnoiz {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "dctdnoiz via libavfilter dctdnoiz=".into(),
                });
            }
            if fftdnoiz {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "fftdnoiz via libavfilter fftdnoiz=".into(),
                });
            }
            if smartblur {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "smartblur via libavfilter smartblur=".into(),
                });
            }
            if sab {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "sab via libavfilter sab=".into(),
                });
            }
            if bilateral {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "bilateral via libavfilter bilateral=".into(),
                });
            }
            if cas {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "cas via libavfilter cas=".into(),
                });
            }
            if vignette {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "vignette via libavfilter vignette=".into(),
                });
            }
            if curves {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "curves via libavfilter curves=".into(),
                });
            }
            if colorbalance {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorbalance via libavfilter colorbalance=".into(),
                });
            }
            if colorlevels {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorlevels via libavfilter colorlevels=".into(),
                });
            }
            if colorchannelmixer {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorchannelmixer via libavfilter colorchannelmixer=".into(),
                });
            }
            if deflicker {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "deflicker via libavfilter deflicker=".into(),
                });
            }
            if photosensitivity {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "photosensitivity via libavfilter photosensitivity=".into(),
                });
            }
            if monochrome {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "monochrome via libavfilter monochrome=".into(),
                });
            }
            if grayworld {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "grayworld via libavfilter grayworld".into(),
                });
            }
            if drawbox {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "drawbox via libavfilter drawbox=".into(),
                });
            }
            if drawgrid {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "drawgrid via libavfilter drawgrid=".into(),
                });
            }
            if lagfun {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "lagfun via libavfilter lagfun=".into(),
                });
            }
            if amplify {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "amplify via libavfilter amplify=".into(),
                });
            }
            if bitplanenoise {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "bitplanenoise via libavfilter bitplanenoise=".into(),
                });
            }
            if deband {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "deband via libavfilter deband=".into(),
                });
            }
            if gradfun {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "gradfun via libavfilter gradfun=".into(),
                });
            }
            if lenscorrection {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "lenscorrection via libavfilter lenscorrection=".into(),
                });
            }
            if pixelize {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "pixelize via libavfilter pixelize=".into(),
                });
            }
            if removegrain {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "removegrain via libavfilter removegrain=".into(),
                });
            }
            if yaepblur {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "yaepblur via libavfilter yaepblur=".into(),
                });
            }
            if vibrance {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "vibrance via libavfilter vibrance=".into(),
                });
            }
            if dilation {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "dilation via libavfilter dilation=".into(),
                });
            }
            if erosion {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "erosion via libavfilter erosion=".into(),
                });
            }
            if colorize {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorize via libavfilter colorize=".into(),
                });
            }
            if exposure {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "exposure via libavfilter exposure=".into(),
                });
            }
            if chromashift {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "chromashift via libavfilter chromashift=".into(),
                });
            }
            if colorcontrast {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorcontrast via libavfilter colorcontrast=".into(),
                });
            }
            if colorcorrect {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorcorrect via libavfilter colorcorrect=".into(),
                });
            }
            if histeq {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "histeq via libavfilter histeq=".into(),
                });
            }
            if shuffleplanes {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "shuffleplanes via libavfilter shuffleplanes=".into(),
                });
            }
            if lutyuv {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "lutyuv via libavfilter lutyuv=".into(),
                });
            }
            if colorhold {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorhold via libavfilter colorhold=".into(),
                });
            }
            if fade {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "fade via libavfilter fade=".into(),
                });
            }
            if perspective {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "perspective via libavfilter perspective=".into(),
                });
            }
            if lumakey {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "lumakey via libavfilter lumakey=".into(),
                });
            }
            if chromakey {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "chromakey via libavfilter chromakey=".into(),
                });
            }
            if colorkey {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorkey via libavfilter colorkey=".into(),
                });
            }
            if despill {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "despill via libavfilter despill=".into(),
                });
            }
            if selectivecolor {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "selectivecolor via libavfilter selectivecolor=".into(),
                });
            }
            if stereo3d {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "stereo3d via libavfilter stereo3d=".into(),
                });
            }
            if field {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "field via libavfilter field=".into(),
                });
            }
            if hqx {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "hqx via libavfilter hqx=".into(),
                });
            }
            if xbr {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "xbr via libavfilter xbr=".into(),
                });
            }
            if il {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "il via libavfilter il=".into(),
                });
            }
            if super2xsai {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "super2xsai via libavfilter super2xsai".into(),
                });
            }
            if kerndeint {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "kerndeint via libavfilter kerndeint=".into(),
                });
            }
            if phase {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "phase via libavfilter phase=".into(),
                });
            }
            if estdif {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "estdif via libavfilter estdif=".into(),
                });
            }
            if tinterlace {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "tinterlace via libavfilter tinterlace=".into(),
                });
            }
            if separatefields {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "separatefields via libavfilter separatefields".into(),
                });
            }
            if weave {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "weave via libavfilter weave".into(),
                });
            }
            if doubleweave {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "doubleweave via libavfilter doubleweave".into(),
                });
            }
            if framepack {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "framepack via libavfilter framepack".into(),
                });
            }
            if telecine {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "telecine via libavfilter telecine".into(),
                });
            }
            if pullup {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "pullup via libavfilter pullup".into(),
                });
            }
            if decimate {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "decimate via libavfilter decimate".into(),
                });
            }
            if mpdecimate {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "mpdecimate via libavfilter mpdecimate".into(),
                });
            }
            if framestep {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "framestep via libavfilter framestep".into(),
                });
            }
            if tile {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "tile via libavfilter tile".into(),
                });
            }
            if untile {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "untile via libavfilter untile".into(),
                });
            }
            if shuffleframes {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "shuffleframes via libavfilter shuffleframes".into(),
                });
            }
            if reverse {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "reverse via libavfilter reverse".into(),
                });
            }
            if vloop {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "loop via libavfilter loop=".into(),
                });
            }
            if vthumbnail {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "thumbnail via libavfilter thumbnail=".into(),
                });
            }
            if vfreezedetect {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "freezedetect via libavfilter freezedetect=".into(),
                });
            }
            if pseudocolor {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "pseudocolor via libavfilter pseudocolor=".into(),
                });
            }
            if colorspace {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "colorspace via libavfilter colorspace=".into(),
                });
            }
            if zscale {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "zscale via libavfilter zscale=".into(),
                });
            }
            if tonemap {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "tonemap via libavfilter tonemap=".into(),
                });
            }
            if fmt {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "pixel format conversion via libswscale (format=)".into(),
                });
            }
            if minterpolate {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "minterpolate via libavfilter minterpolate=".into(),
                });
            }
            if fps {
                steps.push(PlanStep {
                    action: "materialize".into(),
                    detail: "fps via libavfilter fps=".into(),
                });
            }
        }
    }
    let encoder_name = encoder.unwrap_or("ffv1");
    steps.push(PlanStep {
        action: "encode".into(),
        detail: format!("{encoder_name} encode of primary video"),
    });
    for s in &streams {
        if s.index != primary {
            let detail = match s.disposition.as_str() {
                "decode_pcm" => format!(
                    "decode {} stream {} ({}) to sample-exact PCM for interval",
                    s.media_type, s.index, s.codec
                ),
                "trim_pcm" => format!(
                    "packet-trim {} stream {} ({}) inside interval",
                    s.media_type, s.index, s.codec
                ),
                "secondary_interval_copy" => format!(
                    "interval stream-copy {} stream {} ({}); H.264/HEVC use dedicated demuxer RAP path",
                    s.media_type, s.index, s.codec
                ),
                _ => format!(
                    "stream-copy {} stream {} ({})",
                    s.media_type, s.index, s.codec
                ),
            };
            steps.push(PlanStep {
                action: if s.disposition == "decode_pcm" {
                    "decode".into()
                } else {
                    "copy".into()
                },
                detail,
            });
        }
    }
    if let Some(ref g) = graph {
        notes.push(format!("ffmpeg-equivalent -vf: {g}"));
    }
    if transform.seek {
        notes.push(
            "primary demuxer seek; secondary video and compressed audio use dedicated demuxers"
                .into(),
        );
    }
    if transform.interval.is_some() && videos.len() > 1 {
        notes.push(
            "secondary non-reordered video is stream-copied on exact packet boundaries; H.264/HEVC secondary uses closed-GOP RAP stream-copy on a dedicated demuxer".into(),
        );
    }
    notes.push("GPU/codec surface interop not used on this path".into());
    notes.push("estimated DPB admission applies when --max-memory-mib is set".into());
    Ok(MediaPlan {
        command: "transcode-lossless".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps,
        graph,
        notes,
    })
}

fn classify_trim_stream(input: &Input, index: usize) -> Result<&'static str> {
    // SAFETY: index validated by selection.
    unsafe {
        let p = &*(*input.streams()[index]).codecpar;
        match p.codec_type {
            t if t == AVMediaType_AVMEDIA_TYPE_VIDEO => {
                if p.video_delay > 0
                    || p.codec_id == AVCodecID_AV_CODEC_ID_H264
                    || p.codec_id == AVCodecID_AV_CODEC_ID_HEVC
                {
                    Ok("stream_copy_rap")
                } else {
                    Ok("stream_copy")
                }
            }
            t if t == AVMediaType_AVMEDIA_TYPE_AUDIO => match pcm::classify_interval_audio(p) {
                Ok(pcm::IntervalAudio::Decode) => Ok("seam_decode_pcm"),
                Ok(pcm::IntervalAudio::Pcm) => Ok("stream_copy_pcm"),
                Err(_) => Ok("stream_copy"),
            },
            _ => Ok("stream_copy"),
        }
    }
}

/// Plan a strict stream-copy trim without writing output.
pub fn plan_trim(
    source: &Path,
    from_us: i64,
    to_us: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if from_us < 0 || to_us <= from_us {
        return Err("plan trim requires 0 <= from < to".into());
    }
    if crate::owned_wave_plan::supports(source, options) {
        return crate::owned_wave_plan::plan_trim(source, from_us, to_us, options);
    }
    let input = Input::open_fast(source)?;
    let selected = selection(&input, options)?;
    if selected.is_empty() {
        return Err("plan trim requires at least one selected stream".into());
    }
    let mut streams = stream_plans(&input, &selected)?;
    let mut steps = Vec::new();
    steps.push(PlanStep {
        action: "interval".into(),
        detail: format!("presentation window [{from_us},{to_us}) µs relative to container start"),
    });
    for s in &mut streams {
        let disposition = classify_trim_stream(&input, s.index)?;
        s.disposition = disposition.into();
        let detail = match disposition {
            "stream_copy_rap" => format!(
                "closed/open-GOP stream-copy {} stream {} ({}) with RAP pre/post-roll as needed",
                s.media_type, s.index, s.codec
            ),
            "seam_decode_pcm" => format!(
                "packet-aligned copy or mid-packet AAC/MP3/FLAC → PCM seam for {} stream {} ({})",
                s.media_type, s.index, s.codec
            ),
            "stream_copy_pcm" => format!(
                "exact packet/sample trim {} stream {} ({})",
                s.media_type, s.index, s.codec
            ),
            _ => format!(
                "stream-copy {} stream {} ({}) on exact packet boundaries",
                s.media_type, s.index, s.codec
            ),
        };
        steps.push(PlanStep {
            action: if disposition == "seam_decode_pcm" {
                "decode".into()
            } else {
                "copy".into()
            },
            detail,
        });
    }
    Ok(MediaPlan {
        command: "trim".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps,
        graph: None,
        notes: vec![
            "no encoder; fails closed if boundaries are not stream-copyable without the PCM seam fallback".into(),
            "ffmpeg-equivalent: -ss FROM -to TO -c copy (plus qualified RAP/PCM seam behavior)".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan trim-pcm (packed PCM packet-view slice) without executing.
pub fn plan_trim_pcm(
    source: &Path,
    from_us: i64,
    to_us: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if from_us < 0 || to_us <= from_us {
        return Err("plan trim-pcm requires 0 <= from < to".into());
    }
    if crate::owned_wave_plan::supports(source, options) {
        return crate::owned_wave_plan::plan_trim_pcm(source, from_us, to_us, options);
    }
    let input = Input::open_fast(source)?;
    let selected = selection(&input, options)?;
    if selected.is_empty() {
        return Err("plan trim-pcm requires at least one selected stream".into());
    }
    for &index in &selected {
        // SAFETY: Stream indices were checked against the live input table.
        pcm::validate(unsafe { &*(*input.streams()[index]).codecpar })?;
    }
    let mut streams = stream_plans(&input, &selected)?;
    for s in &mut streams {
        s.disposition = "trim_pcm".into();
    }
    let mut steps = Vec::new();
    steps.push(PlanStep {
        action: "interval".into(),
        detail: format!("presentation window [{from_us},{to_us}) µs relative to container start"),
    });
    steps.push(PlanStep {
        action: "chapters".into(),
        detail: "retime chapters to interval origin".into(),
    });
    for s in &streams {
        steps.push(PlanStep {
            action: "trim".into(),
            detail: format!(
                "packet-view trim {} stream {} ({}): offset data pointer, shrink size, retimestamp PTS/DTS/duration; AVPacket.buf unchanged",
                s.media_type, s.index, s.codec
            ),
        });
    }
    steps.push(PlanStep {
        action: "copy".into(),
        detail: "mux retained packets with strict timestamp rescale".into(),
    });
    Ok(MediaPlan {
        command: "trim-pcm".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps,
        graph: None,
        notes: vec![
            "no decoder/encoder; sample payload stays in original AVPacket allocation".into(),
            "requires timestamped whole sample frames without side data".into(),
            "ffmpeg-equivalent: atrim+asetpts (re-encodes payload); native path preserves zero-copy packet view".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan a strict stream-copy concat without writing output.
pub fn plan_concat(sources: &[PathBuf], options: &CopyOptions) -> Result<MediaPlan> {
    if sources.len() < 2 || sources.len() > 256 {
        return Err("plan concat requires 2..=256 inputs".into());
    }
    if sources
        .iter()
        .all(|source| crate::owned_wave_plan::supports(source, options))
    {
        return crate::owned_wave_plan::plan_concat(sources, options);
    }
    let first = Input::open_fast(&sources[0])?;
    let selected = selection(&first, options)?;
    if selected.is_empty() {
        return Err("plan concat requires at least one selected stream".into());
    }
    let streams = stream_plans(&first, &selected)?;
    // Compatibility check: same stream count/codecs as first input.
    for (i, path) in sources.iter().enumerate().skip(1) {
        let other = Input::open_fast(path)?;
        let other_selected = selection(&other, options)?;
        if other_selected.len() != selected.len() {
            return Err(format!(
                "concat input {i} selects {} streams; expected {}",
                other_selected.len(),
                selected.len()
            ));
        }
        for (&a, &b) in selected.iter().zip(other_selected.iter()) {
            // SAFETY: both selections validated.
            unsafe {
                let pa = &*(*first.streams()[a]).codecpar;
                let pb = &*(*other.streams()[b]).codecpar;
                if pa.codec_type != pb.codec_type || pa.codec_id != pb.codec_id {
                    return Err(format!(
                        "concat input {i} stream codec mismatch vs first input"
                    ));
                }
            }
        }
    }
    let mut steps = vec![PlanStep {
        action: "concat".into(),
        detail: format!("stream-copy concatenate {} inputs in order", sources.len()),
    }];
    for s in &streams {
        steps.push(PlanStep {
            action: "copy".into(),
            detail: format!(
                "append {} stream {} ({}) across segments without re-encode",
                s.media_type, s.index, s.codec
            ),
        });
    }
    Ok(MediaPlan {
        command: "concat".into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
        streams,
        steps,
        graph: None,
        notes: vec![
            "requires compatible codec configuration/extradata across inputs".into(),
            "ffmpeg-equivalent: concat demuxer + -c copy".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan an external overlay composite without executing.
pub fn plan_overlay(
    source: &Path,
    overlay: &Path,
    x: i32,
    y: i32,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    let input = Input::open_fast(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("plan overlay requires a main video stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![video];
    }
    let streams = stream_plans(&input, &selected.streams)?;
    if !overlay.is_file() {
        return Err("overlay video file not found".into());
    }
    let graph = overlay_cli_vf(overlay, x, y)?;
    Ok(MediaPlan {
        command: "overlay".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf(), overlay.to_path_buf()],
        streams,
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "decode main video".into(),
            },
            PlanStep {
                action: "materialize".into(),
                detail: format!("overlay via libavfilter movie=+overlay={x}:{y}"),
            },
            PlanStep {
                action: "encode".into(),
                detail: "FFV1 level 3 in Matroska (video-only)".into(),
            },
        ],
        graph: Some(graph),
        notes: vec![
            "ffmpeg-equivalent: -vf movie=OVERLAY[ov];[in][ov]overlay=x:y".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan a dual-input xfade without executing.
pub fn plan_xfade(
    main: &Path,
    other: &Path,
    transition: &str,
    duration_us: i64,
    offset_us: i64,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    crate::filter::validate_xfade_transition(transition)?;
    if duration_us <= 0 {
        return Err("xfade duration must be > 0".into());
    }
    let input = Input::open_fast(main)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("plan xfade requires a main video stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![video];
    }
    let streams = stream_plans(&input, &selected.streams)?;
    if !other.is_file() {
        return Err("xfade other video file not found".into());
    }
    let graph = xfade_cli_vf(other, transition, duration_us, offset_us, 0, 0)?;
    Ok(MediaPlan {
        command: "xfade".into(),
        input: main.to_path_buf(),
        inputs: vec![main.to_path_buf(), other.to_path_buf()],
        streams,
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "decode dual video inputs".into(),
            },
            PlanStep {
                action: "materialize".into(),
                detail: format!("xfade via libavfilter transition={transition}"),
            },
            PlanStep {
                action: "encode".into(),
                detail: "FFV1 level 3 in Matroska (video-only)".into(),
            },
        ],
        graph: Some(graph),
        notes: vec![
            "ffmpeg-equivalent: -filter_complex [0:v][1:v]xfade=...,format=yuv420p".into(),
            "inputs must share size/fps/pix_fmt".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan external SRT burn-in without executing.
pub fn plan_burn_subtitles(source: &Path, subs: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    let input = Input::open_fast(source)?;
    let video = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_VIDEO })
        .ok_or("plan burn-subtitles requires a video stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![video];
    }
    let streams = stream_plans(&input, &selected.streams)?;
    if !subs.is_file() {
        return Err("subtitle file not found".into());
    }
    let graph = subtitles_cli_vf(subs)?;
    Ok(MediaPlan {
        command: "burn-subtitles".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "decode video".into(),
            },
            PlanStep {
                action: "materialize".into(),
                detail: "burn-in via libavfilter subtitles=".into(),
            },
            PlanStep {
                action: "encode".into(),
                detail: "FFV1 level 3 in Matroska (video-only)".into(),
            },
        ],
        graph: Some(graph),
        notes: vec![
            "ffmpeg-equivalent: -vf subtitles=FILE".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan an ebur128 loudness measure without executing.
pub fn plan_loudness(source: &Path, options: &CopyOptions) -> Result<MediaPlan> {
    let input = Input::open_fast(source)?;
    let audio = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_AUDIO })
        .ok_or("plan loudness requires an audio stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![audio];
    }
    let streams = stream_plans(&input, &selected.streams)?;
    Ok(MediaPlan {
        command: "loudness".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "decode audio".into(),
            },
            PlanStep {
                action: "analyze".into(),
                detail: "measure via libavfilter ebur128=peak=true".into(),
            },
        ],
        graph: Some("ebur128=peak=true".into()),
        notes: vec![
            "ffmpeg-equivalent: -af ebur128=peak=true -f null -".into(),
            "reports integrated LUFS, LRA, and true peak".into(),
        ],
    })
}

/// Plan a loudnorm apply (optional dual-pass) without executing.
pub fn plan_loudnorm(
    source: &Path,
    args: Option<&str>,
    dual_pass: bool,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    let resolved = match args {
        None | Some("") => crate::loudness::DEFAULT_LOUDNORM_ARGS.to_owned(),
        Some(value) => {
            crate::loudness::validate_loudnorm_args(value)?;
            value.to_owned()
        }
    };
    let input = Input::open_fast(source)?;
    let audio = input
        .streams()
        .iter()
        .position(|&s| unsafe { (*(*s).codecpar).codec_type == AVMediaType_AVMEDIA_TYPE_AUDIO })
        .ok_or("plan loudnorm requires an audio stream")?;
    let mut selected = options.clone();
    if selected.streams.is_empty() {
        selected.streams = vec![audio];
    }
    let streams = stream_plans(&input, &selected.streams)?;
    let mut steps = Vec::new();
    if dual_pass {
        steps.push(PlanStep {
            action: "analyze".into(),
            detail: "measure loudnorm print_format=json for measured_*".into(),
        });
    }
    steps.push(PlanStep {
        action: "filter".into(),
        detail: if dual_pass {
            "apply loudnorm with measured_* + linear=true → float PCM".into()
        } else {
            "apply loudnorm= → float PCM WAV".into()
        },
    });
    let graph = if dual_pass {
        format!("loudnorm={resolved}:measured_*=…:linear=true,aformat=sample_fmts=flt")
    } else {
        format!("loudnorm={resolved},aformat=sample_fmts=flt")
    };
    Ok(MediaPlan {
        command: "loudnorm".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps,
        graph: Some(graph),
        notes: vec![
            if dual_pass {
                "ffmpeg-equivalent: two-pass loudnorm measure then apply + pcm_f32le".into()
            } else {
                "ffmpeg-equivalent: -af loudnorm=ARGS,aformat=sample_fmts=flt -c:a pcm_f32le".into()
            },
            "partial outputs are not published on failure".into(),
        ],
    })
}

/// Plan multi-input amix without executing.
pub fn plan_mix_audio(sources: &[PathBuf], options: &crate::MixAudioOptions) -> Result<MediaPlan> {
    if !(2..=16).contains(&sources.len()) {
        return Err("plan mix-audio requires 2..=16 inputs".into());
    }
    for source in sources {
        if !source.is_file() {
            return Err(format!("mix-audio input not found: {}", source.display()).into());
        }
    }
    let n = sources.len();
    let normalize = if options.normalize { 1 } else { 0 };
    let mut graph = format!(
        "amix=inputs={n}:duration={}:dropout_transition=0:normalize={normalize}",
        options.duration.as_str()
    );
    if !options.weights.is_empty() {
        let weights = options
            .weights
            .iter()
            .map(|w| format!("{w}"))
            .collect::<Vec<_>>()
            .join(" ");
        graph.push_str(&format!(":weights={weights}"));
    }
    Ok(MediaPlan {
        command: "mix-audio".into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
        streams: Vec::new(),
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: format!("decode {n} audio inputs to float PCM"),
            },
            PlanStep {
                action: "mix".into(),
                detail: "amix-compatible weighted float mix (shortest)".into(),
            },
            PlanStep {
                action: "encode".into(),
                detail: "write float WAV (pcm_f32le)".into(),
            },
        ],
        graph: Some(graph),
        notes: vec![
            "ffmpeg-equivalent: -filter_complex amix=inputs=N:duration=shortest:dropout_transition=0:normalize=0|1[:weights=…]".into(),
            "inputs must share sample rate and channel count".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}

fn default_layout_name(channels: i32) -> Result<String> {
    // SAFETY: default layout for N is built and described into a UTF-8 buffer.
    unsafe {
        let mut layout = AVChannelLayout {
            order: 0,
            nb_channels: 0,
            u: std::mem::zeroed(),
            opaque: ptr::null_mut(),
        };
        av_channel_layout_default(&mut layout, channels);
        if layout.nb_channels != channels {
            av_channel_layout_uninit(&mut layout);
            return Err("failed to build default channel layout".into());
        }
        let mut buf = [0i8; 64];
        let written = av_channel_layout_describe(&layout, buf.as_mut_ptr(), buf.len());
        av_channel_layout_uninit(&mut layout);
        if written < 0 {
            return Err("channel layout describe failed".into());
        }
        let bytes = buf
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect::<Vec<_>>();
        String::from_utf8(bytes).map_err(|_| "channel layout is not UTF-8".into())
    }
}

/// Build the FFmpeg-equivalent `-af` chain for decode-audio transforms.
fn audio_decode_graph(
    transform: &AudioDecodeTransform,
    source_rate: i32,
    source_channels: i32,
) -> Option<String> {
    let mut parts = Vec::new();
    let out_rate = transform.sample_rate.unwrap_or(source_rate);
    if transform
        .sample_rate
        .is_some_and(|rate| rate != source_rate)
    {
        parts.push(format!("aresample={out_rate}"));
    }
    let out_channels = transform.channels.unwrap_or(source_channels);
    if transform
        .channels
        .is_some_and(|channels| channels != source_channels)
    {
        if let Ok(layout) = default_layout_name(out_channels) {
            parts.push(format!("aformat=channel_layouts={layout}"));
        }
    }
    if let Some(gain) = transform.volume {
        if (gain - 1.0).abs() > 1e-15 {
            parts.push(format!("volume={gain}"));
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(","))
    }
}

/// Plan decode-audio (PCM WAV export) without executing.
pub fn plan_decode_audio(
    source: &Path,
    transform: &AudioDecodeTransform,
    options: &CopyOptions,
) -> Result<MediaPlan> {
    if transform
        .interval
        .is_some_and(|(from, to)| from < 0 || to <= from)
    {
        return Err("decode-audio interval requires 0 <= from < to".into());
    }
    if let Some(rate) = transform.sample_rate {
        if !(8_000..=384_000).contains(&rate) {
            return Err("sample rate must be within 8000..=384000".into());
        }
    }
    if let Some(channels) = transform.channels {
        if !(1..=64).contains(&channels) {
            return Err("channels must be within 1..=64".into());
        }
    }
    if let Some(gain) = transform.volume {
        if !gain.is_finite() || !(0.0..=64.0).contains(&gain) {
            return Err("volume must be a finite linear gain within 0..=64".into());
        }
    }
    let input = Input::open_fast(source)?;
    // SAFETY: Input owns a live format context.
    if unsafe { (*input.0).nb_chapters } != 0 {
        return Err("decode-audio with chapters requires explicit timeline mapping".into());
    }
    let selected = selection(&input, options)?;
    if selected.len() != 1 {
        return Err(
            "decode-audio requires exactly one selected audio stream; use --streams".into(),
        );
    }
    let index = selected[0];
    let (source_rate, source_channels) = unsafe {
        let stream = &*input.streams()[index];
        let p = &*stream.codecpar;
        if p.codec_type != AVMediaType_AVMEDIA_TYPE_AUDIO {
            return Err("selected stream is not audio".into());
        }
        (p.sample_rate, p.ch_layout.nb_channels)
    };
    let streams = stream_plans(&input, &selected)?;
    let mut steps = vec![PlanStep {
        action: "decode".into(),
        detail: "decode one audio stream".into(),
    }];
    if transform
        .sample_rate
        .is_some_and(|rate| rate != source_rate)
    {
        steps.push(PlanStep {
            action: "resample".into(),
            detail: format!(
                "libswresample {} Hz → {} Hz",
                source_rate,
                transform.sample_rate.unwrap_or(source_rate)
            ),
        });
    }
    if transform
        .channels
        .is_some_and(|channels| channels != source_channels)
    {
        steps.push(PlanStep {
            action: "rematrix".into(),
            detail: format!(
                "default layout {} → {} channels",
                source_channels,
                transform.channels.unwrap_or(source_channels)
            ),
        });
    }
    if transform
        .volume
        .is_some_and(|gain| (gain - 1.0).abs() > 1e-15)
    {
        steps.push(PlanStep {
            action: "volume".into(),
            detail: format!("linear gain {}", transform.volume.unwrap()),
        });
    }
    if let Some((from, to)) = transform.interval {
        steps.push(PlanStep {
            action: "trim".into(),
            detail: format!("sample window [{from}, {to}) µs from first decoded sample"),
        });
    }
    steps.push(PlanStep {
        action: "encode".into(),
        detail: "write WAV preserving decoded sample precision".into(),
    });
    let graph = audio_decode_graph(transform, source_rate, source_channels);
    let mut notes = vec![
        "ffmpeg-equivalent: optional -af CHAIN; preserves decoded PCM precision (not forced 16-bit)".into(),
        "partial outputs are not published on failure".into(),
    ];
    if transform.interval.is_some() {
        notes.push(
            "interval trims decoded samples (not atrim filter); boundaries round up to sample grid"
                .into(),
        );
    }
    Ok(MediaPlan {
        command: "decode-audio".into(),
        input: source.to_path_buf(),
        inputs: vec![source.to_path_buf()],
        streams,
        steps,
        graph,
        notes,
    })
}

/// Plan two-input amerge without executing.
pub fn plan_merge_audio(sources: &[PathBuf]) -> Result<MediaPlan> {
    if sources.len() != 2 {
        return Err("plan merge-audio requires exactly two inputs".into());
    }
    for source in sources {
        if !source.is_file() {
            return Err(format!("merge-audio input not found: {}", source.display()).into());
        }
    }
    Ok(MediaPlan {
        command: "merge-audio".into(),
        input: sources[0].clone(),
        inputs: sources.to_vec(),
        streams: Vec::new(),
        steps: vec![
            PlanStep {
                action: "decode".into(),
                detail: "decode two audio inputs to float PCM".into(),
            },
            PlanStep {
                action: "merge".into(),
                detail: "amerge channel join".into(),
            },
            PlanStep {
                action: "encode".into(),
                detail: "write float WAV (pcm_f32le)".into(),
            },
        ],
        graph: Some("amerge=inputs=2".into()),
        notes: vec![
            "ffmpeg-equivalent: -filter_complex amerge=inputs=2".into(),
            "inputs must share sample rate".into(),
            "partial outputs are not published on failure".into(),
        ],
    })
}
