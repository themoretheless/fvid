//! Native HEVC I/P/B reconstruction with independent slices, WPP and in-loop filters.
use super::{
    hevc_block,
    hevc_cabac::{HevcCabac, SliceType, Syntax},
    hevc_inter_syntax::{self, Partition},
    hevc_intra_syntax,
    hevc_motion::{self, Motion, Reference, Spatial},
    hevc_plane::Plane,
    hevc_pps::Pps,
    hevc_qp, hevc_sao,
    hevc_slice::SliceHeader,
    hevc_sps::Sps,
    hevc_transform_tree,
    hevc_tree::{self, Node, Visitor},
};
use crate::{Result, invalid};

pub struct Picture {
    #[cfg(test)]
    pub(crate) pcm_luma_samples: usize,
    #[cfg(test)]
    pub(crate) cross_component_blocks: usize,
    #[cfg(test)]
    pub(crate) act_blocks: usize,
    #[cfg(test)]
    pub(crate) current_picture_blocks: usize,
    #[cfg(test)]
    pub(crate) completed_picture_blocks: usize,
    #[cfg(test)]
    pub(crate) bipredicted_blocks: usize,
    #[cfg(test)]
    pub(crate) fractional_current_chroma_blocks: usize,
    pub dimensions: [u32; 2],
    pub crop: [u32; 4],
    pub depth: [u8; 2],
    pub planes: [Plane; 3],
    /// Resolved SAO parameters in raster CTU order, including merged values.
    pub sao: Vec<hevc_sao::CtuSao>,
    /// Collocated motion at the normative 16x16 temporal-prediction grid.
    pub(crate) motion: Vec<Motion>,
    pub(crate) reference_pocs: [Vec<i32>; 2],
    pub(crate) reference_long_term: [Vec<bool>; 2],
    /// Independent reconstructed/motion state for separately coded planes.
    pub(crate) colour_planes: Option<[std::sync::Arc<Picture>; 3]>,
}
fn component_shifts(component: usize, format: u8) -> [usize; 2] {
    if component == 0 {
        [0, 0]
    } else {
        match format {
            1 => [1, 1],
            2 => [1, 0],
            _ => [0, 0],
        }
    }
}
/// Decode a complete Main/Main10 IDR slice, including WPP, QP deltas and filters.
/// Output planes have coded dimensions; crop is metadata. The supplied budget
/// covers owned pixels, CU/PB metadata and reconstruction scratch storage.
pub fn decode_idr(sps: &Sps, pps: &Pps, slice: &SliceHeader, budget: usize) -> Result<Picture> {
    if !slice.nal.is_idr() || slice.slice_type != SliceType::I {
        return Err(invalid("expected HEVC IDR picture"));
    }
    decode(sps, pps, slice, 0, &[Vec::new(), Vec::new()], budget)
}
pub fn decode(
    sps: &Sps,
    pps: &Pps,
    slice: &SliceHeader,
    poc: i32,
    lists: &[Vec<Reference>; 2],
    budget: usize,
) -> Result<Picture> {
    if sps.chroma_format > 3
        || sps.separate_colour_plane
        || !slice.first
        || slice.address != 0
        || slice.nal.layer_id != 0
    {
        return Err(crate::unsupported("unsupported HEVC picture tools"));
    }
    if slice.slice_type != SliceType::I && !(1..=5).contains(&slice.max_merge_candidates) {
        return Err(invalid("invalid HEVC merge candidate count"));
    }
    for list in 0..2 {
        if lists[list].len() != slice.references[list] as usize {
            return Err(invalid("HEVC reference-list length mismatch"));
        }
        for r in &lists[list] {
            if r.picture.is_none()
                && (!pps.current_picture_reference || r.poc != poc || !r.long_term)
            {
                return Err(invalid("invalid HEVC current-picture reference descriptor"));
            }
            if r.picture
                .as_ref()
                .is_some_and(|p| p.dimensions != sps.dimensions || p.depth != sps.depth)
            {
                return Err(invalid("HEVC reference geometry mismatch"));
            }
        }
    }
    let [min_cb, max_cb] = sps.coding_block_log2;
    let [min_tb, max_tb] = sps.transform_block_log2;
    if !(4..=6).contains(&max_cb)
        || !(3..=max_cb).contains(&min_cb)
        || !(2..=min_cb.min(5)).contains(&min_tb)
        || !(min_tb..=max_cb.min(5)).contains(&max_tb)
        || sps.transform_hierarchy_depth[1] > max_cb - min_tb
        || sps.depth.iter().any(|d| !(8..=16).contains(d))
        || sps.id != pps.sps_id
        || pps.id != slice.pps_id
    {
        return Err(invalid("invalid HEVC picture parameters"));
    }
    let [w, h] = sps.dimensions;
    if w == 0
        || h == 0
        || w % (1 << min_cb) != 0
        || h % (1 << min_cb) != 0
        || u64::from(sps.crop[0]) + u64::from(sps.crop[1]) >= u64::from(w)
        || u64::from(sps.crop[2]) + u64::from(sps.crop[3]) >= u64::from(h)
    {
        return Err(invalid("invalid HEVC picture dimensions or crop"));
    }
    let count = (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| invalid("HEVC picture size overflow"))?;
    let [chroma_shift_x, chroma_shift_y] = component_shifts(1, sps.chroma_format);
    let required = count
        .checked_mul(24)
        .and_then(|n| n.checked_add(65536))
        .ok_or_else(|| invalid("HEVC picture budget overflow"))?;
    if required > budget {
        return Err(invalid("HEVC picture exceeds decode budget"));
    }
    let tile_layout = pps
        .tiles
        .as_ref()
        .map(|tiles| {
            let side = 1u32 << max_cb;
            super::hevc_tiles::TileLayout::new(
                tiles,
                [w.div_ceil(side), h.div_ceil(side)],
                budget - required,
            )
        })
        .transpose()?;
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components_with_format(
        slice.qp,
        sps.depth,
        chroma_offsets,
        [0; 2],
        sps.chroma_format,
    )?;

    let mut decoder = Decoder {
        palette: if sps.palette.is_some() {
            Some(super::hevc_palette::Predictor::from_parameters(sps, pps)?)
        } else {
            None
        },
        sps,
        pps,
        slice,
        poc,
        lists,
        slice_start: 0,
        tile_bounds: None,
        tile_layout,
        qp: slice.qp,
        chroma_offsets,
        cu_chroma_offsets: [0; 2],
        chroma_qp_coded: false,
        qp_coded: false,
        qp_prediction: slice.qp,
        qp_grid: vec![slice.qp; count / 64],
        edges: vec![[0; 2]; count / 16],
        cells: vec![Cell::default(); count / 16],
        jobs: None,
        reconstruction: Vec::new(),
        scratch: Vec::new(),
        pred_scratch: Vec::new(),
        transform_scratch: Vec::new(),
        residual_scratch: Vec::new(),
        #[cfg(test)]
        cross_component_blocks: 0,
        #[cfg(test)]
        act_blocks: 0,
        #[cfg(test)]
        current_picture_blocks: 0,
        #[cfg(test)]
        completed_picture_blocks: 0,
        #[cfg(test)]
        bipredicted_blocks: 0,
        #[cfg(test)]
        fractional_current_chroma_blocks: 0,
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let mut sao = Vec::with_capacity(columns as usize * rows as usize);
    let expected = if let Some(layout) = &decoder.tile_layout {
        if pps.entropy_sync {
            layout.rectangles.iter().map(|r| r[3] as usize).sum()
        } else {
            layout.rectangles.len()
        }
    } else if pps.entropy_sync {
        rows as usize
    } else {
        1
    };
    if slice.entropy_substreams.len() != expected {
        return Err(invalid(
            "HEVC entropy substream count does not match tile/row layout",
        ));
    }
    if slice
        .entropy_substreams
        .iter()
        .any(|r| r.start >= r.end || r.end > slice.rbsp.len())
    {
        return Err(invalid("invalid HEVC entropy substream bounds"));
    }
    let mut bins = HevcCabac::new(
        &slice.rbsp[slice.entropy_substreams[0].clone()],
        0,
        slice.slice_type,
        slice.cabac_init,
        slice.qp,
    )?;
    let initial_contexts = bins.contexts()?;
    let mut saved_contexts = None;
    let initial_palette = decoder.palette.clone();
    let mut saved_palette = None;
    if let Some(layout) = decoder.tile_layout.take() {
        sao = decode_tile_ctus(&mut decoder, slice, &layout)?;
        decoder.tile_layout = Some(layout);
    }
    std::thread::scope(|scope| -> Result<()> {
        if decoder.tile_layout.is_some() {
            return Ok(());
        }
        let workers = if count >= 128 * 96 && !pps.constrained_intra {
            let (sender, receiver) = std::sync::mpsc::sync_channel(rows as usize);
            let placeholders = [
                Plane::new(1, 1, 8, 3)?,
                Plane::new(1, 1, 8, 3)?,
                Plane::new(1, 1, 8, 3)?,
            ];
            let planes = std::mem::replace(&mut decoder.planes, placeholders);
            decoder.jobs = Some(sender);
            // Reconstruction owns shared sample planes and waits for every
            // preceding row. More consumers cannot run concurrently; one worker
            // overlaps reconstruction with parsing without per-row wake-up races.
            let num_workers = 1;
            let completed =
                std::sync::Arc::new((std::sync::Mutex::new(0usize), std::sync::Condvar::new()));
            let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
            let planes = std::sync::Arc::new(std::sync::Mutex::new(planes));
            let handles: Vec<_> = (0..num_workers)
                .map(|_| {
                    let receiver = receiver.clone();
                    let planes = planes.clone();
                    let completed = completed.clone();
                    scope.spawn(move || -> Result<()> {
                        #[cfg(feature = "player")]
                        fvid_platform::prioritize_playback_thread();
                        let mut scratch = Vec::new();
                        let mut pred_scratch = Vec::new();
                        let mut transform_scratch = Vec::new();
                        let mut residual_scratch = Vec::new();
                        loop {
                            let (row, commands) = {
                                let rx = receiver.lock().unwrap_or_else(|e| e.into_inner());
                                match rx.recv() {
                                    Ok(data) => data,
                                    Err(_) => return Ok(()),
                                }
                            };
                            let (lock, cvar) = &*completed;
                            {
                                let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
                                while *done < row {
                                    done = cvar.wait(done).unwrap_or_else(|e| e.into_inner());
                                }
                            }
                            {
                                let mut planes = planes.lock().unwrap_or_else(|e| e.into_inner());
                                reconstruct_row(
                                    &mut planes,
                                    commands,
                                    sps,
                                    pps,
                                    slice,
                                    lists,
                                    &mut scratch,
                                    &mut pred_scratch,
                                    &mut transform_scratch,
                                    &mut residual_scratch,
                                )?;
                            }
                            {
                                let mut done = lock.lock().unwrap_or_else(|e| e.into_inner());
                                *done = row + 1;
                                cvar.notify_all();
                            }
                        }
                    })
                })
                .collect();
            Some((handles, planes, completed))
        } else {
            None
        };
        let parsed = (|| -> Result<()> {
            for row in 0..rows {
                if pps.entropy_sync {
                    if row > 0 {
                        decoder.palette = saved_palette
                            .clone()
                            .unwrap_or_else(|| initial_palette.clone());
                        bins = HevcCabac::from_contexts(
                            &slice.rbsp[slice.entropy_substreams[row as usize].clone()],
                            0,
                            saved_contexts.as_ref().unwrap_or(&initial_contexts),
                        )?;
                    }
                    decoder.qp = slice.qp;
                }
                for col in 0..columns {
                    let index = sao.len();
                    let parameters = hevc_sao::read_ctu_with_scale(
                        &mut bins,
                        slice.sao,
                        sps.depth,
                        pps.sao_offset_scale,
                        if col > 0 { sao.get(index - 1) } else { None },
                        if row > 0 {
                            sao.get(index - columns as usize)
                        } else {
                            None
                        },
                    )?;
                    sao.push(parameters);
                    hevc_tree::read_ctu(
                        &mut bins,
                        &mut decoder,
                        [w, h],
                        [col * side, row * side],
                        max_cb,
                        min_cb,
                    )?;
                    if pps.entropy_sync && col == 1 {
                        saved_contexts = Some(bins.contexts()?);
                        saved_palette = Some(decoder.palette.clone());
                    }
                    let end = bins.terminate()?;
                    let last = row == rows - 1 && col == columns - 1;
                    if end != last {
                        return Err(invalid(
                            "HEVC slice termination does not match picture extent",
                        ));
                    }
                    if pps.entropy_sync && col == columns - 1 && !last && !bins.terminate()? {
                        return Err(invalid("missing Hevc end-of-substream bit"));
                    }
                }
                if let Some(sender) = &decoder.jobs {
                    sender
                        .send((row as usize, std::mem::take(&mut decoder.reconstruction)))
                        .map_err(|_| invalid("HEVC reconstruction worker stopped"))?;
                }
            }
            Ok(())
        })();
        // Close the channel on every parse exit, including truncated bitstreams.
        decoder.jobs.take();
        if let Some((handles, planes, _)) = workers {
            for handle in handles {
                handle
                    .join()
                    .map_err(|_| invalid("HEVC reconstruction worker panicked"))??;
            }
            parsed?;
            decoder.planes = std::sync::Arc::try_unwrap(planes)
                .map_err(|_| invalid("HEVC reconstruction planes still shared"))?
                .into_inner()
                .unwrap_or_else(|e| e.into_inner());
        } else {
            parsed?;
        }
        Ok(())
    })?;
    if !decoder.planes.iter().all(Plane::complete) {
        return Err(invalid("incomplete HEVC picture"));
    }
    if !slice.deblocking.disabled {
        deblock(&mut decoder, slice)?;
    }
    if slice.sao != [false, false] {
        for (component, plane) in decoder
            .planes
            .iter_mut()
            .take(if sps.chroma_format == 0 { 1 } else { 3 })
            .enumerate()
        {
            let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
            let [shift_x, shift_y] = component_shifts(component, sps.chroma_format);
            let cells = &decoder.cells;
            let stride = sps.dimensions[0] as usize / 4;
            plane.apply_sao_rectangular(
                [max_cb - shift_x as u8, max_cb - shift_y as u8],
                &parameters,
                |a, b| {
                    if let Some(layout) = &decoder.tile_layout
                        && !pps.tiles.as_ref().unwrap().loop_filter_across
                    {
                        let side = 1usize << max_cb;
                        let address = |p: [usize; 2]| {
                            ((p[1] << shift_y) / side) * columns as usize + (p[0] << shift_x) / side
                        };
                        return layout.tile_ids[address(a)] == layout.tile_ids[address(b)];
                    }
                    true
                },
                |p| {
                    let c = cells[((p[1] << shift_y) / 4) * stride + (p[0] << shift_x) / 4];
                    c.bypass || (c.pcm && sps.pcm.as_ref().is_some_and(|p| p.loop_filter_disabled))
                },
            )?;
        }
    }
    let motion = if slice.slice_type == SliceType::I {
        Vec::new()
    } else {
        let mut grid = Vec::with_capacity(w.div_ceil(16) as usize * h.div_ceil(16) as usize);
        for y in (0..h).step_by(16) {
            for x in (0..w).step_by(16) {
                grid.push(decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4].motion);
            }
        }
        grid
    };
    Ok(Picture {
        colour_planes: None,
        #[cfg(test)]
        pcm_luma_samples: decoder.cells.iter().filter(|c| c.pcm).count() * 16,
        #[cfg(test)]
        cross_component_blocks: decoder.cross_component_blocks,
        #[cfg(test)]
        act_blocks: decoder.act_blocks,
        #[cfg(test)]
        current_picture_blocks: decoder.current_picture_blocks,
        #[cfg(test)]
        completed_picture_blocks: decoder.completed_picture_blocks,
        #[cfg(test)]
        bipredicted_blocks: decoder.bipredicted_blocks,
        #[cfg(test)]
        fractional_current_chroma_blocks: decoder.fractional_current_chroma_blocks,
        motion,
        reference_pocs: std::array::from_fn(|l| lists[l].iter().map(|r| r.poc).collect()),
        reference_long_term: std::array::from_fn(|l| {
            lists[l].iter().map(|r| r.long_term).collect()
        }),
        dimensions: [w, h],
        crop: sps.crop,
        depth: sps.depth,
        planes: decoder.planes,
        sao,
    })
}

