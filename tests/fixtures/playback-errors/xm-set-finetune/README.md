# Owned XM E5x set-finetune acceptance

Regenerate explicitly: `python3 scripts/generate_xm_set_finetune.py`.
All 70 artifacts regenerate byte-identically with PATH empty and denied
subprocess/socket creation; all 58 prior tone-portamento files remain identical.
Ordinary tests read saved fixtures only and require no FFmpeg, codec tool or
network. Samples and parameter bytes are authored, never copied from private
media. The independently decoded owned VP9/PCM companion contains no XM stream.

Sixty-four linear/Amiga scores supply literal periods, volume targets and sample
reset positions. The independent ramp renderer interprets no E5x commands,
finetune mapping, sample defaults, portamento targets or command memories.
Before implementation, nibble-0-linear differed at 11025 Hz mono with equal PCM
lengths; actual worker packet0 also differed (28224 bytes each). Four omitted-E5x
controls match independent default-period PCM and differ from intended tuning at
equal lengths.

Coverage includes all sixteen nibbles, no-note/instrument-only/key-off commands
not retuning, bare-note overrides, active finetune retained by arpeggio/tone and
column portamento/glissando, neutral override of sample tuning, default sample
finetune restored by a new trigger/E9, held rows executing setup once, speed1,
relative-note composition and independent channel overrides/targets. Column Fx
preparation retains the active finetune and does not apply E5x as a note trigger.

Reference semantics: triggerNote, preparePortamento and getNewNote in:
https://github.com/8bitbubsy/ft2-clone/blob/master/src/ft2_replayer.c
The active tuning belongs to the voice; portamento must not reload the sample's
original finetune. Values retain the native computed pitch/mixer profile;
absolute FT2 table/mixer parity is not established.

Every acceptance PCM byte matches at 11025/44100 Hz mono/stereo with no ignored
features. Actual worker tests compare independent PCM sequentially, after rewind,
second-row seek and EOF through a mock sink. Physical speaker acceptance, other
unqualified behavior and complete codec coverage remain open.
