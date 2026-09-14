# Обычные видеофайлы: native media adapter

Добавлен необязательный `fvid-media`: Rust-адаптер к установленным libavformat/libavcodec/libavutil/libavfilter. Production-код не запускает FFmpeg CLI. Собственный Y4M CPU/GPU-движок остаётся отдельным; наличие адаптера не означает, что FFmpeg-кодеки переписаны на Rust.

## Сборка

```sh
cargo build --release --features media
./target/release/fvid media --help
```

Нужны FFmpeg development headers/libraries и libclang для генерации bindings. На проверенном Apple Silicon хосте используются `/opt/homebrew/include`, `/opt/homebrew/lib`, FFmpeg 9.0.1 и Command Line Tools libclang. Другой prefix задаётся `FVID_FFMPEG_PREFIX`. `cargo build --release --no-default-features --features media` отключает wgpu/CUDA, сохраняя native media.

Feature `media` не входит в стандартную сборку, чтобы GPU/CPU-ядро не требовало FFmpeg. Для библиотечных пользователей интерфейс доступен как `fvid::media` при включённом feature. Native media проверен на macOS; прежние Windows/Linux-проверки GPU не являются проверками нового адаптера.

## Работающие команды

```sh
# Анализ контейнера, потоков, размеров и временных шкал.
fvid media probe input.mp4

# Перепаковка без декодирования/кодирования.
fvid media remux input.mp4 output.mkv

# Выбор дорожек: только видео либо извлечение совместимого аудио в WAV.
fvid media remux input.mp4 video.mp4 --streams 0
fvid media remux input.mkv audio.wav --streams 1

# PCM-аудио: рез внутри пакета без декодирования.
fvid media trim-pcm input.wav clip.wav --from 0.125 --to 0.525
fvid media trim-pcm input.mkv clip.wav --streams 1 --from 0.125 --to 0.525

# Точное вырезание на допустимых границах с сохранением сжатых пакетов.
fvid media trim input.mp4 clip.mp4 --from 1 --to 2 --streams 0

# Последовательная склейка совместимых потоков.
fvid media concat joined.mp4 first.mp4 second.mp4 --streams 0

# Кроп декодированных пикселей и lossless-экспорт FFV1 в Matroska.
fvid media crop-lossless input.mp4 cropped.mkv --crop 2:2:640:360

# Lossless-перекодирование всего кадра; crop и vflip можно объединить.
fvid media transcode-lossless input.mp4 lossless.mkv
fvid media transcode-lossless input.mp4 flipped.mkv --crop 2:2:640:360 --vflip

# Рез внутри GOP с lossless-перекодированием выбранных кадров.
fvid media transcode-lossless input.mp4 clip.mkv --from 0.12 --to 0.52 --streams 0
fvid media transcode-lossless input.mp4 clip-seek.mkv --from 8.12 --to 8.52 --streams 0 --seek

# Инвентаризация установленной библиотеки — не список квалифицированных workflows.
fvid media capabilities
```

Индексы потоков следует брать из `probe`. Если `--streams` отсутствует, выбираются все потоки. Извлечение в WAV не преобразует AAC в PCM: для WAV нужен поддерживаемый этим muxer исходный codec. Неподходящий контейнер возвращает ошибку. Обычные media-команды работают с локальными файлами, без stdin/stdout, сетевых протоколов и пользовательских AVIO callbacks.

## Stream copy: качество, время, память

`remux`, `trim`, `concat` передают refcounted `AVPacket` из demuxer в muxer. Fvid не создаёт копию packet payload и не декодирует видео для этих операций. Muxer может выполнять необходимую упаковку/bitstream adaptation внутри FFmpeg; `fvid_payload_copies=0` не означает ноль копий внутри всех библиотек/ОС.

`remux` сохраняет параметры кодеков, metadata контейнера/потоков, disposition и главы. Семантика всех расширений всех контейнеров не квалифицирована; проверены MP4→MKV, MP4→MP4 с AAC и извлечение PCM в WAV. Файл целиком не обязан быть побайтно одинаковым: контейнер переписывается.

