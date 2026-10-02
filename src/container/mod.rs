//! FVid-owned container readers. No external demultiplexer or codec backend.
pub mod adts;
pub mod audio_timeline;
pub mod avi;
pub mod mp4;
pub mod mp4_relocate;
pub mod mp4_write;
pub mod mp4_matroska;
pub mod mp4_concat;
pub mod matroska_write;
pub mod matroska_copy;
pub mod ogg;
pub mod opus_packet;
pub mod smf;
pub mod webm;
pub mod xm;

/// Reduce a ratio to the smallest pair that states it, which is what lets a
/// player name the shape on screen. A zero part states nothing, and a ratio too
/// wide to state in the pair is taken to state nothing either.
pub(crate) fn reduce_ratio(numerator: u64, denominator: u64) -> (u32, u32) {
    if numerator == 0 || denominator == 0 {
        return (1, 1);
    }
    let (mut a, mut b) = (numerator, denominator);
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    match (u32::try_from(numerator / a), u32::try_from(denominator / a)) {
        (Ok(n), Ok(d)) => (n, d),
        _ => (1, 1),
    }
}

pub use fvid_media::owned_file_tags::FileTags;
