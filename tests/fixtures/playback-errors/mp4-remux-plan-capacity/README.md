# Edited AAC remux timing capacity

`generate_mp4_remux_plan_capacity.py` creates an owned 48 kHz mono AAC pulse track with 512 packets and one contiguous edit retaining exactly 2048 samples (two packets). Generation needs neither FFmpeg nor a network; ordinary tests only read the checked-in fixture.

The original remux budget admitted 20 KiB by counting two initialized timing entries even though storage for 512 remained reserved. Refusal and successful playback are separate tests: a tight budget must reject before publishing, while a generous budget must copy two packets and decode exactly 2048 mono PCM frames at 48 kHz. This is an additional witness for FVID-0324, not another improvement ID.
