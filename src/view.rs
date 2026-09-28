//! Borrowed CPU frame geometry. Creating views never copies pixel payloads.
use crate::y4m::{Plan, Plane};

/// A transformed frame borrowing both the checked plan and the original storage.
/// The source cannot be mutated or recycled while a view remains in use.
#[derive(Clone, Copy)]
pub struct FrameView<'a> {
    plan: &'a Plan,
    input: &'a [u8],
}
impl<'a> FrameView<'a> {
    pub(crate) fn new(plan: &'a Plan, input: &'a [u8]) -> Self {
        Self { plan, input }
    }
    pub fn width(&self) -> usize {
        self.plan.width
    }
    pub fn height(&self) -> usize {
        self.plan.height
    }
    /// Y, U, V, in that order. Invalid indices return `None`.
    pub fn plane(&self, index: usize) -> Option<PlaneView<'a>> {
        self.plan.planes.get(index).map(|plane| PlaneView {
            plane,
            input: self.input,
            horizontal: self.plan.horizontal,
            vertical: self.plan.vertical,
        })
    }
}

#[derive(Clone, Copy)]
pub struct PlaneView<'a> {
    plane: &'a Plane,
    input: &'a [u8],
    horizontal: bool,
    vertical: bool,
}
impl<'a> PlaneView<'a> {
    pub fn width(&self) -> usize {
        self.plane.width
    }
    pub fn height(&self) -> usize {
        self.plane.height
    }
    /// A row in output order, still backed by a slice of the input allocation.
    pub fn row(&self, index: usize) -> Option<RowView<'a>> {
        let p = self.plane;
        if index >= p.height {
            return None;
        }
        let row = p.y
            + if self.vertical {
                p.height - 1 - index
            } else {
                index
            };
        let start = p.input_offset + row * p.stride + p.x;
        Some(RowView {
            bytes: &self.input[start..start + p.width],
            reversed: self.horizontal,
        })
    }
}

#[derive(Clone, Copy)]
pub struct RowView<'a> {
    bytes: &'a [u8],
    reversed: bool,
}
impl<'a> RowView<'a> {
    /// Original ascending-address bytes; consult `is_reversed` for orientation.
    pub fn storage_bytes(&self) -> &'a [u8] {
        self.bytes
    }
    pub fn is_reversed(&self) -> bool {
        self.reversed
    }
    /// Iterate in visible pixel order without making a reversed copy.
    pub fn pixels(&self) -> impl ExactSizeIterator<Item = u8> + DoubleEndedIterator + use<'a> {
        let bytes = self.bytes;
        let reversed = self.reversed;
        (0..bytes.len()).map(move |i| bytes[if reversed { bytes.len() - 1 - i } else { i }])
    }
}
