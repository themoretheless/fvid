//! Sample-clock scheduling for unit-rate MP4 audio edits.
use super::mp4::Edit;
use crate::{Result, invalid};
include!("../../crates/fvid-media/src/owned_mp4_audio_schedule_impl.rs");

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gap_repeat_seek_and_fractional_boundaries() {
        let edits = [
            Edit {
                duration: 20,
                media_time: -1,
            },
            Edit {
                duration: 100,
                media_time: 0,
            },
            Edit {
                duration: 100,
                media_time: 0,
            },
        ];
        let t = AudioTimeline::new(&edits, 48000, 48000, 1000, 48000).unwrap();
        assert_eq!(t.sample_frames, 10560);
        assert_eq!(t.locate(0), Some((0, None)));
        assert_eq!(t.locate(960), Some((1, Some(0))));
        assert_eq!(t.locate(5759), Some((1, Some(4799))));
        assert_eq!(t.locate(5760), Some((2, Some(0))));
        assert_eq!(t.locate(10560), None);
        let fractional = [
            Edit {
                duration: 1,
                media_time: -1,
            },
            Edit {
                duration: 1,
                media_time: -1,
            },
            Edit {
                duration: 1,
                media_time: -1,
            },
        ];
        let t = AudioTimeline::new(&fractional, 0, 44100, 1000, 44100).unwrap();
        assert_eq!(t.sample_frames, 133);
        assert_eq!(t.segments[1].presentation, 45..89);
        assert!(AudioTimeline::new(&edits, 0, 0, 1000, 48000).is_err());
        assert!(
            AudioTimeline::new(
                &[Edit {
                    duration: 1,
                    media_time: 1
                }],
                0,
                1000,
                1000,
                44100
            )
            .is_err()
        );
    }
}
