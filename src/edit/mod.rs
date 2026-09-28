//! Non-interactive media editing model and plan selection.
//!
//! This module describes editing intent. It does not demux, decode, process
//! frames, or mux output; those responsibilities belong to `container`,
//! `codec`, `y4m`, and the execution backends.

mod concat;
mod plan;
mod transform;
mod trim;

pub use plan::{execution_mode, ExecutionMode};
pub use transform::{CropRect, Transform};
pub use trim::TimeRange;

/// Editing intent. Operations are applied in declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Trim(TimeRange),
    Concat,
    Transform(Transform),
}

/// A small, explainable editing request.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EditProject {
    operations: Vec<Operation>,
}

impl EditProject {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn trim(mut self, range: TimeRange) -> Self {
        self.operations.push(Operation::Trim(range));
        self
    }

    pub fn transform(mut self, transform: Transform) -> Self {
        self.operations.push(Operation::Transform(transform));
        self
    }

    pub fn concat(mut self) -> Self {
        self.operations.push(Operation::Concat);
        self
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn execution_mode(&self) -> ExecutionMode {
        execution_mode(&self.operations)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn geometry_requires_decode() {
        let crop = CropRect::new(0, 0, 640, 360).unwrap();
        let project = EditProject::new().transform(Transform::Crop(crop));
        assert_eq!(project.execution_mode(), ExecutionMode::DecodeProcessEncode);
    }

    #[test]
    fn trim_can_use_packet_copy() {
        let range = TimeRange::new(Duration::from_secs(1), Duration::from_secs(2)).unwrap();
        let project = EditProject::new().trim(range);
        assert_eq!(project.execution_mode(), ExecutionMode::PacketCopy);
    }
}
