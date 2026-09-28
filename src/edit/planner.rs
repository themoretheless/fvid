use super::{execution_mode, EditProject, ExecutionMode, StreamMode};

/// Stream-level decision made by the planner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamPlan {
    Copy,
    Transcode,
    Drop,
}

/// Explainable plan for a complete edit request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditPlan {
    pub mode: ExecutionMode,
    pub video: StreamPlan,
    pub audio: StreamPlan,
    pub subtitles: StreamPlan,
}

pub fn plan(project: &EditProject) -> EditPlan {
    let policy = project.stream_policy();
    let operation_mode = execution_mode(project.operations());
    let video = choose(policy.video, operation_mode == ExecutionMode::DecodeProcessEncode);
    let audio = choose(policy.audio, false);
    let subtitles = choose(policy.subtitles, false);
    let mode = if matches!(video, StreamPlan::Transcode)
        || matches!(audio, StreamPlan::Transcode)
    {
        ExecutionMode::DecodeProcessEncode
    } else {
        ExecutionMode::PacketCopy
    };

    EditPlan { mode, video, audio, subtitles }
}

fn choose(mode: StreamMode, needs_transcode: bool) -> StreamPlan {
    match mode {
        StreamMode::Copy => StreamPlan::Copy,
        StreamMode::Transcode => StreamPlan::Transcode,
        StreamMode::Drop => StreamPlan::Drop,
        StreamMode::Auto if needs_transcode => StreamPlan::Transcode,
        StreamMode::Auto => StreamPlan::Copy,
    }
}
