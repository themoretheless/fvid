//! Video cropping filter.
//!
//! This filter crops video frames to a specified region, removing pixels
//! outside the crop area.

#![forbid(unsafe_code)]
#![allow(clippy::cast_lossless)]
#![allow(clippy::cast_precision_loss)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_sign_loss)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::similar_names)]
#![allow(clippy::many_single_char_names)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::match_same_arms)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::unused_self)]
#![allow(clippy::unnecessary_cast)]
#![allow(clippy::bool_to_int_with_if)]
#![allow(clippy::needless_range_loop)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unnecessary_wraps)]
#![allow(clippy::map_unwrap_or)]
#![allow(clippy::no_effect_underscore_binding)]
#![allow(clippy::unreadable_literal)]
#![allow(dead_code)]

use crate::error::{GraphError, GraphResult};
use crate::frame::FilterFrame;
use crate::node::{Node, NodeId, NodeState, NodeType};
use crate::port::{InputPort, OutputPort, PortFormat, PortId, PortType, VideoPortFormat};
use oximedia_codec::{Plane, VideoFrame};

/// Configuration for the crop filter.
#[derive(Clone, Debug)]
pub struct CropConfig {
    /// Left offset (pixels from left edge).
    pub left: u32,
    /// Top offset (pixels from top edge).
    pub top: u32,
    /// Crop width.
    pub width: u32,
    /// Crop height.
    pub height: u32,
    /// Automatically center the crop region.
    pub auto_center: bool,
    /// Preserve aspect ratio when cropping.
    pub preserve_aspect: bool,
    /// Target aspect ratio (width/height) when preserve_aspect is true.
    pub target_aspect: Option<f64>,
}

impl CropConfig {
    /// Create a new crop configuration.
    #[must_use]
    pub fn new(left: u32, top: u32, width: u32, height: u32) -> Self {
        Self {
            left,
            top,
            width,
            height,
            auto_center: false,
            preserve_aspect: false,
            target_aspect: None,
        }
    }

    /// Create a centered crop configuration.
    #[must_use]
    pub fn centered(width: u32, height: u32) -> Self {
        Self {
            left: 0,
            top: 0,
            width,
            height,
            auto_center: true,
            preserve_aspect: false,
            target_aspect: None,
        }
    }

    /// Create a crop configuration that preserves aspect ratio.
    #[must_use]
    pub fn with_aspect_ratio(width: u32, height: u32, aspect: f64) -> Self {
        Self {
            left: 0,
            top: 0,
            width,
            height,
            auto_center: true,
            preserve_aspect: true,
            target_aspect: Some(aspect),
        }
    }

    /// Enable auto-centering.
    #[must_use]
    pub fn with_auto_center(mut self, enabled: bool) -> Self {
        self.auto_center = enabled;
        self
    }

    /// Set target aspect ratio for preservation.
    #[must_use]
    pub fn with_target_aspect(mut self, aspect: f64) -> Self {
        self.preserve_aspect = true;
        self.target_aspect = Some(aspect);
        self
    }

    /// Validate the crop configuration against source dimensions.
    pub fn validate(&self, src_width: u32, src_height: u32) -> GraphResult<()> {
        if self.width == 0 || self.height == 0 {
            return Err(GraphError::ConfigurationError(
                "Crop dimensions cannot be zero".to_string(),
            ));
        }

        if !self.auto_center {
            if self.left + self.width > src_width {
                return Err(GraphError::ConfigurationError(format!(
                    "Crop region exceeds source width: {} + {} > {}",
                    self.left, self.width, src_width
                )));
            }

            if self.top + self.height > src_height {
                return Err(GraphError::ConfigurationError(format!(
                    "Crop region exceeds source height: {} + {} > {}",
                    self.top, self.height, src_height
                )));
            }
        }

        Ok(())
    }

    /// Calculate the actual crop region for given source dimensions.
    #[must_use]
    pub fn calculate_region(&self, src_width: u32, src_height: u32) -> CropRegion {
        let (width, height) = if self.preserve_aspect {
            if let Some(target_aspect) = self.target_aspect {
                calculate_aspect_crop(src_width, src_height, target_aspect)
            } else {
                (self.width.min(src_width), self.height.min(src_height))
            }
        } else {
            (self.width.min(src_width), self.height.min(src_height))
        };

        let (left, top) = if self.auto_center {
            let left = (src_width.saturating_sub(width)) / 2;
            let top = (src_height.saturating_sub(height)) / 2;
            (left, top)
        } else {
            (
                self.left.min(src_width.saturating_sub(width)),
                self.top.min(src_height.saturating_sub(height)),
            )
        };

        CropRegion {
            left,
            top,
            width,
            height,
        }
    }
}

