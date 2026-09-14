# Оптимизация декодирования аудио

120-second stereo 48kHz audio to WAV, PCM precision matched; whole CLI cached-input/file-output without fsync, 11 randomized runs after exact sample equality warmup.

Сравнение старого и нового release-бинарников в одном запуске на Apple M4 Max. Все варианты предварительно проверены по точному совпадению PCM-сэмплов.

| Codec | До, мс | После, мс | FFmpeg, мс | Сокращение времени Fvid |
|---|---:|---:|---:|---:|
| aac | 86.02 | 62.30 | 81.77 | 27.6% |
| libmp3lame | 100.24 | 76.55 | 93.03 | 23.6% |
| flac | 73.84 | 73.19 | 76.32 | 0.9% |

Изменения: refcounted pool вместо новой allocation на каждый interleave; mono planar передаётся по ссылке; stereo 32-bit interleave использует NEON на aarch64; остальные layouts используют переносимый код с последовательной записью. Функции не выполняют операций над float-значениями, сохраняя все биты.

Проверены 84 media e2e-сценария, а также SIMD tails, unaligned buffers, byte guards, lifetime/reuse/padding пула. Clippy прошёл. Аппаратные GPU-тесты этим изменением не затронуты и заново не выполнялись.

Результат относится к трём указанным stereo 48 kHz fixtures, cached I/O и этому хосту. Меньше 1% изменения FLAC не считается доказанным ускорением. Общая скорость видео, всех кодеков и всех платформ этим тестом не доказана.

[Сырые замеры и команды](audio-benchmark.json) · [Предыдущий снимок](audio-benchmark-before-optimization.json) · [Проверки](media-validation.json)
