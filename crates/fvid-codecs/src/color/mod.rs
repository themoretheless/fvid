//! Shared video signal descriptions and scalar color definitions.
pub mod hdr;
pub mod log;
pub mod primaries;
pub mod tonemap;
pub mod transfer;

pub use primaries::apply;
pub use transfer::Transfer;