/// Calculated crop region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRegion {
    /// Left offset.
    pub left: u32,
    /// Top offset.
    pub top: u32,
    /// Crop width.
    pub width: u32,
    /// Crop height.
    pub height: u32,
}

impl CropRegion {
    /// Get the right edge position.
    #[must_use]
    pub fn right(&self) -> u32 {
        self.left + self.width
    }

    /// Get the bottom edge position.
    #[must_use]
    pub fn bottom(&self) -> u32 {
        self.top + self.height
    }

    /// Check if a point is within the crop region.
    #[must_use]
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.left && x < self.right() && y >= self.top && y < self.bottom()
    }

    /// Scale the region for chroma planes.
    #[must_use]
    pub fn scale_for_chroma(&self, h_ratio: u32, v_ratio: u32) -> Self {
        Self {
            left: self.left / h_ratio,
            top: self.top / v_ratio,
            width: self.width / h_ratio,
            height: self.height / v_ratio,
        }
    }
}

/// Calculate crop dimensions to achieve target aspect ratio.
fn calculate_aspect_crop(src_width: u32, src_height: u32, target_aspect: f64) -> (u32, u32) {
    let src_aspect = src_width as f64 / src_height as f64;

    if src_aspect > target_aspect {
        // Source is wider, crop width
        let new_width = (src_height as f64 * target_aspect).round() as u32;
        (new_width, src_height)
    } else {
        // Source is taller, crop height
        let new_height = (src_width as f64 / target_aspect).round() as u32;
        (src_width, new_height)
    }
}

/// Video cropping filter.
///
/// Crops video frames to a specified region, with support for automatic
/// centering and aspect ratio preservation.
///
/// # Example
///
/// ```ignore
/// use oximedia_graph::filters::video::{CropFilter, CropConfig};
/// use oximedia_graph::node::NodeId;
///
/// // Create a centered crop
/// let config = CropConfig::centered(1280, 720);
/// let filter = CropFilter::new(NodeId(0), "crop", config);
///
/// // Create a crop for specific aspect ratio (16:9)
/// let config = CropConfig::with_aspect_ratio(0, 0, 16.0 / 9.0);
/// let filter = CropFilter::new(NodeId(1), "aspect_crop", config);
/// ```
pub struct CropFilter {
    id: NodeId,
    name: String,
    state: NodeState,
    inputs: Vec<InputPort>,
    outputs: Vec<OutputPort>,
    config: CropConfig,
}

impl CropFilter {
    /// Create a new crop filter.
    #[must_use]
    pub fn new(id: NodeId, name: impl Into<String>, config: CropConfig) -> Self {
        let output_format =
            PortFormat::Video(VideoPortFormat::any().with_dimensions(config.width, config.height));

        Self {
            id,
            name: name.into(),
            state: NodeState::Idle,
            inputs: vec![InputPort::new(PortId(0), "input", PortType::Video)
                .with_format(PortFormat::Video(VideoPortFormat::any()))],
            outputs: vec![
                OutputPort::new(PortId(0), "output", PortType::Video).with_format(output_format)
            ],
            config,
        }
    }

    /// Get the current configuration.
    #[must_use]
    pub fn config(&self) -> &CropConfig {
        &self.config
    }

    /// Update the crop configuration.
    pub fn set_config(&mut self, config: CropConfig) {
        self.config = config;
    }

    /// Crop a single plane.
    fn crop_plane(&self, src: &Plane, _src_width: u32, region: &CropRegion) -> Plane {
        let mut dst_data = vec![0u8; region.width as usize * region.height as usize];

        for y in 0..region.height as usize {
            let src_y = region.top as usize + y;
            let src_row = src.row(src_y);
            let dst_start = y * region.width as usize;

            for x in 0..region.width as usize {
                let src_x = region.left as usize + x;
                dst_data[dst_start + x] = src_row.get(src_x).copied().unwrap_or(0);
            }
        }

        Plane::new(dst_data, region.width as usize)
    }

    /// Crop a video frame.
    fn crop_frame(&self, input: &VideoFrame) -> GraphResult<VideoFrame> {
        let region = self.config.calculate_region(input.width, input.height);

        // Ensure crop region is within bounds
        if region.right() > input.width || region.bottom() > input.height {
            return Err(GraphError::ConfigurationError(
                "Crop region exceeds frame dimensions".to_string(),
            ));
        }

        let mut output = VideoFrame::new(input.format, region.width, region.height);
        output.timestamp = input.timestamp;
        output.frame_type = input.frame_type;
        output.color_info = input.color_info;

        for (i, src_plane) in input.planes.iter().enumerate() {
            let (src_w, _src_h) = input.plane_dimensions(i);

            let plane_region = if i > 0 && input.format.is_yuv() {
                let (h_ratio, v_ratio) = input.format.chroma_subsampling();
                region.scale_for_chroma(h_ratio, v_ratio)
            } else {
                region
            };

            let plane = self.crop_plane(src_plane, src_w, &plane_region);
            output.planes.push(plane);
        }

        Ok(output)
    }
}

