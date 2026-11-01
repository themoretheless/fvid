# FVID-0484 split-edit follow-up (zero additional IDs)

The published 9feaadc690 source fixed single finite AAC edit wall-time seeks. The same pre-conversion quantization remains when scheduling multiple exact AAC edits. Four newly authored AAC960/neutral VP9 videos cover two 1ms source spans, 48kHz and 50Hz media clocks, and optional 1ms leading silence. Their complete PCM is compared with existing owned independent stored AAC reference slices and literal silence, before testing seek behavior.

`before-final.log` uses the final identical test and fixtures against published source: one full-PCM/video control passes; the direct and actual-worker acceptance tests fail. Extra samples and entire-clip replay arise before the exact edit scheduler receives the requested time. Initial two-case and intermediate logs remain historical evidence.

The fix routes constant-header AAC edit scheduling through exact Duration navigation, reuses packet-relative native sample alignment, and aligns explicitly silent edit spans to their own native lattice. Raw tick navigation remains unchanged and is checked after an EOF wall-time seek. Real configuration changes retain their existing path; recovered-clock logic remains separate. No performance or physical audio-device claim is made.

An initial compile failure while extracting an inherent helper (trait methods unavailable under the inherent generic bound) was repaired using the existing negotiated-rate API and equivalent exact packet-duration arithmetic. It is not a new improvement. All seven final qualification gates pass; three repeated ten-test runs also pass. The accepted ledger count remains 484, next FVID-0485; this is the same loss-of-fraction cause as FVID-0484, not another problem for counting.

Accepted under batch-481-validation.json with zero new IDs. The owned temporary build target was deleted after terminal process verification.
