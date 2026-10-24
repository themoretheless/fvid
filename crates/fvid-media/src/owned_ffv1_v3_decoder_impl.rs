struct SliceHistory {
    region: [usize; 4],
    quantizers: [usize; 3],
    state: State,
}
struct V3 {
    config: Configuration,
    history: Vec<Option<Box<SliceHistory>>>,
    live: bool,
}
impl Decoder {
    pub fn from_configuration(
        width: usize,
        height: usize,
        private: &[u8],
        budget: usize,
    ) -> Result<Self> {
        let mut decoder = Self::new(width, height, budget)?;
        if private.is_empty() {
            return Ok(decoder);
        }
        let config = Configuration::parse(private, budget)?;
        if config
            .memory
            .checked_add(std::mem::size_of::<V3>() - std::mem::size_of::<Configuration>())
            .is_none_or(|n| n > budget)
        {
            return Err(invalid("FFV1 configuration exceeds memory budget"));
        }
        if config.grid[0] as usize > width || config.grid[1] as usize > height {
            return Err(invalid("FFV1 slice raster exceeds frame dimensions"));
        }
        decoder.v3 = Some(Box::new(V3 {
            config,
            history: Vec::new(),
            live: false,
        }));
        Ok(decoder)
    }
}
impl Configuration {
    fn change_quantizers(
        &self,
        state: &mut State,
        previous: [usize; 3],
        selected: [usize; 3],
        allowance: usize,
    ) -> Result<()> {
        if previous == selected {
            return Ok(());
        }
        let groups = if self.alpha { 3 } else { 2 };
        let mut sizes = [0usize; 3];
        let mut grow = [false; 3];
        for i in 0..groups {
            let before = self.sets[previous[i]].context_count();
            let after = self.sets[selected[i]].context_count();
            grow[i] = after > before;
            sizes[i] = if grow[i] {
                after
            } else if let Some(rice) = state.rice.as_ref() {
                rice[i].len()
            } else {
                state.models[i].len()
            };
        }
        let memory = std::mem::size_of::<State>()
            + state.tables.len() * std::mem::size_of::<[[i32; 256]; 5]>()
            + sizes.iter().sum::<usize>()
                * if self.coder == 0 {
                    std::mem::size_of::<RiceModel>()
                } else {
                    32
                };
        if memory > allowance {
            return Err(invalid(
                "FFV1 changed quantizer contexts exceed memory budget",
            ));
        }
        for i in 0..groups {
            if grow[i] {
                // An expanded bank is a new bank, not a prefix copied from the
                // old adaptation. Shrinking/equal-sized banks retain history.
                if let Some(rice) = state.rice.as_mut() {
                    rice[i] = Vec::new();
                    rice[i]
                        .try_reserve_exact(sizes[i])
                        .map_err(|_| invalid("cannot allocate FFV1 changed Rice contexts"))?;
                    rice[i].resize(sizes[i], RiceModel::default());
                } else {
                    state.models[i] = Vec::new();
                    state.models[i]
                        .try_reserve_exact(sizes[i])
                        .map_err(|_| invalid("cannot allocate FFV1 changed range contexts"))?;
                    state.models[i].extend_from_slice(&self.sets[selected[i]].initial);
                }
            }
            state.tables[i] = self.sets[selected[i]].tables;
        }
        Ok(())
    }
    fn state_for(&self, selected: [usize; 3], allowance: usize) -> Result<State> {
        let groups = if self.alpha { 3 } else { 2 };
        let contexts = selected
            .iter()
            .take(groups)
            .map(|&i| self.sets[i].context_count())
            .sum::<usize>();
        let bytes = std::mem::size_of::<State>()
            + groups * std::mem::size_of::<[[i32; 256]; 5]>()
            + contexts
                * if self.coder == 0 {
                    std::mem::size_of::<RiceModel>()
                } else {
                    32
                };
        if bytes > allowance {
            return Err(invalid("FFV1 slice contexts exceed memory budget"));
        }
        let mut models = [Vec::new(), Vec::new(), Vec::new()];
        let mut rice = if self.coder == 0 {
            Some([Vec::new(), Vec::new(), Vec::new()])
        } else {
            None
        };
        let mut tables = Vec::new();
        tables
            .try_reserve_exact(groups)
            .map_err(|_| invalid("cannot allocate FFV1 slice quantizers"))?;
        for i in 0..groups {
            let set = &self.sets[selected[i]];
            tables.push(set.tables);
            if let Some(rice) = rice.as_mut() {
                rice[i]
                    .try_reserve_exact(set.context_count())
                    .map_err(|_| invalid("cannot allocate FFV1 slice Rice contexts"))?;
                rice[i].resize(set.context_count(), RiceModel::default());
            } else {
                models[i]
                    .try_reserve_exact(set.context_count())
                    .map_err(|_| invalid("cannot allocate FFV1 slice contexts"))?;
                models[i].extend_from_slice(&set.initial);
            }
        }
        Ok(State {
            rice,
            has_alpha: self.alpha,
            rgb: self.rgb,
            depth: self.depth,
            has_chroma: self.chroma,
            shifts: self.shifts,
            tables,
            transitions: self.transitions,
            models,
        })
    }
}
impl V3 {
    fn decode(
        &mut self,
        packet: &[u8],
        width: usize,
        height: usize,
        budget: usize,
    ) -> Result<Decoded> {
        let slices = self.config.slices(packet, budget)?;
        let mut first = Range::new(slices[0].payload)?;
        first.transitions = self.config.transitions;
        let keyframe = first.bit(&mut 128)?;
        let mut first = Some(first);
        if self.config.intra && !keyframe {
            return Err(invalid("FFV1 intra stream contains a nonkeyframe"));
        }
        if !keyframe && !self.live {
            return Err(invalid("FFV1 frame requires a preceding keyframe"));
        }
        let factors = if self.config.chroma {
            self.config.shifts.map(|s| 1usize << s)
        } else {
            [1, 1]
        };
        let bytes = if self.config.depth == 8 { 1 } else { 2 };
        let chroma = width
            .div_ceil(factors[0])
            .checked_mul(height.div_ceil(factors[1]))
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let samples = width
            .checked_mul(height)
            .and_then(|n| {
                if self.config.rgb {
                    n.checked_mul(3)
                } else {
                    chroma.checked_mul(2).and_then(|c| n.checked_add(c))
                }
            })
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let length = samples
            .checked_mul(bytes)
            .ok_or_else(|| invalid("FFV1 frame size overflow"))?;
        let alpha_length = if self.config.alpha {
            width
                .checked_mul(height)
                .and_then(|n| n.checked_mul(bytes))
                .ok_or_else(|| invalid("FFV1 alpha size overflow"))?
        } else {
            0
        };
        let cells = (self.config.grid[0] as usize)
            .checked_mul(self.config.grid[1] as usize)
            .ok_or_else(|| invalid("FFV1 raster size overflow"))?;
        let base = cells
            .checked_mul(2 * std::mem::size_of::<Option<Box<SliceHistory>>>() + 1)
            .and_then(|n| n.checked_add(length))
            .and_then(|n| n.checked_add(alpha_length))
            .and_then(|n| n.checked_add(self.config.memory))
            .and_then(|n| {
                n.checked_add(std::mem::size_of::<V3>() - std::mem::size_of::<Configuration>())
            })
            .and_then(|n| n.checked_add(slices.len() * std::mem::size_of::<Slice<'_>>()))
            .ok_or_else(|| invalid("FFV1 v3 storage overflow"))?;
        if base > budget {
            return Err(invalid("FFV1 v3 frame exceeds memory budget"));
        }
        let mut old = std::mem::take(&mut self.history);
        if keyframe {
            old.clear();
        }
        let overhead = std::mem::size_of::<SliceHistory>() - std::mem::size_of::<State>();
        let mut model_bytes = old
            .iter()
            .flatten()
            .try_fold(0usize, |sum, h| {
                sum.checked_add(h.state.context_bytes() + overhead)
            })
            .ok_or_else(|| invalid("FFV1 slice state storage overflow"))?;
        if model_bytes > budget - base {
            return Err(invalid("FFV1 slice contexts exceed memory budget"));
        }
        let mut history = Vec::new();
        history
            .try_reserve_exact(cells)
            .map_err(|_| invalid("cannot allocate FFV1 slice history"))?;
        history.resize_with(cells, || None);
        let mut covered = Vec::new();
        covered
            .try_reserve_exact(cells)
            .map_err(|_| invalid("cannot allocate FFV1 slice coverage"))?;
        covered.resize(cells, false);
        let mut output = GeometryFrame {
            width,
            height,
            subsampling: if self.config.rgb { None } else { Some(factors) },
            data: buffer(length)?,
        };
        let mut alpha = if self.config.alpha {
            Some(buffer(alpha_length)?)
        } else {
            None
        };
        for (index, slice) in slices.iter().enumerate() {
            let mut r = if index == 0 {
                // Keep the interval after the frame keyframe flag.
                first.take().unwrap()
            } else {
                let mut r = Range::new(slice.payload)?;
                r.transitions = self.config.transitions;
                r
            };
            let mut h = [128; 32];
            let x = r.integer(&mut h, false)? as usize;
            let y = r.integer(&mut h, false)? as usize;
            let sw = r.integer(&mut h, false)? as usize + 1;
            let sh = r.integer(&mut h, false)? as usize + 1;
            let [gw, gh] = self.config.grid.map(|n| n as usize);
            if x.checked_add(sw).is_none_or(|n| n > gw) || y.checked_add(sh).is_none_or(|n| n > gh)
            {
                return Err(invalid("FFV1 slice region exceeds raster"));
            }
            let quantizers = [
                r.integer(&mut h, false)? as usize,
                r.integer(&mut h, false)? as usize,
                if self.config.alpha {
                    r.integer(&mut h, false)? as usize
                } else {
                    0
                },
            ];
            if quantizers.iter().any(|&i| i >= self.config.sets.len()) {
                return Err(invalid("invalid FFV1 slice quantizer index"));
            }
            if r.integer(&mut h, false)? > 3 {
                return Err(invalid("invalid FFV1 picture structure"));
            }
            let _sar = [r.integer(&mut h, false)?, r.integer(&mut h, false)?];
            for row in y..y + sh {
                for col in x..x + sw {
                    let slot = &mut covered[row * gw + col];
                    if *slot {
                        return Err(invalid("overlapping FFV1 slices"));
                    }
                    *slot = true;
                }
            }
            let region = [x, y, sw, sh];
            let slot = y * gw + x;
            let state = if keyframe {
                let allowance = budget
                    .checked_sub(base)
                    .and_then(|n| n.checked_sub(model_bytes))
                    .and_then(|n| n.checked_sub(overhead))
                    .ok_or_else(|| invalid("FFV1 slice contexts exceed memory budget"))?;
                let state = self.config.state_for(quantizers, allowance)?;
                model_bytes = model_bytes
                    .checked_add(state.context_bytes() + overhead)
                    .ok_or_else(|| invalid("FFV1 slice context storage overflow"))?;
                state
            } else {
                let previous = old
                    .get_mut(slot)
                    .and_then(Option::take)
                    .ok_or_else(|| invalid("FFV1 nonkeyframe changes slice layout"))?;
                if previous.region != region {
                    return Err(invalid("FFV1 nonkeyframe changes slice layout"));
                }
                let mut state = previous.state;
                let before = state.context_bytes();
                let other = model_bytes - before;
                let allowance = budget
                    .checked_sub(base)
                    .and_then(|n| n.checked_sub(other))
                    .ok_or_else(|| invalid("FFV1 slice contexts exceed memory budget"))?;
                self.config.change_quantizers(
                    &mut state,
                    previous.quantizers,
                    quantizers,
                    allowance,
                )?;
                model_bytes = other + state.context_bytes();
                state
            };
            let left = (x as u64 * width as u64 / gw as u64) as usize;
            let top = (y as u64 * height as u64 / gh as u64) as usize;
            let right = ((x + sw) as u64 * width as u64 / gw as u64) as usize;
            let bottom = ((y + sh) as u64 * height as u64 / gh as u64) as usize;
            let mut rice = if self.config.coder == 0 {
                r.bit(&mut 129)?;
                let at =
                    r.at.checked_sub(1)
                        .filter(|&at| at <= slice.payload.len())
                        .ok_or_else(|| invalid("truncated FFV1 Golomb Rice slice header"))?;
                Some(Rice {
                    data: &slice.payload[at..],
                    at: 0,
                })
            } else {
                None
            };
            let allowance = budget - base - (model_bytes - state.context_bytes());
            let (decoded, state) = decode_content(
                right - left,
                bottom - top,
                allowance,
                &mut r,
                &mut rice,
                state,
                keyframe,
            )?;
            merge_slice(&mut output, &decoded.frame, left, top, bytes)?;
            match (alpha.as_mut(), decoded.alpha.as_ref()) {
                (Some(target), Some(source)) => {
                    let sw = right - left;
                    let sh = bottom - top;
                    if source.len() != sw * sh * bytes {
                        return Err(invalid("invalid FFV1 slice alpha storage"));
                    }
                    for row in 0..sh {
                        let from = row * sw * bytes;
                        let to = ((top + row) * width + left) * bytes;
                        target[to..to + sw * bytes]
                            .copy_from_slice(&source[from..from + sw * bytes]);
                    }
                }
                (None, None) => {}
                _ => return Err(invalid("FFV1 slice alpha layout mismatch")),
            }

            history[slot] = Some(Box::new(SliceHistory {
                region,
                quantizers,
                state,
            }));
        }
        if covered.iter().any(|&v| !v) {
            return Err(invalid("FFV1 frame has missing slices"));
        }
        if old.iter().any(Option::is_some) {
            return Err(invalid("FFV1 nonkeyframe changes slice layout"));
        }
        self.history = history;
        self.live = true;
        Ok(Decoded {
            frame: output,
            alpha,
            depth: self.config.depth,
            keyframe,
        })
    }
}
fn merge_slice(
    output: &mut GeometryFrame,
    slice: &GeometryFrame,
    x: usize,
    y: usize,
    bytes: usize,
) -> Result<()> {
    if output.subsampling != slice.subsampling {
        return Err(invalid("FFV1 slice layout mismatch"));
    }
    let [sx, sy] = output.subsampling.unwrap_or([1, 1]);
    let mut source_offset = 0;
    let mut target_offset = 0;
    for component in 0..if output.subsampling.is_none() { 1 } else { 3 } {
        let (dx, dy, pixel) = if output.subsampling.is_none() {
            (1, 1, 3 * bytes)
        } else if component == 0 {
            (1, 1, bytes)
        } else {
            (sx, sy, bytes)
        };
        let (w, h) = (slice.width.div_ceil(dx), slice.height.div_ceil(dy));
        let (tw, th) = (output.width.div_ceil(dx), output.height.div_ceil(dy));
        let (ox, oy) = (x / dx, y / dy);
        if ox.checked_add(w).is_none_or(|n| n > tw) || oy.checked_add(h).is_none_or(|n| n > th) {
            return Err(invalid("FFV1 slice pixels exceed output frame"));
        }
        for row in 0..h {
            let from = source_offset + row * w * pixel;
            let to = target_offset + ((oy + row) * tw + ox) * pixel;
            output.data[to..to + w * pixel].copy_from_slice(&slice.data[from..from + w * pixel]);
        }
        source_offset += w * h * pixel;
        target_offset += tw * th * pixel;
    }
    Ok(())
}
