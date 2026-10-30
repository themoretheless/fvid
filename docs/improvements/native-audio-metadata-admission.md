# Native audio metadata and constant-format admission

`audio_source_info_selected`, its `_with_options` variant and its
`_with_interval_and_options` variant inspect the initial decoder setup. MP4
sample descriptions may change later. The returned rate, channels and codec
name describe the first actual packet's setup, including when sample-table
entry 1 is unused. Packet, memory, RSS, selection and cancellation checks remain
in force; PS inspection still follows the requested prefix/interval.

Code that used successful inspection to select a DSP path requiring a constant
MP4 decoder configuration must use `constant_audio_source_info_selected` or its
corresponding options/interval variant. Those functions preserve the previous
constant-description refusal and budget behavior. Repository routing callers,
including the CLI, use these explicit admission functions. General inspection
now accepts changing-description inputs supported by the normalized export path.

No signatures or result fields of the existing inspection functions changed.
Previously accepted constant-format sources retain their metadata. These are
initial-source metadata, not a promise about the output format after DSP.
