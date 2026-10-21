# Expanded E-AC-3 with active dependent AHT

Regenerate separately: `python3 scripts/generate_eac3_expanded_aht.py`.
Eighteen owned 48 kHz, six-block cases: stereo independent programme and three
stereo dependents mapped to C/LFE, rear pair and surround pair, yielding eight
semantic channels/mask 0x63f. AHT masks 1/2/3 exercise each or both dependent
coded planes, stationary/mixed spectra and unity/changing/boosting final gain.
All three dependents use the selected mask. The explicit dependent header probe
asserts active AHT; filenames alone are not treated as coverage.

Independent scalar controls contain the matching rounded inverse-transform
coefficients. The oracle independently maps their coded/native order into the
expected roles, scales each authored stream, applies only the final dependent's
gain, and clamps once. The manifest bounds scalar-rounding error analytically;
no observed-error-derived tolerance or external codec is used. Existing dependent
AHT decoder qualification is reused, not re-proven by an external reference.

`tests/eac3_expanded_aht.rs` accepts direct assembly/reset, ordinary decoder
routing, Matroska split-syncframe PCM/PTS/duration and captured worker layout,
rewind and partial seek. Raw elementary output is compared after its 256-sample
priming and reaches worker capture/rewind/40 ms seek into the second programme.
No physical audio device or GUI claim. Ordinary tests do not generate files,
invoke FFmpeg or access the network. All 253 artifacts (5,004,560 bytes excluding
README) reproduce exactly with subprocess/socket creation denied; default
presentation generation remains byte-exact too.

This is the specific six-block stereo-base/three-dependent assembly family.
It does not qualify expanded AHT with all base layouts, LFE coding, coupling,
SPX, quantizers, rates, MP4 packaging or changing stream topology.
