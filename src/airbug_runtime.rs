//! Optional airbug-err + airbug-otel wiring (`--features airbug`).
//!
//! Crates come from `themoretheless/airbug` git branch `release`.
#![cfg(feature = "airbug")]

use airbug_err::Options as ErrOptions;
use airbug_otel::TelemetryConfig;

/// Process-scoped guards; keep until process exit so panic hook + OTLP flush work.
pub struct Runtime {
    _err: airbug_err::Guard,
    _otel: Option<airbug_otel::TelemetryGuard>,
}

/// Install error/panic reporting and (best-effort) OTLP telemetry.
///
/// - `AIRBUG_ERR_ENDPOINT` — hub errors URL (default `http://127.0.0.1:8790/api/v1/errors`)
/// - `OTEL_SERVICE_NAME` / `OTEL_EXPORTER_OTLP_ENDPOINT` — honored by airbug-otel
/// - Set `FVID_AIRBUG_OTEL=0` to skip OTLP init when no collector is running
pub fn install() -> Result<Runtime, String> {
    let err = airbug_err::init(
        ErrOptions::new()
            .release(env!("CARGO_PKG_VERSION"))
            .environment(std::env::var("FVID_ENV").unwrap_or_else(|_| "dev".into()))
            .service("fvid"),
    )
    .map_err(|e| format!("airbug-err init: {e}"))?;

    let otel = if std::env::var("FVID_AIRBUG_OTEL").ok().as_deref() == Some("0") {
        None
    } else {
        match airbug_otel::init(TelemetryConfig::new().service_name("fvid")) {
            Ok(guard) => Some(guard),
            Err(e) => {
                eprintln!("fvid: airbug-otel unavailable ({e}); continuing without OTLP");
                None
            }
        }
    };

    Ok(Runtime {
        _err: err,
        _otel: otel,
    })
}

/// Forward a surfaced CLI/library error to the hub (no-op if init failed earlier).
pub fn capture_error(err: &dyn std::error::Error) {
    let _ = airbug_err::capture_error(err);
}
