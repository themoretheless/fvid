# Clamp wide native MP4 seek requests before tick narrowing

Accepted FVID-0481. All seven final qualification gates passed.

Native MP4 seeking promises to clamp requests to the displayed endpoint. It
previously narrowed Duration into i64 track ticks before applying the edit/end
clamp. Duration::MAX,u64::MAX seconds,and the first second above the track's
signed tick range therefore failed on otherwise valid tiny movies. Both raw
and RGB seek APIs failed; ordinary large representable requests worked.

The identical two-test published baseline has one passing source/control test
and one failing acceptance,with24 failing request/clock/edit/API variants.
All four authored movies decode three independently checked colored VP9 packet
pictures;the two edited presentations expose only the first two. Controls verify
exact YUV16,PTS,native displayed intervals,duration,ordinary end clamping and EOF.

MP4's indexed presentation duration now bounds the Duration request before
signed tick conversion. Existing edit selection,decoder seek,read caps and
error recovery remain unchanged. WebM delegation occurs before this change;
Y4M remains unsupported for seeking. No public API or codec support change.

Both focused tests pass after the correction. Huge requests land on the same
last picture and exact interval as ordinary end seeks,through both APIs,and
reach EOF. Repeated seeks return through zero before every wide request. All
request/clock/edit variants count once;there is no performance claim.

Eleven files regenerate exactly from owned arithmetic-coded colored VP9 and
analytic YUV16 oracles. Generation is independent of ordinary tests and needs
no FFmpeg,private media or network. Software native playback is exercised;
no physical audio/GPU or native-window duration qualification is claimed.
