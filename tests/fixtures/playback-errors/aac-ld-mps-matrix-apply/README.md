# LD-MPS complex matrix application controls

Synthetic QMF component inputs, not spatial movie acceptance or player seek
coverage. No private media or parameter sets are copied. Saved exact integer
outputs cover seven parameter resolutions, 32/64/128 QMF bins and matrix
geometries 1x1, 3x2 and 8x13. Coefficients have denominator 32; sample values
have denominator 16. The generator sums integer complex products, so the
expected result divided by 512 is exactly representable in f64.

Stimulus numerators (t=slot, b=parameter band, p=plane, r=row, c=column,
q=QMF bin): coefficient `(17*t+13*b+11*p+7*r+3*c)%29-14`, real sample
`(7*t+5*c+3*q)%23-11`, imaginary sample `(11*t+3*c+7*q)%19-9`.
The map facts come from the separately frozen preparation corpus.

Generate explicitly with `python3 scripts/generate_aac_ld_mps_matrix_apply_fixtures.py`.
Ordinary `aac_ld_mps_matrix_apply` tests only load saved fixtures and need
neither FFmpeg nor network, regeneration or an external reference compiler.
128-bin matrix arithmetic does not qualify the missing 128-bin QMF DSP or
decorrelator. Whole spatial tree construction/routing and PCM remain open.
