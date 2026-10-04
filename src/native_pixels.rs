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
/// flag order: equalization, unsharp, hue, Gaussian blur, average blur, box blur, inversion, Sobel, Prewitt, Roberts, Kirsch, Scharr, monochrome, pixelize, dilation, erosion, colorize, chroma shift, plane shuffle.
#[derive(Default)]
pub struct PixelFilters {
    pub colorhold: Option<fvid_media::owned_colorhold::ColorHold>,
    pub colorcontrast: Option<fvid_media::owned_colorcontrast::ColorContrast>,
    pub vibrance: Option<fvid_media::owned_vibrance::Vibrance>,
    pub colorlevels: Option<fvid_media::owned_colorlevels::ColorLevels>,
    pub colorchannelmixer: Option<fvid_media::owned_colorchannelmixer::ColorChannelMixer>,
    pub colorcorrect: Option<fvid_media::owned_colorcorrect::ColorCorrect>,
    pub colorbalance: Option<fvid_media::owned_colorbalance::ColorBalance>,
    pub exposure: Option<fvid_media::owned_exposure::Exposure>,
    pub lutyuv: Option<fvid_media::owned_lutyuv::LutYuv>,
    pub unsharp: Option<fvid_media::owned_unsharp::Unsharp>,
    pub eq: Option<fvid_media::owned_eq::Equalizer>,
    pub hue: Option<fvid_media::owned_hue::Hue>,
    pub monochrome: Option<fvid_media::owned_monochrome::Monochrome>,
    pub colorize: Option<fvid_media::owned_colorize::Colorize>,
    pub pixelize: Option<crate::native_pixelize::Pixelize>,
    pub boxblur: Option<crate::native_boxblur::BoxBlur>,
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
    pub fn from_request(request: &crate::media_info::DecodeTransform) -> Result<Self> {
        let mut result = Self {
            colorcorrect: request.colorcorrect.as_deref().map(fvid_media::owned_colorcorrect::ColorCorrect::parse).transpose().map_err(|e|invalid(&e))?,
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
        self.unsharp.is_none()
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
            && self.exposure.is_none()
            && self.colorbalance.is_none()
            && self.colorcorrect.is_none()
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
    pub fn apply_range(&self, frame: &mut GeometryFrame, depth: u8, full_range: bool) -> Result<()> {
        self.apply_colour(frame, depth, full_range, 6)
    }
    pub fn apply_colour(&self, frame: &mut GeometryFrame, depth: u8, full_range: bool, matrix_code:u8) -> Result<()> {
        if let Some(filter) = &self.eq {
            filter.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.unsharp {
            filter.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.hue {
            filter.apply(frame, depth).map_err(|error| invalid(&error))?;
        }
        if let Some(filter) = self.gblur {
            filter.apply(frame, depth).map_err(|e| invalid(&e))?;
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
        if let Some(filter) = self.bilateral {
            filter.apply(frame, depth).map_err(|e| invalid(&e))?;
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
        if let Some(filter)=self.pixelize {filter.apply(frame,depth)?;}
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
        Ok(())
    }
}

#[cfg(test)]
mod colorize_tests {
    use super::*;
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
                        .chunks_exact(2)
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
    use super::*;
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
                        .chunks_exact(2)
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
        let mut frame=GeometryFrame{width:1,height:1,subsampling:Some([1,1]),data:vec![63,102,240]};
        filters.apply_colour(&mut frame,8,false,1).unwrap();assert_eq!(frame.data,[63,102,240]);
        filters.apply_colour(&mut frame,8,false,6).unwrap();assert_eq!(&frame.data[1..],&[128,128]);
        let before=frame.data.clone();assert!(filters.apply_colour(&mut frame,8,false,10).is_err());assert_eq!(frame.data,before);
    }
}
