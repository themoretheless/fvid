//! Per-source-window owned residual MDCT/QMF histories. Full-complex64-bin
//! default-phase domain only; this does not crop or substitute other banks.
use super::{
    Result,
    aac_ld_mps_config::{Config, ExtensionValue},
    aac_ld_mps_decor::Mode,
    aac_ld_mps_residual::Frame,
    aac_mps_mdct_qmf::Decoder,
    aac_sbr_qmf::Complex,
    invalid,
    memory::Footprint,
    unsupported,
};
#[derive(Clone, Debug, PartialEq)]
pub struct Residuals {
    rate: u32,
    slots: u8,
    pub extension: ExtensionValue,
    banks: Vec<Option<Decoder>>,
}
impl Residuals {
    pub fn new(config: &Config, mode: Mode) -> Result<Option<Self>> {
        let active=config.extensions.iter().filter(|e|matches!(&e.value,ExtensionValue::Residual{bands,..}if bands.iter().any(|b|b.unwrap_or(0)>0))).collect::<Vec<_>>();
        if active.is_empty() {
            return Ok(None);
        }
        if active.len() != 1 {
            return Err(unsupported(
                "multiple active residual AAC extensions are not qualified",
            ));
        }
        if mode != Mode::Complex || !(27713..55426).contains(&config.sample_rate) {
            return Err(unsupported(
                "native residual AAC requires the qualified full-complex64-bin QMF domain",
            ));
        }
        let extension = active[0].value.clone();
        let ExtensionValue::Residual {
            frames_code, bands, ..
        } = &extension
        else {
            unreachable!()
        };
        let banks = bands
            .iter()
            .map(|b| {
                if b.unwrap_or(0) > 0 {
                    Decoder::new(
                        usize::from(config.frame_slots),
                        usize::from(*frames_code) + 1,
                    )
                    .map(Some)
                } else {
                    Ok(None)
                }
            })
            .collect::<Result<_>>()?;
        Ok(Some(Self {
            rate: config.sample_rate,
            slots: config.frame_slots,
            extension,
            banks,
        }))
    }
    pub fn inherit(&mut self, old: &Self) {
        let clock = |e: &ExtensionValue| {
            if let ExtensionValue::Residual {
                rate, frames_code, ..
            } = e
            {
                (*rate, *frames_code)
            } else {
                unreachable!()
            }
        };
        if self.rate == old.rate
            && self.slots == old.slots
            && clock(&self.extension) == clock(&old.extension)
        {
            for (new, old) in self.banks.iter_mut().zip(&old.banks) {
                if new.is_some() && old.is_some() {
                    *new = old.clone();
                }
            }
        }
    }
    pub fn reset(&mut self) {
        for b in self.banks.iter_mut().flatten() {
            b.reset();
        }
    }
    pub fn process(&mut self, frame: &Frame) -> Result<Vec<Option<Vec<Vec<Complex>>>>> {
        if frame.boxes.len() != self.banks.len() {
            return Err(invalid("native residual AAC box count mismatch"));
        }
        let mut next = self.clone();
        let mut output = Vec::with_capacity(self.banks.len());
        for (bank, data) in next.banks.iter_mut().zip(&frame.boxes) {
            match (bank, data) {
                (Some(bank), Some(data)) => output.push(Some(bank.process(&data.blocks)?)),
                (None, None) => output.push(None),
                _ => return Err(invalid("native residual AAC active box mismatch")),
            }
        }
        *self = next;
        Ok(output)
    }
    pub(crate) fn visit_retained(&self, f: &mut Footprint) -> std::result::Result<(), String> {
        if let ExtensionValue::Residual { bands, .. } = &self.extension {
            f.vector(bands)?;
        }
        f.vector(&self.banks)?;
        for b in self.banks.iter().flatten() {
            b.visit_retained(f)?;
        }
        Ok(())
    }
}
