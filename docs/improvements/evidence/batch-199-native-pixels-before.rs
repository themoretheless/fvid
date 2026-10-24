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

use fvid_media::owned_expression as gradient_expression;
include!("../crates/fvid-media/src/owned_gradient_impl.rs");

/// Native filter order matches the public media request, independent of CLI
/// flag order: equalization, unsharp, hue, Gaussian blur, average blur, box blur, inversion, Sobel, Prewitt, Roberts, Kirsch, Scharr, monochrome, pixelize, dilation, erosion, colorize, chroma shift, plane shuffle.
#[derive(Default)]
pub struct PixelFilters {
    /// Optional (pixel aspect, nominal frame rate) for owned shading and drawing expressions.
    pub vignette_source: std::cell::Cell<Option<(f64, f64)>>,
    pub hqdn3d: Option<fvid_media::owned_hqdn3d::HqDn3d>,
    pub tmix: Option<fvid_media::owned_tmix::TemporalMix>,
    pub lagfun: Option<fvid_media::owned_lagfun::LagFun>,
    pub fade: Option<fvid_media::owned_fade::Fade>,
    pub fade_state: std::cell::Cell<fvid_media::owned_fade::FadeState>,
    pub colorhold: Option<fvid_media::owned_colorhold::ColorHold>,
    pub colorcontrast: Option<fvid_media::owned_colorcontrast::ColorContrast>,
    pub vibrance: Option<fvid_media::owned_vibrance::Vibrance>,
    pub colorlevels: Option<fvid_media::owned_colorlevels::ColorLevels>,
    pub colorchannelmixer: Option<fvid_media::owned_colorchannelmixer::ColorChannelMixer>,
    pub grayworld: Option<fvid_media::owned_timeline::Timeline>,
    pub cas: Option<fvid_media::owned_cas::Cas>,
    pub colorcorrect: Option<fvid_media::owned_colorcorrect::ColorCorrect>,
    pub sab: Option<fvid_media::owned_sab::Sab>,
    pub bitplanenoise: Option<fvid_media::owned_bitplanenoise::BitPlaneNoise>,
    pub deband: Option<fvid_media::owned_deband::Deband>,
    pub perspective: Option<fvid_media::owned_perspective::Perspective>,
    pub gradfun: Option<fvid_media::owned_gradfun::GradFun>,
    pub lenscorrection: Option<fvid_media::owned_lenscorrection::LensCorrection>,
    pub drawbox: Option<fvid_media::owned_draw::Draw>,
    pub drawgrid: Option<fvid_media::owned_draw::Draw>,
    pub removegrain: Option<fvid_media::owned_removegrain::RemoveGrain>,
    pub yaepblur: Option<fvid_media::owned_yaepblur::YaepBlur>,
    pub smartblur: Option<fvid_media::owned_smartblur::SmartBlur>,
    pub vignette: Option<fvid_media::owned_vignette::Vignette>,
    pub curves: Option<fvid_media::owned_curves::Curves>,
    pub colorbalance: Option<fvid_media::owned_colorbalance::ColorBalance>,
    pub exposure: Option<fvid_media::owned_exposure::Exposure>,
    pub lutyuv: Option<fvid_media::owned_lutyuv::LutYuv>,
    pub unsharp: Option<fvid_media::owned_unsharp::Unsharp>,
    pub eq: Option<fvid_media::owned_eq::EqualizerProgram>,
    pub hue: Option<fvid_media::owned_hue::HueProgram>,
    pub monochrome: Option<fvid_media::owned_monochrome::Monochrome>,
    pub colorize: Option<fvid_media::owned_colorize::Colorize>,
    pub pixelize: Option<crate::native_pixelize::Pixelize>,
    pub boxblur: Option<crate::native_boxblur::BoxBlurProgram>,
    pub bilateral: Option<fvid_media::owned_bilateral::Bilateral>,
    pub gblur: Option<fvid_media::owned_gblur::GaussianBlur>,
    pub avgblur: Option<crate::native_avgblur::AverageBlur>,
    pub negate: Option<Negate>,
    pub chromashift: Option<crate::native_chromashift::ChromaShift>,
    pub gradients: Vec<Gradient>,
    pub morphology: Vec<crate::native_morphology::Morphology>,
    pub shuffleplanes: Option<crate::native_shuffleplanes::ShufflePlanes>,
}
impl PixelFilters {
    pub(crate) fn configure_vignette_source<R: std::io::BufRead + std::io::Seek>(&self, reader: &crate::playback_native::NativeReader<R>, geometry: &crate::native_geometry::VideoGeometry) -> Result<()> {
        if self.vignette.is_none() && self.drawbox.is_none() && self.drawgrid.is_none() {return Ok(());}
        let [w,h]=reader.dimensions();
        let aspect=crate::native_export::transformed_aspect(reader.pixel_aspect(),w,h,geometry)?;
        self.vignette_source.set(Some((aspect.0 as f64/aspect.1 as f64,reader.frame_period().as_secs_f64().recip())));
        Ok(())
    }
    pub fn from_request(request: &crate::media_info::DecodeTransform) -> Result<Self> {
        let mut result = Self {

            vignette_source: Default::default(),
            fade_state: Default::default(),
            hqdn3d: request.hqdn3d.as_deref().map(fvid_media::owned_hqdn3d::HqDn3d::parse).transpose().map_err(|e|invalid(&e))?,
            tmix: request.tmix.as_deref().map(fvid_media::owned_tmix::TemporalMix::parse).transpose().map_err(|e|invalid(&e))?,
            lagfun: request.lagfun.as_deref().map(fvid_media::owned_lagfun::LagFun::parse).transpose().map_err(|e|invalid(&e))?,
            fade: request.fade.as_deref().map(fvid_media::owned_fade::Fade::parse).transpose().map_err(|e|invalid(&e))?,
            grayworld: request.grayworld.as_deref().map(fvid_media::owned_timeline::Timeline::grayworld).transpose().map_err(|e|invalid(&e))?,
            cas: request.cas.as_deref().map(fvid_media::owned_cas::Cas::parse).transpose().map_err(|e|invalid(&e))?,
            colorcorrect: request.colorcorrect.as_deref().map(fvid_media::owned_colorcorrect::ColorCorrect::parse).transpose().map_err(|e|invalid(&e))?,
            sab: request.sab.as_deref().map(fvid_media::owned_sab::Sab::parse).transpose().map_err(|e|invalid(&e))?,
            bitplanenoise: request.bitplanenoise.as_deref().map(fvid_media::owned_bitplanenoise::BitPlaneNoise::parse).transpose().map_err(|e|invalid(&e))?,
            deband: request.deband.as_deref().map(fvid_media::owned_deband::Deband::parse).transpose().map_err(|e|invalid(&e))?,
            perspective: request.perspective.as_deref().map(fvid_media::owned_perspective::Perspective::parse).transpose().map_err(|e|invalid(&e))?,
            gradfun: request.gradfun.as_deref().map(fvid_media::owned_gradfun::GradFun::parse).transpose().map_err(|e|invalid(&e))?,
            lenscorrection: request.lenscorrection.as_deref().map(fvid_media::owned_lenscorrection::LensCorrection::parse).transpose().map_err(|e|invalid(&e))?,
            drawbox: request.drawbox.as_deref().map(fvid_media::owned_draw::Draw::box_filter).transpose().map_err(|e|invalid(&e))?,
            drawgrid: request.drawgrid.as_deref().map(fvid_media::owned_draw::Draw::grid_filter).transpose().map_err(|e|invalid(&e))?,
            removegrain: request.removegrain.as_deref().map(fvid_media::owned_removegrain::RemoveGrain::parse).transpose().map_err(|e|invalid(&e))?,
            yaepblur: request.yaepblur.as_deref().map(fvid_media::owned_yaepblur::YaepBlur::parse).transpose().map_err(|e|invalid(&e))?,
            smartblur: request.smartblur.as_deref().map(fvid_media::owned_smartblur::SmartBlur::parse).transpose().map_err(|e|invalid(&e))?,
            vignette: request.vignette.as_deref().map(fvid_media::owned_vignette::Vignette::parse).transpose().map_err(|e|invalid(&e))?,
            curves: request.curves.as_deref().map(fvid_media::owned_curves::Curves::parse).transpose().map_err(|e|invalid(&e))?,
            colorbalance: request.colorbalance.as_deref().map(fvid_media::owned_colorbalance::ColorBalance::parse).transpose().map_err(|e|invalid(&e))?,
            exposure: request.exposure.as_deref().map(fvid_media::owned_exposure::Exposure::parse).transpose().map_err(|e|invalid(&e))?,
            colorchannelmixer: request.colorchannelmixer.as_deref().map(fvid_media::owned_colorchannelmixer::ColorChannelMixer::parse).transpose().map_err(|e|invalid(&e))?,
            colorlevels: request.colorlevels.as_deref().map(fvid_media::owned_colorlevels::ColorLevels::parse).transpose().map_err(|e|invalid(&e))?,
            vibrance: request.vibrance.as_deref().map(fvid_media::owned_vibrance::Vibrance::parse).transpose().map_err(|e|invalid(&e))?,
            colorcontrast: request.colorcontrast.as_deref().map(fvid_media::owned_colorcontrast::ColorContrast::parse).transpose().map_err(|e| invalid(&e))?,
            colorhold: request.colorhold.as_deref().map(fvid_media::owned_colorhold::ColorHold::parse).transpose().map_err(|e| invalid(&e))?,
            lutyuv: request.lutyuv.as_deref().map(fvid_media::owned_lutyuv::LutYuv::parse).transpose().map_err(|e| invalid(&e))?,
            monochrome: request.monochrome.as_deref().map(fvid_media::owned_monochrome::Monochrome::parse).transpose().map_err(|e| invalid(&e))?,
            colorize: request.colorize.as_deref().map(fvid_media::owned_colorize::Colorize::parse).transpose().map_err(|e| invalid(&e))?,
            unsharp: request
                .unsharp
                .as_deref()
                .map(fvid_media::owned_unsharp::Unsharp::parse)
                .transpose()
                .map_err(|error| invalid(&error))?,
            eq: request
                .eq
                .as_deref()
                .map(fvid_media::owned_eq::EqualizerProgram::parse)
                .transpose()
                .map_err(|error| invalid(&error))?,
            hue: request
                .hue
                .as_deref()
                .map(fvid_media::owned_hue::HueProgram::parse)
                .transpose()
                .map_err(|error| invalid(&error))?,
            pixelize: request.pixelize.as_deref().map(crate::native_pixelize::Pixelize::parse).transpose()?,
            boxblur: request
                .boxblur
                .as_deref()
                .map(crate::native_boxblur::BoxBlurProgram::parse)
                .transpose()?,
            bilateral: request.bilateral.as_deref().map(fvid_media::owned_bilateral::Bilateral::parse).transpose().map_err(|e| invalid(&e))?,
            gblur: request.gblur.as_deref().map(fvid_media::owned_gblur::GaussianBlur::parse).transpose().map_err(|e| invalid(&e))?,
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
        self.hqdn3d.is_none() && self.tmix.is_none() && self.lagfun.is_none() && self.unsharp.is_none()
            && self.eq.is_none()
            && self.hue.is_none()
            && self.pixelize.is_none()
            && self.boxblur.is_none()
            && self.monochrome.is_none()
            && self.lutyuv.is_none()
            && self.colorhold.is_none()
            && self.colorcontrast.is_none()
            && self.vibrance.is_none()
            && self.colorlevels.is_none()
            && self.colorchannelmixer.is_none()
            && self.fade.is_none()
            && self.exposure.is_none()
            && self.colorbalance.is_none()
            && self.sab.is_none()
            && self.bitplanenoise.is_none()
            && self.deband.is_none()
            && self.perspective.is_none()
            && self.gradfun.is_none()
            && self.lenscorrection.is_none()
            && self.drawbox.is_none()
            && self.drawgrid.is_none()
            && self.removegrain.is_none()
            && self.yaepblur.is_none()
            && self.smartblur.is_none()
            && self.vignette.is_none()
            && self.curves.is_none()
            && self.colorcorrect.is_none()
            && self.cas.is_none()
            && self.grayworld.is_none()
            && self.colorize.is_none()
            && self.bilateral.is_none()
            && self.gblur.is_none()
            && self.avgblur.is_none()
            && self.chromashift.is_none()
            && self.negate.is_none()
            && self.gradients.is_empty()
            && self.morphology.is_empty()
            && self.shuffleplanes.is_none()
    }
    pub fn apply(&self, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
        self.apply_range(frame, depth, false)
    }
    pub fn apply_range(&self, frame: &mut GeometryFrame, depth: u8, full_range: bool,
    ) -> Result<()> {
        self.apply_colour(frame, depth, full_range, 6)
    }
    pub fn apply_colour(&self, frame: &mut GeometryFrame, depth: u8, full_range: bool, matrix_code:u8,
    ) -> Result<()> {
        self.apply_colour_at(frame,depth,full_range,matrix_code,0,None)
    }
    pub fn apply_colour_at(&self, frame: &mut GeometryFrame, depth:u8, full_range:bool, matrix_code:u8, n:u64, t:Option<f64>,
    ) -> Result<()> {
        let clock = t
            .map(fvid_media::owned_fade::FrameTime::from_seconds)
            .transpose()
            .map_err(|e| invalid(&e))?;
        self.apply_colour_clock(frame, depth, full_range, matrix_code, n, t, clock)
    }
    #[expect(clippy::too_many_arguments, reason = "Preserve the public export/filter entrypoint signature for existing callers")]
    pub fn apply_colour_clock(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        full_range: bool,
        matrix_code: u8,
        n: u64,
        t: Option<f64>,
        clock: Option<fvid_media::owned_fade::FrameTime>,
    )->Result<()> {
        if let Some(filter)=&self.tmix {filter.apply(frame,depth,n).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.hqdn3d {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter) = &self.eq {
            filter.apply(frame, depth,n,t).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.unsharp {
            filter.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = &self.hue {
            filter.at(n,t).map_err(|error| invalid(&error))?.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.gblur {
            filter.apply(frame, depth).map_err(|e| invalid(&e))?;
        }
        if let Some(filter) = self.avgblur {
            filter.apply(frame, depth)?;
        }
        if let Some(filter) = &self.boxblur {
            filter.apply(frame, depth)?;
        }
        if let Some(negate) = self.negate {
            negate.apply(frame, depth)?;
        }
        for filter in &self.gradients {
            filter.apply(frame, depth)?;
        }
        if let Some(filter)=&self.smartblur {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.sab {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter) = self.bilateral {
            filter.apply(frame, depth).map_err(|e| invalid(&e))?;
        }
        if let Some(filter)=self.cas {filter.apply(frame,depth).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.vignette {filter.apply_clock(frame,depth,fvid_media::owned_vignette::Clock {
                n,t,pts:clock.map(|c|c.ticks as f64/c.quantum as f64),
                time_base:clock.map(|c|c.quantum as f64/c.scale as f64),
                rate:self.vignette_source.get().map(|(_,r)|r),
                sample_aspect:self.vignette_source.get().map_or(1.,|(sar,_)|sar),
            }).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.curves {
            if frame.subsampling.is_none() {filter.apply_rgb_clock(&mut frame.data,depth,3,frame.width,frame.height,n,t).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix,n,t).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter)=&self.colorbalance {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter)=&self.colorlevels {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter)=&self.colorchannelmixer {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter) = self.monochrome {filter.apply(frame,depth).map_err(|e|invalid(&e))?;}
        if let Some(timeline)=&self.grayworld
            && timeline.enabled(n,t,frame.width,frame.height).map_err(|e|invalid(&e))? {
            let filter=fvid_media::owned_grayworld::GrayWorld;
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,frame.width,frame.height,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        let drawing_sar=self.vignette_source.get().map_or(1.,|(sar,_)|sar);
        if let Some(filter)=&self.drawbox {filter.apply_with_aspect(frame,depth,drawing_sar,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.drawgrid {filter.apply_with_aspect(frame,depth,drawing_sar,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.lagfun {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.bitplanenoise {let _=filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.deband {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.gradfun {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.lenscorrection {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=self.pixelize {filter.apply(frame,depth)?;}
        if let Some(filter)=&self.removegrain {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.yaepblur {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        if let Some(filter)=&self.vibrance {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        for filter in &self.morphology {
            filter.apply(frame, depth)?;
        }
        if let Some(filter) = self.colorize {
            filter.apply(frame, depth).map_err(|e| invalid(&e))?;
        }
        if let Some(filter)=&self.exposure {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter) = self.chromashift {
            filter.apply(frame, depth)?;
        }
        if let Some(filter) = &self.colorcontrast {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter)=self.colorcorrect {filter.apply(frame,depth).map_err(|e|invalid(&e))?;}
        if let Some(filter)=self.shuffleplanes {crate::native_shuffleplanes::apply(filter,frame,depth)?;}
        if let Some(filter) = &self.lutyuv {filter.apply(frame,depth,full_range).map_err(|e|invalid(&e))?;}
        if let Some(filter) = &self.colorhold {
            if frame.subsampling.is_none() {filter.apply_rgb(&mut frame.data,depth,3).map_err(|e|invalid(&e))?;}
            else {let matrix=fvid_media::owned_yuv_rgb::Matrix::from_code(matrix_code).map_err(|e|invalid(&e))?;filter.apply_yuv(frame,depth,full_range,matrix).map_err(|e|invalid(&e))?;}
        }
        if let Some(filter)=self.fade {
            let mut state = self.fade_state.get();
            let evaluated = filter.at(n, clock, &mut state).map_err(|e| invalid(&e))?;
            evaluated
                .apply_colour(frame,depth,full_range,matrix_code,n).map_err(|e|invalid(&e))?;
            self.fade_state.set(state); }
        if let Some(filter)=&self.perspective {filter.apply(frame,depth,n,t).map_err(|e|invalid(&e))?;}
        Ok(())
    }
}

#[cfg(test)]
mod colorize_tests {
    #[test]
    fn colorize_native_decode_and_lossless_export_accept_high_depth_odd_frames() {
        use crate::playback_native::NativeReader;
        use std::io::{BufReader, Cursor};
        for depth in [8u8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/colorize-grid-{depth}.y4m"
            ));
            let request = crate::media_info::DecodeTransform {
                colorize: Some("hue=0:saturation=1:lightness=0.5:mix=1".into()),
                ..Default::default()
            };
            assert!(crate::native_media::supports_video_request(&request));
            let lossless = crate::media_info::LosslessTransform {
                colorize: request.colorize.clone(),
                ..Default::default()
            };
            assert!(crate::native_lossless::supports(&lossless));
            let (geometry, filters) = crate::native_lossless::configuration(&lossless).unwrap();
            assert!(!filters.is_empty());
            assert_eq!(
                crate::native_media::decode_video_pipeline(&source, None, &geometry, &filters)
                    .unwrap()
                    .video_frames,
                3
            );
            let mut output = Cursor::new(Vec::new());
            let (stats, _) = crate::native_lossless_y4m::write_processed(
                &source,
                &mut output,
                &geometry,
                &filters,
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(stats.video_frames, 3);
            output.set_position(0);
            let mut reader = NativeReader::software(BufReader::new(output), usize::MAX).unwrap();
            let mut index = 0u64;
            while let Some(raw) = reader.read_frame_raw().unwrap() {
                let frame = geometry.apply(&raw, 3, 3).unwrap();
                let samples: Vec<u16> = if depth == 8 {
                    frame.data.iter().map(|s| *s as u16).collect()
                } else {
                    frame
                        .data
                        .as_chunks::<2>().0.iter()
                        .map(|s| u16::from_le_bytes([s[0], s[1]]))
                        .collect()
                };
                let maximum = (1u64 << depth) - 1;
                let expected: Vec<u16> = (0..9)
                    .map(|n| ((maximum * n / 8 + index) % (maximum + 1)) as u16)
                    .collect();
                assert_eq!(&samples[..9], expected);
                let u = ((0.5 - 0.11457 * 224.0 / 255.0) * maximum as f64) as u16;
                let v = ((0.5 + 0.5 * 224.0 / 255.0) * maximum as f64) as u16;
                assert_eq!(&samples[9..13], &[u; 4]);
                assert_eq!(&samples[13..], &[v; 4]);
                index += 1;
            }
            assert_eq!(index, 3);
        }
    }
}

#[cfg(test)]
mod monochrome_tests {
    #[test]
    fn monochrome_native_decode_and_lossless_export_accept_high_depth_odd_frames() {
        use crate::playback_native::NativeReader;
        use std::io::{BufReader, Cursor};
        for depth in [8u8, 12, 16] {
            let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                "tests/fixtures/playback-errors/monochrome-grid-{depth}.y4m"
            ));
            let request = crate::media_info::DecodeTransform {
                monochrome: Some("cb=0.5:cr=-0.5:size=0.2:high=0.75".into()),
                ..Default::default()
            };
            assert!(crate::native_media::supports_video_request(&request));
            let lossless = crate::media_info::LosslessTransform {
                lutyuv: request.lutyuv.clone(),
            monochrome: request.monochrome.clone(),
                ..Default::default()
            };
            assert!(crate::native_lossless::supports(&lossless));
            let (geometry, filters) = crate::native_lossless::configuration(&lossless).unwrap();
            assert!(!filters.is_empty());
            assert_eq!(
                crate::native_media::decode_video_pipeline(&source, None, &geometry, &filters)
                    .unwrap()
                    .video_frames,
                3
            );
            let mut output = Cursor::new(Vec::new());
            let (stats, _) = crate::native_lossless_y4m::write_processed(
                &source,
                &mut output,
                &geometry,
                &filters,
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(stats.video_frames, 3);
            output.set_position(0);
            let mut reader = NativeReader::software(BufReader::new(output), usize::MAX).unwrap();
            let mut index = 0u64;
            while let Some(raw) = reader.read_frame_raw().unwrap() {
                let frame = geometry.apply(&raw, 3, 3).unwrap();
                let samples: Vec<u16> = if depth == 8 {
                    frame.data.iter().map(|s| *s as u16).collect()
                } else {
                    frame
                        .data
                        .as_chunks::<2>().0.iter()
                        .map(|s| u16::from_le_bytes([s[0], s[1]]))
                        .collect()
                };
                let maximum = (1u64 << depth) - 1;
                let expected: Vec<u16> = (0..9)
                    .map(|n| ((maximum * n / 8 + index) % (maximum + 1)) as u16)
                    .collect();
                assert!(samples[..9].iter().zip(&expected).all(|(a, b)| a <= b));
                assert!(samples[..9].iter().zip(&expected).any(|(a, b)| a < b));
                let center = 1u16 << (depth - 1);
                assert_eq!(&samples[9..13], &[center; 4]);
                assert_eq!(&samples[13..], &[center; 4]);
                index += 1;
            }
            assert_eq!(index, 3);
        }
    }
}

#[cfg(test)]
mod colorhold_tests {
    use super::*;
    #[test]
    fn native_colour_metadata_selects_matrix_and_preserves_selected_samples() {
        let request=crate::media_info::DecodeTransform{colorhold:Some("red:0.02".into()),..Default::default()};
        assert!(crate::native_media::supports_video_request(&request));
        let filters=PixelFilters::from_request(&request).unwrap();
        let mut frame=GeometryFrame{width:1,height:1,subsampling:Some([1,1]),data:vec![63,102,240],
        };
        filters.apply_colour(&mut frame,8,false,1).unwrap();assert_eq!(frame.data,[63,102,240]);
        filters.apply_colour(&mut frame,8,false,6).unwrap();assert_eq!(&frame.data[1..],&[128,128]);
        let before=frame.data.clone();assert!(filters.apply_colour(&mut frame,8,false,10).is_err());assert_eq!(frame.data,before);
    }
}


/// Convert the reader's exact presentation clock without floating-point loss.
pub fn frame_clock<R: std::io::BufRead + std::io::Seek>(
    reader: &crate::playback_native::NativeReader<R>,
) -> Result<Option<fvid_media::owned_fade::FrameTime>> {
    reader
        .frame_interval()
        .map(|(start, _, scale)| {
            fvid_media::owned_fade::FrameTime::new(start, u64::from(scale))?
                .with_quantum(reader.frame_clock_quantum())
        })
        .transpose()
        .map_err(|e| invalid(&e))
}
