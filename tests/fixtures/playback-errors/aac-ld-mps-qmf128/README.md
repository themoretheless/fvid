#128-bin controlled QMF chain

Explicit research generation:
`python3 scripts/generate_aac_ld_mps_qmf128_reference.py /path/to/official.zip`.
Ordinary tests read committed data only. The generator compiles the pinned
unmodified ISO apply/decorrelate/merge routines in a temporary directory; the
probe supplies saved,previously qualified M1/M2 endpoints. HRTF remains unselected
and abort-guarded. No foreign implementation is linked into FVid.

112 sequences cover all8 ordinary trees,all7 parameter resolutions,both real
and complex decorrelator choices,three frames each,and nonzero filter tails.
17,891,328 scalar outputs compare the complete M1/decorrelation/W/M2 chain,
including residual-QMF routing with supplied synthetic residuals. This does not
qualify compressed residual AAC at128 bins.

The ordinary source preserves a one-slot input delay above row70 but bypasses
its all-pass and duck-gain application there. Those rows still participate in
energy sums. Tests probe rows70/71/127,so a64-bin crop or full-bank all-pass/duck
substitution cannot pass. All original state/cursor rollback,checkpoint/reset
and replay checks remain active.

`expected.bin.zlib` losslessly packs the original little-endian f32 output;
manifest `expected_bytes` bounds decoding. Its uncompressed SHA256 and all
source/probe hashes are in the reference report. FVid's existing zlib reader
adds no test dependency or generation/compiler requirement.

This is a controlled component oracle,not the original frontend's layout
selection or independent complete-codec PCM conformance. The prior documented
212 endpoint semantics remain unchanged; high-rate movie acceptance is in
`aac-eld-mps128`.
