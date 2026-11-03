# Independent WAVE block seek

Original e259ba70f2 decodes IMA/MS ADPCM from the beginning even after seeking into a later independent block. A corrupt earlier block therefore prevents an otherwise valid suffix from playing; clean seeks also decode unrelated history. The final baseline stand passes three healthy/refusal/resource controls and fails direct seek, actual-worker seek and bounded-preroll tests. Its accumulated direct failure report names both codec families.

Primary specification: Microsoft/IBM Multimedia Data Standards Update, revision3.0, Microsoft and DVI ADPCM block sections, https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/WAVE/Docs/RIFFNEW.pdf . Each of these blocks reconstructs from its own header. The existing owned decoders reset their predictor state at every block. Other predictive WAVE codings retain their full-history route.

The reader now selects the natural packet window and complete coded block containing the native seek target. The first encoded timestamp includes block_skip * frames_per_block, while the existing presentation floor removes only the residual frames inside that block. Subsequent packet windows, logical fact tails and decoder algorithms remain unchanged. PCM/G711 keeps its existing direct seek route. No public signature changes or new migration are required.

Seventy-three short authored IMA/MS cases span mono/stereo, small and >2048-frame blocks, standard/extended/maximal coefficient tables, full/short fact tails, and RIFF/RF64/Wave64. Literal PCM and neutral video controls remain valid. Full/selected corrupt blocks still produce specific Invalid decoder errors; only unrelated unselected payload blocks are skipped. This is random-access playback, not a whole-file payload validation claim. Container metadata/EOF and input budgets remain strict. Tests check exact native suffixes/clocks, warm raw seeks, EOF/MAX, rewind, less than one block of preceding decoded frames, and146 actual playback-worker sessions.

## Comparative measurements

The separate explicit generator creates temporary120-second files from repeated authored blocks and analytic first/last PCM. Ordinary tests only load checked-in short fixtures. Input generation, file opening and initial payload copying are outside the measured interval; seek, decoder reset and decode until the first presented PCM are inside. The example asserts PCM, PTS and timebase in every run.

The same benchmark source is compiled with original/candidate Wav readers. ABBA campaigns use9 timed rounds per workload and one excluded warmup per campaign/workload; pooled medians have18 samples per variant/workload. All288 timed executions verify their PCM. This is the dev/debug profile on the recorded macOS ARM64 environment, not release, physical device, cold-file I/O or whole-player latency qualification. Source/binary hashes and every input hash are retained. Temporary media and executables can be regenerated independently.

| Late-seek workload | Before median ms | After median us | Decoded frames before / after |
|---|---:|---:|---:|

| ima-c1-late | 96.840 | 4.229 | 960003 / 36 |

| ima-c2-late | 167.211 | 6.792 | 960003 / 36 |

| ms-c1-late | 170.296 | 4.417 | 960000 / 28 |

| ms-c2-late | 252.785 | 6.354 | 960000 / 28 |


The frozen start-of-file controls use the same decoded work. Their observed pooled median ratios range from0.974 to1.027, within the recorded1.25 limit. Late-seek gates also require at least2x median improvement, at most64 decoded frames and at least10x less decoded work; all pass. benchmark-comparison.json and benchmark-protocol.json are authoritative, including the exact scope and acceptance gates. The earlier commentary range was corrected from the saved pooled measurements.


A benchmark authoring error attempted to use a crate-private duration helper; the example now computes the equivalent exact integer duration locally. Its initial compile log is preserved, contributes no accepted ID and is excluded from passing gates. No tests or codec refusals were relaxed. Correctness and reduced preroll are one independently verified improvement, not separate IDs.
