/// Requested handling for one media stream kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StreamMode {
    #[default]
    Auto,
    Copy,
    Transcode,
    Drop,
}

/// Per-kind stream policy for an edit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamPolicy {
    pub video: StreamMode,
    pub audio: StreamMode,
    pub subtitles: StreamMode,
}

impl StreamPolicy {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn video(mut self, mode: StreamMode) -> Self {
        self.video = mode;
        self
    }

    pub fn audio(mut self, mode: StreamMode) -> Self {
        self.audio = mode;
        self
    }

    pub fn subtitles(mut self, mode: StreamMode) -> Self {
        self.subtitles = mode;
        self
    }
}
