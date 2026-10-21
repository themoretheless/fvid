# Expanded E-AC-3 with dependent coupling/AHT

Regenerate separately: `python3 scripts/generate_eac3_expanded_coupling.py`.
Forty-eight owned 48 kHz six-block cases combine a nonzero block-varying base
5.1 with a rear dependent stereo pair using ordinary coupling and active AHT.
Cases cross coupling starts 0/2, phase on/off, AHT masks 4/5/7, stationary/mixed
spectra and unity/changing final gain. The coupling plane itself always uses
AHT; the masks also exercise conventional/hybrid full-band planes. All three
presentation forms are authored: raw elementary bundles, Matroska individual
syncframe packets, and MP4 six-block samples with explicit dec3 metadata.

The oracle assembles separately decoded independent scalar controls by explicit
speaker identity, applies only final-dependent gain, then clamps. The rounding
bound is authored from per-bin inverse-transform rounding errors including the
coupling coordinate upper bound; it is not fitted to observed decoder errors.
Header probes assert coupling in every block and active coupling-plane AHT.
The base is verified nonzero in every one of its six channels.

`tests/eac3_mp4_aht.rs` accepts direct/reset/ordinary dispatch, MP4 PCM/mask/PTS/
duration and captured worker rewind/5,40,80 ms seek over all 84 silent, sounding
and coupled cases. Its dedicated expanded-coupling test accepts all 48 raw and
split-Matroska cases, checks the raw 256-sample priming, and captures worker
rewind/40 ms second-programme seek byte-exactly against sequential output.
This is synthetic acceptance, not an external-codec oracle, GUI, physical audio
device or performance qualification. No production decoder change was needed.

433 artifacts / 9,224,384 bytes excluding README regenerate exactly with
subprocess/socket creation denied. Existing 769 coupling, 291 presentation,
163 silent-base AHT and 163 sounding-base AHT artifacts also remain byte-exact.
No private media/parameter sets, external encoder, FFmpeg or network is needed;
ordinary tests do not generate fixtures.

Remaining: enhanced coupling in expanded streams, SPX combinations, arbitrary
base coding, other rates/quantizers, topology changes and other codec families.
