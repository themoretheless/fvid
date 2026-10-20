//! Bounded, unfiltered HEVC plane reconstruction with decoded-sample availability.
use super::hevc_intra::References;
use crate::{Result, invalid};
use std::sync::Arc;
#[derive(Clone)]
pub struct Plane {
    width: usize,
    height: usize,
    depth: u8,
    samples: Arc<Vec<u16>>,
    ready: Arc<Vec<bool>>,
}
impl Plane {
    pub(crate) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.samples, &other.samples) && Arc::ptr_eq(&self.ready, &other.ready)
    }

    pub(crate) fn absent(depth: u8) -> Self {
        Self {
            width: 0,
            height: 0,
            depth,
            samples: Arc::new(Vec::new()),
            ready: Arc::new(Vec::new()),
        }
    }
    pub(crate) fn ready_rect(&self, rect: [usize; 4]) -> bool {
        let [x, y, w, h] = rect;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|n| n > self.width)
            || y.checked_add(h).is_none_or(|n| n > self.height)
        {
            return false;
        }
        (y..y + h).all(|row| {
            self.ready[row * self.width + x..row * self.width + x + w]
                .iter()
                .all(|&r| r)
        })
    }
    pub(crate) fn bit_depth(&self) -> u8 { self.depth }

    pub fn dimensions(&self) -> [usize; 2] {
        [self.width, self.height]
    }

    /// Apply single-slice, single-tile SAO after reconstruction/deblocking.
    /// Neighbours always come from the unchanged input plane. Extra workspace
    /// is two bytes per sample and is accounted for by the picture decoder.
    pub fn apply_sao(&mut self, log2_ctu: u8, parameters: &[super::hevc_sao::Sao]) -> Result<()> {
        self.apply_sao_with_boundaries(log2_ctu, parameters, |_, _| true)
    }
    pub(crate) fn apply_sao_with_boundaries(
        &mut self,
        log2_ctu: u8,
        parameters: &[super::hevc_sao::Sao],
        available: impl FnMut([usize; 2], [usize; 2]) -> bool,
    ) -> Result<()> {
        self.apply_sao_with_exclusions(log2_ctu, parameters, available, |_| false)
    }
    pub(crate) fn apply_sao_with_exclusions(
        &mut self,
        log2_ctu: u8,
        parameters: &[super::hevc_sao::Sao],
        available: impl FnMut([usize; 2], [usize; 2]) -> bool,
        excluded: impl FnMut([usize; 2]) -> bool,
    ) -> Result<()> {
        self.apply_sao_rectangular([log2_ctu; 2], parameters, available, excluded)
    }
    pub(crate) fn apply_sao_rectangular(
        &mut self,
        log2_ctu: [u8; 2],
        parameters: &[super::hevc_sao::Sao],
        mut available: impl FnMut([usize; 2], [usize; 2]) -> bool,
        mut excluded: impl FnMut([usize; 2]) -> bool,
    ) -> Result<()> {
        if log2_ctu.iter().any(|v| !(3..=6).contains(v)) || !self.complete() {
            return Err(invalid("invalid SAO plane state"));
        }
        let [ctu_width, ctu_height] = log2_ctu.map(|v| 1usize << v);
        let columns = self.width.div_ceil(ctu_width);
        let rows = self.height.div_ceil(ctu_height);
        if columns.checked_mul(rows) != Some(parameters.len()) {
            return Err(invalid("invalid SAO parameter grid"));
        }
        use super::hevc_sao::Sao;
        // Validate once per CTU; reconstructed plane samples already obey depth.
        for &sao in parameters {
            sao.apply(0, None, self.depth)?;
        }
        let mut output = self.samples.as_ref().clone();
        let max = (1i32 << self.depth) - 1;
        for (index, &sao) in parameters.iter().enumerate() {
            let x0 = (index % columns) * ctu_width;
            let y0 = (index / columns) * ctu_height;
            let x1 = (x0 + ctu_width).min(self.width);
            let y1 = (y0 + ctu_height).min(self.height);
            match sao {
                Sao::Off => {}
                Sao::Band { position, offsets } => {
                    let mut bands = [0i32; 32];
                    for (i, &offset) in offsets.iter().enumerate() {
                        bands[(position as usize + i) & 31] = i32::from(offset);
                    }
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if excluded([x, y]) {
                                continue;
                            }
                            let k = y * self.width + x;
                            let value = self.samples[k];
                            output[k] = (i32::from(value)
                                + bands[(value >> (self.depth - 5)) as usize])
                                .clamp(0, max) as u16;
                        }
                    }
                }
                Sao::Edge { offsets, .. } => {
                    let [a, b] = sao.neighbours()?.unwrap();
                    let offsets = [
                        i32::from(offsets[0]),
                        i32::from(offsets[1]),
                        0,
                        i32::from(offsets[2]),
                        i32::from(offsets[3]),
                    ];
                    let x0 = x0.max(usize::from(a[0] < 0 || b[0] < 0));
                    let y0 = y0.max(usize::from(a[1] < 0 || b[1] < 0));
                    let x1 = x1.min(self.width - usize::from(a[0] > 0 || b[0] > 0));
                    let y1 = y1.min(self.height - usize::from(a[1] > 0 || b[1] > 0));
                    let delta = |v: [i32; 2]| v[1] as isize * self.width as isize + v[0] as isize;
                    let da = delta(a);
                    let db = delta(b);
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if excluded([x, y]) {
                                continue;
                            }
                            let neighbour = |v: [i32; 2]| {
                                [
                                    x.checked_add_signed(v[0] as isize).unwrap(),
                                    y.checked_add_signed(v[1] as isize).unwrap(),
                                ]
                            };
                            if !available([x, y], neighbour(a)) || !available([x, y], neighbour(b))
                            {
                                continue;
                            }
                            let k = y * self.width + x;
                            let value = i32::from(self.samples[k]);
                            let a = i32::from(self.samples[k.checked_add_signed(da).unwrap()]);
                            let b = i32::from(self.samples[k.checked_add_signed(db).unwrap()]);
                            let category =
                                ((value - a).signum() + (value - b).signum() + 2) as usize;
                            output[k] = (value + offsets[category]).clamp(0, max) as u16;
                        }
                    }
                }
            }
        }
        self.samples = Arc::new(output);
        Ok(())
    }
    pub fn new(width: usize, height: usize, depth: u8, budget: usize) -> Result<Self> {
        let count = width
            .checked_mul(height)
            .filter(|&n| n > 0 && n <= budget / 3)
            .ok_or_else(|| invalid("HEVC plane exceeds memory budget"))?;
        if !(8..=16).contains(&depth) {
            return Err(invalid("unsupported HEVC plane depth"));
        }
        Ok(Self {
            width,
            height,
            depth,
            samples: Arc::new(vec![0; count]),
            ready: Arc::new(vec![false; count]),
        })
    }
    pub(crate) fn reconstruct_inter(&mut self, rect: [usize; 4], prediction: &[u16]) -> Result<()> {
        let [x, y, w, h] = rect;
        if w == 0
            || h == 0
            || x.checked_add(w).is_none_or(|v| v > self.width)
            || y.checked_add(h).is_none_or(|v| v > self.height)
            || w.checked_mul(h) != Some(prediction.len())
        {
            return Err(invalid("invalid HEVC inter block geometry"));
        }
        for j in 0..h {
            let start = (y + j) * self.width + x;
            let ready = &mut Arc::make_mut(&mut self.ready)[start..start + w];
            if ready.iter().fold(false, |seen, &v| seen | v) {
                return Err(invalid("overlapping HEVC inter prediction"));
            }
            Arc::make_mut(&mut self.samples)[start..start + w]
                .copy_from_slice(&prediction[j * w..(j + 1) * w]);
            ready.fill(true);
        }
        Ok(())
    }
    /// I.8.5.4: constant inter residual in sample space, independent of DLT.
    /// Validate the entire CU before mutation; preserve shared reference planes.
    pub(crate) fn add_dc_residual(&mut self,origin:[usize;2],log:u8,offset:i64)->Result<()> {
        if !(3..=6).contains(&log) {return Err(invalid("invalid HEVC inter DC-only size"));}
        let n=1usize<<log;let [x,y]=origin;
        if x%n!=0 || y%n!=0 || x.checked_add(n).is_none_or(|v|v>self.width)
            || y.checked_add(n).is_none_or(|v|v>self.height) {
            return Err(invalid("invalid HEVC inter DC-only geometry"));
        }
        for yy in y..y+n {
            if !self.ready[yy*self.width+x..yy*self.width+x+n].iter().all(|&v|v) {
                return Err(invalid("HEVC inter DC-only block has no complete prediction"));
            }
        }
        let maximum=(1i128<<self.depth)-1;
        let samples=Arc::make_mut(&mut self.samples);
        for yy in y..y+n {for sample in &mut samples[yy*self.width+x..yy*self.width+x+n] {
            *sample=(i128::from(*sample)+i128::from(offset)).clamp(0,maximum)as u16;
        }}
        Ok(())
    }
    pub(crate) fn add_residual(
        &mut self,
        origin: [usize; 2],
        log: u8,
        residual: &[i32],
    ) -> Result<()> {
        let [x, y] = origin;
        let n = 1usize
            .checked_shl(u32::from(log))
            .ok_or_else(|| invalid("invalid HEVC residual size"))?;
        if x.checked_add(n).is_none_or(|v| v > self.width)
            || y.checked_add(n).is_none_or(|v| v > self.height)
            || n.checked_mul(n) != Some(residual.len())
        {
            return Err(invalid("invalid HEVC residual geometry"));
        }
        let max = (1i32 << self.depth) - 1;
        for j in 0..n {
            let start = (y + j) * self.width + x;
            if !self.ready[start..start + n]
                .iter()
                .fold(true, |ready, &v| ready & v)
            {
                return Err(invalid("HEVC residual has no prediction"));
            }
            for (pixel, &delta) in Arc::make_mut(&mut self.samples)[start..start + n]
                .iter_mut()
                .zip(&residual[j * n..(j + 1) * n])
            {
                *pixel = i32::from(*pixel).saturating_add(delta).clamp(0, max) as u16;
            }
        }
        Ok(())
    }
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
    pub(crate) fn storage_bytes(&self) -> Option<usize> {
        self.samples
            .capacity()
            .checked_mul(std::mem::size_of::<u16>())?
            .checked_add(self.ready.capacity())?
            .checked_add(2 * (std::mem::size_of::<Vec<u16>>() + 2 * std::mem::size_of::<usize>()))
    }
    pub(crate) fn samples_mut(&mut self) -> &mut [u16] {
        Arc::make_mut(&mut self.samples).as_mut_slice()
    }
    pub fn complete(&self) -> bool {
        self.ready.iter().fold(true, |ready, &v| ready & v)
    }
    /// Ordinary intra DC-only reconstruction, then I-95..97 offset assignment.
    /// Lookup failures affect scratch only; plane samples remain unchanged.
    pub fn reconstruct_dc_only(
        &mut self,origin:[usize;2],log:u8,mode:u8,filters:[bool;3],offset:i64,
        tables:&super::hevc_depth_tables::Tables,layer:u8,
        prediction:&mut Vec<u16>,available:impl Fn(usize,usize)->bool,
    )->Result<()> {
        if !(2..=5).contains(&log) {return Err(invalid("invalid HEVC DC-only mode or depth"));}
        self.reconstruct_dc_only_split(origin,log,log,mode,filters,offset,tables,layer,prediction,available)
    }
    pub fn reconstruct_dc_only_split(
        &mut self,origin:[usize;2],log:u8,maximum_log:u8,mode:u8,filters:[bool;3],offset:i64,
        tables:&super::hevc_depth_tables::Tables,layer:u8,
        prediction:&mut Vec<u16>,available:impl Fn(usize,usize)->bool,
    )->Result<()> {
        if !(2..=6).contains(&log) || !(2..=5).contains(&maximum_log) || mode>34 || tables.bit_depth!=self.depth {
            return Err(invalid("invalid HEVC DC-only mode or depth"));
        }
        let n=1usize<<log;let [x,y]=origin;
        if x.checked_add(n).is_none_or(|v|v>self.width) || y.checked_add(n).is_none_or(|v|v>self.height)
            || x%n!=0 || y%n!=0 {return Err(invalid("invalid HEVC DC-only block geometry"));}
        for yy in y..y+n {if self.ready[yy*self.width+x..yy*self.width+x+n].iter().any(|&v|v) {
            return Err(invalid("HEVC block overlaps reconstructed samples"));
        }}
        let sample=|xx:Option<usize>,yy:Option<usize>| {
            let (xx,yy)=(xx?,yy?);
            if xx>=self.width || yy>=self.height || !self.ready[yy*self.width+xx] || !available(xx,yy) {None}
            else {Some(self.samples[yy*self.width+xx])}
        };
        let top:Vec<_>=(0..2*n).map(|i|sample(x.checked_add(i),y.checked_sub(1))).collect();
        let left:Vec<_>=(0..2*n).map(|i|sample(x.checked_sub(1),y.checked_add(i))).collect();
        let references=References::new_depth_skip(log,self.depth,sample(x.checked_sub(1),y.checked_sub(1)),&top,&left)?;
        // Validate lookup ownership before any child publishes prediction.
        tables.dc_prediction(layer,0,offset)?;
        if log>maximum_log {
            self.dc_prediction_subtree(origin,log,maximum_log,mode,filters,prediction,
                &|xx,yy| (xx>=x && xx<x+n && yy>=y && yy<y+n) || available(xx,yy))?;
            let width=self.width;
            tables.assign_dc_offset(layer,Arc::make_mut(&mut self.samples).as_mut_slice(),width,origin,n,offset)?;
            return Ok(());
        }
        prediction.resize(n*n,0);
        references.predict_with_filters(mode,false,filters[0],filters[1],filters[2],prediction)?;
        tables.assign_dc_offset(layer,prediction,n,[0,0],n,offset)?;
        for yy in 0..n {
            let start=(y+yy)*self.width+x;
            Arc::make_mut(&mut self.samples)[start..start+n].copy_from_slice(&prediction[yy*n..(yy+1)*n]);
            Arc::make_mut(&mut self.ready)[start..start+n].fill(true);
        }
        Ok(())
    }
    fn dc_prediction_subtree(
        &mut self,origin:[usize;2],log:u8,maximum_log:u8,mode:u8,filters:[bool;3],
        prediction:&mut Vec<u16>,available:&impl Fn(usize,usize)->bool,
    )->Result<()> {
        if log>maximum_log {
            let half=1usize<<(log-1);
            for [dx,dy] in [[0,0],[half,0],[0,half],[half,half]] {
                self.dc_prediction_subtree([origin[0]+dx,origin[1]+dy],log-1,maximum_log,mode,filters,prediction,available)?;
            }
            Ok(())
        } else {
            self.reconstruct_intra_with_filters(origin,log,mode,false,filters[0],filters[1],filters[2],&[],prediction,available)
        }
    }
    /// Wedge/contour partition prediction and optional residual reconstruction.
    /// Signed DC prediction is clipped only after residual addition.
    pub fn reconstruct_depth_partition(
        &mut self,origin:[usize;2],log:u8,pattern:&[bool],offsets:[i64;2],
        tables:&super::hevc_depth_tables::Tables,layer:u8,residual:&[i32],
        prediction:&mut Vec<i128>,available:impl Fn(usize,usize)->bool,
    )->Result<()> {
        if !(2..=5).contains(&log) || tables.bit_depth!=self.depth {
            return Err(invalid("invalid HEVC depth partition mode or depth"));
        }
        let n=1usize<<log;let [x,y]=origin;
        if x.checked_add(n).is_none_or(|v|v>self.width) || y.checked_add(n).is_none_or(|v|v>self.height)
            || x%n!=0 || y%n!=0 || pattern.len()!=n*n || !(residual.is_empty() || residual.len()==n*n) {
            return Err(invalid("invalid HEVC depth partition block geometry"));
        }
        for yy in y..y+n {if self.ready[yy*self.width+x..yy*self.width+x+n].iter().any(|&v|v) {
            return Err(invalid("HEVC block overlaps reconstructed samples"));
        }}
        let sample=|xx:Option<usize>,yy:Option<usize>| {
            let (xx,yy)=(xx?,yy?);
            if xx>=self.width || yy>=self.height || !self.ready[yy*self.width+xx] || !available(xx,yy) {None}
            else {Some(self.samples[yy*self.width+xx])}
        };
        let mut top=[None;64];let mut left=[None;64];
        for i in 0..2*n {top[i]=sample(x.checked_add(i),y.checked_sub(1));left[i]=sample(x.checked_sub(1),y.checked_add(i));}
        let references=References::new(log,self.depth,sample(x.checked_sub(1),y.checked_sub(1)),&top[..2*n],&left[..2*n])?;
        let mut top_values=[0u16;64];let mut left_values=[0u16;64];
        references.copy_depth_edges(&mut top_values[..2*n],&mut left_values[..2*n])?;
        prediction.resize(n*n,0);
        super::hevc_depth_prediction::predict(tables,layer,super::hevc_depth_prediction::Partition {
            size:n,top:&top_values[..2*n],left:&left_values[..2*n],pattern,offsets,
        },prediction)?;
        let maximum=(1i128<<self.depth)-1;
        for yy in 0..n {
            let start=(y+yy)*self.width+x;
            for xx in 0..n {
                let i=yy*n+xx;
                let value=prediction[i]+i128::from(if residual.is_empty(){0}else{residual[i]});
                Arc::make_mut(&mut self.samples)[start+xx]=value.clamp(0,maximum) as u16;
            }
            Arc::make_mut(&mut self.ready)[start..start+n].fill(true);
        }
        Ok(())
    }
    /// Annex I angular skip-intra modes 0/1 map to angular 26/10.
    /// Unlike INTRA_SINGLE, unavailable references use ordinary substitution.
    pub fn reconstruct_skip_angular(
        &mut self, origin:[usize;2],log:u8,skip_mode:u8,filter_boundary:bool,
        prediction:&mut Vec<u16>,available:impl Fn(usize,usize)->bool,
    )->Result<()> {
        if !(2..=6).contains(&log) || skip_mode>1 {
            return Err(invalid("invalid HEVC angular skip-intra mode or size"));
        }
        let n=1usize<<log;let [x,y]=origin;
        if x.checked_add(n).is_none_or(|v|v>self.width) || y.checked_add(n).is_none_or(|v|v>self.height)
            || x%n!=0 || y%n!=0 {return Err(invalid("invalid HEVC angular skip-intra geometry"));}
        for yy in y..y+n {if self.ready[yy*self.width+x..yy*self.width+x+n].iter().any(|&v|v) {
            return Err(invalid("HEVC block overlaps reconstructed samples"));
        }}
        let sample=|xx:Option<usize>,yy:Option<usize>| {
            let (xx,yy)=(xx?,yy?);
            if xx>=self.width || yy>=self.height || !self.ready[yy*self.width+xx] || !available(xx,yy) {None}
            else {Some(self.samples[yy*self.width+xx])}
        };
        let top:Vec<_>=(0..2*n).map(|i|sample(x.checked_add(i),y.checked_sub(1))).collect();
        let left:Vec<_>=(0..2*n).map(|i|sample(x.checked_sub(1),y.checked_add(i))).collect();
        let references=References::new_depth_skip(log,self.depth,sample(x.checked_sub(1),y.checked_sub(1)),&top,&left)?;
        prediction.resize(n*n,0);
        references.predict_with_filters(if skip_mode==0 {26}else{10},false,false,true,filter_boundary,prediction)?;
        for yy in 0..n {
            let start=(y+yy)*self.width+x;
            Arc::make_mut(&mut self.samples)[start..start+n].copy_from_slice(&prediction[yy*n..(yy+1)*n]);
            Arc::make_mut(&mut self.ready)[start..start+n].fill(true);
        }
        Ok(())
    }
    /// INTRA_SINGLE reconstruction without residuals or reference substitution.
    /// The caller supplies slice/tile/constrained-intra neighbour eligibility.
    pub fn reconstruct_single(
        &mut self,
        origin:[usize;2],
        log:u8,
        skip_mode:u8,
        available:impl Fn(usize,usize)->bool,
    )->Result<()> {
        if !(2..=6).contains(&log) || !matches!(skip_mode,2|3) {
            return Err(invalid("invalid HEVC single-sample block mode or size"));
        }
        let size=1usize<<log;let [x,y]=origin;
        if x.checked_add(size).is_none_or(|v|v>self.width)
            || y.checked_add(size).is_none_or(|v|v>self.height)
            || x%size!=0 || y%size!=0 {
            return Err(invalid("invalid HEVC single-sample block geometry"));
        }
        for row in y..y+size {
            if self.ready[row*self.width+x..row*self.width+x+size].iter().any(|&v|v) {
                return Err(invalid("HEVC block overlaps reconstructed samples"));
            }
        }
        let selected=if skip_mode==2 {x.checked_sub(1).map(|xx|(xx,y+size/2))}
            else {y.checked_sub(1).map(|yy|(x+size/2,yy))};
        let sample=selected.filter(|&(xx,yy)|self.ready[yy*self.width+xx] && available(xx,yy))
            .map(|(xx,yy)|self.samples[yy*self.width+xx]);
        let value=super::hevc_depth_prediction::single_value(self.depth,sample)?;
        for row in y..y+size {
            let start=row*self.width+x;
            Arc::make_mut(&mut self.samples)[start..start+size].fill(value);
            Arc::make_mut(&mut self.ready)[start..start+size].fill(true);
        }
        Ok(())
    }
    /// `available` applies slice/tile/constrained-intra restrictions in addition
    /// to the plane's bounds and already-reconstructed sample checks.
    pub fn reconstruct_intra(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        self.reconstruct_intra_with_reference_filtering(
            origin,
            log,
            mode,
            chroma,
            strong,
            true,
            residual,
            pred_scratch,
            available,
        )
    }

    pub fn reconstruct_intra_with_reference_filtering(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        filter_references: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        self.reconstruct_intra_with_filters(
            origin,
            log,
            mode,
            chroma,
            strong,
            filter_references,
            true,
            residual,
            pred_scratch,
            available,
        )
    }
    pub fn reconstruct_intra_with_filters(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        strong: bool,
        filter_references: bool,
        filter_boundary: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        self.reconstruct_intra_with_full_chroma_filters(
            origin,
            log,
            mode,
            chroma,
            false,
            strong,
            filter_references,
            filter_boundary,
            residual,
            pred_scratch,
            available,
        )
    }
    pub fn reconstruct_intra_with_full_chroma_filters(
        &mut self,
        origin: [usize; 2],
        log: u8,
        mode: u8,
        chroma: bool,
        full_chroma: bool,
        strong: bool,
        filter_references: bool,
        filter_boundary: bool,
        residual: &[i32],
        pred_scratch: &mut Vec<u16>,
        available: impl Fn(usize, usize) -> bool,
    ) -> Result<()> {
        if !(2..=5).contains(&log) {
            return Err(invalid("invalid HEVC plane block size"));
        }
        let n = 1usize << log;
        let [x, y] = origin;
        if x.checked_add(n).is_none_or(|end| end > self.width)
            || y.checked_add(n).is_none_or(|end| end > self.height)
            || x % n != 0
            || y % n != 0
            || !(residual.is_empty() || residual.len() == n * n)
        {
            return Err(invalid("invalid HEVC plane block geometry"));
        }
        for yy in y..y + n {
            if self.ready[yy * self.width + x..yy * self.width + x + n]
                .iter()
                .fold(false, |seen, &v| seen | v)
            {
                return Err(invalid("HEVC block overlaps reconstructed samples"));
            }
        }
        let sample = |xx: Option<usize>, yy: Option<usize>| {
            let (xx, yy) = (xx?, yy?);
            if xx >= self.width
                || yy >= self.height
                || !self.ready[yy * self.width + xx]
                || !available(xx, yy)
            {
                None
            } else {
                Some(self.samples[yy * self.width + xx])
            }
        };
        let top: Vec<_> = (0..2 * n)
            .map(|i| sample(x.checked_add(i), y.checked_sub(1)))
            .collect();
        let left: Vec<_> = (0..2 * n)
            .map(|i| sample(x.checked_sub(1), y.checked_add(i)))
            .collect();
        let references = References::new(
            log,
            self.depth,
            sample(x.checked_sub(1), y.checked_sub(1)),
            &top,
            &left,
        )?;
        pred_scratch.resize(n * n, 0);
        references.predict_with_full_chroma_filters(
            mode,
            chroma,
            full_chroma,
            strong,
            filter_references,
            filter_boundary,
            pred_scratch,
        )?;
        let max = (1i32 << self.depth) - 1;
        for yy in 0..n {
            let start = (y + yy) * self.width + x;
            for (xx, (pixel, &prediction)) in Arc::make_mut(&mut self.samples)[start..start + n]
                .iter_mut()
                .zip(&pred_scratch[yy * n..(yy + 1) * n])
                .enumerate()
            {
                let delta = if residual.is_empty() {
                    0
                } else {
                    residual[yy * n + xx]
                };
                *pixel = i32::from(prediction).saturating_add(delta).clamp(0, max) as u16;
            }
            Arc::make_mut(&mut self.ready)[start..start + n].fill(true);
        }
        Ok(())
    }
}
#[cfg(test)]
mod shared_storage_tests {
    use super::*;
    #[test]
    fn forced_dc_splits_predict_from_unadjusted_internal_children_then_apply_global_offset() {
        use crate::codec::hevc_depth_tables::{Tables,Layer};
        let mut source=Plane::new(192,192,8,1<<20).unwrap();
        source.reconstruct_inter([64,63,128,1],&[20;128]).unwrap();
        source.reconstruct_inter([63,64,1,128],&[100;128]).unwrap();
        source.reconstruct_inter([63,63,1,1],&[50]).unwrap();
        for active in [false,true] {
            let tables=Tables {bit_depth:8,layers:vec![Layer {id:3,values:if active {Some((0..256).step_by(8).collect())}else{None}}]};
            let mut plane=source.clone();let mut scratch=Vec::new();
            plane.reconstruct_dc_only_split([64,64],6,5,1,[false;3],1,&tables,3,&mut scratch,
                |x,y|x<64 || x>=128 || y<64 || y>=128).unwrap();
            let adjustment=if active {12}else{1};
            for y in 0..64 {for x in 0..64 {
                let prediction=match (x/32,y/32) {(0,0)=>60,(1,0)=>40,(0,1)=>80,_=>60};
                assert_eq!(plane.samples()[(64+y)*192+64+x],prediction+adjustment);
                assert_eq!(source.samples()[(64+y)*192+64+x],0);
            }}
            assert!(plane.ready_rect([64,64,64,64]));
        }
    }

