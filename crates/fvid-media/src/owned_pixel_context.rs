//! Cached programs and persistent history owned by one streaming filter chain.
#[derive(Debug)]
pub(crate) struct PixelContext {
    pub boxblur: Option<crate::owned_boxblur::BoxBlurProgram>,
    pub sab: Option<crate::owned_sab::Sab>,
    pub bitplanenoise: Option<crate::owned_bitplanenoise::BitPlaneNoise>,
    pub deband: Option<crate::owned_deband::Deband>,
    pub perspective: Option<crate::owned_perspective::Perspective>,
    pub gradfun: Option<crate::owned_gradfun::GradFun>,
    pub lenscorrection: Option<crate::owned_lenscorrection::LensCorrection>,
    pub drawbox: Option<crate::owned_draw::Draw>,
    pub drawgrid: Option<crate::owned_draw::Draw>,
    pub removegrain: Option<crate::owned_removegrain::RemoveGrain>,
    pub yaepblur: Option<crate::owned_yaepblur::YaepBlur>,
    pub smartblur: Option<crate::owned_smartblur::SmartBlur>,
    pub vignette: Option<crate::owned_vignette::Vignette>,
    pub curves: Option<crate::owned_curves::Curves>,
    pub eq: Option<crate::owned_eq::EqualizerProgram>,
    pub lagfun: Option<crate::owned_lagfun::LagFun>,
    pub hqdn3d: Option<crate::owned_hqdn3d::HqDn3d>,
    pub tmix: Option<crate::owned_tmix::TemporalMix>,
    pub unsharp: Option<crate::owned_unsharp::Unsharp>,
    pub hue: Option<crate::owned_hue::HueProgram>,
    pub gblur: Option<crate::owned_gblur::GaussianBlur>,
    pub avgblur: Option<crate::owned_avgblur::AverageBlur>,
    pub negate: Option<crate::owned_negate::Negate>,
    pub gradients: [Option<crate::owned_gradient::Gradient>; 5],
    pub bilateral: Option<crate::owned_bilateral::Bilateral>,
    pub cas: Option<crate::owned_cas::Cas>,
    pub colorbalance: Option<crate::owned_colorbalance::ColorBalance>,
    pub colorlevels: Option<crate::owned_colorlevels::ColorLevels>,
    pub colorchannelmixer: Option<crate::owned_colorchannelmixer::ColorChannelMixer>,
    pub monochrome: Option<crate::owned_monochrome::Monochrome>,
    pub grayworld: Option<crate::owned_timeline::Timeline>,
    pub pixelize: Option<crate::owned_pixelize::Pixelize>,
    pub vibrance: Option<crate::owned_vibrance::Vibrance>,
    pub morphology: [Option<crate::owned_morphology::Morphology>; 2],
    pub colorize: Option<crate::owned_colorize::Colorize>,
    pub exposure: Option<crate::owned_exposure::Exposure>,
    pub chromashift: Option<crate::owned_chromashift::ChromaShift>,
    pub colorcontrast: Option<crate::owned_colorcontrast::ColorContrast>,
    pub colorcorrect: Option<crate::owned_colorcorrect::ColorCorrect>,
    pub shuffleplanes: Option<crate::owned_shuffleplanes::ShufflePlanes>,
    pub colorhold: Option<crate::owned_colorhold::ColorHold>,
}
impl PixelContext {
    pub fn parse(transform: &fvid_media_info::DecodeTransform) -> Result<Self, String> {
        use crate::owned_gradient::GradientKind::*;
        use crate::owned_morphology::MorphologyKind::*;
        Ok(Self {
            sab: transform.sab.as_deref().map(crate::owned_sab::Sab::parse).transpose()?,
            bitplanenoise: transform.bitplanenoise.as_deref().map(crate::owned_bitplanenoise::BitPlaneNoise::parse).transpose()?,
            deband: transform.deband.as_deref().map(crate::owned_deband::Deband::parse).transpose()?,
            perspective: transform.perspective.as_deref().map(crate::owned_perspective::Perspective::parse).transpose()?,
            gradfun: transform.gradfun.as_deref().map(crate::owned_gradfun::GradFun::parse).transpose()?,
            lenscorrection: transform.lenscorrection.as_deref().map(crate::owned_lenscorrection::LensCorrection::parse).transpose()?,
            drawbox: transform.drawbox.as_deref().map(crate::owned_draw::Draw::box_filter).transpose()?,
            drawgrid: transform.drawgrid.as_deref().map(crate::owned_draw::Draw::grid_filter).transpose()?,
            removegrain: transform.removegrain.as_deref().map(crate::owned_removegrain::RemoveGrain::parse).transpose()?,
            yaepblur: transform.yaepblur.as_deref().map(crate::owned_yaepblur::YaepBlur::parse).transpose()?,
            smartblur: transform.smartblur.as_deref().map(crate::owned_smartblur::SmartBlur::parse).transpose()?,
            vignette: transform.vignette.as_deref().map(crate::owned_vignette::Vignette::parse).transpose()?,
            curves: transform
                .curves
                .as_deref()
                .map(crate::owned_curves::Curves::parse)
                .transpose()?,
            hqdn3d: transform
                .hqdn3d
                .as_deref()
                .map(crate::owned_hqdn3d::HqDn3d::parse)
                .transpose()?,
            boxblur: transform
                .boxblur
                .as_deref()
                .map(crate::owned_boxblur::BoxBlurProgram::parse)
                .transpose()?,
            eq: transform
                .eq
                .as_deref()
                .map(crate::owned_eq::EqualizerProgram::parse)
                .transpose()?,
            lagfun: transform
                .lagfun
                .as_deref()
                .map(crate::owned_lagfun::LagFun::parse)
                .transpose()?,
            tmix: transform
                .tmix
                .as_deref()
                .map(crate::owned_tmix::TemporalMix::parse)
                .transpose()?,
            unsharp: transform.unsharp.as_deref().map(crate::owned_unsharp::Unsharp::parse).transpose()?,
            hue: transform.hue.as_deref().map(crate::owned_hue::HueProgram::parse).transpose()?,
            gblur: transform.gblur.as_deref().map(crate::owned_gblur::GaussianBlur::parse).transpose()?,
            avgblur: transform.avgblur.as_deref().map(crate::owned_avgblur::AverageBlur::parse).transpose()?,
            negate: transform.negate.as_deref().map(crate::owned_negate::Negate::parse).transpose()?,
            gradients: [
                transform.sobel.as_deref().map(|a| crate::owned_gradient::Gradient::parse(Sobel, a)).transpose()?,
                transform.prewitt.as_deref().map(|a| crate::owned_gradient::Gradient::parse(Prewitt, a)).transpose()?,
                transform.roberts.as_deref().map(|a| crate::owned_gradient::Gradient::parse(Roberts, a)).transpose()?,
                transform.kirsch.as_deref().map(|a| crate::owned_gradient::Gradient::parse(Kirsch, a)).transpose()?,
                transform.scharr.as_deref().map(|a| crate::owned_gradient::Gradient::parse(Scharr, a)).transpose()?,
            ],
            bilateral: transform.bilateral.as_deref().map(crate::owned_bilateral::Bilateral::parse).transpose()?,
            cas: transform.cas.as_deref().map(crate::owned_cas::Cas::parse).transpose()?,
            colorbalance: transform.colorbalance.as_deref().map(crate::owned_colorbalance::ColorBalance::parse).transpose()?,
            colorlevels: transform.colorlevels.as_deref().map(crate::owned_colorlevels::ColorLevels::parse).transpose()?,
            colorchannelmixer: transform.colorchannelmixer.as_deref().map(crate::owned_colorchannelmixer::ColorChannelMixer::parse).transpose()?,
            monochrome: transform.monochrome.as_deref().map(crate::owned_monochrome::Monochrome::parse).transpose()?,
            grayworld: transform.grayworld.as_deref().map(crate::owned_timeline::Timeline::grayworld).transpose()?,
            pixelize: transform.pixelize.as_deref().map(crate::owned_pixelize::Pixelize::parse).transpose()?,
            vibrance: transform.vibrance.as_deref().map(crate::owned_vibrance::Vibrance::parse).transpose()?,
            morphology: [
                transform.dilation.as_deref().map(|a| crate::owned_morphology::Morphology::parse(Dilation, a)).transpose()?,
                transform.erosion.as_deref().map(|a| crate::owned_morphology::Morphology::parse(Erosion, a)).transpose()?,
            ],
            colorize: transform.colorize.as_deref().map(crate::owned_colorize::Colorize::parse).transpose()?,
            exposure: transform.exposure.as_deref().map(crate::owned_exposure::Exposure::parse).transpose()?,
            chromashift: transform.chromashift.as_deref().map(crate::owned_chromashift::ChromaShift::parse).transpose()?,
            colorcontrast: transform.colorcontrast.as_deref().map(crate::owned_colorcontrast::ColorContrast::parse).transpose()?,
            colorcorrect: transform.colorcorrect.as_deref().map(crate::owned_colorcorrect::ColorCorrect::parse).transpose()?,
            shuffleplanes: transform.shuffleplanes.as_deref().map(crate::owned_shuffleplanes::ShufflePlanes::parse).transpose()?,
            colorhold: transform.colorhold.as_deref().map(crate::owned_colorhold::ColorHold::parse).transpose()?,
        })
    }
}