/// Reconstruct independent slice segments into one shared picture.
pub fn decode_slices(
    sps: &Sps,
    pps: &Pps,
    slices: &[SliceHeader],
    poc: i32,
    slice_lists: &[[Vec<Reference>; 2]],
    budget: usize,
) -> Result<Picture> {
    if sps.separate_colour_plane {
        return super::hevc_colour_planes::decode(sps, pps, slices, poc, slice_lists, budget);
    }
    if slices.len() != slice_lists.len() {
        return Err(invalid("HEVC slice reference count mismatch"));
    }
    if slices.len() == 1 {
        return decode(sps, pps, &slices[0], poc, &slice_lists[0], budget);
    }
    let slice = slices
        .first()
        .ok_or_else(|| invalid("empty HEVC slice set"))?;
    if slice_lists.len() != slices.len() {
        return Err(invalid("HEVC slice reference count mismatch"));
    }
    let first_lists = &slice_lists[0];
    let mut canonical: [Vec<Reference>; 2] = [Vec::new(), Vec::new()];
    for lists in slice_lists {
        for list in 0..2 {
            for reference in &lists[list] {
                if !canonical[list].iter().any(|r| r.poc == reference.poc) {
                    if canonical[list].len() == 256 {
                        return Err(invalid("HEVC picture reference map exceeds index range"));
                    }
                    canonical[list].push(reference.clone());
                }
            }
        }
    }
    if sps.chroma_format > 3 || sps.separate_colour_plane {
        return Err(crate::unsupported(
            "unsupported HEVC multi-slice picture tools",
        ));
    }
    for (index, slice) in slices.iter().enumerate() {
        if slice.first != (index == 0)
            || (index == 0 && slice.address != 0)
            || slice.nal.layer_id != 0
        {
            return Err(invalid("invalid HEVC slice partition start"));
        }
        if slice
            .entropy_substreams
            .iter()
            .any(|r| r.start >= r.end || r.end > slice.rbsp.len())
        {
            return Err(invalid("invalid HEVC entropy substream bounds"));
        }
        let reference_lists = &slice_lists[index];
        if slice.slice_type != SliceType::I && !(1..=5).contains(&slice.max_merge_candidates) {
            return Err(invalid("invalid HEVC merge candidate count"));
        }
        for list in 0..2 {
            if reference_lists[list].len() != slice.references[list] as usize {
                return Err(invalid("HEVC reference-list length mismatch"));
            }
            for r in &reference_lists[list] {
                if r.picture
                    .as_ref()
                    .is_some_and(|p| p.dimensions != sps.dimensions || p.depth != sps.depth)
                {
                    return Err(invalid("HEVC reference geometry mismatch"));
                }
            }
        }
        let [min_cb, max_cb] = sps.coding_block_log2;
        let [min_tb, max_tb] = sps.transform_block_log2;
        if !(4..=6).contains(&max_cb)
            || !(3..=max_cb).contains(&min_cb)
            || !(2..=min_cb.min(5)).contains(&min_tb)
            || !(min_tb..=max_cb.min(5)).contains(&max_tb)
            || sps.transform_hierarchy_depth[1] > max_cb - min_tb
            || sps.depth.iter().any(|d| !(8..=16).contains(d))
            || sps.id != pps.sps_id
            || pps.id != slice.pps_id
        {
            return Err(invalid("invalid HEVC picture parameters"));
        }
        let [w, h] = sps.dimensions;
        if w == 0
            || h == 0
            || w % (1 << min_cb) != 0
            || h % (1 << min_cb) != 0
            || u64::from(sps.crop[0]) + u64::from(sps.crop[1]) >= u64::from(w)
            || u64::from(sps.crop[2]) + u64::from(sps.crop[3]) >= u64::from(h)
        {
            return Err(invalid("invalid HEVC picture dimensions or crop"));
        }
        let count = (w as usize)
            .checked_mul(h as usize)
            .ok_or_else(|| invalid("HEVC picture size overflow"))?;
        let required = count
            .checked_mul(24)
            .and_then(|n| n.checked_add(65536))
            .ok_or_else(|| invalid("HEVC picture budget overflow"))?;
        if required > budget {
            return Err(invalid("HEVC picture exceeds decode budget"));
        }
    }
    let [min_cb, max_cb] = sps.coding_block_log2;
    let [w, h] = sps.dimensions;
    let count = w as usize * h as usize;
    let [chroma_shift_x, chroma_shift_y] = component_shifts(1, sps.chroma_format);
    let tile_layout = pps
        .tiles
        .as_ref()
        .map(|tiles| {
            let side = 1u32 << max_cb;
            super::hevc_tiles::TileLayout::new(
                tiles,
                [w.div_ceil(side), h.div_ceil(side)],
                budget - (count * 24 + 65536),
            )
        })
        .transpose()?;
    let lists = first_lists;
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components_with_format(
        slice.qp,
        sps.depth,
        chroma_offsets,
        [0; 2],
        sps.chroma_format,
    )?;

    let mut decoder = Decoder {
        palette: if sps.palette.is_some() {
            Some(super::hevc_palette::Predictor::from_parameters(sps, pps)?)
        } else {
            None
        },
        sps,
        pps,
        slice,
        poc,
        lists,
        slice_start: 0,
        tile_bounds: None,
        tile_layout,
        qp: slice.qp,
        chroma_offsets,
        cu_chroma_offsets: [0; 2],
        chroma_qp_coded: false,
        qp_coded: false,
        qp_prediction: slice.qp,
        qp_grid: vec![slice.qp; count / 64],
        edges: vec![[0; 2]; count / 16],
        cells: vec![Cell::default(); count / 16],
        jobs: None,
        reconstruction: Vec::new(),
        scratch: Vec::new(),
        pred_scratch: Vec::new(),
        transform_scratch: Vec::new(),
        residual_scratch: Vec::new(),
        #[cfg(test)]
        cross_component_blocks: 0,
        #[cfg(test)]
        act_blocks: 0,
        #[cfg(test)]
        current_picture_blocks: 0,
        #[cfg(test)]
        completed_picture_blocks: 0,
        #[cfg(test)]
        bipredicted_blocks: 0,
        #[cfg(test)]
        fractional_current_chroma_blocks: 0,
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
            if sps.chroma_format == 0 {
                Plane::absent(sps.depth[1])
            } else {
                Plane::new(
                    w as usize >> chroma_shift_x,
                    h as usize >> chroma_shift_y,
                    sps.depth[1],
                    (count * 3) >> (chroma_shift_x + chroma_shift_y),
                )?
            },
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let total = columns
        .checked_mul(rows)
        .ok_or_else(|| invalid("HEVC CTU grid overflow"))?;
    if slices.iter().any(|s| s.address >= total) {
        return Err(invalid("HEVC slice address is outside the picture"));
    }
    let mut sao = vec![[hevc_sao::Sao::Off; 3]; total as usize];
    let scan = |raster: u32| tile_scan(&decoder.tile_layout, raster);
    let starts: Vec<_> = slices.iter().map(|s| scan(s.address)).collect();
    let initial_palette = decoder.palette.clone();
    let mut saved_palette = None;
    let mut previous_contexts = None;
    let mut saved = None;
    let mut visited = 0;
    for (index, slice) in slices.iter().enumerate() {
        let begin = starts[index];
        let end = starts.get(index + 1).copied().unwrap_or(total);
        if begin != visited || begin >= end || end > total || slice.pps_id != pps.id {
            return Err(invalid("HEVC slices do not partition the picture"));
        }
        decoder.slice = slice;
        decoder.lists = &slice_lists[index];
        if !slice.dependent {
            decoder.palette = initial_palette.clone();
            saved_palette = None;
            decoder.slice_start = begin;
            decoder.qp = slice.qp;
            decoder.qp_prediction = slice.qp;
            decoder.qp_coded = false;
            saved = None;
        }
        decoder.cu_chroma_offsets = [0; 2];
        decoder.chroma_qp_coded = false;
        decoder.chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
        let expected = 1
            + (begin + 1..end)
                .filter(|&a| {
                    let (_, _, _, _, tile_start, row_start) =
                        ctu_position(&decoder.tile_layout, a, columns, rows);
                    tile_start || (pps.entropy_sync && row_start)
                })
                .count();
        if slice.entropy_substreams.len() != expected
            || slice
                .entropy_substreams
                .iter()
                .any(|r| r.start >= r.end || r.end > slice.rbsp.len())
        {
            return Err(invalid(
                "HEVC segment entropy substreams do not match tile/row layout",
            ));
        }
        let mut bins = HevcCabac::new(
            &slice.rbsp[slice.entropy_substreams[0].clone()],
            0,
            slice.slice_type,
            slice.cabac_init,
            slice.qp,
        )?;
        let initial = bins.contexts()?;
        let (_, _, _, _, tile_start, row_start) =
            ctu_position(&decoder.tile_layout, begin, columns, rows);
        if slice.dependent && !tile_start {
            if pps.entropy_sync && row_start {
                decoder.palette = saved_palette
                    .clone()
                    .unwrap_or_else(|| initial_palette.clone());
            }
            let contexts = if pps.entropy_sync && row_start {
                saved.as_ref().unwrap_or(&initial)
            } else {
                previous_contexts
                    .as_ref()
                    .ok_or_else(|| invalid("HEVC dependent segment has no preceding CABAC state"))?
            };
            bins = HevcCabac::from_contexts(
                &slice.rbsp[slice.entropy_substreams[0].clone()],
                0,
                contexts,
            )?;
        }
        let mut stream = 0;
        for address in begin..end {
            let (raster, row, col, [tx, ty, tw, th], tile_start, row_start) =
                ctu_position(&decoder.tile_layout, address, columns, rows);
            decoder.tile_bounds = decoder
                .tile_layout
                .as_ref()
                .map(|_| [tx * side, ty * side, tw * side, th * side]);
            if tile_start {
                decoder.palette = initial_palette.clone();
                saved_palette = None;
                saved = None;
                decoder.qp = slice.qp;
                decoder.qp_prediction = slice.qp;
                decoder.qp_coded = false;
            }
            if address != begin && (tile_start || (pps.entropy_sync && row_start)) {
                stream += 1;
                let bytes = &slice.rbsp[slice.entropy_substreams[stream].clone()];
                bins = if tile_start {
                    HevcCabac::new(bytes, 0, slice.slice_type, slice.cabac_init, slice.qp)?
                } else {
                    decoder.palette = saved_palette
                        .clone()
                        .unwrap_or_else(|| initial_palette.clone());
                    HevcCabac::from_contexts(bytes, 0, saved.as_ref().unwrap_or(&initial))?
                };
                decoder.qp = slice.qp;
            }
            let available =
                |other: u32| tile_scan(&decoder.tile_layout, other) >= decoder.slice_start;
            sao[raster as usize] = hevc_sao::read_ctu_with_scale(
                &mut bins,
                slice.sao,
                sps.depth,
                pps.sao_offset_scale,
                if col > tx && available(raster - 1) {
                    Some(&sao[raster as usize - 1])
                } else {
                    None
                },
                if row > ty && available(raster - columns) {
                    Some(&sao[(raster - columns) as usize])
                } else {
                    None
                },
            )?;
            hevc_tree::read_ctu(
                &mut bins,
                &mut decoder,
                [w, h],
                [col * side, row * side],
                max_cb,
                min_cb,
            )?;
            if pps.entropy_sync && col == tx + 1 {
                saved = Some(bins.contexts()?);
                saved_palette = Some(decoder.palette.clone());
            }
            let last = address + 1 == end;
            if bins.terminate()? != last {
                return Err(invalid(
                    "HEVC slice termination does not match segment extent",
                ));
            }
            let tile_end = col == tx + tw - 1 && row == ty + th - 1;
            let row_end = pps.entropy_sync && col == tx + tw - 1;
            if !last && (tile_end || row_end) && !bins.terminate()? {
                return Err(invalid("missing HEVC end-of-substream bit"));
            }
            visited += 1;
        }
        previous_contexts = Some(bins.contexts()?);
    }
    decoder.tile_bounds = None;
    if !decoder.planes.iter().all(Plane::complete) {
        return Err(invalid("incomplete HEVC multi-slice picture"));
    }
    let canonical_pocs: [Vec<i32>; 2] =
        std::array::from_fn(|l| canonical[l].iter().map(|r| r.poc).collect());
    let slice_pocs: Vec<[Vec<i32>; 2]> = slice_lists
        .iter()
        .map(|lists| std::array::from_fn(|l| lists[l].iter().map(|r| r.poc).collect()))
        .collect();
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let address = y / side * columns + x / side;
            let scan = tile_scan(&decoder.tile_layout, address);
            let owner =
                slices.partition_point(|s| tile_scan(&decoder.tile_layout, s.address) <= scan) - 1;
            let source_pocs = &slice_pocs[owner];
            let cell = &mut decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4];
            cell.motion =
                super::hevc_motion::remap_references(cell.motion, source_pocs, &canonical_pocs)?;
        }
    }
    decoder.lists = &canonical;
    deblock_slices(&mut decoder, slices)?;
    let owners = independent_owners(slices);
    for (component, plane) in decoder
        .planes
        .iter_mut()
        .take(if sps.chroma_format == 0 { 1 } else { 3 })
        .enumerate()
    {
        let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
        let [shift_x, shift_y] = component_shifts(component, sps.chroma_format);
        let owner = |p: [usize; 2]| {
            let address = ((p[1] << shift_y) / side as usize) * columns as usize
                + (p[0] << shift_x) / side as usize;
            let scan = tile_scan(&decoder.tile_layout, address as u32);
            slices.partition_point(|s| tile_scan(&decoder.tile_layout, s.address) <= scan) - 1
        };
        plane.apply_sao_rectangular(
            [max_cb - shift_x as u8, max_cb - shift_y as u8],
            &parameters,
            |a, b| {
                let ia = owner(a);
                let ib = owner(b);
                let slice_allowed =
                    owners[ia] == owners[ib] || slices[ia.max(ib)].loop_filter_across_slices;
                let tile_allowed = decoder.tile_layout.as_ref().is_none_or(|layout| {
                    if pps.tiles.as_ref().unwrap().loop_filter_across {
                        return true;
                    }
                    let raster = |p: [usize; 2]| {
                        (p[1] << shift_y) / side as usize * columns as usize
                            + (p[0] << shift_x) / side as usize
                    };
                    layout.tile_ids[raster(a)] == layout.tile_ids[raster(b)]
                });
                slice_allowed && tile_allowed
            },
            |p| {
                let c = decoder.cells[((p[1] << shift_y) / 4) * (sps.dimensions[0] as usize / 4)
                    + (p[0] << shift_x) / 4];
                c.bypass || (c.pcm && sps.pcm.as_ref().is_some_and(|p| p.loop_filter_disabled))
            },
        )?;
    }
    let mut motion = Vec::new();
    if slices.iter().any(|s| s.slice_type != SliceType::I) {
        for y in (0..h).step_by(16) {
            for x in (0..w).step_by(16) {
                motion
                    .push(decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4].motion);
            }
        }
    }
    Ok(Picture {
        colour_planes: None,
        #[cfg(test)]
        pcm_luma_samples: decoder.cells.iter().filter(|c| c.pcm).count() * 16,
        #[cfg(test)]
        cross_component_blocks: decoder.cross_component_blocks,
        #[cfg(test)]
        act_blocks: decoder.act_blocks,
        #[cfg(test)]
        current_picture_blocks: decoder.current_picture_blocks,
        #[cfg(test)]
        completed_picture_blocks: decoder.completed_picture_blocks,
        #[cfg(test)]
        bipredicted_blocks: decoder.bipredicted_blocks,
        #[cfg(test)]
        fractional_current_chroma_blocks: decoder.fractional_current_chroma_blocks,
        motion,
        reference_pocs: canonical_pocs,
        reference_long_term: std::array::from_fn(|l| {
            canonical[l].iter().map(|r| r.long_term).collect()
        }),
        dimensions: [w, h],
        crop: sps.crop,
        depth: sps.depth,
        planes: decoder.planes,
        sao,
    })
}
#[derive(Clone, Copy, Default)]
struct Cell {
    ready: bool,
    depth: u8,
    skip: bool,
    intra: bool,
    mode: u8,
    motion: Motion,
    cbf: bool,
    bypass: bool,
    pcm: bool,
    palette: bool,
}
struct Decoder<'a> {
    sps: &'a Sps,
    pps: &'a Pps,
    slice: &'a SliceHeader,
    poc: i32,
    lists: &'a [Vec<Reference>; 2],
    slice_start: u32,
    tile_bounds: Option<[u32; 4]>,
    tile_layout: Option<super::hevc_tiles::TileLayout>,
    palette: Option<super::hevc_palette::Predictor>,
    qp: i32,
    chroma_offsets: [i32; 2],
    cu_chroma_offsets: [i32; 2],
    chroma_qp_coded: bool,
    qp_coded: bool,
    qp_prediction: i32,
    qp_grid: Vec<i32>,
    edges: Vec<[u8; 2]>,
    cells: Vec<Cell>,
    planes: [Plane; 3],
    jobs: Option<std::sync::mpsc::SyncSender<(usize, Vec<Reconstruction>)>>,
    reconstruction: Vec<Reconstruction>,
    scratch: Vec<i32>,
    pred_scratch: Vec<u16>,
    transform_scratch: Vec<i32>,
    residual_scratch: Vec<i32>,
    #[cfg(test)]
    cross_component_blocks: usize,
    #[cfg(test)]
    act_blocks: usize,
    #[cfg(test)]
    current_picture_blocks: usize,
    #[cfg(test)]
    completed_picture_blocks: usize,
    #[cfg(test)]
    bipredicted_blocks: usize,
    #[cfg(test)]
    fractional_current_chroma_blocks: usize,
}
impl<'a> Visitor<HevcCabac<'a>> for Decoder<'_> {
    fn neighbouring_depths(&self, n: Node) -> [Option<u8>; 2] {
        let at = |x: u32, y: u32| {
            self.cell(x as i32, y as i32)
                .filter(|v| v.ready)
                .map(|v| v.depth)
        };
        [
            n.x.checked_sub(1).and_then(|x| at(x, n.y)),
            n.y.checked_sub(1).and_then(|y| at(n.x, y)),
        ]
    }
    fn enter(&mut self, n: Node, split: bool) -> Result<()> {
        if self.slice.cu_chroma_qp_offset_enabled {
            let table = self
                .pps
                .chroma_qp_offset_list
                .as_ref()
                .ok_or_else(|| invalid("HEVC active chroma QP selection has no list"))?;
            if n.log2_size >= self.sps.coding_block_log2[1] - table.depth {
                self.chroma_qp_coded = false;
            }
        }
        if let Some(depth) = self.pps.cu_qp_delta_depth {
            let log = self.sps.coding_block_log2[1] - depth;
            if n.log2_size >= log {
                self.qp_coded = false;
                if !split || n.log2_size == log {
                    let width = self.sps.dimensions[0] as usize / 8;
                    let ctu = 1 << self.sps.coding_block_log2[1];
                    let a = (n.x % ctu != 0)
                        .then(|| self.qp_grid[n.y as usize / 8 * width + (n.x as usize - 1) / 8]);
                    let b = (n.y % ctu != 0)
                        .then(|| self.qp_grid[(n.y as usize - 1) / 8 * width + n.x as usize / 8]);
                    self.qp_prediction = hevc_qp::luma(self.qp, [a, b], 0, self.sps.depth[0])?;
                    self.qp = self.qp_prediction;
                }
            }
        }
        Ok(())
    }
    fn leaf(&mut self, b: &mut HevcCabac<'a>, n: Node) -> Result<()> {
        let bypass = self.pps.transquant_bypass && b.decision(Syntax::TransquantBypass, 0)?;
        let skip = if self.slice.slice_type != SliceType::I {
            let context = [
                self.cell(n.x as i32 - 1, n.y as i32),
                self.cell(n.x as i32, n.y as i32 - 1),
            ]
            .into_iter()
            .flatten()
            .filter(|c| c.ready && c.skip)
            .count();
            b.decision(Syntax::Skip, context)?
        } else {
            false
        };
        let intra =
            !skip && (self.slice.slice_type == SliceType::I || b.decision(Syntax::PredMode, 0)?);
        if !intra {
            self.inter(b, n, skip, bypass)?;
            self.finish_cu(n, false, skip);
            return Ok(());
        }
        if self.palette.is_some()
            && n.log2_size <= self.sps.transform_block_log2[1]
            && b.decision(Syntax::PaletteMode, 0)?
        {
            let header = super::hevc_palette::Header::read(
                self.palette.as_ref().unwrap(),
                1usize << n.log2_size,
                b,
            )?;
            if header.escapes {
                if self.pps.cu_qp_delta_depth.is_some() && !self.qp_coded {
                    let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                    self.qp =
                        hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                    self.qp_coded = true;
                }
                self.read_chroma_qp(b, [true; 3], bypass)?;
            }
            let samples = header.samples(
                1usize << n.log2_size,
                self.sps.chroma_format,
                self.sps.depth,
                bypass,
                self.component_qps(false)?.map(i32::from),
                b,
            )?;
            self.palette
                .as_mut()
                .unwrap()
                .update(&header.entries, &header.reused)?;
            let stride = self.sps.dimensions[0] as usize / 4;
            let side = 1usize << n.log2_size;
            for y in n.y as usize / 4..(n.y as usize + side) / 4 {
                for x in n.x as usize / 4..(n.x as usize + side) / 4 {
                    self.cells[y * stride + x] = Cell {
                        bypass,
                        palette: true,
                        mode: 1,
                        ..Cell::default()
                    };
                }
            }
            for (component, samples) in samples
                .into_iter()
                .take(if self.sps.chroma_format == 0 { 1 } else { 3 })
                .enumerate()
            {
                let [sx, sy] = component_shifts(component, self.sps.chroma_format);
                let rect = [
                    n.x as usize >> sx,
                    n.y as usize >> sy,
                    side >> sx,
                    side >> sy,
                ];
                if self.jobs.is_some() {
                    self.reconstruction.push(Reconstruction::Pcm {
                        component,
                        rect,
                        samples,
                    });
                } else {
                    self.planes[component].reconstruct_inter(rect, &samples)?;
                }
            }
            self.finish_cu(n, true, false);
            return Ok(());
        }
        let nxn =
            n.log2_size == self.sps.coding_block_log2[0] && !b.decision(Syntax::PartMode, 0)?;
        if !nxn
            && let Some(pcm) = &self.sps.pcm
            && (pcm.block_log2[0]..=pcm.block_log2[1]).contains(&n.log2_size)
            && b.terminate()?
        {
            let samples = b.read_pcm_with_chroma(
                n.log2_size,
                pcm.depth,
                self.sps.depth,
                self.sps.chroma_format,
            )?;
            let stride = self.sps.dimensions[0] as usize / 4;
            let side = 1usize << n.log2_size;
            for y in n.y as usize / 4..(n.y as usize + side) / 4 {
                for x in n.x as usize / 4..(n.x as usize + side) / 4 {
                    self.cells[y * stride + x] = Cell {
                        pcm: true,
                        bypass,
                        mode: 1,
                        ..Cell::default()
                    };
                }
            }
            for (component, samples) in samples
                .into_iter()
                .take(if self.sps.chroma_format == 0 { 1 } else { 3 })
                .enumerate()
            {
                let [shift_x, shift_y] = component_shifts(component, self.sps.chroma_format);
                let rect = [
                    n.x as usize >> shift_x,
                    n.y as usize >> shift_y,
                    side >> shift_x,
                    side >> shift_y,
                ];
                if self.jobs.is_some() {
                    self.reconstruction.push(Reconstruction::Pcm {
                        component,
                        rect,
                        samples,
                    });
                } else {
                    self.planes[component].reconstruct_inter(rect, &samples)?;
                }
            }
            self.finish_cu(n, true, false);
            return Ok(());
        }
        let codes = hevc_intra_syntax::read_luma(b, nxn)?;
        let mut luma_modes = Vec::with_capacity(codes.len());
        for (i, code) in codes.into_iter().enumerate() {
            let log = n.log2_size - u8::from(nxn);
            let p = Node {
                x: n.x + (i as u32 % 2) * (1 << log),
                y: n.y + (i as u32 / 2) * (1 << log),
                log2_size: log,
                depth: n.depth,
            };
            let neighbour = |x: u32, y: u32| {
                self.cell(x as i32, y as i32)
                    .filter(|v| v.ready)
                    .map(|v| if v.intra { v.mode } else { 1 })
            };
            let left = p.x.checked_sub(1).and_then(|x| neighbour(x, p.y));
            let top = if p.y % (1 << self.sps.coding_block_log2[1]) == 0 {
                None
            } else {
                p.y.checked_sub(1).and_then(|y| neighbour(p.x, y))
            };
            let derived = code.resolve(left, top)?;
            luma_modes.push(derived);
            let stride = self.sps.dimensions[0] as usize / 4;
            for y in p.y as usize / 4..(p.y as usize + (1 << p.log2_size)) / 4 {
                for x in p.x as usize / 4..(p.x as usize + (1 << p.log2_size)) / 4 {
                    self.cells[y * stride + x] = Cell {
                        ready: true,
                        bypass,
                        depth: n.depth,
                        intra: true,
                        mode: derived,
                        ..Cell::default()
                    };
                }
            }
        }
        let chroma_modes =
            hevc_intra_syntax::read_chroma_modes_and_codes(b, &luma_modes, self.sps.chroma_format)?;
        let c = hevc_transform_tree::Config {
            log2_cu: n.log2_size,
            log2_min_transform: self.sps.transform_block_log2[0],
            log2_max_transform: self.sps.transform_block_log2[1],
            max_depth: self.sps.transform_hierarchy_depth[1] + u8::from(nxn),
            intra_split: nxn,
        };
        hevc_transform_tree::read_intra_with_chroma(
            b,
            [n.x, n.y],
            c,
            self.sps.chroma_format,
            |b, u| {
                let index = if self.sps.chroma_format == 3 && nxn {
                    let half = 1u32 << (n.log2_size - 1);
                    ((u.origin[1] - n.y) / half * 2 + (u.origin[0] - n.x) / half) as usize
                } else {
                    0
                };
                let (chroma, chroma_code) = chroma_modes[index];
                let mode = self
                    .cell(u.origin[0] as i32, u.origin[1] as i32)
                    .ok_or_else(|| invalid("HEVC transform has no prediction block"))?
                    .mode;
                let act = self.pps.adaptive_colour_transform
                    && u.coded.iter().any(|&v| v)
                    && chroma_modes.iter().all(|&(_, code)| code == 4)
                    && b.decision(Syntax::ResidualAct, 0)?;
                if act && bypass && self.sps.depth[0] != self.sps.depth[1] {
                    return Err(invalid("HEVC bypass ACT requires equal component depths"));
                }
                #[cfg(test)]
                {
                    self.act_blocks += usize::from(act);
                }
                let mut act_blocks = std::array::from_fn(|_| None);
                let mut act_alphas = [0; 3];
                if self.pps.cu_qp_delta_depth.is_some()
                    && !self.qp_coded
                    && u.coded.iter().any(|&v| v)
                {
                    let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                    self.qp =
                        hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                    self.qp_coded = true;
                }
                let stride = self.sps.dimensions[0] as usize / 4;
                for k in 0..1usize << (u.log2_size - 2) {
                    let x = u.origin[0] as usize / 4;
                    let y = u.origin[1] as usize / 4;
                    self.edges[(y + k) * stride + x][0] |= 2;
                    self.edges[y * stride + x + k][1] |= 2;
                }
                self.read_chroma_qp(b, u.coded_components(), bypass)?;
                let qps = self.component_qps(act)?;
                let mut luma_residual = Vec::new();
                for block in u.component_blocks(self.sps.chroma_format) {
                    let component = block.component;
                    let alpha = if component != 0
                        && self.pps.cross_component_prediction
                        && u.coded[0]
                        && chroma_code == 4
                    {
                        super::hevc_cross_component::read_alpha(b, component)?
                    } else {
                        0
                    };
                    #[cfg(test)]
                    {
                        self.cross_component_blocks += usize::from(alpha != 0);
                    }
                    let c = hevc_block::Config {
                        log2_size: block.log2_size,
                        component: component as u8,
                        bit_depth: self.sps.depth[usize::from(component != 0)],
                        qp: qps[component],
                        intra_mode: Some(if component == 0 { mode } else { chroma }),
                        transform_skip_enabled: self.pps.transform_skip,
                        transquant_bypass: bypass,
                        sign_hiding: self.pps.sign_data_hiding,
                    };
                    let residual = if block.coded {
                        Some(hevc_block::read_with_precision(
                            b,
                            c,
                            self.sps.transform_skip_rotation,
                            self.sps.transform_skip_context,
                            self.sps.implicit_rdpcm,
                            self.sps.explicit_rdpcm,
                            self.pps.transform_skip_max_log2,
                            self.sps.persistent_rice,
                            self.sps.extended_precision,
                            self.sps.cabac_bypass_alignment,
                            self.sps.chroma_format == 3,
                        )?)
                    } else {
                        None
                    };
                    let residual = if act {
                        act_blocks[component] = residual;
                        act_alphas[component] = alpha;
                        None
                    } else {
                        residual
                    };
                    let origin = block.origin;
                    if self.jobs.is_some() {
                        self.reconstruction.push(Reconstruction::Intra {
                            component,
                            origin: origin.map(|v| v as usize),
                            log: c.log2_size,
                            mode: c.intra_mode.unwrap(),
                            filter_boundary: self
                                .sps
                                .filter_intra_boundary(c.intra_mode.unwrap(), bypass),
                            alpha: if act { 0 } else { alpha },
                            residual,
                        });
                    } else {
                        if let Some(block) = residual {
                            block.reconstruct(
                                self.pps
                                    .scaling_lists
                                    .as_ref()
                                    .unwrap_or(&self.sps.scaling_lists),
                                &mut self.transform_scratch,
                                &mut self.residual_scratch,
                            )?;
                        } else {
                            self.residual_scratch.clear();
                        }
                        if !act {
                            cross_residual(
                                component,
                                alpha,
                                c.log2_size,
                                self.sps.depth,
                                self.pps.cross_component_prediction,
                                &mut luma_residual,
                                &mut self.residual_scratch,
                            )?;
                        }
                        self.planes[component].reconstruct_intra_with_full_chroma_filters(
                            origin.map(|v| v as usize),
                            c.log2_size,
                            c.intra_mode.unwrap(),
                            component != 0,
                            self.sps.chroma_format == 3,
                            self.sps.strong_intra_smoothing,
                            !self.sps.intra_smoothing_disabled,
                            self.sps
                                .filter_intra_boundary(c.intra_mode.unwrap(), bypass),
                            &self.residual_scratch,
                            &mut self.pred_scratch,
                            |x, y| {
                                let [shift_x, shift_y] =
                                    component_shifts(component, self.sps.chroma_format);
                                let side = 1usize << self.sps.coding_block_log2[1];
                                let address = (y << shift_y) / side
                                    * (self.sps.dimensions[0] as usize).div_ceil(side)
                                    + (x << shift_x) / side;
                                if let Some([tx, ty, width, height]) = self.tile_bounds {
                                    let x = (x << shift_x) as u32;
                                    let y = (y << shift_y) as u32;
                                    if x < tx || y < ty || x >= tx + width || y >= ty + height {
                                        return false;
                                    }
                                }
                                if tile_scan(&self.tile_layout, address as u32) < self.slice_start {
                                    return false;
                                }
                                if !self.pps.constrained_intra {
                                    return true;
                                }
                                self.cells[((y << shift_y) / 4)
                                    * (self.sps.dimensions[0] as usize / 4)
                                    + (x << shift_x) / 4]
                                    .intra
                            },
                        )?;
                    }
                }
                if act {
                    self.finish_act(u.origin, u.log2_size, bypass, act_blocks, act_alphas)?;
                }
                Ok(())
            },
        )?;
        self.finish_cu(n, true, false);
        Ok(())
    }
}