    #[test]
    fn ordinary_dc_lookup_errors_preserve_plane_samples_and_readiness() {
        use crate::codec::hevc_depth_tables::{Tables,Layer};
        let mut plane=Plane::new(4,4,8,1024).unwrap();
        let tables=Tables {bit_depth:8,layers:vec![Layer {id:3,values:Some(vec![])}]};
        let before=plane.samples().to_vec();let mut scratch=Vec::new();
        assert!(plane.reconstruct_dc_only([0,0],2,1,[false;3],1,&tables,3,&mut scratch,|_,_|true).is_err());
        assert_eq!(plane.samples(),before);assert!(!plane.complete());
        let identity=Tables {bit_depth:8,layers:vec![Layer {id:3,values:None}]};
        plane.reconstruct_dc_only([0,0],2,1,[false;3],1,&identity,3,&mut scratch,|_,_|true).unwrap();
        assert!(plane.complete());assert!(plane.samples().iter().all(|&v|v==129));
    }

    #[test]
    fn depth_partition_reconstruction_combines_signed_prediction_residual_and_lookup() {
        use crate::codec::hevc_depth_tables::{Tables,Layer};
        let mut seed=Plane::new(12,12,8,4096).unwrap();
        seed.reconstruct_inter([4,3,8,1],&[20,30,40,50,60,70,80,90]).unwrap();
        seed.reconstruct_inter([3,4,1,8],&[100,110,120,130,140,150,160,170]).unwrap();
        seed.reconstruct_inter([3,3,1,1],&[75]).unwrap();
        let pattern:Vec<_>=(0..16).map(|i|i%4>=2).collect();
        for lookup in [None,Some(vec![0,60,90,130,170,255])] {
            let tables=Tables {bit_depth:8,layers:vec![Layer {id:3,values:lookup.clone()}]};
            let mut plane=seed.clone();let mut scratch=Vec::new();
            plane.reconstruct_depth_partition([4,4],2,&pattern,[-120,2],&tables,3,&[20;16],&mut scratch,|_,_|true).unwrap();
            assert!(plane.ready_rect([4,4,4,4]));
            for y in 0..4 {for x in 0..4 {
                let expected=if lookup.is_some() {if x<2 {20}else{150}}else{if x<2 {10}else{72}};
                assert_eq!(plane.samples()[(4+y)*12+4+x],expected);
                assert_eq!(seed.samples()[(4+y)*12+4+x],0);
            }}
            if lookup.is_none() {assert_eq!(scratch[0],-10);}
            let saved=plane.samples().to_vec();
            assert!(plane.reconstruct_depth_partition([4,4],2,&pattern,[0;2],&tables,3,&[],&mut scratch,|_,_|true).is_err());
            assert_eq!(plane.samples(),saved);
            let mut invalid=seed.clone();
            assert!(invalid.reconstruct_depth_partition([4,4],2,&pattern,[0;2],&tables,2,&[],&mut scratch,|_,_|true).is_err());
            assert_eq!(invalid.samples(),seed.samples());assert!(!invalid.ready_rect([4,4,4,4]));
        }
        let tables=Tables {bit_depth:8,layers:vec![Layer {id:3,values:None}]};
        let mut plane=seed.clone();let mut scratch=Vec::new();
        plane.reconstruct_depth_partition([4,4],2,&pattern,[-120,2],&tables,3,&[20;16],&mut scratch,|_,_|false).unwrap();
        for y in 0..4 {for x in 0..4 {assert_eq!(plane.samples()[(4+y)*12+4+x],if x<2 {28}else{150});}}
    }

