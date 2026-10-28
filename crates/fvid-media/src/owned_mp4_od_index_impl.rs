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
        self.object_descriptor_prefix_context(chain, pts, scale, None)
    }
    /// Select the OD state for a specific presentation edit. None names the
    /// implicit unedited root; it is distinct from omitting presentation context.
    pub fn object_descriptor_prefix_in_edit(
        &self,
        chain: &[u32],
        pts: i64,
        scale: u32,
        edit: Option<usize>,
    ) -> Result<Option<usize>> {
        self.object_descriptor_prefix_context(chain, pts, scale, Some(edit))
    }
    pub fn has_object_descriptor(&self, root: u32) -> bool {
        self.object_timelines
            .iter()
            .any(|t| t.references.contains(&root))
    }
    fn object_descriptor_prefix_context(
        &self,
        chain: &[u32],
        pts: i64,
        scale: u32,
        context: Option<Option<usize>>,
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
        let root = self
            .tracks
            .iter()
            .find(|t| t.id == *root)
            .ok_or_else(|| invalid("MP4 OD root track is unavailable"))?;
        let controller = self
            .tracks
            .iter()
            .find(|t| t.id == timeline.track_id)
            .ok_or_else(|| invalid("MP4 OD controller track is unavailable"))?;
        if root.edits.is_empty() && context.is_some_and(|edit| edit.is_some()) {
            return Err(invalid("MP4 OD root edit context is unavailable"));
        }
        if root.edits.is_empty() && controller.edits.is_empty() {
            return timeline
                .prefix(chain, pts, scale)
                .map(Some)
                .map_err(|e| invalid(&e));
        }
        if scale == 0 {
            return Err(invalid("zero MP4 OD edit clock"));
        }
        // All coordinates use 1/(root_scale * od_scale * movie_scale).
        // A constant source-clock relation also applies during decoder preroll.
        // Incompatible repeated presentations require presentation-context dispatch;
        // never guess membership from whichever occurrence happens to come first.
        let offset = if let Some(edit) = context {
            od_edit_offset_at(root, controller, self.movie_timescale, pts, scale, edit)?
        } else {
            od_edit_relation(root, controller, self.movie_timescale)?
        };
        let root_unit = i128::from(timeline.timescale) * i128::from(self.movie_timescale);
        let event_unit = i128::from(scale) * i128::from(self.movie_timescale);
        let (mut a, mut b) = (root.timescale, scale);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let source_factor = i128::from(root.timescale / a);
        let query_factor = i128::from(scale / a);
        let at = i128::from(pts)
            .checked_mul(root_unit)
            .and_then(|n| n.checked_mul(source_factor))
            .and_then(|n| {
                offset
                    .checked_mul(query_factor)
                    .and_then(|o| n.checked_add(o))
            })
            .ok_or_else(|| invalid("MP4 OD edit clock overflow"))?;
        let event_unit = event_unit
            .checked_mul(source_factor)
            .ok_or_else(|| invalid("MP4 OD edit clock overflow"))?;
        timeline
            .prefix_units(chain, at, event_unit)
            .map(Some)
            .map_err(|e| invalid(&e))
    }
}

/// Compile a single source-clock relation over every presented root window.
/// Empty root edits have no AU; nonempty root windows require an OD mapping.
fn od_edit_relation(root: &Track, controller: &Track, movie_scale: u32) -> Result<i128> {
    let overflow = || invalid("MP4 OD edit clock overflow");
    let r = i128::from(root.timescale);
    let o = i128::from(controller.timescale);
    let m = i128::from(movie_scale);
    if r == 0 || o == 0 || m == 0 {
        return Err(invalid("zero MP4 OD edit clock"));
    }
    let movie_unit = r * o;
    let mut controllers = od_edit_spans(controller, r * m, movie_unit);
    let mut current = controllers.next().transpose()?;
    let mut relation = None;
    for root in od_edit_spans(root, o * m, movie_unit) {
        let root = root?;
        let Some(root_affine) = root.affine else {
            continue;
        };
        let mut covered = root.start;
        while covered < root.end {
            while current.is_some_and(|od| od.end <= covered) {
                current = controllers.next().transpose()?;
            }
            let od = current.ok_or_else(|| invalid("MP4 OD edit leaves active audio uncovered"))?;
            if od.start > covered {
                return Err(invalid("MP4 OD edit leaves active audio uncovered"));
            }
            let affine = od
                .affine
                .ok_or_else(|| invalid("MP4 OD empty edit overlaps active audio"))?;
            let offset = affine.checked_sub(root_affine).ok_or_else(overflow)?;
            if relation.is_some_and(|old| old != offset) {
                return Err(unsupported(
                    "MP4 OD edits require presentation-dependent membership",
                ));
            }
            relation = Some(offset);
            covered = root.end.min(od.end);
        }
    }
    relation.ok_or_else(|| invalid("MP4 OD has no active audio edit"))
}