/// Parse CTUs in tile-scan order; each tile has independent entropy contexts.
fn decode_tile_ctus<'a>(
    decoder: &mut Decoder<'a>,
    slice: &'a SliceHeader,
    layout: &super::hevc_tiles::TileLayout,
) -> Result<Vec<hevc_sao::CtuSao>> {
    let sps = decoder.sps;
    let pps = decoder.pps;
    let [w, h] = sps.dimensions;
    let [min_cb, max_cb] = sps.coding_block_log2;
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let total = layout.tile_scan_to_raster.len();
    let mut sao = vec![[hevc_sao::Sao::Off; 3]; total];
    let mut stream = 0;
    let mut visited = 0;
    for &[tx, ty, tw, th] in &layout.rectangles {
        decoder.tile_bounds = Some([tx * side, ty * side, tw * side, th * side]);
        decoder.qp = slice.qp;
        decoder.qp_prediction = slice.qp;
        decoder.qp_coded = false;
        decoder.cu_chroma_offsets = [0; 2];
        decoder.chroma_qp_coded = false;
        let mut bins = HevcCabac::new(
            &slice.rbsp[slice.entropy_substreams[stream].clone()],
            0,
            slice.slice_type,
            slice.cabac_init,
            slice.qp,
        )?;
        let initial = bins.contexts()?;
        let mut saved = None;
        decoder.palette = if sps.palette.is_some() {
            Some(super::hevc_palette::Predictor::from_parameters(sps, pps)?)
        } else {
            None
        };
        let initial_palette = decoder.palette.clone();
        let mut saved_palette = None;
        stream += 1;
        for row in ty..ty + th {
            if row != ty && pps.entropy_sync {
                decoder.palette = saved_palette
                    .clone()
                    .unwrap_or_else(|| initial_palette.clone());
                bins = HevcCabac::from_contexts(
                    &slice.rbsp[slice.entropy_substreams[stream].clone()],
                    0,
                    saved.as_ref().unwrap_or(&initial),
                )?;
                stream += 1;
                decoder.qp = slice.qp;
            }
            for col in tx..tx + tw {
                let raster = (row * columns + col) as usize;
                sao[raster] = hevc_sao::read_ctu_with_scale(
                    &mut bins,
                    slice.sao,
                    sps.depth,
                    pps.sao_offset_scale,
                    if col > tx {
                        Some(&sao[raster - 1])
                    } else {
                        None
                    },
                    if row > ty {
                        Some(&sao[raster - columns as usize])
                    } else {
                        None
                    },
                )?;
                hevc_tree::read_ctu(
                    &mut bins,
                    decoder,
                    [w, h],
                    [col * side, row * side],
                    max_cb,
                    min_cb,
                )?;
                if pps.entropy_sync && col == tx + 1 {
                    saved = Some(bins.contexts()?);
                    saved_palette = Some(decoder.palette.clone());
                }
                visited += 1;
                let last = visited == total;
                if bins.terminate()? != last {
                    return Err(invalid(
                        "HEVC tiled slice termination does not match picture extent",
                    ));
                }
                let tile_end = col == tx + tw - 1 && row == ty + th - 1;
                let row_end = pps.entropy_sync && col == tx + tw - 1;
                if !last && (tile_end || row_end) && !bins.terminate()? {
                    return Err(invalid("missing HEVC tile end-of-substream bit"));
                }
            }
        }
    }
    decoder.tile_bounds = None;
    Ok(sao)
}

