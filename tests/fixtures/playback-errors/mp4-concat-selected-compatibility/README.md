# Selected MP4 concat compatibility

Owned 32x32 PCM-coded AVC pictures are identical across all three files. Two files add two AAC-LC pulse packets, correctly configured for 48 kHz or 44.1 kHz. The third has video only. `mvhd.next_track_id` is unused, all original video offsets remain unchanged, and audio has a separately indexed mdat. No private media or parameters, external encoder or network are used.

The original regression admits pairs at either matching audio clock and correctly refuses selected incompatible audio, but rejects identical selected video when only discarded audio differs. The fix compares selected track configurations and only transports selected priming, allowing an absent discarded track safely in either input order. Selecting all tracks still requires a matching selected set and compatible configurations. Parser eligibility rules remain in force.

Player acceptance compares exact RGB with the original picture, checks two frames and 80 ms duration, repeated EOF, rewind and six seek positions in both input orders for both discarded-track variants. Fixture generation is separate from ordinary offline tests.