`trim`/`concat` намеренно строгие:

- Времена задаются десятичными секундами с точностью до микросекунды, без float. Начало trim отсчитывается от container start; диапазон полуоткрытый `[from, to)`.
- Требуются точные packet boundaries, положительные durations, PTS=DTS, отсутствие gaps/overlaps, одинаковые начало и длительность выбранных audio/video потоков. Рез внутри аудиопакета отвергается.
- На входе сегмента требуется независимый кадр: intra-only codec, FFV1 keyframe либо H.264 IDR. Одного keyframe flag для произвольного codec недостаточно. FFV1 может переносить entropy context между неключевыми кадрами.
- B-frame/reordered streams, delay/padding и packet side data не квалифицированы для строгого stream-copy editing. **Обычный remux остаётся доступен**, а B-frame decoding проверен для lossless-кропа.
- Concat сравнивает codec configuration/extradata, format, размеры, цвет, audio layout, time bases и задержки. Изменение конфигурации не исправляется скрытым encoder/resampler.
- Muxer должен точно представлять timestamps выбранных пакетов. Строгий путь отвергает округление времён вместо незаметного смещения.
- Изменение глав при trim/concat пока не реализовано: файлы с главами отвергаются. Remux и crop полной длительности главы сохраняют.

Это ограниченный проверенный режим, а не универсальный smart rendering. Для обычного H.264 с B-frames точный монтаж без перекодирования требует дополнительной работы с зависимостями/pre-roll/edit lists или явного lossless-export режима.

Исполнение pull-based: один Packet guard переиспользуется; частичные выходы не публикуются. Проверяются не более 64 потоков и packet payload до 64 MiB (через Rust CopyOptions лимит настраивается). Probe ограничен 5 MiB/5 сек анализа; mux interleave delta — 1 сек. **Это не строгий общий RAM limit:** demuxer уже мог выделить пакет до проверки, библиотека удерживает свои очереди/codec buffers. Полный бюджет native-адаптера ещё не реализован.

## Lossless-экспорт, кроп и отражения

Native decoder → optional `av_frame_apply_cropping` → optional vertical row view → FFV1 level 3 → Matroska. `transcode-lossless` без флагов сохраняет весь декодированный кадр. `--vflip` меняет начало planes и знак stride; альфа-плоскость отражается на полной высоте, chroma — на своей. Отдельный промежуточный pixel buffer не создаётся. Порядок совместной операции: crop, затем hflip, затем vflip. Fvid меняет представление planes и не копирует pixel payload при crop; encoder может иметь собственные копии/allocations. GPU здесь не используется. Codec surface interop остаётся отдельным этапом.

Pixel format, subsampling и bit depth не преобразуются автоматически. Сохраняются исходные codec color fields; неподдерживаемый FFV1 формат вызывает ошибку. Проверены YUV420p 8-bit и 10-bit, YUV422p, YUV444p, YUVA444p, planar RGB 10-bit и gray16le. Interlaced/dynamic geometry, неизвестный формат и codec side data, требующий обработки metadata, отвергаются. Decoder ограничен 8192×4320 pixels; это не полный memory budget. Decoder/FFV1 encoder используют автоматический выбор числа потоков (`thread_count=0`); их внутренние очереди учитываются пока только библиотекой.

Остальные выбранные потоки remux’ятся. Проверено точное сохранение PCM samples и AAC packet payload/декодированного аудио. Полный HDR/Dolby Vision/rotation/alpha conformance не заявлен. Lossless относится к декодированным сэмплам выбранной области, а не к восстановлению информации, потерянной при исходном lossy-кодировании.

## Надёжность и проверка

Контексты, packets, frames и codec parameters освобождаются RAII guards. Decoder и encoder проходят send/receive и drain; проверено завершение B-frame потока без потери последних кадров. Ошибки native API возвращаются через Rust Result.