// Rows preserve reconstruction order. CABAC and motion metadata can advance
// independently; intra prediction still sees only previously reconstructed pixels.
fn cross_residual(
    component: usize,
    alpha: i8,
    log: u8,
    depths: [u8; 2],
    enabled: bool,
    luma: &mut Vec<i32>,
    residual: &mut Vec<i32>,
) -> Result<()> {
    if !enabled {
        return Ok(());
    }
    if component == 0 {
        luma.clone_from(residual);
    } else if alpha != 0 {
        if residual.is_empty() {
            residual.resize(1usize << (2 * log), 0);
        }
        super::hevc_cross_component::modify(luma, residual, alpha, depths)?;
    }
    Ok(())
}
enum Reconstruction {
    Act {
        origin: [usize; 2],
        log: u8,
        bypass: bool,
        blocks: [Option<hevc_block::Coefficients>; 3],
        alphas: [i8; 3],
    },
    Pcm {
        component: usize,
        rect: [usize; 4],
        samples: Vec<u16>,
    },
    Inter {
        motion: Motion,
        rect: [u32; 4],
    },
    Intra {
        component: usize,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        residual: Option<hevc_block::Coefficients>,
        filter_boundary: bool,
        alpha: i8,
    },
    Residual {
        component: usize,
        origin: [usize; 2],
        log: u8,
        residual: Option<hevc_block::Coefficients>,
        alpha: i8,
    },
}
fn reconstruct_row(
    planes: &mut [Plane; 3],
    commands: Vec<Reconstruction>,
    sps: &Sps,
    pps: &Pps,
    slice: &SliceHeader,
    lists: &[Vec<Reference>; 2],
    scratch: &mut Vec<i32>,
    pred_scratch: &mut Vec<u16>,
    transform_scratch: &mut Vec<i32>,
    residual_scratch: &mut Vec<i32>,
) -> Result<()> {
    let scaling = pps.scaling_lists.as_ref().unwrap_or(&sps.scaling_lists);
    let mut luma_residual = Vec::new();
    for command in commands {
        match command {
            Reconstruction::Act {
                origin,
                log,
                bypass,
                blocks,
                alphas,
            } => {
                reconstruct_act(
                    planes,
                    origin,
                    log,
                    bypass,
                    blocks,
                    alphas,
                    sps,
                    pps,
                    transform_scratch,
                    residual_scratch,
                )?;
            }
            Reconstruction::Pcm {
                component,
                rect,
                samples,
            } => {
                planes[component].reconstruct_inter(rect, &samples)?;
            }
            Reconstruction::Inter { motion, rect } => {
                for c in 0..if sps.chroma_format == 0 { 1 } else { 3 } {
                    let shifts = component_shifts(c, sps.chroma_format);
                    let pixels = hevc_motion::predict_with_current(
                        lists,
                        motion,
                        rect,
                        c,
                        sps.depth[usize::from(c != 0)],
                        slice.weights.as_ref(),
                        scratch,
                        sps.chroma_format,
                        Some((&*planes, sps.depth)),
                    )?;
                    planes[c].reconstruct_inter(
                        std::array::from_fn(|axis| rect[axis] as usize >> shifts[axis % 2]),
                        &pixels,
                    )?;
                }
            }
            Reconstruction::Intra {
                component,
                origin,
                log,
                mode,
                residual,
                filter_boundary,
                alpha,
            } => {
                if let Some(block) = residual {
                    block.reconstruct(scaling, transform_scratch, residual_scratch)?;
                } else {
                    residual_scratch.clear();
                }
                cross_residual(
                    component,
                    alpha,
                    log,
                    sps.depth,
                    pps.cross_component_prediction,
                    &mut luma_residual,
                    residual_scratch,
                )?;
                planes[component].reconstruct_intra_with_full_chroma_filters(
                    origin,
                    log,
                    mode,
                    component != 0,
                    sps.chroma_format == 3,
                    sps.strong_intra_smoothing,
                    !sps.intra_smoothing_disabled,
                    filter_boundary,
                    residual_scratch,
                    pred_scratch,
                    |_, _| true,
                )?;
            }
            Reconstruction::Residual {
                component,
                origin,
                log,
                residual,
                alpha,
            } => {
                if let Some(residual) = residual {
                    residual.reconstruct(scaling, transform_scratch, residual_scratch)?;
                } else {
                    residual_scratch.clear();
                }
                cross_residual(
                    component,
                    alpha,
                    log,
                    sps.depth,
                    pps.cross_component_prediction,
                    &mut luma_residual,
                    residual_scratch,
                )?;
                planes[component].add_residual(origin, log, residual_scratch)?;
            }
        }
    }
    Ok(())
}

