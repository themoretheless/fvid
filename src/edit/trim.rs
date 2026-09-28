use std::time::Duration;

/// A half-open interval on the presentation timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeRange {
    pub start: Duration,
    pub end: Duration,
}

impl TimeRange {
    pub fn new(start: Duration, end: Duration) -> Option<Self> {
        (start < end).then_some(Self { start, end })
    }

    pub fn duration(self) -> Duration {
        self.end - self.start
    }
}