    #[test]
    fn angular_skip_reconstructs_sizes_through_64_with_boundary_rules() {
        for log in 2..=6 {for depth in [8,10,12,16] {for mode in [0,1] {for boundary in [false,true] {
            let n=1usize<<log;
            let mut plane=Plane::new(2*n,2*n,depth,1<<20).unwrap();
            let top:Vec<_>=(0..n).map(|i|100+i as u16).collect();
            let left:Vec<_>=(0..n).map(|i|50+i as u16).collect();
            plane.reconstruct_inter([n,n-1,n,1],&top).unwrap();
            plane.reconstruct_inter([n-1,n,1,n],&left).unwrap();
            plane.reconstruct_inter([n-1,n-1,1,1],&[75]).unwrap();
            let before=plane.clone();let mut scratch=Vec::new();
            plane.reconstruct_skip_angular([n,n],log,mode,boundary,&mut scratch,|_,_|true).unwrap();
            for y in 0..n {for x in 0..n {
                let mut expected=if mode==0 {top[x]}else{left[y]};
                if boundary && n<32 && (mode==0 && x==0 || mode==1 && y==0) {
                    expected=if mode==0 {100+((i32::from(left[y])-75)>>1)}else{50+((i32::from(top[x])-75)>>1)} as u16;
                }
                assert_eq!(plane.samples()[(n+y)*2*n+n+x],expected);
                assert_eq!(before.samples()[(n+y)*2*n+n+x],0);
            }}
            assert!(plane.ready_rect([n,n,n,n]));
            let saved=plane.samples().to_vec();
            assert!(plane.reconstruct_skip_angular([n,n],log,mode,boundary,&mut scratch,|_,_|true).is_err());
            assert_eq!(plane.samples(),saved);
            let mut unavailable=before;
            unavailable.reconstruct_skip_angular([n,n],log,mode,boundary,&mut scratch,|_,_|false).unwrap();
            for y in n..2*n {assert!(unavailable.samples()[y*2*n+n..y*2*n+2*n].iter().all(|&v|v==1<<(depth-1)));}
        }}}}
        assert!(References::new(6,8,None,&[None;128],&[None;128]).is_err());
    }