fn ctu_position(
    layout: &Option<super::hevc_tiles::TileLayout>,
    address: u32,
    columns: u32,
    rows: u32,
) -> (u32, u32, u32, [u32; 4], bool, bool) {
    let raster = layout
        .as_ref()
        .map_or(address, |l| l.tile_scan_to_raster[address as usize]);
    let row = raster / columns;
    let col = raster % columns;
    let rectangle = layout.as_ref().map_or([0, 0, columns, rows], |l| {
        l.rectangles[l.tile_ids[raster as usize] as usize]
    });
    let [tx, ty, _, _] = rectangle;
    (
        raster,
        row,
        col,
        rectangle,
        col == tx && row == ty,
        col == tx,
    )
}
fn tile_scan(layout: &Option<super::hevc_tiles::TileLayout>, raster: u32) -> u32 {
    layout
        .as_ref()
        .map_or(raster, |l| l.raster_to_tile_scan[raster as usize])
}
fn deblock(decoder: &mut Decoder<'_>, slice: &SliceHeader) -> Result<()> {
    deblock_slices(decoder, std::slice::from_ref(slice))
}
fn independent_owners(slices: &[SliceHeader]) -> Vec<usize> {
    let mut owner = 0;
    slices
        .iter()
        .enumerate()
        .map(|(index, slice)| {
            if !slice.dependent {
                owner = index;
            }
            owner
        })
        .collect()
}
fn deblock_slices(decoder: &mut Decoder<'_>, slices: &[SliceHeader]) -> Result<()> {
    use super::hevc_deblock as d;
    let owners = independent_owners(slices);
    let [width, height] = decoder.sps.dimensions.map(|v| v as usize);
    // Complete vertical filtering precedes horizontal filtering.
    for direction in 0..2 {
        for y in (0..height).step_by(4) {
            for x in (0..width).step_by(4) {
                let edge = if direction == 0 { x } else { y };
                if edge == 0
                    || edge % 8 != 0
                    || decoder.edges[y / 4 * (width / 4) + x / 4][direction] == 0
                {
                    continue;
                }
                let side = 1usize << decoder.sps.coding_block_log2[1];
                let owner = |x: usize, y: usize| {
                    slices.partition_point(|s| {
                        tile_scan(&decoder.tile_layout, s.address)
                            <= tile_scan(
                                &decoder.tile_layout,
                                (y / side * width.div_ceil(side) + x / side) as u32,
                            )
                    }) - 1
                };
                let q_owner = owner(x, y);
                let p_owner = if direction == 0 {
                    owner(x - 1, y)
                } else {
                    owner(x, y - 1)
                };
                if let Some(layout) = &decoder.tile_layout
                    && !decoder.pps.tiles.as_ref().unwrap().loop_filter_across
                {
                    let columns = decoder.sps.dimensions[0].div_ceil(side as u32) as usize;
                    let q = y / side * columns + x / side;
                    let p = if direction == 0 {
                        y / side * columns + (x - 1) / side
                    } else {
                        (y - 1) / side * columns + x / side
                    };
                    if layout.tile_ids[p] != layout.tile_ids[q] {
                        continue;
                    }
                }
                let slice = &slices[q_owner];
                if slice.deblocking.disabled
                    || (owners[p_owner] != owners[q_owner]
                        && !slices[p_owner.max(q_owner)].loop_filter_across_slices)
                {
                    continue;
                }
                let q = decoder.qp_grid[y / 8 * (width / 8) + x / 8];
                let p = if direction == 0 {
                    decoder.qp_grid[y / 8 * (width / 8) + (x - 1) / 8]
                } else {
                    decoder.qp_grid[(y - 1) / 8 * (width / 8) + x / 8]
                };
                let a = decoder
                    .cell_raw(
                        if direction == 0 {
                            x as i32 - 1
                        } else {
                            x as i32
                        },
                        if direction == 0 {
                            y as i32
                        } else {
                            y as i32 - 1
                        },
                    )
                    .unwrap();
                let b = decoder.cell_raw(x as i32, y as i32).unwrap();
                let pcm_excluded = decoder
                    .sps
                    .pcm
                    .as_ref()
                    .is_some_and(|p| p.loop_filter_disabled);
                let filter_enabled = [
                    !(a.bypass || a.palette || (a.pcm && pcm_excluded)),
                    !(b.bypass || b.palette || (b.pcm && pcm_excluded)),
                ];
                let strength = if a.intra || b.intra {
                    2
                } else if decoder.edges[y / 4 * (width / 4) + x / 4][direction] & 2 != 0
                    && (a.cbf || b.cbf)
                {
                    1
                } else {
                    u8::from(motion_boundary(a.motion, b.motion, decoder.lists))
                };
                if strength == 0 {
                    continue;
                }
                let [beta, tc] = d::luma_thresholds(
                    [p, q],
                    strength,
                    slice.deblocking.offsets_div2,
                    decoder.sps.depth[0],
                )?;
                let plane = decoder.planes[0].samples_mut();
                let index = |line: usize, side: usize, distance: usize| {
                    let delta = if side == 0 {
                        -(distance as isize) - 1
                    } else {
                        distance as isize
                    };
                    if direction == 0 {
                        (y + line) * width + x.checked_add_signed(delta).unwrap()
                    } else {
                        y.checked_add_signed(delta).unwrap() * width + x + line
                    }
                };
                let input: [[[u16; 4]; 2]; 4] = std::array::from_fn(|l| {
                    std::array::from_fn(|s| std::array::from_fn(|v| plane[index(l, s, v)]))
                });
                let mode =
                    d::luma_decision([input[0], input[3]], [beta, tc], decoder.sps.depth[0])?;
                // Convert to fvid-cpu's HevcLumaFilter for batched processing
                let batch_filter = match mode {
                    d::LumaFilter::Off => fvid_cpu::HevcLumaFilter::Off,
                    d::LumaFilter::Strong => fvid_cpu::HevcLumaFilter::Strong,
                    d::LumaFilter::Weak { second } => fvid_cpu::HevcLumaFilter::Weak { second },
                };
                let p_samples: [[u16; 4]; 4] = std::array::from_fn(|l| input[l][0]);
                let q_samples: [[u16; 4]; 4] = std::array::from_fn(|l| input[l][1]);
                let (out_p, out_q) = fvid_cpu::hevc_deblock_luma_batch4(
                    p_samples,
                    q_samples,
                    tc,
                    decoder.sps.depth[0],
                    batch_filter,
                    [filter_enabled; 4],
                );
                for l in 0..4 {
                    for s in 0..2 {
                        let output = if s == 0 { out_p[l] } else { out_q[l] };
                        for v in 0..3 {
                            plane[index(l, s, v)] = output[v];
                        }
                    }
                }
                let shifts = component_shifts(1, decoder.sps.chroma_format);
                if edge % (8 << shifts[direction]) == 0
                    && (if direction == 0 { y } else { x }) % (4 << shifts[1 - direction]) == 0
                    && strength == 2
                    && decoder.sps.chroma_format != 0
                {
                    for c in 1..3 {
                        let tc = d::chroma_tc_with_format(
                            p,
                            q,
                            decoder.pps.chroma_qp_offsets[c - 1],
                            slice.deblocking.offsets_div2[1],
                            decoder.sps.depth,
                            decoder.sps.chroma_format,
                        )?;
                        let [shift_x, shift_y] = shifts;
                        let plane = decoder.planes[c].samples_mut();
                        for l in 0..4 {
                            let index = |side: usize, v: usize| {
                                let delta = if side == 0 {
                                    -(v as isize) - 1
                                } else {
                                    v as isize
                                };
                                if direction == 0 {
                                    ((y >> shift_y) + l) * (width >> shift_x)
                                        + (x >> shift_x).checked_add_signed(delta).unwrap()
                                } else {
                                    (y >> shift_y).checked_add_signed(delta).unwrap()
                                        * (width >> shift_x)
                                        + (x >> shift_x)
                                        + l
                                }
                            };
                            let a = [plane[index(0, 0)], plane[index(0, 1)]];
                            let b = [plane[index(1, 0)], plane[index(1, 1)]];
                            let output =
                                d::chroma_sample(a, b, tc, decoder.sps.depth[1], filter_enabled)?;
                            plane[index(0, 0)] = output[0];
                            plane[index(1, 0)] = output[1];
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

impl Decoder<'_> {
    fn read_chroma_qp(
        &mut self,
        b: &mut HevcCabac<'_>,
        coded: [bool; 3],
        bypass: bool,
    ) -> Result<()> {
        if self.slice.cu_chroma_qp_offset_enabled
            && !self.chroma_qp_coded
            && !bypass
            && (coded[1] || coded[2])
        {
            let table = self
                .pps
                .chroma_qp_offset_list
                .as_ref()
                .ok_or_else(|| invalid("HEVC active chroma QP selection has no list"))?;
            self.cu_chroma_offsets = hevc_qp::read_chroma_offset(b, &table.entries)?;
            self.chroma_qp_coded = true;
        }
        Ok(())
    }

    fn cell(&self, x: i32, y: i32) -> Option<Cell> {
        if x < 0
            || y < 0
            || x as u32 >= self.sps.dimensions[0]
            || y as u32 >= self.sps.dimensions[1]
        {
            return None;
        }
        if let Some([tx, ty, width, height]) = self.tile_bounds {
            if x < tx as i32 || y < ty as i32 || x as u32 >= tx + width || y as u32 >= ty + height {
                return None;
            }
        }
        if x >= 0 && y >= 0 {
            let side = 1u32 << self.sps.coding_block_log2[1];
            let columns = self.sps.dimensions[0].div_ceil(side);
            if tile_scan(
                &self.tile_layout,
                y as u32 / side * columns + (x as u32 / side),
            ) < self.slice_start
            {
                return None;
            }
        }
        self.cell_raw(x, y)
    }
    fn cell_raw(&self, x: i32, y: i32) -> Option<Cell> {
        let [w, h] = self.sps.dimensions;
        if x < 0 || y < 0 || x as u32 >= w || y as u32 >= h {
            None
        } else {
            Some(self.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4])
        }
    }
    fn finish_cu(&mut self, n: Node, intra: bool, skip: bool) {
        let stride = self.sps.dimensions[0] as usize / 8;
        for y in n.y as usize / 8..(n.y as usize + (1 << n.log2_size)) / 8 {
            for x in n.x as usize / 8..(n.x as usize + (1 << n.log2_size)) / 8 {
                self.qp_grid[y * stride + x] = self.qp;
            }
        }
        let stride = stride * 2;
        let x = n.x as usize / 4;
        let y = n.y as usize / 4;
        let side = 1usize << (n.log2_size - 2);
        for k in 0..side {
            self.edges[(y + k) * stride + x][0] |= 2;
            self.edges[y * stride + x + k][1] |= 2;
        }
        for y in n.y as usize / 4..(n.y as usize + (1 << n.log2_size)) / 4 {
            for x in n.x as usize / 4..(n.x as usize + (1 << n.log2_size)) / 4 {
                let cell = &mut self.cells[y * stride + x];
                cell.ready = true;
                cell.depth = n.depth;
                cell.intra = intra;
                cell.skip = skip;
            }
        }
    }
    fn inter(&mut self, b: &mut HevcCabac<'_>, n: Node, skip: bool, bypass: bool) -> Result<()> {
        let partition = if skip {
            Partition::Full
        } else {
            hevc_inter_syntax::partition(
                b,
                n.log2_size,
                self.sps.coding_block_log2[0],
                self.sps.amp,
            )?
        };
        let stride = self.sps.dimensions[0] as usize / 4;
        let mut full_merge = false;
        for (part_index, [dx, dy, w, h]) in partition
            .rectangles(1 << n.log2_size)
            .into_iter()
            .enumerate()
        {
            let rect = [n.x + dx, n.y + dy, w, h];
            let syntax = hevc_inter_syntax::prediction(b, self.slice, skip, n.depth, [w, h])?;
            full_merge = partition == Partition::Full
                && matches!(syntax, hevc_inter_syntax::Prediction::Merge(_));
            let spatial = Spatial {
                rect,
                cu: [n.x, n.y, 1 << n.log2_size],
                partition,
                part_index,
                merge_log2: self.pps.parallel_merge_log2,
                ctu_log2: self.sps.coding_block_log2[1],
                poc: self.poc,
                lists: self.lists,
            };
            let motion = spatial.resolve(syntax, self.slice, |x, y| {
                self.cell(x, y)
                    .filter(|c| c.ready && !c.intra)
                    .map(|c| c.motion)
            })?;
            hevc_motion::validate_current_vectors(
                self.lists,
                motion,
                rect,
                [n.x, n.y],
                self.sps.coding_block_log2[1],
                self.sps.chroma_format,
                |x, y| self.cell(x, y).is_some_and(|c| c.ready),
            )?;
            #[cfg(test)]
            if motion.iter().all(Option::is_some) {
                self.bipredicted_blocks += 1;
            }
            #[cfg(test)]
            for (list, vector) in motion.iter().enumerate() {
                if let Some(v) = vector {
                    if self.lists[list][v.reference as usize].picture.is_none() {
                        self.current_picture_blocks += 1;
                        if self.sps.chroma_format == 1 && v.mv.iter().any(|x| x & 7 != 0) {
                            self.fractional_current_chroma_blocks += 1;
                        }
                    } else {
                        self.completed_picture_blocks += 1;
                    }
                }
            }
            if self.jobs.is_some() {
                self.reconstruction
                    .push(Reconstruction::Inter { motion, rect });
            } else {
                for c in 0..if self.sps.chroma_format == 0 { 1 } else { 3 } {
                    let shifts = component_shifts(c, self.sps.chroma_format);
                    let pixels = hevc_motion::predict_with_current(
                        self.lists,
                        motion,
                        rect,
                        c,
                        self.sps.depth[usize::from(c != 0)],
                        self.slice.weights.as_ref(),
                        &mut self.scratch,
                        self.sps.chroma_format,
                        Some((&self.planes, self.sps.depth)),
                    )?;
                    self.planes[c].reconstruct_inter(
                        std::array::from_fn(|axis| rect[axis] as usize >> shifts[axis % 2]),
                        &pixels,
                    )?;
                }
            }
            let [x, y, w, h] = rect.map(|v| v as usize / 4);
            for yy in y..y + h {
                for xx in x..x + w {
                    self.cells[yy * stride + xx] = Cell {
                        ready: true,
                        bypass,
                        depth: n.depth,
                        skip,
                        motion,
                        ..Cell::default()
                    };
                }
            }
            for yy in y..y + h {
                self.edges[yy * stride + x][0] |= 1;
            }
            for xx in x..x + w {
                self.edges[y * stride + xx][1] |= 1;
            }
        }
        if skip || (!full_merge && !b.decision(Syntax::RootCbf, 0)?) {
            return Ok(());
        }
        let config = hevc_transform_tree::Config {
            log2_cu: n.log2_size,
            log2_min_transform: self.sps.transform_block_log2[0],
            log2_max_transform: self.sps.transform_block_log2[1],
            max_depth: self.sps.transform_hierarchy_depth[0],
            intra_split: false,
        };
        hevc_transform_tree::read_inter_with_chroma(
            b,
            [n.x, n.y],
            config,
            partition != Partition::Full,
            self.sps.chroma_format,
            |b, u| {
                let act = self.pps.adaptive_colour_transform
                    && u.coded.iter().any(|&v| v)
                    && b.decision(Syntax::ResidualAct, 0)?;
                if act && bypass && self.sps.depth[0] != self.sps.depth[1] {
                    return Err(invalid("HEVC bypass ACT requires equal component depths"));
                }
                #[cfg(test)]
                {
                    self.act_blocks += usize::from(act);
                }
                let mut act_blocks = std::array::from_fn(|_| None);
                let mut act_alphas = [0; 3];
                if self.pps.cu_qp_delta_depth.is_some()
                    && !self.qp_coded
                    && u.coded.iter().any(|&v| v)
                {
                    let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                    self.qp =
                        hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                    self.qp_coded = true;
                }
                let x = u.origin[0] as usize / 4;
                let y = u.origin[1] as usize / 4;
                let side = 1usize << (u.log2_size - 2);
                for k in 0..side {
                    self.edges[(y + k) * stride + x][0] |= 2;
                    self.edges[y * stride + x + k][1] |= 2;
                }
                for yy in y..y + side {
                    for xx in x..x + side {
                        self.cells[yy * stride + xx].cbf = u.coded[0];
                    }
                }
                self.read_chroma_qp(b, u.coded_components(), bypass)?;
                let qps = self.component_qps(act)?;
                let mut luma_residual = Vec::new();
                for block in u.component_blocks(self.sps.chroma_format) {
                    let c = block.component;
                    let alpha = if c != 0 && self.pps.cross_component_prediction && u.coded[0] {
                        super::hevc_cross_component::read_alpha(b, c)?
                    } else {
                        0
                    };
                    #[cfg(test)]
                    {
                        self.cross_component_blocks += usize::from(alpha != 0);
                    }
                    if !block.coded && alpha == 0 {
                        continue;
                    }
                    let config = hevc_block::Config {
                        log2_size: block.log2_size,
                        component: c as u8,
                        bit_depth: self.sps.depth[usize::from(c != 0)],
                        qp: qps[c],
                        intra_mode: None,
                        transform_skip_enabled: self.pps.transform_skip,
                        transquant_bypass: bypass,
                        sign_hiding: self.pps.sign_data_hiding,
                    };
                    let residual = if block.coded {
                        Some(hevc_block::read_with_precision(
                            b,
                            config,
                            self.sps.transform_skip_rotation,
                            self.sps.transform_skip_context,
                            self.sps.implicit_rdpcm,
                            self.sps.explicit_rdpcm,
                            self.pps.transform_skip_max_log2,
                            self.sps.persistent_rice,
                            self.sps.extended_precision,
                            self.sps.cabac_bypass_alignment,
                            self.sps.chroma_format == 3,
                        )?)
                    } else {
                        None
                    };
                    if act {
                        act_blocks[c] = residual;
                        act_alphas[c] = alpha;
                        continue;
                    }
                    let origin = block.origin;
                    if self.jobs.is_some() {
                        self.reconstruction.push(Reconstruction::Residual {
                            component: c,
                            origin: origin.map(|v| v as usize),
                            log: config.log2_size,
                            alpha,
                            residual,
                        });
                    } else {
                        if let Some(residual) = residual {
                            residual.reconstruct(
                                self.pps
                                    .scaling_lists
                                    .as_ref()
                                    .unwrap_or(&self.sps.scaling_lists),
                                &mut self.transform_scratch,
                                &mut self.residual_scratch,
                            )?;
                        } else {
                            self.residual_scratch.clear();
                        }
                        cross_residual(
                            c,
                            alpha,
                            config.log2_size,
                            self.sps.depth,
                            self.pps.cross_component_prediction,
                            &mut luma_residual,
                            &mut self.residual_scratch,
                        )?;
                        self.planes[c].add_residual(
                            origin.map(|v| v as usize),
                            config.log2_size,
                            &self.residual_scratch,
                        )?;
                    }
                }
                if act {
                    self.finish_act(u.origin, u.log2_size, bypass, act_blocks, act_alphas)?;
                }
                Ok(())
            },
        )
    }
    fn component_qps(&self, act: bool) -> Result<[u8; 3]> {
        if !act {
            return hevc_qp::components_with_format(
                self.qp,
                self.sps.depth,
                self.chroma_offsets,
                self.cu_chroma_offsets,
                self.sps.chroma_format,
            );
        }
        let offsets = self.slice.act_qp_offsets.map(i32::from);
        let mut qps = hevc_qp::components_with_format(
            self.qp,
            self.sps.depth,
            [offsets[1], offsets[2]],
            self.cu_chroma_offsets,
            3,
        )?;
        qps[0] = (self.qp + 6 * i32::from(self.sps.depth[0] - 8) + offsets[0])
            .clamp(0, 51 + 6 * i32::from(self.sps.depth[0] - 8)) as u8;
        Ok(qps)
    }
    fn finish_act(
        &mut self,
        origin: [u32; 2],
        log: u8,
        bypass: bool,
        blocks: [Option<hevc_block::Coefficients>; 3],
        alphas: [i8; 3],
    ) -> Result<()> {
        let origin = origin.map(|v| v as usize);
        if self.jobs.is_some() {
            self.reconstruction.push(Reconstruction::Act {
                origin,
                log,
                bypass,
                blocks,
                alphas,
            });
            Ok(())
        } else {
            reconstruct_act(
                &mut self.planes,
                origin,
                log,
                bypass,
                blocks,
                alphas,
                self.sps,
                self.pps,
                &mut self.transform_scratch,
                &mut self.residual_scratch,
            )
        }
    }
}

fn motion_boundary(a: Motion, b: Motion, lists: &[Vec<Reference>; 2]) -> bool {
    let convert = |m: Motion| {
        let mut output = [(0, [0; 2]); 2];
        let mut count = 0;
        for (l, v) in m.into_iter().enumerate() {
            if let Some(v) = v {
                output[count] = (lists[l][v.reference as usize].poc, v.mv);
                count += 1;
            }
        }
        (output, count)
    };
    let (a, na) = convert(a);
    let (b, nb) = convert(b);
    if na != nb {
        return true;
    }
    let same = |a: (i32, [i16; 2]), b: (i32, [i16; 2])| {
        a.0 == b.0 && (0..2).all(|i| (i32::from(a.1[i]) - i32::from(b.1[i])).abs() < 4)
    };
    match na {
        0 => false,
        1 => !same(a[0], b[0]),
        _ => !(same(a[0], b[0]) && same(a[1], b[1]) || same(a[0], b[1]) && same(a[1], b[0])),
    }
}

fn reconstruct_act(
    planes: &mut [Plane; 3],
    origin: [usize; 2],
    log: u8,
    bypass: bool,
    blocks: [Option<hevc_block::Coefficients>; 3],
    alphas: [i8; 3],
    sps: &Sps,
    pps: &Pps,
    transform_scratch: &mut Vec<i32>,
    residual_scratch: &mut Vec<i32>,
) -> Result<()> {
    let mut samples = std::array::from_fn(|_| vec![0; 1usize << (2 * log)]);
    for (c, block) in blocks.into_iter().enumerate() {
        if let Some(block) = block {
            block.reconstruct(
                pps.scaling_lists.as_ref().unwrap_or(&sps.scaling_lists),
                transform_scratch,
                residual_scratch,
            )?;
            samples[c].copy_from_slice(residual_scratch);
        }
        if c != 0 && alphas[c] != 0 {
            let (y, chroma) = samples.split_at_mut(c);
            super::hevc_cross_component::modify(&y[0], &mut chroma[0], alphas[c], sps.depth)?;
        }
    }
    super::hevc_act::inverse(&mut samples, sps.depth, sps.extended_precision, bypass)?;
    for c in 0..3 {
        planes[c].add_residual(origin, log, &samples[c])?;
    }
    Ok(())
}
