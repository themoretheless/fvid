The remaining signed-padding baseline needs a contract audit, not an assertion change.

`tests/owned_adts_remux.rs::synthetic_leading_aac_padding_does_not_return_as_silence`
expects compact 1664-frame PCM from two coded AAC packets with 128 head and 256
tail padding frames. Current container-timeline decoding returns 1792 frames.

`tests/matroska_padding_rounding.rs::rounded_padding_metadata_removes_the_known_sample_count_without_losing_a_neighbor`
explicitly distinguishes interval rendering from packet presentation: its owned
container-time oracle includes leading zero samples; its packet oracle contains
only audible samples at their original presentation timestamp. This applies to
PCM and Opus, with exact sample clocks also used by AAC in the shared timeline.

Simply removing the `!exact_clock` condition on leading-padding accumulation
would change the shared PCM/Opus contract. No such edit or test relaxation was
made. The old failure is retained for further API/timeline compatibility work.

Reference definition checked: https://www.matroska.org/technical/elements.html#DiscardPadding
It defines signed head/tail padding; it does not by itself settle this legacy
API-versus-container-rendering contract difference.
