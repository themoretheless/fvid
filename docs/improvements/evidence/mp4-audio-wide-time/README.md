# Wide MP4 audio time conversion

Status: accepted FVID-0449; count449/1000.

Baseline95114ab958f6f801aaae546143317e47fa8fe01e accepts authored fragmented IMA4 coordinates beyond the u64 nanosecond boundary, but duration() and time_of() clamp to18446744073.709551615seconds. Actual packets and AudioPacket already represent their source ticks correctly. Seeking one native sample into the same accepted source fails with the misleading negative MP4 audio presentation time error. Baseline1control passes and2clock/PCM-seek acceptances fail. before-test.rs.txt preserves the exact baseline version; actual worker acceptance was added after that baseline.

Candidate uses the existing exact signed edit clock to split Duration into seconds/remainder without narrowing total nanoseconds. Normal packet seek timestamps retain their previous nanosecond spelling when representable; wide timestamps fall back to an exact reduced fraction instead. No public interface migration or performance claim.

Five authored tiny videos carry three neutral VP9frames60ms and three analytic IMA4blocks24ms. tfdt V1 shifts only audio coordinates; there is no huge media allocation or real-time wait. Cases cover ordinary zero, below/above u64 nanosecond boundary, near signed tick endpoint, and genuinely invalid signed composition overflow. Seven committed files regenerate byte-identically. Ordinary tests use no encoder,network or generator.

Focused after20passes across5suites includes exact audio duration/time,raw fragment clocks,scalar PCM,short video,rewind,sample seek,EOF and actual AudioPlayback worker PCM/PTS/seek/EOF,plus partial/zero windows and recovered clocks. True signed PTS overflow is still refused. Full126-suite integration588pass,rootMP458pass,WebM67pass,media547pass,and core/player builds all terminal0 from frozen sources;1256distinct after4duplicate events. Two prior ignores unchanged.
