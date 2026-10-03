//! Owned library components plus decoders supplied by the root FVid crate.
//! Component presence does not imply every profile or workflow is implemented.
use crate::media_info::Capabilities;

pub fn inventory() -> Capabilities {
    let mut inventory = fvid_media::capabilities();
    inventory.library_version = format!("FVid {}", env!("CARGO_PKG_VERSION"));
    inventory
        .decoders
        .extend(["av1", "h264", "hevc", "opus", "vp9"].map(str::to_owned));
    inventory.decoders.sort();
    inventory.decoders.dedup();
    inventory
}
