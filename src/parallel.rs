//! Shared CPU work partitioning policy.

/// How many workers a packed frame of this many bytes is cut between. Each span
/// has to hold enough pixels to outpay the cost of starting its worker, so a
/// small picture keeps the one thread it came with. Both the YUV-to-RGB pass and
/// the grade read this: each costs a few nanoseconds a pixel, so the same floor
/// serves both.
pub(crate) fn span_workers(bytes: usize) -> usize {
    /// Pixels a span has to carry before a worker is worth waking for it: well
    /// over the tens of microseconds a thread costs to start.
    const MIN_SPAN_PIXELS: usize = 1 << 14;
    let parallelism = std::thread::available_parallelism().map_or(1, |n| n.get());
    parallelism.min(bytes / 3 / MIN_SPAN_PIXELS).max(1)
}
