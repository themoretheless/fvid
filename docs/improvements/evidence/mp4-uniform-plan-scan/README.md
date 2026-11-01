# Compact MP4 description queries (accepted FVID-0482)

Accepted after final performance criteria and all seven correctness/build gates pass. Registry482.

A5.46second mono48kHz PCM source contains262144 samples in one UniformIndex
run. Controlled MP4 admission previously called description_for_sample for every
virtual sample to decide whether its selected decoder header changes. Default
unbounded planning was already fast and is not claimed defective or improved.

A crate-private Track helper compares selected headers over sparse compact runs and
checks their first/last addressable samples. Expanded indexes and dense prefixes retain the old
per-sample reference query. Equality compares complete headers,not entry numbers,
so duplicate identical entries remain equivalent. Gaps/malformed runs retain
reference behavior,and endpoint checks retain offset/signed timestamp failures.
The controlled-memory admission and normalization-target query use this helper.
No public API,decoder,input cap or resource admission rule is removed.

The first fast candidate omitted coordinate validity while resolving run headers.
A new private virtual-index regression reproduced the mismatch against the
reference per-sample predicate. The corrected version keeps unreadable samples
visible,including a near-u64 offset and a2.1billion-sample virtual signed-clock
boundary,without a large allocation. This candidate correction and its test
add0IDs;it is not a newly claimed external media parsing defect.

The unchanged public three-test baseline passes. Controls verify compact index
shape,complete and prefix PCM against an analytic0.125 waveform,duplicate/unused
headers,reached little-to-big-endian PCM changes,exact prefix boundary,continued
memory refusal for changed headers and packet-byte refusal. Native neutral video
pictures,seek,rewind and EOF are exercised. Six authored files regenerate offline,
without FFmpeg,network or private media/codec parameters. Ordinary tests never
run the generator or performance harness.

Before implementation,criteria.json required seven alternating old/new pairs,
three warmups/fifteen calls per run,at least20percent median reduction,six wins,
and unchanged default-mode controls. Before paired measurements,the maximum
default-mode median regression was made explicit as20percent. Each binary's complete
serialized MediaPlan must match its counterpart. Cargo release uses O3,thin LTO,
one codegen unit,debug0 and jobs2,on identical local input and source/harness.

Set1 passes:bounded64MiB large-source median1.880083ms ->0.098084ms,94.78percent,
7/7 wins. Default large-source median0.098500ms ->0.096500ms. The64-frame bounded
control was0.098833ms ->0.116625ms,an18percent slowdown. That uncertainty caused
an identical independent repeat without production or harness changes;set1 is
retained intact,not discarded.

Set2 also passes:bounded large-source median1.789875ms ->0.085084ms,95.25percent,
7/7 wins. Default0.089459ms ->0.089208ms;bounded small0.091250ms ->0.090625ms.
The small-file slowdown did not reproduce. No small/default-mode speedup or
whole-decoder/player speedup is claimed. All42 paired plan comparisons across
both sets and three configurations are identical. Raw samples,binary hashes,
build logs,source hashes and the declared criteria are preserved.

The initial default-only profile had no useful optimization target. Its logs
and source hashes remain historical;controlled-mode measurements isolate the
actual per-frame scan. Baseline/candidate binaries exist only in the owned
temporary build target and must be deleted after final qualification. Fixtures,
source and proof remain. No physical audio/GPU/native-window duration claim.

The initial default-configuration command accidentally omitted --test flags
for appended targets. Cargo rejected it before test execution. The command was
repaired without production/test changes;the three completed gates were retained
from identical frozen sources,and the failed command plus remaining gates rerun.
This stand repair adds0IDs and its original failure log/results remain archived.

After pre-density correctness qualification passed,a cost guard was added:
use two endpoint lookups per run only when twice the total run count is smaller
than the reached prefix. Dense indexes retain the reference walk. Prior completed
gates are archived under pre-density-qualification;the final binary must meet
the original performance criteria and all correctness gates again. Sets1/2
are historical confirmation,not final-binary acceptance after that guard.

Final set3,using the density-guarded binary,passes the original criteria:
bounded large1.845458ms ->0.094917ms,94.86percent reduction,7/7 winning pairs.
Default0.095625ms ->0.094042ms;bounded small0.095375ms ->0.094000ms.
All21 final paired plan comparisons match. Only this set is used for final
performance acceptance;sets1/2 remain historical confirmation. No default/small
input or whole-decoder/player speedup claim is added.

Final correctness qualification:221 integration passes,83 MP4 units,67 WebM
units,32 core reexecutions,548 media units,and core/player builds all terminal0.
The same two prior ignored tests remain unchanged. No combined distinct-test
count is claimed.
