use fvid::edit::{
    CropRect, EditProject, ExecutionMode, StreamMode, StreamPlan, StreamPolicy, Transform, plan,
};

#[test]
fn subtitle_only_transcoding_is_not_a_packet_copy_plan() {
    let project = EditProject::new().streams(
        StreamPolicy::new()
            .video(StreamMode::Drop)
            .audio(StreamMode::Drop)
            .subtitles(StreamMode::Transcode),
    );
    let result = plan(&project);
    assert_eq!(result.video, StreamPlan::Drop);
    assert_eq!(result.audio, StreamPlan::Drop);
    assert_eq!(result.subtitles, StreamPlan::Transcode);
    assert_eq!(result.mode, ExecutionMode::DecodeProcessEncode);
}

#[test]
fn copy_and_dropped_streams_do_not_request_transcoding() {
    let result = plan(
        &EditProject::new()
            .transform(Transform::HorizontalFlip)
            .streams(
                StreamPolicy::new()
                    .video(StreamMode::Drop)
                    .audio(StreamMode::Copy)
                    .subtitles(StreamMode::Copy),
            ),
    );
    assert_eq!(result.mode, ExecutionMode::PacketCopy);
    assert_eq!(result.video, StreamPlan::Drop);
}

#[test]
fn crop_constructor_rejects_unrepresentable_edges() {
    assert!(CropRect::new(u32::MAX, 0, 1, 1).is_none());
    assert!(CropRect::new(0, u32::MAX, 1, 1).is_none());
    assert!(CropRect::new(1, 0, u32::MAX, 1).is_none());
    assert!(CropRect::new(0, 1, 1, u32::MAX).is_none());
    assert!(CropRect::new(0, 0, 0, 1).is_none());
    assert!(CropRect::new(0, 0, 1, 0).is_none());
    assert!(CropRect::new(0, 0, u32::MAX, u32::MAX).is_some());
    assert!(CropRect::new(u32::MAX - 1, u32::MAX - 1, 1, 1).is_some());
}
