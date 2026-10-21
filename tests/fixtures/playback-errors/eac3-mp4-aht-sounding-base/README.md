# Expanded AHT with a sounding base 5.1

Regenerate separately: `python3 scripts/generate_eac3_mp4_aht_sounding_base.py`.
Eighteen authored 48 kHz six-block cases combine a nonzero independent base 5.1
and an active rear dependent AHT pair. The base uses per-block shared SNR offsets
(strategy 1), alternating high allocation and true zero allocation with explicitly
disabled dither. This retains a sounding six-channel base within the maximum
4096-byte syncframe size. The dependent uses AHT masks 1/2/3, stationary/mixed
spectra, and unity/changing/boosting final gain. dec3 describes acmod=7/lfeon=1,
one dependent, chan_loc=2 and the complete-programme bitrate.

Scalar independent controls provide the assembly oracle; authored speaker roles,
coded/native permutations, per-stream amplitude, final-dependent gain and final
clamp are applied separately. The analytical scalar-rounding bound is inherited
from the AHT controls. Probes require real AHT, per-block base SNR and nonzero
base PCM, distinguishing these witnesses from the older silent-base family.

The shared `tests/eac3_mp4_aht.rs` suite accepts all 36 silent/sounding cases:
direct assembly/reset, normal decoder routing, MP4 channels/mask/PCM/PTS/duration,
and captured worker rewind plus seeks at 5/40/80 ms. No physical device, GUI,
throughput or all-tools coverage claim. The existing decoder needed no change.

163 artifacts / 3,459,599 bytes excluding README. No private media or parameter
sets, external codec, subprocess or network; generation is separate from tests.
Ordinary tests use existing artifacts. This qualifies the specific conventional
block-varying base plus dependent AHT family, not arbitrary sounding-base coding,
other rates, AHT quantizers, coupling/SPX or topology changes.
