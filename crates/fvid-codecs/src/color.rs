//! Shared video signal descriptions and scalar color definitions.
#[path = "../../../src/color/hdr.rs"]
pub mod hdr;
#[path = "../../../src/color/log.rs"]
pub mod log;
#[path = "../../../src/color/primaries.rs"]
pub mod primaries;
#[path = "../../../src/color/tonemap.rs"]
pub mod tonemap;
#[path = "../../../src/color/transfer.rs"]
pub mod transfer;

pub use primaries::apply;
pub use transfer::Transfer;
