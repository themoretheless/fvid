# MP4 audio metadata admission

Both files contain the same owned 136 signed PCM samples (8 kHz mono, 17 ms). The padded variant adds a legal 2 MiB `moov/free` element without changing media or offsets. The generator imports only owned byte-authoring helpers and requires no FFmpeg, network or private media.

Before the fix, audio geometry and decode planning accepted the padded file under a 1 MiB controlled-memory ceiling; small input at 1 MiB and padded input at 8 MiB passed. The parser's wire metadata allocation itself exceeded the requested ceiling.

Refusal checks cover geometry, planning, native probes and root/library decode without temporary outputs. Acceptance decodes small and padded files with the same ample budget, verifies 136 mono sample frames at 8 kHz, and compares the complete WAVE format and PCM data chunks byte-for-byte. This is audio route completion of FVID-0332, not another improvement ID. The earlier video fixture remains the video/parser witness. Parser metadata ceilings are not a claim of complete aggregate RSS accounting.
