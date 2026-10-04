//! Cached programs and persistent history owned by one streaming filter chain.
#[derive(Debug)]
pub(crate) struct PixelContext {
    pub boxblur: Option<crate::owned_boxblur::BoxBlurProgram>,
    pub sab: Option<crate::owned_sab::Sab>,
    pub bitplanenoise: Option<crate::owned_bitplanenoise::BitPlaneNoise>,
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
}
impl PixelContext {
    pub fn parse(transform: &fvid_media_info::DecodeTransform) -> Result<Self, String> {
        Ok(Self {
            sab: transform.sab.as_deref().map(crate::owned_sab::Sab::parse).transpose()?,
            bitplanenoise: transform.bitplanenoise.as_deref().map(crate::owned_bitplanenoise::BitPlaneNoise::parse).transpose()?,
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
        })
    }
}
