# Unknown movie duration and chapter clocks

Owned synthetic neutral 16x16 VP9, three independent frames at 50Hz/60ms.
Movie clock is 1000Hz. V0/V1 regular and fragmented files differ only in stated
movie duration (known, unknown all-ones, zero) and chapter clock (10MHz vs legacy
movie ticks). Expected chapter marks are exactly 0/20/40ms. Four additional
movie-clock companions have a leading empty edit of 5000 movie ticks followed
by 60 ticks of pictures; their absolute chapter metadata stays 5000/5020/5040ms.
NativeReader's documented normalization of leading empty edits is unchanged.

Generation: scripts/generate_mp4_unknown_movie_chapters.py, offline, no FFmpeg
or private inputs. Ordinary tests read checked-in files only. Tests check chapter
names/times, both parsers, exact packets, budgets, native RGB, duration, sequential
intervals, chapter-target seeks, rewind and repeated EOF. See manifest hashes.