Выход резервируется во временном файле рядом с destination и публикуется no-clobber hard link после успешного trailer/close. Существующий файл не заменяется. Нужна поддержка hard links; crash durability/fsync не гарантированы.

```sh
cargo test --features media
cargo test --manifest-path crates/fvid-media/Cargo.toml
cargo clippy --manifest-path crates/fvid-media/Cargo.toml --all-targets -- -D warnings
python3 scripts/validate_media.py --binary target/release/fvid
```

[Зафиксированные 84 end-to-end проверки](../benchmarks/media-validation.json) включают сравнения packet hashes, decoded pixel/audio samples, 8/10-bit crop, B-frame drain, сохранение и retiming глав, несовместимые границы и сохранение существующего output. JSON фиксирует результаты сравнений, хеш release-бинарника и исходников; временные fixtures удаляются, их генерация воспроизводима скриптом. В production нет subprocess; FFmpeg/ffprobe CLI используются в тестах как генератор и oracle.

## Источники контрактов

- [FFmpeg remux example](https://www.ffmpeg.org/doxygen/7.1/remux_8c-example.html): packet ownership, timebase и muxing.
- [AVPacket API](https://ffmpeg.org/doxygen/trunk/group__lavc__packet.html): reference-counted packet buffers и timestamp rescale.
- [FFV1 specification](https://github.com/FFmpeg/FFV1/blob/master/ffv1.md): keyframes и entropy contexts.
- [Предметно изученные проекты](../research/FINDINGS.md): идеи типов времени, владения buffers и явных capabilities.

Лицензия собственного адаптера — MIT; она не изменяет лицензию подключённой сборки FFmpeg и её codec libraries.

## Измерения

`python3 scripts/benchmark_media.py` сравнивает remux, full-frame FFV1 и crop+vflip+FFV1 с FFmpeg default и one-thread. Перед замером каждого варианта сверяются все декодированные пиксели, для remux также packet hashes. [Результаты](../benchmarks/media-benchmark.json) содержат команды, каждое измерение, размеры файлов и provenance. Это CPU codec benchmark; GPU-путь здесь не измеряется.

## Lossless-обрезка внутри GOP

`transcode-lossless --from A --to B` декодирует исходный поток, выбирает кадры по presentation timestamp в `[A,B)` и кодирует их в FFV1. B-frames и отсутствие IDR на границе допустимы. Crop/vflip применяются после выбора. Сжатые пакеты изменяются, декодированные пиксели выбранных кадров сохраняются.

Времена отсчитываются от начала контейнера; из PTS результата вычитается A. Если A находится между кадрами, выбирается первый кадр с PTS >= A и сохраняется соответствующий начальный временной зазор. Длительность последнего выбранного кадра не подрезается: это выбор кадров, а не синтез дробного кадра. Границы должны точно представляться во входном time base. Matroska может округлять timestamps до своего time base; точность 25 fps проверена отдельно, универсальная субмиллисекундная точность не заявлена.

Требуется ровно одна выбранная видеодорожка; дополнительные выбранные аудиодорожки могут содержать поддерживаемый packed PCM. Главы обрезаются по интервалу, сдвигаются к его началу и сохраняют metadata; главы вне интервала удаляются. Для сжатого аудио нужно явно выбрать видео через `--streams`; иначе ошибка. PCM режется внутри пакетов на точных границах сэмплов. Декодирование и обрезка AAC/других сжатых форматов остаются незавершёнными. Времена глав должны точно представляться в микросекундах; неподходящий time base возвращает ошибку. По умолчанию обрабатывается весь вход с drain декодера. Явный `--seek` использует `avformat_seek_file` с верхней границей не позже начала интервала; декодирование начинается с выбранной demuxer точки. После seek оставшаяся часть файла пока читается до EOF. Проверены внутренний рез H.264 B-frame, комбинация с crop/vflip, последний интервал перед EOF, PTS результата, отказ без частичного файла.

### Нечётные размеры

Lossless-путь допускает нечётные ширину и высоту полного кадра и области crop. Начало crop должно быть выровнено по chroma subsampling; размеры области такого требования не имеют. Проверяются 129×73 и crop 65×49 для YUV420p, YUV422p и YUV420p10le, включая vflip. Высота chroma-плоскостей округляется вверх; последний ряд не отбрасывается.

### Seek: проверенная граница

`--seek` проверен на H.264 MP4 с закрытыми GOP и B-frames: выбранные пиксели и PTS совпадают с полным декодированием. Общая корректность random access для open-GOP, повреждённых индексов и всех demuxers не квалифицирована, поэтому ускорение явно включается флагом. Ошибка seek возвращается без публикации результата. `decoded_frames` показывает все декодированные кадры, `video_frames` — отправленные в encoder, `seek_used` — выбранный режим. [Воспроизводимый замер](../benchmarks/seek-benchmark.json), скрипт `scripts/benchmark_seek.py`.

## PCM-аудио при lossless-обрезке

`transcode-lossless --from 0.125 --to 0.525` может сохранять выбранные PCM-дорожки вместе с видео. Поддерживается packed PCM integer 8/16/24/32-bit и float32/64 (для многобайтных форматов LE/BE). Проверены mono s16le и stereo s24le/s32le/f32le, 48 kHz, включая рез внутри пакета. Пакетный указатель и размер ограничиваются нужными sample frames; `AVPacket.buf` остаётся владельцем исходной памяти, копии payload не создаются. `trimmed_audio_sample_frames` суммирует сохранённые sample frames по аудиодорожкам, без умножения на число каналов.

Положительные duration, целые sample frames и согласованность payload с временем проверяются. Границы и новые packet timestamps должны точно представляться во входной временной шкале; иначе отказ без output. Это ограничивает точность контейнерами с грубым audio time base. Padding, packet side data и сжатые аудиокодеки не принимаются. `--seek` вместе с обрезаемым PCM пока запрещён до квалификации audio preroll. Видео сохраняет начальный зазор относительно выбранного времени, а не сдвигается независимо от аудио. Проверочный pixel oracle использует passthrough frame rate, чтобы не создавать повторные кадры при таком зазоре.

### Отдельные PCM-файлы

`trim-pcm` принимает выбранные PCM-дорожки без видео. Он использует тот же packet slicing, сохраняет bit depth и channels, пересчитывает главы, проверяет точность rescale в выходной контейнер. Decoder/encoder не создаются; WAV→WAV и MKV→WAV проверены по всем сэмплам. По умолчанию выбираются все дорожки: наличие видео или сжатого аудио вызывает ошибку, для извлечения используется `--streams`. Пока читается весь вход. [CPU-бенчмарк](../benchmarks/pcm-benchmark.json) сравнивает с FFmpeg seek и atrim и проверяет stereo 24-bit PCM перед замерами.

### Горизонтальное отражение

`transcode-lossless input.mp4 output.mkv --hflip` отражает software-плоскости по горизонтали и сохраняет FFV1. `--crop` и `--vflip` можно сочетать с `--hflip`, включая lossless interval. На каждой строке переставляются целые pixel groups, чтобы не менять порядок байтов многобайтного сэмпла или компонентов packed RGB. Проверены planar YUV420/422, 10-bit, RGB10, alpha и нечётная ширина.

Перед записью вызывается `av_frame_make_writable`: декодер может ещё использовать исходную плоскость как reference frame. В таком случае copy-on-write копирует кадр; если буфер уникальный, перестановки выполняются на месте. Это не обещание нуля копий для hflip. Packed subsampled форматы с неоднородными pixel groups и аппаратные плоскости отвергаются. Y4M CPU/GPU-реализация по-прежнему отдельная.

## Явный выбор видеокодека

Команда `transcode` требует `--encoder NAME`; её качество определяется выбранным энкодером и параметрами. Пример H.264:

```sh
fvid media transcode input.mp4 output.mp4 --encoder libx264 --encoder-option crf=23 --encoder-option preset=fast
fvid media transcode input.mp4 exact.mp4 --encoder libx264 --encoder-option crf=0 --crop 2:2:640:360 --hflip
fvid media transcode input.mp4 exact.webm --encoder libvpx-vp9 --encoder-option lossless=1
```

`transcode-lossless` по-прежнему фиксирует FFV1/Matroska и отвергает подмену энкодера. В общем режиме доступны существующие crop/hflip/vflip, interval, обработка PCM и глав; ограничения соответствующих операций сохраняются. Дополнительные выбранные потоки remux’ятся. Контейнер определяется расширением выходного файла. Encoder parameters передаются библиотеке напрямую; до 64 параметров, неиспользованные ключи отвергаются. Формат пикселей не преобразуется автоматически; несовместимость с энкодером вызывает ошибку.

Проверены libx264 CRF0 и libvpx-vp9 lossless=1 с точным сравнением пикселей после crop/hflip, а также lossy H.264 CRF28 с сохранением количества кадров/геометрии. Для CRF28 равенство пикселей не заявляется. Наличие остальных энкодеров в capabilities не доказывает их end-to-end поддержку. Hardware encoder selection не создаёт hardware decoder/filter interop и не означает zero-copy GPU. [H.264 lossless бенчмарк](../benchmarks/encoder-benchmark.json) сравнивает одинаковое преобразование и качество на коротком корпусе.

Rust API: `EncoderSettings { name, options }` и `transcode(...)`; тип результата исторически называется `LosslessStats`, но в общем режиме это статистика обработки, а не гарантия lossless. Поле `encoder` указывает фактически запрошенный энкодер.

## Декодирование сжатого аудио в PCM

```sh
fvid media decode-audio input.m4a decoded.wav
fvid media decode-audio video.mp4 decoded.wav --streams 1
```

`decode-audio` выбирает ровно одну аудиодорожку. Сохраняются sample rate, channel layout и точность **декодированного** sample format: AAC/MP3 float не квантуется скрыто до 16-bit. Для packed frame packet удерживает ссылку на буфер декодера. Для многоканального planar frame нужен interleave в PCM-пакет; mono передаётся по ссылке, как packed. PCM-буферы берутся из refcounted пула и возвращаются туда только после освобождения всех ссылок. Stereo 32-bit interleave на aarch64 использует NEON с сохранением всех битов; остальные размеры/платформы используют переносимый путь. число переставленных байтов записывается в `planar_interleave_bytes`. Формат, layout и rate не могут меняться посередине потока. Размер сформированного блока ограничивается packet budget; это не полный бюджет памяти декодера.

Экспорт формирует непрерывную последовательность сэмплов с начала WAV, без синтеза тишины для разрывов исходных PTS. Источники с главами пока отвергаются до реализации их отображения на такую шкалу. Проверены AAC→float32, MP3→float32 с codec delay и FLAC→s16, побайтное равенство с PCM FFmpeg. Это ещё не встроенная обрезка сжатого аудио в видеоконвейере. Можно отдельно декодировать файл, затем использовать `trim-pcm`, но единый потоковый путь остаётся работой на следующий этап.

[Аудиобенчмарк](../benchmarks/audio-benchmark.json) сравнивает одинаковую точность PCM на 30-секундных stereo 48 kHz fixtures. Production-код вызывает библиотеку напрямую; внешние FFmpeg-команды используются только для проверки и измерений.

### Проверка аудиооптимизаций

Добавлены тесты unaligned buffers, SIMD tails, byte guards, разных размеров сэмплов/числа каналов, удержания ссылок и padding пула. Отдельный stereo AAC fixture содержит разные сигналы в каналах. Скрипт `benchmark_audio.py --baseline PATH --runs 11 --duration 120` перемешивает новый Fvid, старый бинарник и FFmpeg в каждом раунде; выход каждого варианта сверяется по PCM-сэмплам перед измерениями. JSON сохраняет оба binary hashes и исходные замеры.
