// Shared by the library and player MP4 reader wrappers.
impl<R: Read + Seek> Mp4Reader<R> {
    fn index_object_descriptors(&mut self) -> Result<()> {
        if !self.initial_object_tracks.is_empty()
            && self.refused.iter().any(|t| t.handler == *b"odsm")
        {
            return Err(unsupported(
                "MP4 referenced OD stream codec is not implemented",
            ));
        }
        let indices: Vec<_> = self
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler == *b"odsm" && self.initial_object_tracks.contains(&t.id))
            .map(|(i, _)| i)
            .collect();
        let mut retained = 0usize;
        for index in indices {
            let track = &self.tracks[index];
            if track.id > u32::from(u16::MAX) {
                return Err(invalid(
                    "MP4 OD elementary track ID exceeds ES identifier range",
                ));
            }
            if !track.edits.is_empty() {
                return Err(unsupported("MP4 OD edit mapping is not implemented"));
            }
            let mut timeline = od::Timeline {
                track_id: track.id,
                timescale: track.timescale,
                references: track.od_references.clone(),
                events: Vec::new(),
            };
            if timeline
                .references
                .iter()
                .any(|id| !self.tracks.iter().any(|t| t.id == *id))
            {
                return Err(invalid("MP4 mpod references unavailable track"));
            }
            let count = track.samples.len();
            let mut state = od::State::default();
            let mut packet = Vec::new();
            for sample in 0..count {
                let stamp = self.tracks[index]
                    .samples
                    .get(sample)
                    .ok_or_else(|| invalid("missing MP4 OD sample"))?;
                if timeline.events.last().is_some_and(|e| e.pts >= stamp.pts) {
                    return Err(invalid("MP4 OD event times must increase"));
                }
                let setup = self.tracks[index]
                    .description_for_sample(sample)
                    .ok_or_else(|| invalid("missing MP4 OD setup"))?;
                if setup.codec != *b"mp4s" {
                    return Err(unsupported("unsupported MP4 OD sample entry"));
                }
                od::validate_setup(&setup.configuration).map_err(|e| invalid(&e))?;
                if self.packet_bit_len(index, sample)? != stamp.size as usize * 8 {
                    return Err(invalid("MP4 OD access unit is not byte aligned"));
                }
                let before = retained
                    .checked_add(timeline.heap_bytes().map_err(|e| invalid(&e))?)
                    .and_then(|v| {
                        state
                            .heap_bytes()
                            .ok()
                            .and_then(|h| h.checked_mul(2))
                            .and_then(|h| v.checked_add(h))
                    })
                    .and_then(|v| v.checked_add(stamp.size as usize))
                    .ok_or_else(|| invalid("MP4 OD index size overflow"))?;
                if before > self.limits.metadata_bytes {
                    return Err(invalid("MP4 OD index exceeds metadata budget"));
                }
                self.read_packet(index, sample, &mut packet)?;
                state
                    .apply(&packet, &timeline.references)
                    .map_err(|e| invalid(&e))?;
                let heap = state.heap_bytes().map_err(|e| invalid(&e))?;
                let next = retained
                    .checked_add(timeline.heap_bytes().map_err(|e| invalid(&e))?)
                    .and_then(|v| heap.checked_mul(2).and_then(|h| v.checked_add(h)))
                    .and_then(|v| v.checked_add(packet.capacity()))
                    .and_then(|v| v.checked_add(std::mem::size_of::<od::Event>()))
                    .ok_or_else(|| invalid("MP4 OD index size overflow"))?;
                if next > self.limits.metadata_bytes {
                    return Err(invalid("MP4 OD index exceeds metadata budget"));
                }
                timeline
                    .events
                    .try_reserve_exact(1)
                    .map_err(|_| invalid("MP4 OD event allocation failed"))?;
                timeline.events.push(od::Event {
                    pts: stamp.pts,
                    state: state.clone(),
                });
            }
            retained = retained
                .checked_add(timeline.heap_bytes().map_err(|e| invalid(&e))?)
                .ok_or_else(|| invalid("MP4 OD index size overflow"))?;
            self.object_timelines
                .try_reserve_exact(1)
                .map_err(|_| invalid("MP4 OD timeline allocation failed"))?;
            self.object_timelines.push(timeline);
        }
        Ok(())
    }
    /// Explicit OD state only; absent samples and matching ASCs never imply
    /// dynamic membership on their own. The selected members must be a prefix.
    pub fn object_descriptor_prefix(
        &self,
        chain: &[u32],
        pts: i64,
        scale: u32,
    ) -> Result<Option<usize>> {
        let Some(root) = chain.first() else {
            return Err(invalid("empty MP4 OD programme chain"));
        };
        let mut timelines = self
            .object_timelines
            .iter()
            .filter(|t| t.references.contains(root));
        let Some(timeline) = timelines.next() else {
            return Ok(None);
        };
        if timelines.next().is_some() {
            return Err(invalid("multiple MP4 OD streams control selected root"));
        }
        if chain.iter().any(|id| {
            self.tracks
                .iter()
                .any(|t| t.id == *id && !t.edits.is_empty())
        }) {
            return Err(unsupported("MP4 OD edit mapping is not implemented"));
        }
        timeline
            .prefix(chain, pts, scale)
            .map(Some)
            .map_err(|e| invalid(&e))
    }
}
