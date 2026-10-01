//! Owned sample-plane shuffle adapter; processing lives in fvid-media.
use crate::{Result, invalid, native_geometry::GeometryFrame};
pub use fvid_media::owned_shuffleplanes::ShufflePlanes;
pub fn apply(filter: ShufflePlanes, frame: &mut GeometryFrame, depth: u8) -> Result<()> {
    if frame.width == 0 || frame.height == 0 || !(8..=16).contains(&depth) {
        return Err(invalid("invalid shuffleplanes frame geometry"));
    }
    if let Some([sx, sy]) = frame.subsampling {
        let sampling = filter
            .apply_yuv(&mut frame.data, frame.width, frame.height, [sx, sy], depth)
            .map_err(|e| invalid(&e))?;
        frame.subsampling = Some(sampling);
        Ok(())
    } else {
        if depth != 8
            || frame
                .width
                .checked_mul(frame.height)
                .and_then(|n| n.checked_mul(3))
                != Some(frame.data.len())
        {
            return Err(invalid("invalid shuffleplanes RGB geometry"));
        }
        filter.apply_rgb24(&mut frame.data).map_err(|e| invalid(&e))
    }
}
