# In-band layered HEVC automatic dispatch

Generate separately with `python3 scripts/generate_hevc_inband_layered_dispatch.py`.
No FFmpeg, external codec, network or private source media is used. The author
repackages saved original synthetic HEVC entropy/parameters; it does not rewrite
coded samples or synthesize a refusal by corrupting the bitstream.

25 short `hev1` movies combine independent/dependent layers, dependent slice
segments, separate colour planes and inter prediction with five entry layouts:
base-only hvcC followed by inline layered parameter updates; those updates in a
separate metadata sample; empty hvcC with either layout; and multiple recorded
VPS where the first, single-layer VPS is not the one activated by the first
picture. The active chain is PPS -> SPS -> VPS. `hev1` permits parameter sets in
the sample entry or samples; see the MP4 Registration Authority definition:
https://mp4ra.org/registered-types/codecs

Before the fix, automatic playback selected the base decoder from the recorded
first VPS, then failed with `multilayer or external-base HEVC VPS is not
implemented` at the first real picture. Explicit layered-source controls already
accepted all nonempty-config streams and matched the original YUV samples,
excluding unrelated header/entropy errors. The enabled acceptance now checks
all 25 automatic inputs, highest displayed layer identity, complete packed
pixels, source sample/PTS preservation, exact durations, EOF, rewind and seek.
Metadata-only prefix samples retain their real timeline offset. A counted-read
control ensures ordinary complete `hvc1` does not add coded-packet reads on open.

The source may bootstrap its initial VPS from the first activated in-band
binding; decoder selection is bounded and stops at the first coded picture.
This covers initial dispatch/bootstrap, not arbitrary later changes between
single-layer and multilayer output-set policies, extractor tracks or every
HEVC feature/profile. Ordinary tests read only saved fixtures and own oracles.
