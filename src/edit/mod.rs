//! Non-interactive media editing model and plan selection.
//!
//! This module describes editing intent. It does not demux, decode, process
//! frames, or mux output; those responsibilities belong to `container`,
//! `codec`, `y4m`, and the execution backends.

mod plan;
mod planner;
mod project;
mod stream_policy;
mod transform;
mod trim;

pub use plan::{execution_mode, ExecutionMode};
pub use planner::{plan, EditPlan, StreamPlan};
pub use project::EditProject;
pub use stream_policy::{StreamMode, StreamPolicy};
pub use transform::{CropRect, Transform};
pub use trim::TimeRange;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Trim(TimeRange),
    Concat,
    Transform(Transform),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn geometry_requires_decode() {
        let crop = CropRect::new(0, 0, 640, 360).unwrap();
        let project = EditProject::new().transform(Transform::Crop(crop));
        assert_eq!(plan(&project).mode, ExecutionMode::DecodeProcessEncode);
    }

    #[test]
    fn hybrid_plan_transcodes_video_and_copies_audio() {
        let range = TimeRange::new(Duration::from_secs(1), Duration::from_secs(2)).unwrap();
        let project = EditProject::new()
            .trim(range)
            .transform(Transform::VerticalFlip)
            .streams(StreamPolicy::new().audio(StreamMode::Copy));
        let plan = plan(&project);
        assert_eq!(plan.video, StreamPlan::Transcode);
        assert_eq!(plan.audio, StreamPlan::Copy);
    }
}
