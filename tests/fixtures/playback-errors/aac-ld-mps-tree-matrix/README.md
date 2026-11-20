# OTT-only LD-MPS tree matrix controls

1184 owned synthetic parameter combinations cover trees 0/1/5/6/7, varied
CLD/ICC extremes, every non-LFE residual mask and both LFE cutoff regions.
No private media or codec parameter sets are copied. The saved binary contains
original unmodified ISO C M1, M2-residual and M2-decor matrices for each case.

Explicit research generation (external frozen archive, C compiler required):

```sh
python3 scripts/generate_aac_ld_mps_tree_matrix_fixtures.py /path/to/saoc-reference.zip
```

The archive hash and source/driver hashes are in the reference report. Ordinary
`aac_ld_mps_tree_matrix` tests load these saved matrices without regeneration,
network, FFmpeg or external reference software. Production uses an owned
symbolic graph of OTT nodes, with separate CLD energy paths for M1 and LFE.

The original 212 routine writes H12/H22 into the residual matrix's auxiliary
column instead of H12_res/H22_res. Its raw outputs are preserved. Tests identify
these 64 fields separately: nonresidual W-dry auxiliary inputs are zero anyway;
in residual bands the owned signed-unit injection is checked against the
independently qualified covariance kernel and an analytic [X/2+R,X/2-R] case.
No reference outputs are patched. This identifies a reference discrepancy, not
an accepted spatial movie or a production player regression.

These are matrix component checks. TTT trees 2/3/4, arbitrary downmix/HRTF,
LFE residual routing, residual AAC decoding, temporal-shape processing, full
spatial PCM routing and actual playback/seek/rewind remain unqualified. A
passing matrix test is not movie acceptance; native renderer refusal remains.
