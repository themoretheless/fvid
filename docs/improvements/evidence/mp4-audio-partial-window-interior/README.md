# FVID-0448 extension: interior positive windows

Status: accepted extension of FVID-0448; all six gates terminal0; count448 stays unchanged.

Baselinef22e9bebe99a869f6f3212d5fb979b52e41cb221 fixes the terminal window but still emits480 instead of360frames when the middle ALAC packet has duration40 rather than160. Full/terminal controls and IMA4 pass. Baseline2pass2fail confirms the PCM/seek discrepancy and actual worker mismatch. Exact coded audio packet identity and synthetic video decoding pass independently. Original test body is preserved as before-test.rs.txt; after adds exact worker timestamps and clarifies the existing test name without removing any assertion.

The same source-window cause is corrected by binding each emitted decoded packet to the matching source sample endpoint. Reuse the existing cached ordered lookup from raw seek; do not introduce a linear scan per decoded packet. Negative timestamps use exact signed rational native-frame ceiling; recovered clocks keep their earlier path. Decoder validation still occurs before PCM clipping. Unsupported composition order remains refused by the existing check.

The corpus now contains6authored short VP9 videos and6independent scalar PCM oracles plus manifest. All eight original media/oracle files are preserved byte-identically. Middle IMA4 is already-correct control, not another improvement. Fixture generation is separate; ordinary tests use no encoder or network. All13files regenerate identically.

After:25passes across6focused suites, including packet seek, quantized clocks, zero windows, presentation endpoints, composition-order controls/refusals and actual worker with exact source timestamps. Full125-suite integration584pass,rootMP458pass,WebM67pass,media547pass,core/player builds all terminal0 from frozen sources;1252distinct passes after4duplicate events. No performance claim and no extra registry ID.
