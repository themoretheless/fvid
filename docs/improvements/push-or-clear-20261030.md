# Preservation and publication gates, 2026-10-30

Saved current verified media work and regression evidence at `669ec6d8be2f2a8cb74407e38d5578edae568fc5`. Registry remains 415 accepted improvements. No new item is counted for Git maintenance.

Fresh locked/offline fvid-media library qualification: 540 passed, 5 failed, 1 ignored. The five failed names match the preceding publication gate; complete output is retained under `evidence/git-preservation-20261030/owned-media.log`. Publication is held under push-or-clear until failures are resolved; no remote push or main merge was performed.

A standalone offline manifest probe also confirms that the current local Forma path dependency needs a portable/pinned delivery before CI publication. Generic Metal source hashes still match the previous qualification; unrelated Forma changes are untouched.

Active worktrees, four stashes, release branches and tags are preserved. No branch/worktree cleanup was performed. Temporary qualification target and manifest probe were removed after preserving evidence. The 1000-improvement goal remains active.

Update: batch-413 resolves the five library failures with strict transport/decoder admission and error-precedence oracles; 545 passed, 0 failed, 1 ignored. Batch-414 publishes only the generic GPU extraction to Forma main at `82ff6067546bdc11bc8d415b97dd1cf10c5fc5a9` and pins that immutable Git dependency in fvid. Forma library: 15 passed, 1 hardware test ignored. Fvid VT: 2 passed, 3 hardware tests ignored. Player locked/offline check and 13 MP4 integration tests pass. Same isolated manifest probe fails on the old sibling path and resolves all three GPU packages from the pinned Git source after the change. No new media improvement is counted. Fvid main publication remains pending the remaining known baseline checks; other active worktrees stay untouched.