#[derive(Clone, Copy)]
struct OdEditSpan {
    start: i128,
    end: i128,
    affine: Option<i128>,
}

fn od_edit_spans(
    track: &Track,
    unit: i128,
    movie_unit: i128,
) -> impl Iterator<Item = Result<OdEditSpan>> + '_ {
    let default = track
        .edits
        .is_empty()
        .then(|| {
            Ok(OdEditSpan {
                start: 0,
                end: i128::from(track.duration)
                    .checked_mul(unit)
                    .ok_or_else(|| invalid("MP4 OD edit clock overflow"))?,
                affine: Some(0),
            })
        })
        .into_iter();
    let edits = track.edits.iter().scan(0i128, move |start, edit| {
        Some((|| {
            let end = i128::from(edit.duration)
                .checked_mul(movie_unit)
                .and_then(|n| start.checked_add(n))
                .ok_or_else(|| invalid("MP4 OD edit clock overflow"))?;
            if end <= *start {
                return Err(invalid("empty MP4 OD edit"));
            }
            let affine = match edit.media_time {
                -1 => None,
                n if n >= 0 => Some(
                    i128::from(n)
                        .checked_mul(unit)
                        .and_then(|n| n.checked_sub(*start))
                        .ok_or_else(|| invalid("MP4 OD edit clock overflow"))?,
                ),
                _ => return Err(invalid("invalid MP4 OD media edit time")),
            };
            let span = OdEditSpan {
                start: *start,
                end,
                affine,
            };
            *start = end;
            Ok(span)
        })())
    });
    default.chain(edits)
}

fn od_edit_offset_at(
    root: &Track,
    controller: &Track,
    movie_scale: u32,
    pts: i64,
    scale: u32,
    edit: Option<usize>,
) -> Result<i128> {
    let fail = || invalid("MP4 OD edit clock overflow");
    let (r, o, m) = (
        i128::from(root.timescale),
        i128::from(controller.timescale),
        i128::from(movie_scale),
    );
    if r == 0 || o == 0 || m == 0 || scale == 0 {
        return Err(invalid("zero MP4 OD edit clock"));
    }
    if root.edits.is_empty() != edit.is_none() {
        return Err(invalid("MP4 OD root edit context is unavailable"));
    }
    let mut selected_root = None;
    for (index, span) in od_edit_spans(root, o * m, r * o).enumerate() {
        let span = span?;
        if index == edit.unwrap_or(0) {
            selected_root = Some(span);
            break;
        }
    }
    let root = selected_root.ok_or_else(|| invalid("MP4 OD root edit context is unavailable"))?;
    let affine = root
        .affine
        .ok_or_else(|| invalid("MP4 OD root edit is empty"))?;
    // Coordinates are scaled by the query clock so fractional root ticks
    // remain exact. Clamp only the *edit selection*, never the OD source time:
    // decoder preroll/lookahead extrapolates the boundary edit's own mapping.
    let q = i128::from(scale);
    let source = i128::from(pts)
        .checked_mul(o * m)
        .and_then(|n| n.checked_mul(r))
        .ok_or_else(fail)?;
    let movie = source
        .checked_sub(affine.checked_mul(q).ok_or_else(fail)?)
        .ok_or_else(fail)?;
    let first = root.start.checked_mul(q).ok_or_else(fail)?;
    let last = root.end.checked_mul(q).ok_or_else(fail)?;
    let after = movie >= last;
    let anchor = movie.max(first).min(last);
    let mut covered = root.start;
    let mut selected = None;
    for od in od_edit_spans(controller, r * m, r * o) {
        let od = od?;
        let begin = od.start.max(root.start);
        let end = od.end.min(root.end);
        if begin >= end {
            continue;
        }
        if begin != covered {
            return Err(invalid("MP4 OD edit leaves active audio uncovered"));
        }
        let od_affine = od
            .affine
            .ok_or_else(|| invalid("MP4 OD empty edit overlaps active audio"))?;
        let start = begin.checked_mul(q).ok_or_else(fail)?;
        let end_scaled = end.checked_mul(q).ok_or_else(fail)?;
        if if after {
            start < anchor && anchor <= end_scaled
        } else {
            start <= anchor && anchor < end_scaled
        } {
            selected = Some(od_affine.checked_sub(affine).ok_or_else(fail)?);
        }
        covered = end;
    }
    if covered != root.end {
        return Err(invalid("MP4 OD edit leaves active audio uncovered"));
    }
    selected.ok_or_else(|| invalid("MP4 OD edit context has no source mapping"))
}
