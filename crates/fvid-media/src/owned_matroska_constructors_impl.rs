impl<'a, W: Write + Seek> PacketWriter<'a, W> {
    pub fn new(output: &'a mut W, tracks: &[TrackSpec<'_>]) -> Result<Self> {
        Self::new_with_video_metadata(output, tracks, &[])
    }
    /// Metadata is empty (none for every track), or one optional entry per track.
    /// Validate all track properties before writing any output bytes.
    pub fn new_with_video_metadata(
        output: &'a mut W,
        tracks: &[TrackSpec<'_>],
        metadata: &[Option<VideoMetadata>],
    ) -> Result<Self> {
        if tracks.is_empty() || tracks.len() > 126 {
            return Err(invalid("Matroska requires 1..=126 tracks"));
        }
        if !metadata.is_empty() && metadata.len() != tracks.len() {
            return Err(invalid("Matroska metadata track count mismatch"));
        }
        let options: Vec<_> = metadata
            .iter()
            .map(|video| TrackOptions {
                video: *video,
                rotation: 0,
                codec_delay_ns: 0,
                default_duration_ns: 0,
            })
            .collect();
        Self::new_with_options(output, tracks, &options)
    }
    /// Empty options select defaults, otherwise supply one entry per track.
    pub fn new_with_options(
        output: &'a mut W,
        tracks: &[TrackSpec<'_>],
        options: &[TrackOptions],
    ) -> Result<Self> {
        Self::new_with_metadata(output, tracks, options, &FileMetadata::default())
    }
    /// Supply file tags and chapters alongside track metadata. Metadata is
    /// validated before writing any output; packet payloads remain streamed.
    pub fn new_with_metadata(
        output: &'a mut W,
        tracks: &[TrackSpec<'_>],
        options: &[TrackOptions],
        metadata: &FileMetadata,
    ) -> Result<Self> {
        Self::new_with_text_metadata(output,tracks,options,metadata,&Default::default(),&Default::default())
    }
    /// Preserve file-wide and per-track text tags while replacing video coding.
    pub(crate) fn new_with_text_metadata(
        output:&'a mut W,tracks:&[TrackSpec<'_>],options:&[TrackOptions],metadata:&FileMetadata,
        text:&std::collections::BTreeMap<String,String>,
        scoped:&std::collections::BTreeMap<usize,std::collections::BTreeMap<String,String>>,
    )->Result<Self> {
        let mut file_elements = file_metadata(metadata)?;
        file_elements.extend(extra_text_tags(text)?);
        for (&track,tags) in scoped {
            if track>=tracks.len() {return Err(invalid("text metadata refers to missing track"));}
            file_elements.extend(text_tags_element(tags,Some(track as u64+1))?);
        }

        if tracks.is_empty() || tracks.len() > 126 {
            return Err(invalid("Matroska requires 1..=126 tracks"));
        }
        if !options.is_empty() && options.len() != tracks.len() {
            return Err(invalid("Matroska metadata track count mismatch"));
        }
        let mut entries = Vec::new();
        for (index, track) in tracks.iter().enumerate() {
            entries.extend(track_entry(track, index as u64 + 1, options.get(index))?);
        }
        let delays = (0..tracks.len())
            .map(|i| options.get(i).map_or(0, |o| o.codec_delay_ns))
            .collect();
        let pcm = tracks.iter().map(|t| match t.encoding {
            Encoding::PcmFloat32 { sample_rate, channels } => Some((sample_rate, channels)),
            _ => None,
        }).collect();
        Self::new_prepared(output, &entries, &file_elements, delays, pcm)
    }
}
