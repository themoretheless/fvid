//! Cached programs and persistent history owned by one streaming filter chain.
#[derive(Debug)]
pub(crate) struct PixelContext {
    pub boxblur: Option<crate::owned_boxblur::BoxBlurProgram>,
    pub curves: Option<crate::owned_curves::Curves>,
    pub eq: Option<crate::owned_eq::EqualizerProgram>,
    pub lagfun: Option<crate::owned_lagfun::LagFun>,
    pub hqdn3d: Option<crate::owned_hqdn3d::HqDn3d>,
    pub tmix: Option<crate::owned_tmix::TemporalMix>,
}
impl PixelContext {
    pub fn parse(transform: &fvid_media_info::DecodeTransform) -> Result<Self, String> {
        Ok(Self {
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