impl Node for CropFilter {
    fn id(&self) -> NodeId {
        self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn node_type(&self) -> NodeType {
        NodeType::Filter
    }

    fn state(&self) -> NodeState {
        self.state
    }

    fn set_state(&mut self, state: NodeState) -> GraphResult<()> {
        if !self.state.can_transition_to(state) {
            return Err(GraphError::InvalidStateTransition {
                node: self.id,
                from: self.state.to_string(),
                to: state.to_string(),
            });
        }
        self.state = state;
        Ok(())
    }

    fn inputs(&self) -> &[InputPort] {
        &self.inputs
    }

    fn outputs(&self) -> &[OutputPort] {
        &self.outputs
    }

    fn process(&mut self, input: Option<FilterFrame>) -> GraphResult<Option<FilterFrame>> {
        match input {
            Some(FilterFrame::Video(frame)) => {
                let cropped = self.crop_frame(&frame)?;
                Ok(Some(FilterFrame::Video(cropped)))
            }
            Some(_) => Err(GraphError::PortTypeMismatch {
                expected: "Video".to_string(),
                actual: "Audio".to_string(),
            }),
            None => Ok(None),
        }
    }
}

/// Detect black borders in a frame for automatic cropping.
#[derive(Debug)]
pub struct BorderDetector {
    /// Threshold for considering a pixel as black.
    pub threshold: u8,
    /// Minimum number of consecutive black rows/columns to consider a border.
    pub min_border_size: u32,
}

impl Default for BorderDetector {
    fn default() -> Self {
        Self {
            threshold: 16,
            min_border_size: 4,
        }
    }
}

impl BorderDetector {
    /// Create a new border detector.
    #[must_use]
    pub fn new(threshold: u8, min_border_size: u32) -> Self {
        Self {
            threshold,
            min_border_size,
        }
    }

    /// Detect borders in a frame and return the crop region.
    #[must_use]
    pub fn detect(&self, frame: &VideoFrame) -> CropRegion {
        if frame.planes.is_empty() {
            return CropRegion {
                left: 0,
                top: 0,
                width: frame.width,
                height: frame.height,
            };
        }

        let luma = &frame.planes[0];

        // Detect top border
        let mut top = 0u32;
        for y in 0..frame.height {
            if !self.is_row_black(luma, y, frame.width) {
                top = y;
                break;
            }
        }

        // Detect bottom border
        let mut bottom = frame.height;
        for y in (0..frame.height).rev() {
            if !self.is_row_black(luma, y, frame.width) {
                bottom = y + 1;
                break;
            }
        }

        // Detect left border
        let mut left = 0u32;
        for x in 0..frame.width {
            if !self.is_column_black(luma, x, frame.height) {
                left = x;
                break;
            }
        }

        // Detect right border
        let mut right = frame.width;
        for x in (0..frame.width).rev() {
            if !self.is_column_black(luma, x, frame.height) {
                right = x + 1;
                break;
            }
        }

        // Apply minimum border size filter
        if top < self.min_border_size {
            top = 0;
        }
        if (frame.height - bottom) < self.min_border_size {
            bottom = frame.height;
        }
        if left < self.min_border_size {
            left = 0;
        }
        if (frame.width - right) < self.min_border_size {
            right = frame.width;
        }

        CropRegion {
            left,
            top,
            width: right.saturating_sub(left),
            height: bottom.saturating_sub(top),
        }
    }

    /// Check if a row is entirely black.
    fn is_row_black(&self, plane: &Plane, y: u32, width: u32) -> bool {
        let row = plane.row(y as usize);
        for x in 0..width as usize {
            if row.get(x).copied().unwrap_or(0) > self.threshold {
                return false;
            }
        }
        true
    }

