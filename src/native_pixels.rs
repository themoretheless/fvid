//! Owned pointwise operations on packed decoded sample planes.
use crate::{Result, invalid, native_geometry::GeometryFrame};
mod overlay;
pub use overlay::{overlay_opaque, overlay_opaque_depth};

/// Invert every colour sample. The legacy `1` option includes alpha; native
/// RGB24 and YUV frames have no alpha, so both modes produce the same pixels.
#[derive(Clone, Copy, Debug)]
pub struct Negate;

impl Negate {
    pub fn parse(args: &str) -> Result<Self> {
        fvid_media::owned_negate::Negate::parse(args).map_err(|error| invalid(&error))?;
        Ok(Self)
    }
    pub fn apply(self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if frame.subsampling.is_none() && depth != 8 {
            return Err(invalid("unsupported negate sample depth"));
        }
        fvid_media::owned_negate::Negate
            .apply(&mut frame.data, depth)
            .map_err(|error| invalid(&error))
    }
}

include!("../crates/fvid-media/src/owned_gradient_impl.rs");

/// Native filter order matches the public media request, independent of CLI
/// flag order: equalization, hue, average blur, box blur, inversion, Sobel, Prewitt, Roberts, Kirsch, Scharr, pixelize, dilation, erosion, chroma shift, plane shuffle.
#[derive(Default)]
pub struct PixelFilters {
    pub eq: Option<fvid_media::owned_eq::Equalizer>,
    pub hue: Option<fvid_media::owned_hue::Hue>,
    pub pixelize: Option<crate::native_pixelize::Pixelize>,
    pub boxblur: Option<crate::native_boxblur::BoxBlur>,
    pub avgblur: Option<crate::native_avgblur::AverageBlur>,
    pub negate: Option<Negate>,
    pub chromashift: Option<crate::native_chromashift::ChromaShift>,
    pub gradients: Vec<Gradient>,
    pub morphology: Vec<crate::native_morphology::Morphology>,
    pub shuffleplanes: Option<crate::native_shuffleplanes::ShufflePlanes>,
}
impl PixelFilters {
    pub fn from_request(request: &crate::media_info::DecodeTransform) -> Result<Self> {
        let mut result = Self {
            eq: request
                .eq
                .as_deref()
                .map(fvid_media::owned_eq::Equalizer::parse)
                .transpose()
                .map_err(|error| invalid(&error))?,
            hue: request
                .hue
                .as_deref()
                .map(fvid_media::owned_hue::Hue::parse)
                .transpose()
                .map_err(|error| invalid(&error))?,
            pixelize: request.pixelize.as_deref().map(crate::native_pixelize::Pixelize::parse).transpose()?,
            boxblur: request
                .boxblur
                .as_deref()
                .map(crate::native_boxblur::BoxBlur::parse)
                .transpose()?,
            avgblur: request
                .avgblur
                .as_deref()
                .map(crate::native_avgblur::AverageBlur::parse)
                .transpose()?,
            negate: request.negate.as_deref().map(Negate::parse).transpose()?,
            chromashift: request
                .chromashift
                .as_deref()
                .map(crate::native_chromashift::ChromaShift::parse)
                .transpose()?,
            gradients: Vec::new(),
            morphology: Vec::new(),
            shuffleplanes: request.shuffleplanes.as_deref().map(crate::native_shuffleplanes::ShufflePlanes::parse).transpose().map_err(|e|invalid(&e))?,
        };
        for (kind, args) in [
            (GradientKind::Sobel, &request.sobel),
            (GradientKind::Prewitt, &request.prewitt),
            (GradientKind::Roberts, &request.roberts),
            (GradientKind::Kirsch, &request.kirsch),
            (GradientKind::Scharr, &request.scharr),
        ] {
            if let Some(args) = args {
                result.gradients.push(Gradient::parse(kind, args)?);
            }
        }
        use crate::native_morphology::{Morphology, MorphologyKind};
        for (kind, args) in [
            (MorphologyKind::Dilation, &request.dilation),
            (MorphologyKind::Erosion, &request.erosion),
        ] {
            if let Some(args) = args {
                result.morphology.push(Morphology::parse(kind, args)?);
            }
        }
        Ok(result)
    }
    /// Match the fixed option order used by media transform requests.
    /// Explicit callers can otherwise keep their vector pipeline order.
    pub fn canonicalize_option_order(&mut self) {
        self.gradients.sort_by_key(|filter|filter.kind);
        self.morphology.sort_by_key(|filter|filter.kind());
    }

    pub fn is_empty(&self) -> bool {
        self.eq.is_none()
            && self.hue.is_none()
            && self.pixelize.is_none()
            && self.boxblur.is_none()
            && self.avgblur.is_none()
            && self.chromashift.is_none()
            && self.negate.is_none()
            && self.gradients.is_empty()
            && self.morphology.is_empty()
            && self.shuffleplanes.is_none()
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        if let Some(filter) = &self.eq {
            filter.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.hue {
            filter.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.avgblur {
            filter.apply(frame, depth)?;
        }
        if let Some(filter) = self.boxblur {
            filter.apply(frame, depth)?;
        }
        if let Some(negate) = self.negate {
            negate.apply(frame, depth)?;
        }
        for filter in &self.gradients {
            filter.apply(frame, depth)?;
        }
        if let Some(filter)=self.pixelize {filter.apply(frame,depth)?;}
        for filter in &self.morphology {
            filter.apply(frame, depth)?;
        }
        if let Some(filter) = self.chromashift {
            filter.apply(frame, depth)?;
        }
        if let Some(filter)=self.shuffleplanes {crate::native_shuffleplanes::apply(filter,frame,depth)?;}
        Ok(())
    }
}
