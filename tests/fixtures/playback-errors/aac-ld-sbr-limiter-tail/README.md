# Discarded HF tail and limiter borders

Eight own eight-AU programmes cover480/512 core frames and all four limiter
densities. Each has a synthetic MP4 video and an audio-only Matroska control.
The one retained patch maps source2..10 to target10..18;the two-band tail18..20
is discarded by HF generation. Its end18 can coincide with a low-resolution
border but is not a boundary between retained patches.

The oracle uses full direct core cosine/window convolution,independent CLDFB
column superposition,Decimal limiter spacing and per-interval gain/noise/boost
equations. Complete synthesis endpoints10 and20 remain covered. Expected borders
are [10,20],[10,14,20],[10,12,16,20],[10,12,14,16,20] for modes0–3.

The old decoder accepts every video. Mode0 controls match expected PCM;modes1–3
reproduce the specific numeric failure. The corrected decoder matches frozen
PCM in both transports. Tests never generate media or require FFmpeg/network.
Generate explicitly with `python3 scripts/generate_aac_ld_sbr_limiter_tail_fixtures.py`.
No private/ISO media or codec parameter sets are copied.
