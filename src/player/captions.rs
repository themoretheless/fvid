use super::{
    Player,
    playlist::track_step,
};
use crate::container::FileTags;
use crate::subtitles::{self, Cue};
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// One list of cues the player can show: a sidecar file or a text track the
/// container carries. `label` is what the track list and the OSD show.
pub(super) struct SubtitleSource {
    pub(super) label: String,
    pub(super) cues: Vec<Cue>,
}

/// A named part of the open file, from the chapter list its container carries.
pub(super) struct ChapterMark {
    pub(super) start: Duration,
    pub(super) title: String,
}

/// The chapter a moment falls in, counted from one, with its title: the last
/// one starting at or before it. `None` before the first chapter starts.
pub(super) fn chapter_at(chapters: &[ChapterMark], at: Duration) -> Option<(usize, &str)> {
    chapters
        .iter()
        .rposition(|mark| mark.start <= at)
        .map(|nth| (nth + 1, chapters[nth].title.as_str()))
}

/// The plain-text subtitle tracks inside a file, in container order, read by
/// whichever container reader its first bytes belong to: Matroska says so with
/// its EBML magic, and everything else this player opens that can hold a text
/// track is MP4 or QuickTime. A Y4M stream, or a file either reader refuses,
/// offers none.
pub(super) fn embedded_subtitles(path: &Path) -> Vec<SubtitleSource> {
    let Ok(file) = crate::playback_spool::open_shared(path) else {
        return Vec::new();
    };
    let mut buffered = BufReader::new(file);
    let mut magic = [0u8; 4];
    if buffered.read_exact(&mut magic).is_err() || buffered.rewind().is_err() {
        return Vec::new();
    }
    if magic == *b"\x1a\x45\xdf\xa3" {
        let Ok(mut reader) = crate::playback_webm_subtitles::WebmSubtitleReader::open(
            buffered,
            crate::container::webm::Limits::default(),
        ) else {
            return Vec::new();
        };
        let labels: Vec<String> = reader
            .tracks()
            .iter()
            .map(crate::playback_webm_subtitles::SubtitleTrack::label)
            .collect();
        return listed(&labels, &mut |nth| reader.cues(nth));
    }
    let Ok(mut reader) = crate::playback_mp4_subtitles::Mp4SubtitleReader::open(
        buffered,
        crate::container::mp4::Limits::default(),
    ) else {
        return Vec::new();
    };
    let labels: Vec<String> = reader
        .tracks()
        .iter()
        .map(crate::playback_mp4_subtitles::Mp4SubtitleTrack::label)
        .collect();
    listed(&labels, &mut |nth| reader.cues(nth))
}

/// What a container says about the file as a whole besides its tracks: the
/// chapter points it names and the facts its own tags state about it. Read by
/// whichever reader the file's first bytes belong to, as `embedded_subtitles`
/// picks its reader. This costs the second index pass a container reader always
/// makes, which is what reading the file's own tracks costs anyway, and it is
/// what both kinds of item pay alike — a file with nothing to show is named here
/// too. A stream carrying neither, or one the reader cannot open, leaves both
/// empty.
pub(super) fn container_facts(path: &Path) -> ContainerFacts {
    let Ok(file) = crate::playback_spool::open_shared(path) else {
        return ContainerFacts::default();
    };
    let mut buffered = BufReader::new(file);
    let mut magic = [0u8; 4];
    if buffered.read_exact(&mut magic).is_err() || buffered.rewind().is_err() {
        return ContainerFacts::default();
    }
    let (runs, tags): (Vec<(u64, String)>, FileTags) = if magic == *b"\x1a\x45\xdf\xa3" {
        crate::container::webm::WebmReader::open(
            buffered,
            crate::container::webm::Limits::default(),
        )
        .map(|reader| {
            (
                reader
                    .chapters
                    .iter()
                    .map(|chapter| (chapter.start_ns, chapter.title.clone()))
                    .collect(),
                reader.tags,
            )
        })
        .unwrap_or_default()
    } else {
        crate::container::mp4::Mp4Reader::open(buffered, crate::container::mp4::Limits::default())
            .map(|reader| {
                (
                    reader
                        .chapters()
                        .iter()
                        .map(|chapter| (chapter.start_ns, chapter.title.clone()))
                        .collect(),
                    reader.tags().clone(),
                )
            })
            .unwrap_or_default()
    };
    ContainerFacts {
        chapters: runs
            .into_iter()
            .map(|(start_ns, title)| ChapterMark {
                start: Duration::from_nanos(start_ns),
                title,
            })
            .collect(),
        tags,
    }
}

