//! STP energy-envelope history in normalized QMF energy units.
//!
//! This is the scalar stage of ISO reference `sac_TPprocess.c::subbandTP`:
//! refresh the old energies, normalize the current energies, damp, limit and
//! smooth the wet gains. Signal grouping, frequency weighting and mixing are
//! deliberately outside this component. It does not enable STP playback.
use super::{Result, invalid, memory::Footprint};

#[derive(Clone, Debug, PartialEq)]
struct Energy {
    running: f64,
    old: f64,
}
impl Default for Energy {
    fn default() -> Self {
        Self {
            running: 0.,
            old: 1.,
        }
    }
}
impl Energy {
    fn normalize(&mut self, value: f64) -> Result<f64> {
        self.running = 0.95 * self.running + 0.05 * value;
        let normalized = value / (self.old + 1e-9);
        if !self.running.is_finite() || !normalized.is_finite() {
            return Err(invalid("overflowing LD-MPS STP energy history"));
        }
        Ok(normalized)
    }
}

/// Each output refers to its dry downmix-energy group; None is an unshaped row.
/// Energy values must already include the STP frequency weights and low-band
/// dry/wet merge. Histories advance on every slot, including disabled signalling.
#[derive(Clone, Debug, PartialEq)]
pub struct EnergyTracker {
    groups: Vec<Option<usize>>,
    dry: Vec<Energy>,
    wet: Vec<Energy>,
    previous: Vec<f64>,
    slots_since_refresh: u8,
}
impl EnergyTracker {
    pub fn new(dry_groups: usize, groups: &[Option<usize>]) -> Result<Self> {
        if !(1..=2).contains(&dry_groups)
            || groups.is_empty()
            || groups.len() > 8
            || groups.iter().flatten().any(|&g| g >= dry_groups)
        {
            return Err(invalid("invalid LD-MPS STP energy grouping"));
        }
        Ok(Self {
            groups: groups.to_vec(),
            dry: vec![Energy::default(); dry_groups],
            wet: vec![Energy::default(); groups.len()],
            previous: vec![1.; groups.len()],
            slots_since_refresh: 0,
        })
    }
    pub fn reset(&mut self) {
        self.dry.fill(Energy::default());
        self.wet.fill(Energy::default());
        self.previous.fill(1.);
        self.slots_since_refresh = 0;
    }
    /// Consume one QMF slot atomically. This returns candidate gains even when
    /// the bitstream disables shaping; the caller applies signalling after this.
    pub fn step(&mut self, dry: &[f64], wet: &[f64]) -> Result<Vec<f64>> {
        if dry.len() != self.dry.len()
            || wet.len() != self.wet.len()
            || dry.iter().chain(wet).any(|v| !v.is_finite() || *v < 0.)
        {
            return Err(invalid("invalid LD-MPS STP slot energies"));
        }
        let mut next = self.clone();
        // Reference refresh precedes the current slot's running-energy update.
        // First refresh therefore occurs on slot 33, followed by slots 65, 97...
        if next.slots_since_refresh == 32 {
            for e in next.dry.iter_mut().chain(&mut next.wet) {
                e.old = e.running;
            }
            next.slots_since_refresh = 1;
        } else {
            next.slots_since_refresh += 1;
        }
        let dry = next
            .dry
            .iter_mut()
            .zip(dry)
            .map(|(state, &e)| state.normalize(e))
            .collect::<Result<Vec<_>>>()?;
        let mut gains = vec![1.; wet.len()];
        for (row, (&energy, group)) in wet.iter().zip(&next.groups).enumerate() {
            let Some(group) = group else { continue };
            let wet = next.wet[row].normalize(energy)?;
            let ratio = dry[*group] / (wet + 1e-9);
            if !ratio.is_finite() {
                return Err(invalid("overflowing LD-MPS STP energy ratio"));
            }
            let target = (0.1 + 0.9 * ratio.sqrt()).clamp(1. / 2.82, 2.82);
            gains[row] = 0.45 * target + 0.55 * next.previous[row];
            next.previous[row] = gains[row];
        }
        *self = next;
        Ok(gains)
    }
    pub fn retained_payload_bytes(&self) -> Result<usize> {
        let mut f = Footprint::new();
        f.vector(&self.groups).map_err(|e| invalid(&e))?;
        f.vector(&self.dry).map_err(|e| invalid(&e))?;
        f.vector(&self.wet).map_err(|e| invalid(&e))?;
        f.vector(&self.previous).map_err(|e| invalid(&e))?;
        Ok(f.total())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clamp_history_has_closed_form_and_is_owned_by_each_stream() {
        let mut decoder = EnergyTracker::new(2, &[Some(0), Some(1), None]).unwrap();
        let untouched = decoder.clone();
        for n in 1..=96 {
            let gains = decoder.step(&[0., 1.], &[1., 0., 10.]).unwrap();
            let decay = 0.55f64.powi(n);
            assert!((gains[0] - (1. / 2.82 + (1. - 1. / 2.82) * decay)).abs() < 1e-14);
            assert!((gains[1] - (2.82 + (1. - 2.82) * decay)).abs() < 1e-14);
            assert_eq!(gains[2], 1.);
        }
        assert_eq!(untouched.slots_since_refresh, 0);
        assert_eq!(untouched.previous, vec![1.; 3]);
        decoder.reset();
        assert_eq!(decoder, untouched);
    }
    #[test]
    fn refresh_uses_previous_32_slots_before_current_energy() {
        let mut decoder = EnergyTracker::new(1, &[Some(0)]).unwrap();
        for _ in 0..32 {
            decoder.step(&[4.], &[1.]).unwrap();
        }
        assert_eq!(decoder.dry[0].old, 1.);
        let before = decoder.clone();
        let gain = decoder.step(&[2.], &[3.]).unwrap()[0];
        let retained = 1. - 0.95f64.powi(32);
        assert!((decoder.dry[0].old - 4. * retained).abs() < 1e-14);
        assert!((decoder.wet[0].old - retained).abs() < 1e-14);
        let normalized_dry = 2. / (4. * retained + 1e-9);
        let normalized_wet = 3. / (retained + 1e-9);
        let expected = 0.45 * (0.1 + 0.9 * (normalized_dry / (normalized_wet + 1e-9)).sqrt())
            + 0.55 * before.previous[0];
        assert!((gain - expected).abs() < 1e-14);
        assert_eq!(decoder.slots_since_refresh, 1);
    }
    #[test]
    fn errors_at_refresh_preserve_all_history_and_replay() {
        let mut decoder = EnergyTracker::new(1, &[Some(0)]).unwrap();
        for _ in 0..32 {
            decoder.step(&[0.], &[0.]).unwrap();
        }
        let saved = decoder.clone();
        for (dry, wet) in [
            (f64::NAN, 1.),
            (1., -1.),
            (1., f64::INFINITY),
            (f64::MAX, 0.),
        ] {
            assert!(decoder.step(&[dry], &[wet]).is_err());
            assert_eq!(decoder, saved);
        }
        assert!(decoder.step(&[], &[1.]).is_err());
        assert_eq!(decoder, saved);
        let mut checkpoint = saved.clone();
        for _ in 0..70 {
            assert_eq!(
                decoder.step(&[0.25], &[0.5]).unwrap(),
                checkpoint.step(&[0.25], &[0.5]).unwrap()
            );
        }
        assert_eq!(decoder, checkpoint);
        assert!(EnergyTracker::new(0, &[Some(0)]).is_err());
        assert!(EnergyTracker::new(1, &[Some(1)]).is_err());
    }
}
