//! Persistent history owned by one streaming filter chain.
#[derive(Debug)]
pub(crate) struct PixelHistory {
    pub lagfun: Option<crate::owned_lagfun::LagFun>,
    pub tmix: Option<crate::owned_tmix::TemporalMix>,
}
impl PixelHistory {
    pub fn parse(transform: &fvid_media_info::DecodeTransform) -> Result<Self, String> {
        Ok(Self {
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
