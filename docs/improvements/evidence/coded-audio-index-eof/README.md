# Coded/tracker indexed EOF variants

These are FVID-0516 variants,not additional IDs. See ../audio-index-eof/README.md
and batch-527-validation.json for final accepted scope and qualification.

baseline.log passes valid decoded tone/worker and specific refusal controls,
then fails the6 indexed-EOF checks. All5 relevant coded accessor files were
byte-identical to original f87769a3c2 during this before run. A checked PCM
candidate was present but does not provide the coded packet accessors.

The initial nonseekable FLAC producer caused an unrelated STREAMINFO refusal.
initial-baseline.log retains it. The same pipe bytes are preserved as a named
refusal fixture;the finalized seekable file reaches the actual EOF failure.
No frame/CRC/STREAMINFO check was relaxed. Regeneration reproduces all11 files,
including that separate refusal. FFmpeg is only a generation tool. Ordinary
regressions read checked-in files and also pass with FFmpeg absent from PATH.

XM has one empty row and a valid release tail;both must remain available.
The silence MP1 file is authored allocation-zero LayerI,not copied media.
The other tone files are generated from a public997Hz sine expression.
