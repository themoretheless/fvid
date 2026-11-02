# AIFF/AIFC integer precision — FVID-0515

Accepted batch526:one compatibility capability,192 precision/channel/byte-order
variants,not one ID per fixture. Accepted total515/1000.

The primary AIFF1.3 Common Chunk/Sample Points contract allows signed integer
precision1..32,stored in whole bytes per channel,left-justified with zero low
unused bits. AIFF-C NONE retains uncompressed PCM;the existing FVid sowt byte
order adapter applies to the same stored sample width. Primary sources:
https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-1.3.pdf
https://www.mmsp.ece.mcgill.ca/Documents/AudioFormats/AIFF/Docs/AIFF-C.9.26.91.pdf

Original0aef5cfaaa passes the24 whole-byte controls and neutral VP9 video but
refuses valid nonempty1-bit AIFF before either reader or actual worker can play.
The before suite passes2/fails4;baseline.log and baseline-source.json retain the
exact unsupported-width failures. New companion negative tests initially fail
their expected *specific* refusal reasons;they are not additional acceptance
failures or independent counted bugs. AIFF/AIFC cannot contain video:the short
authored VP9 movie is a separate control,not a claim that it reproduces audio.

The reader retains logical COMM precision,constructs the existing PCM decoder
with ceil(bits/8)*8 stored bits,and rounds stride per channel. Thus12-bit stereo
occupies4 coded bytes/frame,not3. Zero unused bits keep the mathematical scale
when widened by the existing decoder. Nonzero unused low bits are refused.
Stored-payload budgets and channel alignment run before this validation;
explicit SSND storage padding is already excluded from valid sample points.
The public PCM format enum/decoder support contract is unchanged. Generic
PcmDecoder::Int still describes whole-byte stored width,not AIFF logical bits.

The192 authored files span all32 precisions,mono/stereo and AIFF/NONE/sowt.
Independent integer mathematics supplies f32 oracles;no golden is decoded from
a source file. Exact PCM,logical metadata,stored stride,native clocks,2048-frame
packet boundary,raw/Duration seek,EOF/MAX and rewind pass. The actual worker
captures168 nonbyte variants. Dirty unused bits,invalid widths,partial frames,
truncated declared audio,full coded/file budgets and block padding are checked.
All266 files regenerate byte-identically using standalone pure Python. Ordinary
tests read checked-in files and need neither FFmpeg nor a network/private media.

Initial extended integration found an unrelated old native_pcm_trim expectation:
plan trim-pcm --progress was already accepted as a no-op on the original source.
The exact old failure is reproduced in original-native-pcm-trim.log/json;an
independent original-binary probe proves identical JSON with/without --progress
and preserves invalid-stream/sub-microsecond refusals. The corrected test now
requires full-plan equality,no stderr events,no source changes/no destination,
and those same invalid options with/without the flag. CLI production code is
unchanged;this FVID-0228 follow-up adds zero IDs. Initial qualification logs and
hashes are retained. The older12-bit empty-payload unit also retains refusal and
now names its actual missing-samples cause;nonempty acceptance is in this corpus.
The new resource assertion was strengthened to require the precise budget error.

Seven final gates pass146 Rust executions,zero ignored,and both core/player
checks. Three repeats pass14 tests each. All2693 hashes in candidate-source.json
remain stable;qualification-results.json stores exact commands/timings. No
performance,physical output-device,visual UI or universal interoperability claim.
