/// Explicit encoder selection without a dependency on any codec backend.
/// Geometry and quality options remain separate; no pixel conversion is implied.
#[derive(Clone, Debug)]
pub struct EncoderSettings {
    pub name: String,
    pub options: Vec<(String, String)>,
}

impl EncoderSettings {
    /// Validate transport limits before a backend allocates an option dictionary.
    pub fn validate(&self) -> Result<(), String> {
        if self.options.len() > 64 {
            return Err("too many encoder options".into());
        }
        if self.name.contains('\0') {
            return Err("embedded NUL".into());
        }
        for (key, value) in &self.options {
            if key.is_empty() || key.len() > 256 || value.len() > 8192 {
                return Err("invalid encoder option size".into());
            }
            if key.contains('\0') || value.contains('\0') {
                return Err("embedded NUL".into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_order_duplicates_and_boundary_sizes() {
        let settings = EncoderSettings {
            name: "ffv1".into(),
            options: vec![("x".repeat(256), "y".repeat(8192)); 64],
        };
        settings.validate().unwrap();
        assert_eq!(settings.options.len(), 64);
        for bad in [
            EncoderSettings {
                options: vec![("x".into(), "".into()); 65],
                ..settings.clone()
            },
            EncoderSettings {
                options: vec![("".into(), "".into())],
                ..settings.clone()
            },
            EncoderSettings {
                options: vec![("x".repeat(257), "".into())],
                ..settings.clone()
            },
            EncoderSettings {
                options: vec![("x".into(), "y".repeat(8193))],
                ..settings.clone()
            },
            EncoderSettings {
                name: "a\0b".into(),
                ..settings.clone()
            },
            EncoderSettings {
                options: vec![("a\0b".into(), "".into())],
                ..settings.clone()
            },
            EncoderSettings {
                options: vec![("a".into(), "b\0c".into())],
                ..settings.clone()
            },
        ] {
            assert!(bad.validate().is_err());
        }
    }
}
