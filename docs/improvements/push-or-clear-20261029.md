# Push-or-clear 2026-10-29

Publication stopped before push because fresh owned-media qualification failed:
540 passed, 5 failed, 1 ignored. The core and player gates were not started.
No assertions were weakened to make qualification pass.

Preserved primary snapshot: `35739ac2369320d5e4c9e6114dac4d8d12e70831`.
Preserved active AAC snapshot: `8ba7c70e95e8c8bf4a1fb061410dc4469dcf45a9`.
The original working trees and staging areas were not reset or switched.

The candidate excludes two unfixed, unaccepted drafts (time-header sizes and
the new simultaneous MP4/ADTS packet working-set test). Their complete source,
fixtures and evidence remain in the primary preservation commit and original
working tree. All previously accepted tests and assertions remain in the
candidate. See `push-or-clear-20261029.json` for exact exclusions.

The active AAC/EP branch, four stashes, release branches, tags, historical WIP
preservation branches and worktrees remain untouched. Older completed feature
branches mostly have patch-equivalent tips already in main; SSR and quantized
seek regression files also match main, and the audio configuration export test
has a later supported-geometry acceptance change in main.

No remote push, main update or branch/worktree deletion was performed.

Failed checks and full log:
`evidence/push-or-clear-20261029/status.json` and `owned-media.log`.
