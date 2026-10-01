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

/// What a container's own tags say about the file as a whole, in the facts
/// both families of writers agree on and a player of the same generation shows
/// beside it, plus the numbers — the track and the disc — VLC lists beside
/// them. Each reader maps its own spelling onto these names, so `©ART` in a
/// box and an `ARTIST` tag reach the player as one field; a fact no writer
/// thought to record stays empty rather than standing for something. The
/// numbers are kept as the file spells them, `3` or `3/12` alike, since a
/// number split by a slash is how both families write a place within an album.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub date: String,
    pub comment: String,
    pub track: String,
    /// Whose recording the album is, when that is not the artist of the track,
    /// the disc the track sits on when a file says it — the same number-pair
    /// writing as the track — who published the whole thing, who holds the
    /// rights to it, and what the file says its contents are. A player of the
    /// same generation lists all five beside the facts above. The rating is
    /// the one of these only Matroska carries: a writer of the same generation
    /// drops it from an MP4 without a word.
    pub album_artist: String,
    pub disc: String,
    pub publisher: String,
    pub copyright: String,
    pub description: String,
    pub rating: String,
}

impl FileTags {
    /// Records a value under the fact its tag names, in whichever capitalisation
    /// the writer chose, and under any of the names one fact travels by: the
    /// track answers to `TRACK`, to the Vorbis field `TRACKNUMBER` and to the
    /// spelling ffmpeg's Matroska muxer writes, `PART_NUMBER`. A name outside
    /// these, or a value with nothing in it, is not a fact this reader keeps and
    /// answers `false`; a fact the file already stated keeps its first answer,
    /// since a writer that repeats a tag gives no reason to prefer the second
    /// copy. Album artist answers to the Vorbis field and to the spaced
    /// spelling ffmpeg's Matroska muxer keeps in its own maps, and disc to the
    /// plain word and to `DISCNUMBER`.
    pub fn insert(&mut self, name: &str, value: &str) -> bool {
        if value.is_empty() {
            return false;
        }
        let field = match name.to_ascii_uppercase().as_str() {
            "TITLE" => &mut self.title,
            "ARTIST" => &mut self.artist,
            "ALBUM" => &mut self.album,
            "GENRE" => &mut self.genre,
            "DATE" => &mut self.date,
            "COMMENT" => &mut self.comment,
            "TRACK" | "TRACKNUMBER" | "PART_NUMBER" => &mut self.track,
            "ALBUMARTIST" | "ALBUM_ARTIST" => &mut self.album_artist,
            "DISC" | "DISCNUMBER" => &mut self.disc,
            "PUBLISHER" => &mut self.publisher,
            "COPYRIGHT" => &mut self.copyright,
            "DESCRIPTION" => &mut self.description,
            "RATING" => &mut self.rating,
            _ => return false,
        };
        if field.is_empty() {
            *field = value.to_owned();
        }
        true
    }
}
