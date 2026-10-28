fn key(pts: i64, scale: u32) -> (i64, u32) {
    let (mut a, mut b) = (pts.unsigned_abs(), u64::from(scale));
    while b != 0 {
        let rest = a % b;
        a = b;
        b = rest;
    }
    (pts / a as i64, scale / a as u32)
}
struct Stream {
    index: usize,
    id: u32,
    es_id: u16,
    parent: Option<u16>,
    timescale: u32,
    configuration: Vec<u8>,
    descriptions: Vec<([u8; 4], Vec<u8>)>,
    used_descriptions: Vec<bool>,
    samples: Vec<((i64, u32), usize)>,
}
pub struct Group {
    root: usize,
    streams: Vec<Stream>,
}
pub struct AccessUnit {
    pub stream_id: u16,
    pub depends_on: Option<u16>,
    pub data: Vec<u8>,
    pub bit_len: usize,
    pub pts: i64,
    pub duration: u64,
    pub timescale: u32,
}
impl AccessUnit {
    pub fn borrowed(&self) -> aac_ep1::AccessUnit<'_> {
        aac_ep1::AccessUnit {
            stream_id: self.stream_id,
            depends_on: self.depends_on,
            data: &self.data,
            bit_len: self.bit_len,
            pts: self.pts,
            duration: self.duration,
            timescale: self.timescale,
        }
    }
}
#[derive(Clone, Copy)]
struct Binding {
    id: u16,
    parent: Option<u16>,
    file: bool,
}
// Track's convenience header describes stsd[0], which can be unused. Bind the
// initial group to the description selected by its first actual sample.
fn initial_setup(track: &Track) -> ([u8; 4], &[u8]) {
    track
        .description_for_sample(0)
        .map_or((track.codec, track.configuration.as_slice()), |d| {
            (d.codec, d.configuration.as_slice())
        })
}
fn has_separate_esc(track: &Track) -> Result<bool> {
    if track.sample_descriptions.is_empty() {
        return Ok(
            AudioSpecificConfig::parse(aac_stream_descriptor(&track.configuration)?.asc)?
                .separate_esc,
        );
    }
    for (i, setup) in track.sample_descriptions.iter().enumerate() {
        if setup.codec != *b"mp4a"
            || !(0..track.samples.len()).any(|n| {
                track
                    .samples
                    .get(n)
                    .is_some_and(|s| s.description_index as usize == i + 1)
            })
        {
            continue;
        }
        if AudioSpecificConfig::parse(aac_stream_descriptor(&setup.configuration)?.asc)?
            .separate_esc
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn binding(track: &Track) -> Result<Binding> {
    binding_config(track, initial_setup(track).1)
}
fn binding_config(track: &Track, configuration: &[u8]) -> Result<Binding> {
    let d = aac_stream_descriptor(configuration)?;
    let canonical = d.es_id == 0 && d.depends_on.is_none() && d.ocr_es_id.is_none();
    if canonical || !track.dependencies.is_empty() {
        if !canonical {
            return Err(invalid("EP1 MP4 mixes descriptor dependencies with dpnd"));
        }
        if track.dependencies.len() > 1 {
            return Err(invalid(
                "EP1 MP4 multiple dependency parents are not supported",
            ));
        }
        let id = u16::try_from(track.id)
            .ok()
            .filter(|id| *id != 0)
            .ok_or_else(|| invalid("EP1 MP4 elementary track ID exceeds ES identifier range"))?;
        let parent = track
            .dependencies
            .first()
            .map(|id| {
                u16::try_from(*id)
                    .map_err(|_| invalid("EP1 MP4 dependency ID exceeds ES identifier range"))
            })
            .transpose()?;
        if parent == Some(id) {
            return Err(invalid("EP1 MP4 self dependency"));
        }
        Ok(Binding {
            id,
            parent,
            file: true,
        })
    } else {
        // Compatibility with already supported noncanonical descriptor-carried
        // stream metadata; never override a file's authoritative dpnd binding.
        Ok(Binding {
            id: d.es_id,
            parent: d.depends_on,
            file: false,
        })
    }
}
impl Group {
    pub fn dependent_track(track: &Track) -> bool {
        if track.handler != *b"soun" || initial_setup(track).0 != *b"mp4a" {
            return false;
        }
        aac_stream_descriptor(initial_setup(track).1).is_ok_and(|d| {
            (d.depends_on.is_some() || !track.dependencies.is_empty())
                && has_separate_esc(track).unwrap_or(false)
        })
    }
    pub fn sample_at_source_time(&self, pts: i64) -> Result<usize> {
        let root = &self.streams[0];
        root.samples
            .binary_search_by_key(&key(pts, root.timescale), |entry| entry.0)
            .map(|i| root.samples[i].1)
            .map_err(|_| invalid("EP1 MP4 root packet source time is absent"))
    }
    /// Root audio candidates only, with the same enabled-track preference as
    /// ordinary MP4 selection. Dependent EP1 ESs are not alternative programs.
    pub fn automatic_audio_tracks<R: Read + Seek>(reader: &Mp4Reader<R>) -> Result<Vec<usize>> {
        let mut indices = Vec::new();
        for (index, track) in reader.tracks().iter().enumerate() {
            if track.handler != *b"soun" {
                continue;
            }
            if Self::dependent_track(track) {
                continue;
            }
            indices.push(index);
        }
        let rank = |index| match reader.track_header_flags(index).unwrap_or(0) & 3 {
            3 => 2,
            1 => 1,
            _ => 0,
        };
        if let Some(best) = indices.iter().map(|&i| rank(i)).max() {
            indices.retain(|&i| rank(i) == best);
        }
        Ok(indices)
    }
    pub fn for_root(tracks: &[Track], root: usize) -> Result<Option<Self>> {
        let track = tracks
            .get(root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        if initial_setup(track).0 != *b"mp4a" {
            return Ok(None);
        }
        if !has_separate_esc(track)? {
            return Ok(None);
        }
        Self::new(tracks, root).map(Some)
    }
    /// Conservative pre-allocation bound for this group's timing index and one
    /// simultaneous decoded-input working set. No implicit limit is imposed.
    pub fn admission_upper_bound(tracks: &[Track], root: usize) -> Result<usize> {
        let source = tracks
            .get(root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        if initial_setup(source).0 != *b"mp4a" {
            return Ok(0);
        }
        if !has_separate_esc(source)? {
            return Ok(0);
        }
        let root_binding = binding(source)?;
        let mut total = 0usize;
        let mut add = |n: usize| -> Result<()> {
            total = total
                .checked_add(n)
                .ok_or_else(|| invalid("EP1 MP4 memory estimate overflow"))?;
            Ok(())
        };
        for (index, track) in tracks.iter().enumerate() {
            if initial_setup(track).0 != *b"mp4a" || track.handler != *b"soun" {
                continue;
            }
            let mut member = index == root;
            if index != root && !has_separate_esc(track)? {
                continue;
            }
            let own_binding = binding(track)?;
            if own_binding.file != root_binding.file {
                continue;
            }
            let mut parent = own_binding.parent;
            for _ in 0..tracks.len() {
                let Some(id) = parent else {
                    break;
                };
                if id == root_binding.id {
                    member = true;
                    break;
                }
                let mut found = None;
                for candidate in tracks
                    .iter()
                    .filter(|t| initial_setup(t).0 == *b"mp4a" && t.handler == *b"soun")
                {
                    if !has_separate_esc(candidate)? {
                        continue;
                    }
                    let candidate = binding(candidate)?;
                    if candidate.file == root_binding.file && candidate.id == id {
                        found = Some(candidate.parent);
                        break;
                    }
                }
                let Some(next) = found else {
                    break;
                };
                parent = next;
            }
            if !member {
                continue;
            }
            add(std::mem::size_of::<u32>())?;
            add(track
                .samples
                .len()
                .checked_mul(std::mem::size_of::<((i64, u32), usize)>())
                .ok_or_else(|| invalid("EP1 MP4 memory estimate overflow"))?)?;
            add(track.configuration.len())?;
            add(track.sample_descriptions.len())?;
            add(track
                .sample_descriptions
                .len()
                .checked_mul(std::mem::size_of::<([u8; 4], Vec<u8>)>())
                .ok_or_else(|| invalid("EP1 MP4 memory estimate overflow"))?)?;
            for description in &track.sample_descriptions {
                add(description
                    .configuration
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| invalid("EP1 MP4 memory estimate overflow"))?)?;
            }
            add(4 * std::mem::size_of::<Stream>())?;
            // Owned inputs, borrowed views, recovered class tables and allocation
            // growth coexist. The existing decoder admission covers DSP/checkpoints.
            add(1024)?;
            let largest = (0..track.samples.len())
                .filter_map(|i| track.samples.get(i))
                .map(|s| s.size as usize)
                .max()
                .unwrap_or(0);
            add(largest
                .checked_mul(32)
                .ok_or_else(|| invalid("EP1 MP4 memory estimate overflow"))?)?;
        }
        Ok(total)
    }
    /// Select a root ES explicitly. Unrelated audio tracks are not dependency
    /// members. A fork, repeated ES ID or ambiguous AU timestamp is an error.
    pub fn new(tracks: &[Track], root: usize) -> Result<Self> {
        let track = tracks
            .get(root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        if track.handler != *b"soun" || initial_setup(track).0 != *b"mp4a" {
            return Err(invalid("EP1 MP4 root is not MPEG-4 audio"));
        }
        let root_binding = binding(track)?;
        if root_binding.parent.is_some() {
            return Err(invalid("EP1 MP4 selected track is not a root ES"));
        }
        if !has_separate_esc(track)? {
            return Err(invalid("EP1 MP4 root does not declare separate ESCs"));
        }
        let mut streams = Vec::new();
        let mut at = root;
        loop {
            let track = &tracks[at];
            let d = aac_stream_descriptor(initial_setup(track).1)?;
            let current = binding(track)?;
            if current.file != root_binding.file {
                return Err(invalid("EP1 MP4 mixes file and stream dependency bindings"));
            }
            if d.url.is_some() {
                return Err(invalid(
                    "EP1 MP4 external ES URL requires an external source",
                ));
            }
            if d.sl_config != Some(&[2][..]) {
                return Err(invalid("EP1 MP4 requires SL predefined configuration 2"));
            }
            if track.timescale == 0 {
                return Err(invalid("EP1 MP4 zero source timescale"));
            }
            if streams.iter().any(|s: &Stream| s.es_id == current.id) {
                return Err(invalid("EP1 MP4 repeated or cyclic ES ID"));
            }
            let mut descriptions = Vec::new();
            descriptions
                .try_reserve_exact(track.sample_descriptions.len())
                .map_err(|_| invalid("EP1 MP4 description allocation failed"))?;
            for description in &track.sample_descriptions {
                descriptions.push((description.codec, description.configuration.clone()));
            }
            let mut used_descriptions = Vec::new();
            used_descriptions
                .try_reserve_exact(descriptions.len())
                .map_err(|_| invalid("EP1 MP4 description allocation failed"))?;
            used_descriptions.resize(descriptions.len(), false);
            let mut samples = Vec::new();
            samples
                .try_reserve_exact(track.samples.len())
                .map_err(|_| invalid("EP1 MP4 timing index allocation failed"))?;
            let mut checked_description = None;
            for i in 0..track.samples.len() {
                let sample = track
                    .samples
                    .get(i)
                    .ok_or_else(|| invalid("EP1 MP4 sample index out of range"))?;
                let slot = sample
                    .description_index
                    .checked_sub(1)
                    .ok_or_else(|| invalid("EP1 MP4 zero sample description"))?
                    as usize;
                *used_descriptions
                    .get_mut(slot)
                    .ok_or_else(|| invalid("EP1 MP4 unknown sample description"))? = true;
                if checked_description != Some(sample.description_index) {
                    let setup = track
                        .description_for_sample(i)
                        .ok_or_else(|| invalid("EP1 MP4 missing sample description"))?;
                    let b = binding_config(track, &setup.configuration)?;
                    let d = aac_stream_descriptor(&setup.configuration)?;
                    if setup.codec != *b"mp4a"
                        || (at != root && !AudioSpecificConfig::parse(d.asc)?.separate_esc)
                    {
                        return Err(invalid(
                            "EP1 MP4 description must retain separate ESC coding",
                        ));
                    }
                    if b.id != current.id || b.parent != current.parent || b.file != current.file {
                        return Err(invalid("EP1 MP4 description changes dependency binding"));
                    }
                    if d.sl_config != Some(&[2][..]) || d.url.is_some() {
                        return Err(invalid(
                            "EP1 MP4 description requires self-contained SL configuration 2",
                        ));
                    }
                    checked_description = Some(sample.description_index);
                }
                samples.push((key(sample.pts, track.timescale), i));
            }
            samples.sort_unstable_by_key(|row| row.0);
            if samples.windows(2).any(|rows| rows[0].0 == rows[1].0) {
                return Err(invalid("EP1 MP4 ambiguous source timestamp"));
            }
            streams
                .try_reserve(1)
                .map_err(|_| invalid("EP1 MP4 stream table allocation failed"))?;
            streams.push(Stream {
                index: at,
                id: track.id,
                es_id: current.id,
                parent: current.parent,
                timescale: track.timescale,
                configuration: track.configuration.clone(),
                descriptions,
                used_descriptions,
                samples,
            });
            let mut next = None;
            for (index, candidate) in tracks.iter().enumerate() {
                if candidate.handler != *b"soun" || initial_setup(candidate).0 != *b"mp4a" {
                    continue;
                }
                let child = aac_stream_descriptor(initial_setup(candidate).1)?;
                if !has_separate_esc(candidate)?
                    && !candidate.dependencies.contains(&track.id)
                    && child.depends_on != Some(current.id)
                {
                    continue;
                }
                let child = binding(candidate)?;
                if child.file
                    && child
                        .parent
                        .is_some_and(|id| !tracks.iter().any(|t| t.id == u32::from(id)))
                {
                    return Err(invalid("EP1 MP4 missing dependency track"));
                }
                if child.id == current.id && child.file == current.file && index != at {
                    return Err(invalid("EP1 MP4 duplicate ES ID"));
                }
                if child.parent == Some(current.id) {
                    if next.replace(index).is_some() {
                        return Err(invalid("EP1 MP4 forked ES dependency"));
                    }
                }
            }
            let Some(next) = next else {
                break;
            };
            at = next;
        }
        Ok(Self { root, streams })
    }
    pub fn root_track(&self) -> usize {
        self.root
    }
    /// Decide from the selected sample, not from the initial track header.
    /// Ordinary epochs must not request auxiliary ESC AUs at that timestamp.
    pub fn requires_esc(&self, tracks: &[Track], sample: usize) -> Result<bool> {
        let track = tracks
            .get(self.root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        let stream = &self.streams[0];
        if track.id != stream.id
            || track.timescale != stream.timescale
            || track.configuration != stream.configuration
            || track.samples.len() != stream.samples.len()
        {
            return Err(invalid(
                "EP1 MP4 group belongs to different track configuration",
            ));
        }
        let entry = track
            .samples
            .get(sample)
            .ok_or_else(|| invalid("EP1 MP4 root sample index out of range"))?;
        let setup = track
            .description_for_sample(sample)
            .ok_or_else(|| invalid("EP1 MP4 missing sample description"))?;
        let saved = stream
            .descriptions
            .get(
                entry
                    .description_index
                    .checked_sub(1)
                    .ok_or_else(|| invalid("EP1 MP4 zero sample description"))?
                    as usize,
            )
            .ok_or_else(|| invalid("EP1 MP4 unknown sample description"))?;
        if setup.codec != saved.0 || setup.configuration != saved.1 {
            return Err(invalid(
                "EP1 MP4 group belongs to different track configuration",
            ));
        }
        Ok(
            AudioSpecificConfig::parse(aac_stream_descriptor(&setup.configuration)?.asc)?
                .separate_esc,
        )
    }
    /// Maximum stored chain width; each frame can select a shorter active prefix.
    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut total = self
            .streams
            .capacity()
            .checked_mul(std::mem::size_of::<Stream>())
            .ok_or_else(|| invalid("EP1 MP4 retained payload size overflow"))?;
        for stream in &self.streams {
            total = total
                .checked_add(stream.used_descriptions.capacity())
                .ok_or_else(|| invalid("EP1 MP4 retained payload size overflow"))?;
            total = total
                .checked_add(
                    stream
                        .descriptions
                        .capacity()
                        .checked_mul(std::mem::size_of::<([u8; 4], Vec<u8>)>())
                        .ok_or_else(|| invalid("EP1 MP4 retained payload size overflow"))?,
                )
                .ok_or_else(|| invalid("EP1 MP4 retained payload size overflow"))?;
            for (_, configuration) in &stream.descriptions {
                total = total
                    .checked_add(configuration.capacity())
                    .ok_or_else(|| invalid("EP1 MP4 retained payload size overflow"))?;
            }
            total = total
                .checked_add(stream.configuration.capacity())
                .and_then(|n| {
                    stream
                        .samples
                        .capacity()
                        .checked_mul(std::mem::size_of::<((i64, u32), usize)>())
                        .and_then(|bytes| n.checked_add(bytes))
                })
                .ok_or_else(|| invalid("EP1 MP4 retained payload size overflow"))?;
        }
        Ok(total)
    }
    /// Read the active dependency prefix at the selected root's exact time.
    /// Reachable OD events explicitly select the active prefix when present.
    /// Otherwise a tail which never uses the root's active ASC may be absent.
    /// Missing active AUs, clock mismatches and active chain gaps
    /// refuse; no padding or substitution changes decoder input or history.
    pub fn read_frame<R: Read + Seek>(
        &self,
        reader: &mut Mp4Reader<R>,
        root_sample: usize,
    ) -> Result<Vec<AccessUnit>> {
        self.read_frame_checked(reader, root_sample, || {
            Ok::<(), std::convert::Infallible>(())
        })
    }
    pub fn read_frame_checked<R: Read + Seek, E: std::fmt::Display>(
        &self,
        reader: &mut Mp4Reader<R>,
        root_sample: usize,
        mut check: impl FnMut() -> std::result::Result<(), E>,
    ) -> Result<Vec<AccessUnit>> {
        check().map_err(|e| invalid(&e.to_string()))?;
        if !self.requires_esc(reader.tracks(), root_sample)? {
            return Err(invalid(
                "EP1 MP4 selected sample uses ordinary AAC transport",
            ));
        }
        let root = reader
            .tracks()
            .get(self.root)
            .ok_or_else(|| invalid("EP1 MP4 root track index out of range"))?;
        let stamp = root
            .samples
            .get(root_sample)
            .ok_or_else(|| invalid("EP1 MP4 root sample index out of range"))?;
        let root_setup = root
            .description_for_sample(root_sample)
            .ok_or_else(|| invalid("EP1 MP4 missing root sample description"))?;
        let root_asc = aac_stream_descriptor(&root_setup.configuration)?
            .asc
            .to_vec();
        let time = key(stamp.pts, root.timescale);
        let root_scale = root.timescale;
        let chain:Vec<_> = self.streams.iter().map(|s|s.id).collect();
        let od_width=reader.object_descriptor_prefix(&chain,stamp.pts,root_scale).map_err(|e|invalid(&e.to_string()))?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.streams.len())
            .map_err(|_| invalid("EP1 MP4 AU table allocation failed"))?;
        let mut inactive_tail = false;
        for (ordinal,stream) in self.streams.iter().enumerate() {
            if od_width.is_some_and(|width|ordinal>=width){break;}
            check().map_err(|e| invalid(&e.to_string()))?;
            let track = reader
                .tracks()
                .get(stream.index)
                .ok_or_else(|| invalid("EP1 MP4 stream track index out of range"))?;
            let current = binding(track)?;
            if current.id != stream.es_id || current.parent != stream.parent {
                return Err(invalid(
                    "EP1 MP4 group belongs to different track configuration",
                ));
            }
            if track.id != stream.id
                || track.timescale != stream.timescale
                || track.configuration != stream.configuration
                || track.samples.len() != stream.samples.len()
            {
                return Err(invalid(
                    "EP1 MP4 group belongs to different track configuration",
                ));
            }
            let found = match stream.samples.binary_search_by_key(&time, |entry| entry.0) {
                Ok(found) => {
                    if inactive_tail {
                        return Err(invalid("EP1 MP4 active dependency chain has a gap"));
                    }
                    found
                }
                Err(_) => {
                    let mut required = false;
                    for (i, saved) in stream.descriptions.iter().enumerate() {
                        if !stream.used_descriptions[i] {
                            continue;
                        }
                        let live = track
                            .sample_descriptions
                            .get(i)
                            .ok_or_else(|| invalid("EP1 MP4 unknown sample description"))?;
                        if live.codec != saved.0 || live.configuration != saved.1 {
                            return Err(invalid(
                                "EP1 MP4 group belongs to different track configuration",
                            ));
                        }
                        if aac_stream_descriptor(&saved.1)?.asc == root_asc {
                            required = true;
                        }
                    }
                    if required || od_width.is_some() {
                        return Err(invalid("EP1 MP4 missing simultaneous ESC access unit"));
                    }
                    // Only an inactive tail may disappear. The root's current
                    // ASC has never been used by this stream; unused entries do
                    // not turn it into a required AU for that configuration.
                    inactive_tail = true;
                    continue;
                }
            };
            let index = stream.samples[found].1;
            let sample = track
                .samples
                .get(index)
                .ok_or_else(|| invalid("EP1 MP4 sample index out of range"))?;
            if key(sample.pts, track.timescale) != time
                || u128::from(sample.duration) * u128::from(root_scale)
                    != u128::from(stamp.duration) * u128::from(track.timescale)
            {
                return Err(invalid("EP1 MP4 ESC source clocks disagree"));
            }
            let setup = track
                .description_for_sample(index)
                .ok_or_else(|| invalid("EP1 MP4 missing sample description"))?;
            let saved = stream
                .descriptions
                .get(
                    sample
                        .description_index
                        .checked_sub(1)
                        .ok_or_else(|| invalid("EP1 MP4 zero sample description"))?
                        as usize,
                )
                .ok_or_else(|| invalid("EP1 MP4 unknown sample description"))?;
            if setup.codec != saved.0 || setup.configuration != saved.1 {
                return Err(invalid(
                    "EP1 MP4 group belongs to different track configuration",
                ));
            }
            let descriptor = aac_stream_descriptor(&setup.configuration)?;
            if descriptor.asc != root_asc {
                return Err(invalid(
                    "EP1 MP4 simultaneous descriptions disagree with root ASC",
                ));
            }
            let bit_len = reader
                .packet_bit_len(stream.index, index)
                .map_err(|e| invalid(&e.to_string()))?;
            let mut data = Vec::new();
            reader
                .read_packet(stream.index, index, &mut data)
                .map_err(|e| invalid(&e.to_string()))?;
            output.push(AccessUnit {
                stream_id: stream.es_id,
                depends_on: stream.parent,
                data,
                bit_len,
                pts: sample.pts,
                duration: u64::from(sample.duration),
                timescale: stream.timescale,
            });
        }
        Ok(output)
    }
}
