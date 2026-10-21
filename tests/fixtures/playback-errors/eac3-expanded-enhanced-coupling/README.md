# Expanded E-AC-3 enhanced coupling

Regenerate separately: `python3 scripts/generate_eac3_expanded_enhanced_coupling.py`.
Forty-eight owned six-block 48 kHz programmes combine a sounding conventional
base 5.1 and a dependent rear pair with enhanced coupling and AHT in its coupling
plane. Geometry starts 0/4/8, coordinate seeds 0/31, default/explicit band maps,
stationary/mixed coefficients and unity/changing final gain form the matrix.
Raw elementary, individual-syncframe Matroska and six-block MP4 are authored.
MP4 dec3 states base acmod=7/lfeon=1, one dependent and rear-pair chan_loc=2.

Controls expand the enhanced coordinates into explicitly coded uncoupled scalar
coefficients. The oracle assembles decoded controls by speaker identity and
applies common final-dependent gain/clamp. The analytical rounding bound comes
from the two scalar representations, not observed decoder errors. Tests require
active coupling throughout the frame, active coupling-plane AHT, base SNR strategy
1, and nonzero PCM in all six base channels. The existing enhanced-coupling writer
was extended only with an optional dependent envelope and per-block gain.

`tests/eac3_mp4_aht.rs` covers all 132 silent/sounding/ordinary/enhanced MP4
cases, plus raw and split-Matroska worker acceptance for the 96 coupled cases.
Checks include direct/reset/dispatch, PCM, masks, PTS/duration, priming, rewind
and partial-sample seeks (MP4 5/40/80 ms; raw/Matroska 40 ms into the second
programme). No production decoder change was needed for this family.
This is synthetic acceptance, not external-reference, GUI, physical-device or
performance qualification. Other rates/quantizers, enhanced phase transitions,
SPX and dynamic topology remain unqualified.

433 artifacts / 9,226,020 bytes excluding README reproduce exactly with process
and socket construction denied. Existing 1729 enhanced-coupling artifacts,
433 ordinary-expanded coupling artifacts and 291 presentation artifacts remain
byte-identical. No private media/codec parameters, external encoder, FFmpeg or
network. Generator execution stays separate from ordinary test execution.