/// The two answers a container gives about the file itself, gathered by the one
/// walk `container_facts` makes.
#[derive(Default)]
pub(super) struct ContainerFacts {
    pub(super) chapters: Vec<ChapterMark>,
    pub(super) tags: FileTags,
}

/// Which chapter the next or previous key answers with: the first one starting
/// after the position, or the last one starting before it. Neither end of the
/// film has anywhere further to walk.
pub(super) fn chapter_ahead(starts: &[Duration], at: Duration, forward: bool) -> Option<usize> {
    if forward {
        (0..starts.len()).find(|&nth| starts[nth] > at)
    } else {
        (0..starts.len()).rev().find(|&nth| starts[nth] < at)
    }
}

/// One source per track whose cues are there to show: a track that yields
/// nothing is not a choice worth cycling through, and a track that cannot be
/// read leaves its neighbours alone.
pub(super) fn listed(
    labels: &[String],
    cues: &mut dyn FnMut(usize) -> crate::Result<Vec<Cue>>,
) -> Vec<SubtitleSource> {
    let mut sources = Vec::with_capacity(labels.len());
    for (nth, label) in labels.iter().enumerate() {
        if let Ok(cues) = cues(nth)
            && !cues.is_empty()
        {
            sources.push(SubtitleSource {
                label: label.clone(),
                cues,
            });
        }
    }
    sources
}

impl Player {
    /// Gather the subtitle sources of a file: a SubRip, WebVTT or ASS file
    /// sitting next to the video, the way VLC loads a sidecar without being
    /// told, then the plain-text tracks the container itself carries. The first
    /// source wins, so a sidecar the user placed deliberately beats an embedded
    /// track; a file nothing can be read from leaves the picture bare. A file
    /// `--sub-file` names leads all of them, and one that yields nothing says so
    /// rather than falling back unheard.
    pub(super) fn load_subtitles(&mut self, path: &std::path::Path) {
        self.subtitle_sources = Vec::new();
        self.subtitle_source = 0;
        let mut unread = None;
        if let Some(named) = self.subtitle_file.clone() {
            match subtitles::load(&named) {
                Some(cues) => self.subtitle_sources.push(SubtitleSource {
                    label: named
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    cues,
                }),
                None => unread = Some(named),
            }
        }
        for candidate in subtitles::sidecars(path) {
            if let Some(cues) = subtitles::load(&candidate) {
                self.subtitle_sources.push(SubtitleSource {
                    label: candidate
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    cues,
                });
                break;
            }
        }
        let embedded_start = self.subtitle_sources.len();
        self.subtitle_sources.extend(embedded_subtitles(path));
        if let Some(nth) = self
            .preferred_subtitle
            .filter(|nth| *nth < self.subtitle_sources.len())
        {
            self.subtitle_source = nth;
        }
        if let Some(nth) = self.preferred_embedded_subtitle
            && let Some(selected) = embedded_start.checked_add(nth)
            && selected < self.subtitle_sources.len()
        { self.subtitle_source = selected; }
        self.apply_subtitle_source();
        if let Some(named) = unread {
            // The name that was typed out loud outranks the list it fell back
            // to, so the fallback does not answer for it.
            self.show_osd(format!("Cannot read subtitles: {}", named.display()));
            return;
        }
        if !self.subtitle_name.is_empty() {
            self.show_osd(format!("Subtitles: {}", self.subtitle_name));
        }
    }

