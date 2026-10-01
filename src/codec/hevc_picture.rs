//! Native HEVC I/P/B reconstruction with independent slices, WPP and in-loop filters.
use super::{
    hevc_block,
    hevc_cabac::{HevcCabac, SliceType, Syntax},
    hevc_inter_syntax::{self, Partition},
    hevc_intra, hevc_intra_syntax,
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
    pub dimensions: [u32; 2],
    pub crop: [u32; 4],
    pub depth: [u8; 2],
    pub planes: [Plane; 3],
    /// Resolved SAO parameters in raster CTU order, including merged values.
    pub sao: Vec<hevc_sao::CtuSao>,
    /// Collocated motion at the normative 16x16 temporal-prediction grid.
    pub(crate) motion: Vec<Motion>,
    pub(crate) reference_pocs: [Vec<i32>; 2],
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
    if sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.pcm.is_some()
        || pps.tiles.is_some()
        || ((slice.sao != [false, false] || !slice.deblocking.disabled) && pps.transquant_bypass)
        || !slice.first
        || slice.address != 0
        || slice.nal.layer_id != 0
    {
        return Err(invalid("unsupported HEVC picture tools"));
    }
    if slice.slice_type != SliceType::I && !(1..=5).contains(&slice.max_merge_candidates) {
        return Err(invalid("invalid HEVC merge candidate count"));
    }
    if sps.depth[0] != sps.depth[1] {
        return Err(invalid("HEVC mixed component bit depths are not supported"));
    }
    for list in 0..2 {
        if lists[list].len() != slice.references[list] as usize {
            return Err(invalid("HEVC reference-list length mismatch"));
        }
        for r in &lists[list] {
            if r.picture.dimensions != sps.dimensions || r.picture.depth != sps.depth {
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
        || sps.depth.iter().any(|d| !(8..=10).contains(d))
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
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components(slice.qp, sps.depth, chroma_offsets)?;

    let mut decoder = Decoder {
        sps,
        pps,
        slice,
        poc,
        lists,
        slice_start: 0,
        qp: slice.qp,
        chroma_offsets,
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
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            Plane::new(w as usize / 2, h as usize / 2, sps.depth[1], count * 3 / 4)?,
            Plane::new(w as usize / 2, h as usize / 2, sps.depth[1], count * 3 / 4)?,
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let mut sao = Vec::with_capacity(columns as usize * rows as usize);
    let expected = if pps.entropy_sync { rows as usize } else { 1 };
    if slice.entropy_substreams.len() != expected {
        return Err(invalid(
            "HEVC WPP entry-point count does not match CTU rows",
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
    std::thread::scope(|scope| -> Result<()> {
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
                    let parameters = hevc_sao::read_ctu(
                        &mut bins,
                        slice.sao,
                        sps.depth,
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
        for (component, plane) in decoder.planes.iter_mut().enumerate() {
            let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
            plane.apply_sao(max_cb - u8::from(component != 0), &parameters)?;
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
        motion,
        reference_pocs: std::array::from_fn(|l| lists[l].iter().map(|r| r.poc).collect()),
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
    if sps.chroma_format != 1
        || sps.separate_colour_plane
        || sps.pcm.is_some()
        || pps.tiles.is_some()
        || pps.transquant_bypass
        || sps.depth[0] != sps.depth[1]
    {
        return Err(invalid("unsupported HEVC multi-slice picture tools"));
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
        if sps.depth[0] != sps.depth[1] {
            return Err(invalid("HEVC mixed component bit depths are not supported"));
        }
        for list in 0..2 {
            if reference_lists[list].len() != slice.references[list] as usize {
                return Err(invalid("HEVC reference-list length mismatch"));
            }
            for r in &reference_lists[list] {
                if r.picture.dimensions != sps.dimensions || r.picture.depth != sps.depth {
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
            || sps.depth.iter().any(|d| !(8..=10).contains(d))
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
    let lists = first_lists;
    let chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
    hevc_qp::components(slice.qp, sps.depth, chroma_offsets)?;

    let mut decoder = Decoder {
        sps,
        pps,
        slice,
        poc,
        lists,
        slice_start: 0,
        qp: slice.qp,
        chroma_offsets,
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
        planes: [
            Plane::new(w as usize, h as usize, sps.depth[0], count * 3)?,
            Plane::new(w as usize / 2, h as usize / 2, sps.depth[1], count * 3 / 4)?,
            Plane::new(w as usize / 2, h as usize / 2, sps.depth[1], count * 3 / 4)?,
        ],
    };
    let side = 1u32 << max_cb;
    let columns = w.div_ceil(side);
    let rows = h.div_ceil(side);
    let total = columns
        .checked_mul(rows)
        .ok_or_else(|| invalid("HEVC CTU grid overflow"))?;
    let mut sao = Vec::with_capacity(total as usize);
    let mut previous_contexts = None;
    let mut saved = None;
    for (index, slice) in slices.iter().enumerate() {
        let begin = slice.address;
        let end = slices.get(index + 1).map_or(total, |s| s.address);
        if begin != sao.len() as u32 || begin >= end || end > total || slice.pps_id != pps.id {
            return Err(invalid("HEVC slices do not partition the picture"));
        }
        decoder.slice = slice;
        decoder.lists = &slice_lists[index];
        if !slice.dependent {
            decoder.slice_start = begin;
            decoder.qp = slice.qp;
            decoder.qp_prediction = slice.qp;
            decoder.qp_coded = false;
            saved = None;
        }
        decoder.chroma_offsets = slice.chroma_qp_offsets.map(i32::from);
        let expected = if pps.entropy_sync {
            (end - 1) / columns - begin / columns + 1
        } else {
            1
        };
        if slice.entropy_substreams.len() != expected as usize {
            return Err(invalid("HEVC slice WPP substream count mismatch"));
        }
        let mut bins = HevcCabac::new(
            &slice.rbsp[slice.entropy_substreams[0].clone()],
            0,
            slice.slice_type,
            slice.cabac_init,
            slice.qp,
        )?;
        let initial = bins.contexts()?;
        if slice.dependent {
            let contexts = if pps.entropy_sync && begin % columns == 0 {
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
        for address in begin..end {
            let row = address / columns;
            let col = address % columns;
            if pps.entropy_sync && col == 0 && address != begin {
                let substream = (row - begin / columns) as usize;
                bins = HevcCabac::from_contexts(
                    &slice.rbsp[slice.entropy_substreams[substream].clone()],
                    0,
                    saved.as_ref().unwrap_or(&initial),
                )?;
                decoder.qp = slice.qp;
            }
            let available = |other: u32| other >= decoder.slice_start;
            let parameters = hevc_sao::read_ctu(
                &mut bins,
                slice.sao,
                sps.depth,
                if col > 0 && available(address - 1) {
                    sao.get(address as usize - 1)
                } else {
                    None
                },
                if row > 0 && available(address - columns) {
                    sao.get((address - columns) as usize)
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
                saved = Some(bins.contexts()?);
            }
            let last = address + 1 == end;
            if bins.terminate()? != last {
                return Err(invalid(
                    "HEVC slice termination does not match segment extent",
                ));
            }
            if pps.entropy_sync && col + 1 == columns && !last && !bins.terminate()? {
                return Err(invalid("missing HEVC end-of-substream bit"));
            }
        }
        previous_contexts = Some(bins.contexts()?);
    }
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
            let owner = slices.partition_point(|s| s.address <= address) - 1;
            let source_pocs = &slice_pocs[owner];
            let cell = &mut decoder.cells[y as usize / 4 * (w as usize / 4) + x as usize / 4];
            cell.motion =
                super::hevc_motion::remap_references(cell.motion, source_pocs, &canonical_pocs)?;
        }
    }
    decoder.lists = &canonical;
    deblock_slices(&mut decoder, slices)?;
    let owners = independent_owners(slices);
    for (component, plane) in decoder.planes.iter_mut().enumerate() {
        let parameters: Vec<_> = sao.iter().map(|p| p[component]).collect();
        let shift = usize::from(component != 0);
        let owner = |p: [usize; 2]| {
            let address = ((p[1] << shift) / side as usize) * columns as usize
                + (p[0] << shift) / side as usize;
            slices.partition_point(|s| s.address as usize <= address) - 1
        };
        plane.apply_sao_with_boundaries(
            max_cb - u8::from(component != 0),
            &parameters,
            |a, b| {
                let ia = owner(a);
                let ib = owner(b);
                owners[ia] == owners[ib] || slices[ia.max(ib)].loop_filter_across_slices
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
        motion,
        reference_pocs: canonical_pocs,
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
}
struct Decoder<'a> {
    sps: &'a Sps,
    pps: &'a Pps,
    slice: &'a SliceHeader,
    poc: i32,
    lists: &'a [Vec<Reference>; 2],
    slice_start: u32,
    qp: i32,
    chroma_offsets: [i32; 2],
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
        let nxn =
            n.log2_size == self.sps.coding_block_log2[0] && !b.decision(Syntax::PartMode, 0)?;
        let codes = hevc_intra_syntax::read_luma(b, nxn)?;
        let mut mode = 0;
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
            if i == 0 {
                mode = derived;
            }
            let stride = self.sps.dimensions[0] as usize / 4;
            for y in p.y as usize / 4..(p.y as usize + (1 << p.log2_size)) / 4 {
                for x in p.x as usize / 4..(p.x as usize + (1 << p.log2_size)) / 4 {
                    self.cells[y * stride + x] = Cell {
                        ready: true,
                        depth: n.depth,
                        intra: true,
                        mode: derived,
                        ..Cell::default()
                    };
                }
            }
        }
        let chroma = hevc_intra::chroma_mode(mode, hevc_intra_syntax::read_chroma(b)?)?;
        let c = hevc_transform_tree::Config {
            log2_cu: n.log2_size,
            log2_min_transform: self.sps.transform_block_log2[0],
            log2_max_transform: self.sps.transform_block_log2[1],
            max_depth: self.sps.transform_hierarchy_depth[1] + u8::from(nxn),
            intra_split: nxn,
        };
        hevc_transform_tree::read_intra(b, [n.x, n.y], c, |b, u| {
            let mode = self
                .cell(u.origin[0] as i32, u.origin[1] as i32)
                .ok_or_else(|| invalid("HEVC transform has no prediction block"))?
                .mode;
            if self.pps.cu_qp_delta_depth.is_some() && !self.qp_coded && u.coded.iter().any(|&v| v)
            {
                let delta = hevc_qp::read_delta(b, self.sps.depth[0])?;
                self.qp = hevc_qp::luma(self.qp_prediction, [None; 2], delta, self.sps.depth[0])?;
                self.qp_coded = true;
            }
            let stride = self.sps.dimensions[0] as usize / 4;
            for k in 0..1usize << (u.log2_size - 2) {
                let x = u.origin[0] as usize / 4;
                let y = u.origin[1] as usize / 4;
                self.edges[(y + k) * stride + x][0] |= 2;
                self.edges[y * stride + x + k][1] |= 2;
            }
            let qps = hevc_qp::components(self.qp, self.sps.depth, self.chroma_offsets)?;
            for component in 0..3 {
                if component != 0 && !u.owns_chroma {
                    continue;
                }
                let c = hevc_block::Config {
                    log2_size: if component == 0 {
                        u.log2_size
                    } else {
                        u.log2_chroma_size
                    },
                    component: component as u8,
                    bit_depth: self.sps.depth[usize::from(component != 0)],
                    qp: qps[component],
                    intra_mode: Some(if component == 0 { mode } else { chroma }),
                    transform_skip_enabled: self.pps.transform_skip,
                    transquant_bypass: bypass,
                    sign_hiding: self.pps.sign_data_hiding,
                };
                let residual = if u.coded[component] {
                    Some(hevc_block::read_with_tools(
                        b, c,
                        self.sps.transform_skip_rotation,
                        self.sps.transform_skip_context,
                    )?)
                } else {
                    None
                };
                let origin = if component == 0 {
                    u.origin
                } else {
                    u.chroma_origin
                };
                if self.jobs.is_some() {
                    self.reconstruction.push(Reconstruction::Intra {
                        component,
                        origin: origin.map(|v| v as usize),
                        log: c.log2_size,
                        mode: c.intra_mode.unwrap(),
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
                    self.planes[component].reconstruct_intra_with_reference_filtering(
                        origin.map(|v| v as usize),
                        c.log2_size,
                        c.intra_mode.unwrap(),
                        component != 0,
                        self.sps.strong_intra_smoothing,
                        !self.sps.intra_smoothing_disabled,
                        &self.residual_scratch,
                        &mut self.pred_scratch,
                        |x, y| {
                            let shift = usize::from(component != 0);
                            let side = 1usize << self.sps.coding_block_log2[1];
                            let address = (y << shift) / side
                                * (self.sps.dimensions[0] as usize).div_ceil(side)
                                + (x << shift) / side;
                            if address < self.slice_start as usize {
                                return false;
                            }
                            if !self.pps.constrained_intra {
                                return true;
                            }
                            self.cells[((y << shift) / 4) * (self.sps.dimensions[0] as usize / 4)
                                + (x << shift) / 4]
                                .intra
                        },
                    )?;
                }
            }
            Ok(())
        })?;
        self.finish_cu(n, true, false);
        Ok(())
    }
}

// Rows preserve reconstruction order. CABAC and motion metadata can advance
// independently; intra prediction still sees only previously reconstructed pixels.
enum Reconstruction {
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
    },
    Residual {
        component: usize,
        origin: [usize; 2],
        log: u8,
        residual: hevc_block::Coefficients,
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
    for command in commands {
        match command {
            Reconstruction::Inter { motion, rect } => {
                for c in 0..3 {
                    let shift = usize::from(c != 0);
                    let pixels = hevc_motion::predict(
                        lists,
                        motion,
                        rect,
                        c,
                        sps.depth[shift],
                        slice.weights.as_ref(),
                        scratch,
                    )?;
                    planes[c].reconstruct_inter(rect.map(|v| v as usize >> shift), &pixels)?;
                }
            }
            Reconstruction::Intra {
                component,
                origin,
                log,
                mode,
                residual,
            } => {
                if let Some(block) = residual {
                    block.reconstruct(scaling, transform_scratch, residual_scratch)?;
                } else {
                    residual_scratch.clear();
                }
                planes[component].reconstruct_intra_with_reference_filtering(
                    origin,
                    log,
                    mode,
                    component != 0,
                    sps.strong_intra_smoothing,
                    !sps.intra_smoothing_disabled,
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
            } => {
                residual.reconstruct(scaling, transform_scratch, residual_scratch)?;
                planes[component].add_residual(origin, log, residual_scratch)?;
            }
        }
    }
    Ok(())
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
                        s.address as usize <= y / side * width.div_ceil(side) + x / side
                    }) - 1
                };
                let q_owner = owner(x, y);
                let p_owner = if direction == 0 {
                    owner(x - 1, y)
                } else {
                    owner(x, y - 1)
                };
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
                    [[true; 2]; 4],
                );
                for l in 0..4 {
                    for s in 0..2 {
                        let output = if s == 0 { out_p[l] } else { out_q[l] };
                        for v in 0..3 {
                            plane[index(l, s, v)] = output[v];
                        }
                    }
                }
                if edge % 16 == 0 && (if direction == 0 { y } else { x }) % 8 == 0 && strength == 2
                {
                    for c in 1..3 {
                        let tc = d::chroma_tc(
                            p,
                            q,
                            decoder.pps.chroma_qp_offsets[c - 1],
                            slice.deblocking.offsets_div2[1],
                            decoder.sps.depth,
                        )?;
                        let plane = decoder.planes[c].samples_mut();
                        for l in 0..4 {
                            let index = |side: usize, v: usize| {
                                let delta = if side == 0 {
                                    -(v as isize) - 1
                                } else {
                                    v as isize
                                };
                                if direction == 0 {
                                    (y / 2 + l) * (width / 2)
                                        + (x / 2).checked_add_signed(delta).unwrap()
                                } else {
                                    (y / 2).checked_add_signed(delta).unwrap() * (width / 2)
                                        + x / 2
                                        + l
                                }
                            };
                            let a = [plane[index(0, 0)], plane[index(0, 1)]];
                            let b = [plane[index(1, 0)], plane[index(1, 1)]];
                            let output =
                                d::chroma_sample(a, b, tc, decoder.sps.depth[1], [true; 2])?;
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
    fn cell(&self, x: i32, y: i32) -> Option<Cell> {
        if x >= 0 && y >= 0 {
            let side = 1u32 << self.sps.coding_block_log2[1];
            let columns = self.sps.dimensions[0].div_ceil(side);
            if y as u32 / side * columns + (x as u32 / side) < self.slice_start {
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
            if self.jobs.is_some() {
                self.reconstruction
                    .push(Reconstruction::Inter { motion, rect });
            } else {
                for c in 0..3 {
                    let shift = usize::from(c != 0);
                    let pixels = hevc_motion::predict(
                        self.lists,
                        motion,
                        rect,
                        c,
                        self.sps.depth[shift],
                        self.slice.weights.as_ref(),
                        &mut self.scratch,
                    )?;
                    self.planes[c].reconstruct_inter(rect.map(|v| v as usize >> shift), &pixels)?;
                }
            }
            let [x, y, w, h] = rect.map(|v| v as usize / 4);
            for yy in y..y + h {
                for xx in x..x + w {
                    self.cells[yy * stride + xx] = Cell {
                        ready: true,
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
        hevc_transform_tree::read_inter(
            b,
            [n.x, n.y],
            config,
            partition != Partition::Full,
            |b, u| {
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
                let qps = hevc_qp::components(self.qp, self.sps.depth, self.chroma_offsets)?;
                for c in 0..3 {
                    if !u.coded[c] || (c != 0 && !u.owns_chroma) {
                        continue;
                    }
                    let config = hevc_block::Config {
                        log2_size: if c == 0 {
                            u.log2_size
                        } else {
                            u.log2_chroma_size
                        },
                        component: c as u8,
                        bit_depth: self.sps.depth[usize::from(c != 0)],
                        qp: qps[c],
                        intra_mode: None,
                        transform_skip_enabled: self.pps.transform_skip,
                        transquant_bypass: bypass,
                        sign_hiding: self.pps.sign_data_hiding,
                    };
                    let residual = hevc_block::read_with_tools(
                        b, config,
                        self.sps.transform_skip_rotation,
                        self.sps.transform_skip_context,
                    )?;
                    let origin = if c == 0 { u.origin } else { u.chroma_origin };
                    if self.jobs.is_some() {
                        self.reconstruction.push(Reconstruction::Residual {
                            component: c,
                            origin: origin.map(|v| v as usize),
                            log: config.log2_size,
                            residual,
                        });
                    } else {
                        residual.reconstruct(
                            self.pps
                                .scaling_lists
                                .as_ref()
                                .unwrap_or(&self.sps.scaling_lists),
                            &mut self.transform_scratch,
                            &mut self.residual_scratch,
                        )?;
                        self.planes[c].add_residual(
                            origin.map(|v| v as usize),
                            config.log2_size,
                            &self.residual_scratch,
                        )?;
                    }
                }
                Ok(())
            },
        )
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
