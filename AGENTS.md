# Media regression policy

For every newly discovered media parsing, decoding, playback, timestamp or seek
failure, add a short synthetic video reproducer and an automated regression test.
Use `tests/fixtures/playback-errors` and extend its generator where appropriate.
Do not copy private source videos, frames, audio or codec parameter sets.

Verify that the fixture reproduces the specific failure, not an unrelated error.
For unsupported behavior, distinguish a passing refusal/reproduction test from
an acceptance test for the intended playback. Enable the acceptance test with
the fix and update the old refusal expectation. Keep fixture generation separate
from test execution; ordinary tests must not require FFmpeg or network access.
