# Unaccepted experiment notes

The current unordered branch recomputes the prefix maximum for every seek.
A sync sample i is admissible exactly when pts[i] <= target and every preceding
PTS is strictly smaller than pts[i]. Preserve the last admissible decode index;
do not substitute nearest-PTS seeking, which loses previously qualified coverage.

The existing full-length future_pts suffix-minimum array is charged against half
of decoder_budget. It is required by the output reorder queue. An extra uncharged
seek cache would violate this contract; reducing queue capacity without preserving
all earlier successful decodes is also unacceptable. Picture conversion and
hardware setup consume queue_budget before retain_frame, so eviction only in
retain_frame would be insufficient.

If the baseline proves material cost, consider paired prefix-maximum and
suffix-minimum summaries at fixed small block boundaries instead of an additional
full-length index. Exact within-block scans can recover prefix maxima and suffix
minima without changing their semantics. Prefix maxima are monotone, bounding the
search for admissible sync samples; backward exact checks retain the original
landing rule. Handle i64::MIN and the empty prefix explicitly. This is only an
implementation candidate, not an accepted optimization.

Any replacement must account actual retained capacity, preserve strict ordered
tracks with zero auxiliary payload, compare all landings and full decoded
coverage against the original loop, and keep resource refusal/acceptance meaningful.
Add short authored block-boundary/equal/reordered cases where needed. Update the
existing allocation ledger test to the real independently derived allocation;
never remove its budget equality assertion. Paired timing criteria are recorded
in plan.json before any production edit. Reject and revert inconclusive results.