    #[test]
    fn single_reconstruction_respects_readiness_restrictions_and_copy_on_write() {
        for (mode, permitted, ready, expected) in [(2,true,true,77), (3,true,true,88), (2,false,true,512), (2,true,false,512)] {
            let mut plane=Plane::new(8,8,10,4096).unwrap();
            plane.reconstruct_inter([4,3,4,1], &[11,22,88,44]).unwrap();
            if ready {plane.reconstruct_inter([3,4,1,4], &[10,20,77,40]).unwrap();}
            else {plane.reconstruct_inter([3,4,1,2], &[10,20]).unwrap();}
            let original=plane.clone();
            plane.reconstruct_single([4,4],2,mode,|_,_|permitted).unwrap();
            assert!(plane.ready_rect([4,4,4,4]));
            assert!(!original.ready_rect([4,4,4,4]));
            for y in 4..8 {assert!(plane.samples()[y*8+4..y*8+8].iter().all(|&v|v==expected));
                assert!(original.samples()[y*8+4..y*8+8].iter().all(|&v|v==0));}
            let before=plane.samples().to_vec();
            assert!(plane.reconstruct_single([4,4],2,mode,|_,_|true).is_err());
            assert_eq!(plane.samples(),before);
        }
        let mut plane=Plane::new(64,64,16,65536).unwrap();
        plane.reconstruct_single([0,0],6,2,|_,_|panic!("no in-bounds neighbour")).unwrap();
        assert!(plane.complete());assert!(plane.samples().iter().all(|&v|v==32768));
    }