    /// Check if a column is entirely black.
    fn is_column_black(&self, plane: &Plane, x: u32, height: u32) -> bool {
        for y in 0..height {
            let row = plane.row(y as usize);
            if row.get(x as usize).copied().unwrap_or(0) > self.threshold {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oximedia_core::PixelFormat;

    fn create_test_frame(width: u32, height: u32) -> VideoFrame {
        let mut frame = VideoFrame::new(PixelFormat::Yuv420p, width, height);
        frame.allocate();

        // Fill Y plane with a gradient
        if let Some(plane) = frame.planes.get_mut(0) {
            let mut data = vec![0u8; width as usize * height as usize];
            for y in 0..height as usize {
                for x in 0..width as usize {
                    data[y * width as usize + x] = ((x + y) % 256) as u8;
                }
            }
            *plane = Plane::new(data, width as usize);
        }

        frame
    }

    #[test]
    fn test_crop_config_creation() {
        let config = CropConfig::new(10, 20, 100, 80);
        assert_eq!(config.left, 10);
        assert_eq!(config.top, 20);
        assert_eq!(config.width, 100);
        assert_eq!(config.height, 80);
        assert!(!config.auto_center);
    }

    #[test]
    fn test_crop_config_centered() {
        let config = CropConfig::centered(640, 480);
        assert!(config.auto_center);
        assert_eq!(config.width, 640);
        assert_eq!(config.height, 480);
    }

    #[test]
    fn test_crop_config_with_aspect() {
        let config = CropConfig::with_aspect_ratio(0, 0, 16.0 / 9.0);
        assert!(config.preserve_aspect);
        assert!(config.target_aspect.is_some());
    }

    #[test]
    fn test_crop_region_calculation() {
        let config = CropConfig::centered(320, 240);
        let region = config.calculate_region(640, 480);

        assert_eq!(region.left, 160);
        assert_eq!(region.top, 120);
        assert_eq!(region.width, 320);
        assert_eq!(region.height, 240);
    }

    #[test]
    fn test_crop_region_contains() {
        let region = CropRegion {
            left: 10,
            top: 20,
            width: 100,
            height: 80,
        };

        assert!(region.contains(10, 20));
        assert!(region.contains(50, 50));
        assert!(region.contains(109, 99));
        assert!(!region.contains(9, 20));
        assert!(!region.contains(110, 50));
    }

    #[test]
    fn test_crop_region_scale_for_chroma() {
        let region = CropRegion {
            left: 100,
            top: 200,
            width: 400,
            height: 300,
        };

        let scaled = region.scale_for_chroma(2, 2);
        assert_eq!(scaled.left, 50);
        assert_eq!(scaled.top, 100);
        assert_eq!(scaled.width, 200);
        assert_eq!(scaled.height, 150);
    }

    #[test]
    fn test_crop_filter_creation() {
        let config = CropConfig::new(0, 0, 640, 480);
        let filter = CropFilter::new(NodeId(0), "crop", config);

        assert_eq!(filter.id(), NodeId(0));
        assert_eq!(filter.name(), "crop");
        assert_eq!(filter.node_type(), NodeType::Filter);
    }

    #[test]
    fn test_crop_filter_process() {
        let config = CropConfig::centered(320, 240);
        let mut filter = CropFilter::new(NodeId(0), "crop", config);

        let input = create_test_frame(640, 480);
        let result = filter
            .process(Some(FilterFrame::Video(input)))
            .expect("operation should succeed")
            .expect("operation should succeed");

        if let FilterFrame::Video(frame) = result {
            assert_eq!(frame.width, 320);
            assert_eq!(frame.height, 240);
        } else {
            panic!("Expected video frame");
        }
    }

    #[test]
    fn test_crop_config_validation() {
        let config = CropConfig::new(0, 0, 0, 100);
        assert!(config.validate(640, 480).is_err());

        let config = CropConfig::new(600, 0, 100, 100);
        assert!(config.validate(640, 480).is_err());

        let config = CropConfig::new(0, 0, 320, 240);
        assert!(config.validate(640, 480).is_ok());
    }

    #[test]
    fn test_aspect_crop_calculation() {
        // 4:3 source to 16:9 target (wider)
        let (w, h) = calculate_aspect_crop(640, 480, 16.0 / 9.0);
        let result_aspect = w as f64 / h as f64;
        assert!((result_aspect - 16.0 / 9.0).abs() < 0.01);

        // 16:9 source to 4:3 target (taller)
        let (w, h) = calculate_aspect_crop(1920, 1080, 4.0 / 3.0);
        let result_aspect = w as f64 / h as f64;
        assert!((result_aspect - 4.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn test_border_detector_default() {
        let detector = BorderDetector::default();
        assert_eq!(detector.threshold, 16);
        assert_eq!(detector.min_border_size, 4);
    }

    #[test]
    fn test_border_detector_on_test_frame() {
        let detector = BorderDetector::new(16, 1);
        let frame = create_test_frame(640, 480);

        let region = detector.detect(&frame);
        // Test frame has gradient, so no black borders
        assert!(region.width > 0);
        assert!(region.height > 0);
    }

    #[test]
    fn test_node_state_transitions() {
        let config = CropConfig::centered(320, 240);
        let mut filter = CropFilter::new(NodeId(0), "crop", config);

        assert_eq!(filter.state(), NodeState::Idle);
        filter
            .set_state(NodeState::Processing)
            .expect("set_state should succeed");
        assert_eq!(filter.state(), NodeState::Processing);
    }

    #[test]
    fn test_process_none_input() {
        let config = CropConfig::centered(320, 240);
        let mut filter = CropFilter::new(NodeId(0), "crop", config);

        let result = filter.process(None).expect("process should succeed");
        assert!(result.is_none());
    }
}