    /// Put a file the viewer named among the sources of the item on screen and
    /// turn to it, the way the sidecar and `--sub-file` are turned to. It joins
    /// the list rather than leading it, since it was asked for after everything
    /// the file came with; a file nothing can be read from leaves the list as it
    /// was and says so, the way a `--sub-file` that yields nothing does.
    pub(super) fn add_subtitle_file(&mut self, path: PathBuf) {
        let named = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let Some(cues) = subtitles::load(&path) else {
            self.show_osd(format!("Cannot read subtitles: {}", path.display()));
            return;
        };
        self.subtitle_sources
            .push(SubtitleSource { label: named, cues });
        self.subtitle_source = self.subtitle_sources.len() - 1;
        self.apply_subtitle_source();
        self.subtitle_shown = true;
        self.show_osd(format!("Subtitles: {}", self.subtitle_name));
    }

    /// Put the selected source's cues on screen.
    pub(super) fn apply_subtitle_source(&mut self) {
        match self.subtitle_sources.get(self.subtitle_source) {
            Some(source) => {
                self.cues = source.cues.clone();
                self.subtitle_name = source.label.clone();
            }
            None => {
                self.cues = Vec::new();
                self.subtitle_name.clear();
            }
        }
    }

    /// Walk the subtitle sources of the open file: sidecar first, then the
    /// container's tracks, wrapping at both ends.
    pub(super) fn cycle_subtitle_track(&mut self, forward: bool) {
        let len = self.subtitle_sources.len();
        if len < 2 {
            self.show_osd(if len == 0 {
                "No subtitles"
            } else {
                "Subtitles 1/1"
            });
            return;
        }
        let nth = track_step(self.subtitle_source, len, forward);
        self.subtitle_source = nth;
        self.apply_subtitle_source();
        self.show_osd(format!(
            "Subtitles {}/{} {}",
            nth + 1,
            len,
            self.subtitle_name
        ));
    }

    /// The line to paint over the picture at a media time, if one is live.
    /// A positive delay holds the cues back, so the picture is asked what was
    /// on screen that much earlier.
    pub(super) fn subtitle_line(&self, at: Duration) -> Option<&str> {
        if !self.subtitle_shown || self.cues.is_empty() {
            return None;
        }
        let shift = Duration::from_millis(self.subtitle_delay_ms.unsigned_abs());
        let shifted = if self.subtitle_delay_ms >= 0 {
            at.checked_sub(shift)?
        } else {
            at.checked_add(shift)?
        };
        subtitles::active(&self.cues, shifted, Duration::ZERO).map(|cue| cue.text.as_str())
    }

    /// Walk the chapters of the open file: the picture moves to the start of
    /// the next or previous chapter, and the message names the one arrived at.
    pub(super) fn walk_chapter(&mut self, forward: bool) {
        if self.chapters.is_empty() {
            self.show_osd("No chapters");
            return;
        }
        if !self.seekable {
            self.show_osd("Cannot seek");
            return;
        }
        let Some(now) = self.presentation_clock() else {
            return;
        };
        let starts: Vec<Duration> = self.chapters.iter().map(|mark| mark.start).collect();
        let Some(nth) = chapter_ahead(&starts, now, forward) else {
            self.show_osd(if forward {
                "Last chapter"
            } else {
                "First chapter"
            });
            return;
        };
        let start = self.chapters[nth].start;
        let title = self.chapters[nth].title.clone();
        self.seek_time(start);
        let named = if title.is_empty() {
            String::new()
        } else {
            format!(" {title}")
        };
        self.show_osd(format!(
            "Chapter {}/{}{named}",
            nth + 1,
            self.chapters.len()
        ));
    }
}