    #[test]
    fn cloned_plane_shares_storage_until_reconstruction_mutates_it() {
        let mut original = Plane::new(4, 4, 8, 1024).unwrap();
        let held = original.clone();
        assert!(Arc::ptr_eq(&original.samples, &held.samples));
        assert!(Arc::ptr_eq(&original.ready, &held.ready));
        original.reconstruct_inter([0, 0, 4, 4], &[77; 16]).unwrap();
        assert_eq!(held.samples(), &[0; 16]);
        assert!(!held.complete());
        assert_eq!(original.samples(), &[77; 16]);
        assert!(original.complete());
        assert!(!Arc::ptr_eq(&original.samples, &held.samples));
        assert!(!Arc::ptr_eq(&original.ready, &held.ready));
        let held = original.clone();
        original.add_residual([0, 0], 2, &[3; 16]).unwrap();
        assert_eq!(held.samples(), &[77; 16]);
        assert_eq!(original.samples(), &[80; 16]);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rectangular_sao_matches_scalar_for_partial_ctus_and_boundaries() {
        use super::super::hevc_sao::Sao;
        for depth in [8, 10, 12] {
            let (width, height) = (19usize, 35usize);
            let samples: Vec<u16> = (0..width * height)
                .map(|i| ((i * 73 + i / width * 31) & ((1 << depth) - 1)) as u16)
                .collect();
            for class in 0..5 {
                let parameters: Vec<_> = (0..9)
                    .map(|index| {
                        if class == 4 {
                            Sao::Band {
                                position: (index * 3) as u8,
                                offsets: [2, -2, 1, -1],
                            }
                        } else {
                            Sao::Edge {
                                class,
                                offsets: [3, 1, -1, -3],
                            }
                        }
                    })
                    .collect();
                let mut p = Plane::new(width, height, depth, width * height * 3).unwrap();
                Arc::make_mut(&mut p.samples).clone_from(&samples);
                Arc::make_mut(&mut p.ready).fill(true);
                let mut expected = samples.clone();
                for y in 0..height {
                    for x in 0..width {
                        if x == 4 {
                            continue;
                        }
                        let mode = parameters[(y / 16) * 3 + x / 8];
                        let neighbours = if let Some([a, b]) = mode.neighbours().unwrap() {
                            let point = |v: [i32; 2]| -> Option<[usize; 2]> {
                                let nx = x.checked_add_signed(v[0] as isize)?;
                                let ny = y.checked_add_signed(v[1] as isize)?;
                                (nx < width && ny < height && ny / 16 == y / 16).then_some([nx, ny])
                            };
                            let (Some(a), Some(b)) = (point(a), point(b)) else {
                                continue;
                            };
                            Some([samples[a[1] * width + a[0]], samples[b[1] * width + b[0]]])
                        } else {
                            None
                        };
                        expected[y * width + x] = mode
                            .apply(samples[y * width + x], neighbours, depth)
                            .unwrap();
                    }
                }
                p.apply_sao_rectangular(
                    [3, 4],
                    &parameters,
                    |a, b| a[1] / 16 == b[1] / 16,
                    |p| p[0] == 4,
                )
                .unwrap();
                assert_eq!(p.samples(), expected);
                let saved = p.samples.clone();
                assert!(
                    p.apply_sao_rectangular([3, 4], &parameters[..8], |_, _| true, |_| false)
                        .is_err()
                );
                assert_eq!(p.samples, saved);
            }
        }
    }
    #[test]
    fn sao_preserves_protected_samples_but_uses_them_as_neighbours() {
        use super::super::hevc_sao::Sao;
        let samples: Vec<u16> = (0..64).map(|i| if i % 2 == 0 { 10 } else { 20 }).collect();
        for (mode, expected) in [
            (
                Sao::Edge {
                    class: 0,
                    offsets: [7, 0, 0, -7],
                },
                [10, 20, 10, 20, 17, 13, 17, 20],
            ),
            (
                Sao::Band {
                    position: 1,
                    offsets: [2, 0, 0, 0],
                },
                [10, 20, 10, 20, 12, 20, 12, 20],
            ),
        ] {
            let mut plane = Plane::new(8, 8, 8, 4096).unwrap();
            plane.reconstruct_inter([0, 0, 8, 8], &samples).unwrap();
            plane
                .apply_sao_with_exclusions(3, &[mode], |_, _| true, |p| p[0] < 4)
                .unwrap();
            for row in plane.samples().chunks_exact(8) {
                assert_eq!(row, expected);
            }
        }
    }
    #[test]
    fn residual_addition_matches_wide_clipping_at_integer_limits() {
        let residual = [
            i32::MIN,
            i32::MAX,
            -2048,
            2048,
            -128,
            127,
            0,
            1,
            -1,
            255,
            -255,
            1023,
            -1023,
            32,
            -32,
            42,
        ];
        for depth in [8, 10, 12] {
            let prediction = 1u16 << (depth - 1);
            let expected: Vec<u16> = residual
                .iter()
                .map(|&v| {
                    (i64::from(prediction) + i64::from(v)).clamp(0, (1i64 << depth) - 1) as u16
                })
                .collect();
            let mut inter = Plane::new(4, 4, depth, 48).unwrap();
            inter
                .reconstruct_inter([0, 0, 4, 4], &[prediction; 16])
                .unwrap();
            inter.add_residual([0, 0], 2, &residual).unwrap();
            assert_eq!(inter.samples(), expected);
            let mut intra = Plane::new(4, 4, depth, 48).unwrap();
            let mut pred_scratch = Vec::new();
            intra
                .reconstruct_intra(
                    [0, 0],
                    2,
                    1,
                    false,
                    false,
                    &residual,
                    &mut pred_scratch,
                    |_, _| true,
                )
                .unwrap();
            assert_eq!(intra.samples(), expected);
        }
    }
    #[test]
    fn sao_uses_original_neighbours_and_commits_only_valid_output() {
        use super::super::hevc_sao::Sao;
        let mut p = Plane::new(4, 4, 8, 48).unwrap();
        p.samples = Arc::new([100, 101, 100, 99].repeat(4));
        Arc::make_mut(&mut p.ready).fill(true);
        p.apply_sao(
            3,
            &[Sao::Edge {
                class: 0,
                offsets: [7, 3, -3, -7],
            }],
        )
        .unwrap();
        assert_eq!(p.samples(), [100, 94, 100, 99].repeat(4));
        let saved = p.samples.clone();
        assert!(
            p.apply_sao(
                3,
                &[Sao::Band {
                    position: 32,
                    offsets: [0; 4]
                }]
            )
            .is_err()
        );
        assert_eq!(p.samples, saved);
        assert!(p.apply_sao(3, &[]).is_err());
    }
    #[test]
    fn published_blocks_feed_prediction_but_future_samples_do_not() {
        let mut p = Plane::new(8, 4, 8, 96).unwrap();
        let mut pred_scratch = Vec::new();
        p.reconstruct_intra(
            [0, 0],
            2,
            1,
            false,
            false,
            &[12; 16],
            &mut pred_scratch,
            |_, _| true,
        )
        .unwrap();
        p.reconstruct_intra(
            [4, 0],
            2,
            10,
            false,
            false,
            &[0; 16],
            &mut pred_scratch,
            |_, _| true,
        )
        .unwrap();
        assert!(p.complete());
        assert_eq!(p.samples(), [140; 32]);
        let saved = p.samples().to_vec();
        assert!(
            p.reconstruct_intra(
                [0, 0],
                2,
                1,
                false,
                false,
                &[0; 16],
                &mut pred_scratch,
                |_, _| true
            )
            .is_err()
        );
        assert_eq!(p.samples(), saved);
        assert!(Plane::new(8, 4, 8, 95).is_err());
    }
    #[test]
    fn unavailable_boundaries_and_extreme_residuals_are_safe() {
        let mut p = Plane::new(8, 4, 10, 96).unwrap();
        let mut pred_scratch = Vec::new();
        p.reconstruct_intra(
            [0, 0],
            2,
            1,
            true,
            false,
            &[i32::MAX; 16],
            &mut pred_scratch,
            |_, _| true,
        )
        .unwrap();
        p.reconstruct_intra(
            [4, 0],
            2,
            10,
            true,
            false,
            &[0; 16],
            &mut pred_scratch,
            |_, _| false,
        )
        .unwrap();
        for row in p.samples().chunks_exact(8) {
            assert_eq!(&row[..4], &[1023; 4]);
            assert_eq!(&row[4..], &[512; 4]);
        }
    }
}

#[cfg(test)]
mod slice_filter_tests {
    use std::sync::Arc;
    #[test]
    fn sao_edge_neighbours_obey_independent_slice_boundaries() {
        use super::{super::hevc_sao::Sao, Plane};
        let samples: Vec<u16> = (0..16)
            .flat_map(|y| vec![if y % 2 == 0 { 200 } else { 100 }; 16])
            .collect();
        let mut plane = Plane::new(16, 16, 8, 768).unwrap();
        plane.samples = Arc::new(samples.clone());
        Arc::make_mut(&mut plane.ready).fill(true);
        let sao = Sao::Edge {
            class: 1,
            offsets: [1, 2, -1, -2],
        };
        plane
            .apply_sao_with_boundaries(3, &[sao; 4], |a, b| a[1] / 8 == b[1] / 8)
            .unwrap();
        for y in 0..16 {
            for x in 0..16 {
                let neighbours = if y == 0 || y == 7 || y == 8 || y == 15 {
                    None
                } else {
                    Some([samples[(y - 1) * 16 + x], samples[(y + 1) * 16 + x]])
                };
                assert_eq!(
                    plane.samples[y * 16 + x],
                    sao.apply(samples[y * 16 + x], neighbours, 8).unwrap()
                );
            }
        }
    }
}

#[cfg(test)]
mod inter_dc_tests {
    use super::*;
    #[test]
    fn uniform_inter_dc_handles_64_and_16bit_cow_and_preflights_prediction() {
        let mut plane=Plane::new(64,64,16,1<<20).unwrap();
        plane.reconstruct_inter([0,0,64,64],&vec![65534;4096]).unwrap();
        let reference=plane.clone();
        plane.add_dc_residual([0,0],6,3).unwrap();
        assert!(plane.samples().iter().all(|&v|v==65535));
        assert!(reference.samples().iter().all(|&v|v==65534));
        plane.add_dc_residual([0,0],6,i64::MIN).unwrap();
        assert!(plane.samples().iter().all(|&v|v==0));
        plane.add_dc_residual([0,0],6,i64::MAX).unwrap();
        assert!(plane.samples().iter().all(|&v|v==65535));
        let mut incomplete=Plane::new(64,64,8,1<<20).unwrap();
        incomplete.reconstruct_inter([0,0,64,32],&vec![99;2048]).unwrap();
        let before=incomplete.clone();
        assert_eq!(incomplete.add_dc_residual([0,0],6,10).err().unwrap().to_string(),
            "HEVC inter DC-only block has no complete prediction");
        assert_eq!(incomplete.samples(),before.samples());
        assert_eq!(incomplete.ready,before.ready);
        assert!(incomplete.add_dc_residual([1,0],5,10).is_err());
        assert!(incomplete.add_dc_residual([0,0],7,10).is_err());
    }
}
