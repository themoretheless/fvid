//! Shared CPU work partitioning policy.

/// How many workers a packed frame of this many bytes is cut between. Each span
/// has to hold enough pixels to outpay the cost of starting its worker, so a
/// small picture keeps the one thread it came with. Both the YUV-to-RGB pass and
/// the grade read this: each costs a few nanoseconds a pixel, so the same floor
/// serves both.
#[inline]
pub(crate) fn span_workers(bytes: usize) -> usize {
    fvid_cpu::span_workers(bytes)
}
