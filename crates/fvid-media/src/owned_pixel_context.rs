//! Cached programs and persistent history owned by one streaming filter chain.
#[derive(Debug)]
pub(crate) struct PixelContext {
    pub boxblur: Option<crate::owned_boxblur::BoxBlurProgram>,
    pub eq: Option<crate::owned_eq::EqualizerProgram>,
    pub lagfun: Option<crate::owned_lagfun::LagFun>,
    pub tmix: Option<crate::owned_tmix::TemporalMix>,
}
impl PixelContext {
    pub fn parse(transform: &fvid_media_info::DecodeTransform) -> Result<Self, String> {
        Ok(Self {
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
