/// A rectangle in decoded video coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CropRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CropRect {
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Option<Self> {
        (width > 0 && height > 0).then_some(Self { x, y, width, height })
    }
}

/// Geometry operations which require decoded frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transform {
    Crop(CropRect),
    HorizontalFlip,
    VerticalFlip,
}
