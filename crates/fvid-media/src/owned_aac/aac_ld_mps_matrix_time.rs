//! Owned LD-MPS matrix smoothing and time-slot interpolation.
//! Coefficients come from the spatial matrix builder, not from PCM samples.
use super::{Result, aac_ld_mps_prepare::SmoothControl, invalid};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub planes: u8,
    pub rows: u8,
    pub columns: u8,
}
impl Layout {
    fn width(self) -> Result<usize> {
        // Seven real/imaginary matrix planes; dimensions of the full reference
        // M0/M1/M2 workspace (up to 8 outputs and 2*6+1 inputs).
        if !(1..=7).contains(&self.planes)
            || !(1..=8).contains(&self.rows)
            || !(1..=13).contains(&self.columns)
        {
            return Err(invalid("invalid LD-MPS matrix layout"));
        }
        Ok(usize::from(self.planes) * usize::from(self.rows) * usize::from(self.columns))
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixFrame {
    /// Smoothed matrix endpoints, including any held final parameter set.
    pub endpoints: Vec<Vec<f64>>,
    /// One complete matrix per time slot, flattened [band][plane][row][column].
    pub slots: Vec<Vec<f64>>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixTime {
    bands: u8,
    layout: Layout,
    previous: Vec<f64>,
}
impl MatrixTime {
    pub fn new(bands: u8, layout: Layout) -> Result<Self> {
        if !(1..=23).contains(&bands) {
            return Err(invalid("invalid LD-MPS matrix band count"));
        }
        Ok(Self {
            bands,
            layout,
            previous: vec![0.0; 23 * layout.width()?],
        })
    }
    /// Ordinary LD-MPS retains indexed matrix history in the 23-band
    /// workspace, including bands temporarily outside the current resolution.
    /// No frequency averaging or history reset is implied by an SSC change.
    pub fn reconfigure_bands(&mut self, bands: u8) -> Result<()> {
        if !(1..=23).contains(&bands) {
            return Err(invalid("invalid LD-MPS matrix band count"));
        }
        self.bands = bands;
        Ok(())
    }
    pub fn reset(&mut self) {
        self.previous.fill(0.0);
    }
    /// All encoded endpoints must cover the frame's final slot. Preparation
    /// supplies a held endpoint when needed. Residual bands bypass smoothing.
    /// Errors, including nonfinite later coefficients/results, retain history.
    pub fn process(
        &mut self,
        frame_slots: u8,
        slots: &[u8],
        controls: &[SmoothControl],
        raw: &[Vec<f64>],
        residual_cutoff: u8,
    ) -> Result<MatrixFrame> {
        let width = self.layout.width()?;
        let bands = usize::from(self.bands);
        let size = bands * width;
        if !(1..=32).contains(&frame_slots)
            || !(1..=3).contains(&slots.len())
            || slots.last().copied() != Some(frame_slots - 1)
            || slots.iter().any(|&x| x >= frame_slots)
            || slots.windows(2).any(|x| x[0] >= x[1])
            || raw.len() != slots.len()
            || controls.len() != slots.len()
            || residual_cutoff > self.bands
        {
            return Err(invalid("invalid LD-MPS matrix frame geometry"));
        }
        if raw
            .iter()
            .any(|r| r.len() != size || r.iter().any(|x| !x.is_finite()))
            || controls.iter().any(|c| {
                c.mask.len() != bands
                    || ![0, 64, 128, 256, 512].contains(&c.time)
                    || c.time == 0 && c.mask.iter().any(|&x| x)
            })
        {
            return Err(invalid(
                "invalid LD-MPS matrix coefficients or smoothing control",
            ));
        }
        let mut endpoints = Vec::<Vec<f64>>::with_capacity(slots.len());
        let mut output = Vec::with_capacity(usize::from(frame_slots));
        let mut last_slot = -1i16;
        for (ps, &slot) in slots.iter().enumerate() {
            let previous = endpoints
                .last()
                .map_or(&self.previous[..size], Vec::as_slice);
            let delta =
                f64::from(i16::from(slot) - last_slot) / f64::from(controls[ps].time.max(1));
            let mut endpoint = raw[ps].clone();
            for (i, value) in endpoint.iter_mut().enumerate() {
                let band = i / width;
                if band >= usize::from(residual_cutoff) && controls[ps].mask[band] {
                    *value = delta * (*value) + (1.0 - delta) * previous[i];
                }
            }
            for ts in last_slot + 1..=i16::from(slot) {
                let alpha = f64::from(ts - last_slot) / f64::from(i16::from(slot) - last_slot);
                let row = previous
                    .iter()
                    .zip(&endpoint)
                    .map(|(&a, &b)| (1.0 - alpha) * a + alpha * b)
                    .collect::<Vec<_>>();
                if row.iter().any(|x| !x.is_finite()) {
                    return Err(invalid("overflowing LD-MPS matrix interpolation"));
                }
                output.push(row);
            }
            if endpoint.iter().any(|x| !x.is_finite()) {
                return Err(invalid("overflowing LD-MPS matrix smoothing"));
            }
            endpoints.push(endpoint);
            last_slot = i16::from(slot);
        }
        self.previous[..size].copy_from_slice(endpoints.last().unwrap());
        Ok(MatrixFrame {
            endpoints,
            slots: output,
        })
    }
}

impl MatrixTime {
    pub(crate) fn visit_retained(
        &self,
        f: &mut super::memory::Footprint,
    ) -> std::result::Result<(), String> {
        f.vector(&self.previous)?;
        Ok(())
    }
}

#[cfg(test)]
mod grid_history_tests {
    use super::*;
    #[test]
    fn dormant_rows_are_owned_counted_and_detectably_different_from_resize_with_zero_fill() {
        let layout = Layout {
            planes: 1,
            rows: 1,
            columns: 1,
        };
        let mut good = MatrixTime::new(23, layout).unwrap();
        let control = |bands| SmoothControl {
            time: 0,
            mask: vec![false; bands],
        };
        good.process(
            8,
            &[7],
            &[control(23)],
            &[(0..23).map(|i| i as f64 / 8. + 1.).collect()],
            0,
        )
        .unwrap();
        good.reconfigure_bands(4).unwrap();
        good.process(8, &[7], &[control(4)], &[vec![-1.; 4]], 0)
            .unwrap();
        let mut footprint = super::super::memory::Footprint::new();
        good.visit_retained(&mut footprint).unwrap();
        assert_eq!(footprint.total(), 23 * 8);
        let mut wrong = good.clone();
        wrong.previous[4..].fill(0.); // A resize-to-active-length loses these rows.
        good.reconfigure_bands(23).unwrap();
        wrong.reconfigure_bands(23).unwrap();
        let raw = vec![0.; 23];
        let a = good
            .process(8, &[7], &[control(23)], &[raw.clone()], 0)
            .unwrap();
        let b = wrong.process(8, &[7], &[control(23)], &[raw], 0).unwrap();
        assert_eq!(&a.slots[0][..4], &b.slots[0][..4]);
        assert_ne!(&a.slots[0][4..], &b.slots[0][4..]);
        assert_eq!(a.slots[0][22], (22. / 8. + 1.) * 7. / 8.);
        good.reset();
        assert!(good.previous.iter().all(|v| *v == 0.));
    }
}
