# Обычные видеофайлы: native media adapter

Добавлен необязательный `fvid-media`: Rust-адаптер к установленным libavformat/libavcodec/libavutil/libavfilter. Production-код не запускает FFmpeg CLI. Собственный Y4M CPU/GPU-движок остаётся отдельным; наличие адаптера не означает, что FFmpeg-кодеки переписаны на Rust.

## Сборка

```sh
cargo build --release --features media
./target/release/fvid media --help
```

Нужны FFmpeg development headers/libraries и libclang для генерации bindings. На проверенном Apple Silicon хосте используются `/opt/homebrew/include`, `/opt/homebrew/lib`, FFmpeg 9.0.1 и Command Line Tools libclang. Другой prefix задаётся `FVID_FFMPEG_PREFIX`. `cargo build --release --no-default-features --features media` отключает wgpu/CUDA, сохраняя native media.

### Windows

1. Установите LLVM (`winget install LLVM.LLVM`) и задайте `LIBCLANG_PATH` на каталог `bin` с `libclang.dll`.
2. Скачайте shared+dev FFmpeg 9.0: `powershell -File scripts/setup_ffmpeg_windows.ps1` (по умолчанию `C:\ffmpeg-shared` с `include/`, `lib/*.lib`, `bin/*.dll` из BtbN `win64-gpl-shared-9.0`).
3. В сессии: `$env:FVID_FFMPEG_PREFIX='C:\ffmpeg-shared'`; добавьте `C:\ffmpeg-shared\bin` в `PATH`.
4. `cargo build --release --features media` (или `--features mcp`). Для NVDEC/NVENC: `--features media-cuda` и `fvid media hw-filter …`.

Qualified: macOS/FFmpeg 9.0.1; Windows/FFmpeg 9.0.1 shared (BtbN n9.0.1-29). `validate_media.py` threshold ≥314. Feature `media` не входит в стандартную сборку, чтобы GPU/CPU-ядро не требовало FFmpeg.

## Работающие команды

```sh
# Анализ контейнера, metadata/глав, потоков, codec profile/level,
# bitrate, frame rate, disposition, размеров и временных шкал.
fvid media probe input.mp4

# Окно: software decode, звук устройства вывода. Space — пауза, Esc — выход.
fvid play input.mp4
fvid media play input.mp4 --no-audio

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

# Lossless-перекодирование всего кадра; crop, scale и vflip можно объединить.
fvid media transcode-lossless input.mp4 lossless.mkv
fvid media transcode-lossless input.mp4 flipped.mkv --crop 2:2:640:360 --vflip
fvid media transcode-lossless input.mp4 scaled.mkv --scale 960:540
fvid media transcode-lossless input.mp4 yuv422.mkv --pix-fmt yuv422p
fvid media transcode-lossless input.mp4 rotated.mkv --transpose clock
fvid media transcode-lossless input.mp4 angled.mkv --rotate 30
fvid media transcode-lossless input.mp4 padded.mkv --pad 1920:1080:0:0
fvid media decode input.mp4 --scale 960:540
fvid media decode input.mp4 --pix-fmt yuv422p
fvid media decode input.mp4 --transpose clock
fvid media decode input.mp4 --rotate 45
fvid media decode input.mp4 --pad 1920:1080:0:0
fvid media burn-subtitles input.mp4 burned.mkv --subs captions.srt
fvid media overlay main.mp4 pip.mp4 stacked.mkv --overlay-x 32 --overlay-y 24
fvid media xfade first.mkv second.mkv cross.mkv --xfade-duration 0.5 --xfade-offset 1.0 --transition fade
fvid media transcode-lossless bt601.mkv bt709.mkv --colorspace iall=bt470bg:all=bt709
fvid media transcode-lossless hdr.mkv sdr.mkv --tonemap tonemap=hable --pix-fmt yuv420p
fvid media transcode-lossless input.mp4 deint.mkv --yadif mode=0
fvid media transcode-lossless input.mp4 bwdif.mkv --bwdif mode=0
fvid media transcode-lossless input.mp4 blended.mkv --tblend all_mode=average
fvid media transcode-lossless input.mp4 clean.mkv --hqdn3d 4:3:6:4.5
fvid media transcode-lossless input.mp4 cfr.mkv --fps 12
fvid media transcode-lossless input.mp4 soft.mkv --gblur sigma=1.5:steps=1
fvid media transcode-lossless input.mp4 graded.mkv --eq brightness=0.06:contrast=1.2
fvid media transcode-lossless input.mp4 sharp.mkv --unsharp 5:5:1.0:5:5:0.0
fvid media transcode-lossless input.mp4 tint.mkv --hue h=45:s=1.2
fvid media transcode-lossless input.mp4 softbox.mkv --boxblur 2:1
fvid media transcode-lossless input.mp4 inverted.mkv --negate 0
fvid media transcode-lossless input.mp4 edges.mkv --edgedetect mode=colormix
fvid media transcode-lossless input.mp4 denoised.mkv --atadenoise 0a=0.02:0b=0.04
fvid media transcode-lossless input.mp4 nlmeans.mkv --nlmeans s=1.0
fvid media transcode-lossless input.mp4 smart.mkv --smartblur lr=1.5:ls=-0.5
fvid media transcode-lossless input.mp4 sharpened.mkv --cas strength=0.5
fvid media transcode-lossless input.mp4 vignetted.mkv --vignette angle=PI/4
fvid media transcode-lossless input.mp4 graded.mkv --curves preset=vintage
fvid media transcode-lossless input.mp4 balanced.mkv --colorbalance rs=.1:gs=.05:bs=-.1
fvid media transcode-lossless input.mp4 leveled.mkv --colorlevels rimin=0.1:gimin=0.1:bimin=0.1
fvid media transcode-lossless input.mp4 mixed.mkv --colorchannelmixer rr=1.1:gg=0.9:bb=1.0
fvid media transcode-lossless input.mp4 deflickered.mkv --deflicker mode=am:size=5
fvid media transcode-lossless input.mp4 safe.mkv --photosensitivity f=5
fvid media transcode-lossless input.mp4 boxed.mkv --drawbox x=10:y=10:w=40:h=20:color=red
fvid media transcode-lossless input.mp4 gridded.mkv --drawgrid w=16:h=16:color=white
fvid media loudness input.m4a
fvid media loudnorm input.m4a normalized.wav
fvid media loudnorm input.m4a normalized.wav --loudnorm-args I=-16:TP=-1.5:LRA=11
fvid media loudnorm input.m4a normalized.wav --dual-pass
fvid media loudnorm input.m4a normalized.wav --loudnorm-args I=-16:TP=-1.5:LRA=11 --dual-pass

# Рез внутри GOP с lossless-перекодированием выбранных кадров.
fvid media transcode-lossless input.mp4 clip.mkv --from 0.12 --to 0.52 --streams 0
fvid media transcode-lossless input.mp4 clip-seek.mkv --from 8.12 --to 8.52 --streams 0 --seek
fvid media transcode-lossless av-aac.mkv clip.mkv --from 0.125 --to 0.525

# Explain без записи: remux / geometry / trim / concat.
fvid media plan input.mp4
fvid media plan input.mp4 --crop 2:2:640:360 --scale 960:540
fvid media plan trim input.mp4 --from 1 --to 2
fvid media plan trim-pcm input.wav --from 0.125 --to 0.525
fvid media plan concat first.mp4 second.mp4
fvid media plan decode-audio input.m4a --rate 44100 --channels 1 --volume 0.5

# Инвентаризация установленной библиотеки — не список квалифицированных workflows.
fvid media capabilities
```

Индексы потоков следует брать из `probe`. Если `--streams` отсутствует, выбираются все потоки. Извлечение в WAV не преобразует AAC в PCM: для WAV нужен поддерживаемый этим muxer исходный codec. Неподходящий контейнер возвращает ошибку. Обычные media-команды работают с локальными файлами, без stdin/stdout, сетевых протоколов и пользовательских AVIO callbacks.

## Stream copy: качество, время, память

`remux`, `trim`, `concat` передают refcounted `AVPacket` из demuxer в muxer. Fvid не создаёт копию packet payload и не декодирует видео для этих операций. Muxer может выполнять необходимую упаковку/bitstream adaptation внутри FFmpeg; `fvid_payload_copies=0` не означает ноль копий внутри всех библиотек/ОС.

`remux` сохраняет параметры кодеков, metadata контейнера/потоков, disposition и главы. Опционально можно изменить tags при stream copy: `--metadata KEY=VALUE`, `--metadata-delete KEY`, `--stream-metadata INDEX:KEY=VALUE`, `--stream-metadata-delete INDEX:KEY` (после копирования исходных словарей, до записи header; packet payload не меняется). `convert-subtitles` декодирует SubRip и кодирует ASS в Matroska (`--codec ass`). `burn-subtitles` накладывает внешний `.srt` через libavfilter `subtitles=` (libass) и пишет video-only FFV1/Matroska (fair-pair с FFmpeg `-vf subtitles=`). `overlay` композитит второй ролик через `movie=`+`overlay=` (fair-pair с FFmpeg `-vf movie=+overlay=`). `xfade` — dual-input cross-fade (fair-pair с FFmpeg `-filter_complex [0:v][1:v]xfade=...,format=`); inputs must share size/fps/pix_fmt; video-only FFV1/Matroska. Квалифицирован также passthrough SubRip: standalone SRT использует header-complete fast-open, а пакеты, timestamps, language metadata и disposition сохраняются при SRT→Matroska, Matroska→Matroska remux и при lossless-перекодировании видео. Семантика всех расширений всех контейнеров не квалифицирована; проверены MP4→MKV, MP4→MP4 с AAC, MOV→MKV, MPEG-TS→MKV, SRT/Matroska с SubRip и извлечение PCM в WAV. Файл целиком не обязан быть побайтно одинаковым: контейнер переписывается.

`trim`/`concat` намеренно строгие:

- Времена задаются десятичными секундами с точностью до микросекунды, без float. Начало trim отсчитывается от container start; диапазон полуоткрытый `[from, to)`.
- Требуются точные packet boundaries для stream-copyable потоков, положительные durations, PTS=DTS (кроме квалифицированного video reorder), отсутствие gaps/overlaps, одинаковые начало и длительность выбранных audio/video потоков. Рез внутри packed-PCM пакета отвергается (`trim-pcm` / lossless). Рез внутри AAC/MP3/FLAC декодирует seam в PCM.
- Audio delay/padding: AAC (и сходные) с `initial_padding` допускаются в strict trim/concat. Trim от `from=0` сохраняет priming; mid-stream trim на точных packet boundaries (кратно AAC frame в time base) квалифицирован против FFmpeg `-c copy`. Cut внутри AAC/MP3/FLAC-пакета декодирует эту дорожку в sample-exact PCM в том же output (audio-only или рядом с video stream-copy); backend сообщает `compressed-audio seam → PCM`. Packed PCM mid-packet по-прежнему требует `trim-pcm` / lossless interval.
- На входе сегмента требуется независимый кадр: intra-only codec, FFV1 keyframe, H.264 IDR либо HEVC IRAP. Одного keyframe flag для произвольного codec недостаточно. FFV1 может переносить entropy context между неключевыми кадрами.
- B-frame/reordered streams: closed-GOP H.264 IDR→IDR и HEVC IRAP→IRAP `trim` копирует decode-span от RAP ≤ `from` до первого keyframe ≥ `to` (excluded). Mid-GOP `from`/`to` на точных packet PTS: pre-roll через отрицательные PTS, post-roll через `AV_PKT_FLAG_DISCARD` + clipped duration. Open-GOP end на keyframe допускается при closed RAP на старте; **open-GOP start** stream-copy ищет предшествующий closed IDR/IRAP (pre-roll scan) и использует тот же present_origin/negative-PTS механизм. Delay/padding на video и packet side data не квалифицированы. **Обычный remux остаётся доступен**.
- Concat сравнивает codec configuration/extradata, format, размеры, цвет, audio layout, time bases и задержки. Изменение конфигурации не исправляется скрытым encoder/resampler.
- Muxer должен точно представлять timestamps выбранных пакетов. Строгий путь отвергает округление времён вместо незаметного смещения.
- Изменение глав при trim/concat пока не реализовано: файлы с главами отвергаются. Remux и crop полной длительности главы сохраняют.

Это ограниченный проверенный режим, а не универсальный smart rendering. Open-GOP start (presentation на open key) по-прежнему требует дополнительной квалификации decoder-visible pre-roll.

Исполнение pull-based: один Packet guard переиспользуется; частичные выходы не публикуются. Проверяются не более 64 потоков и packet payload до 64 MiB (через Rust CopyOptions лимит настраивается). CLI/API `--max-packets N` и `CopyOptions.cancel` (`CancelFlag`) дают cooperative abort между пакетами без публикации частичного output. `--progress` / `CopyOptions.progress` (`ProgressHook`) пишет NDJSON-сэмплы на stderr каждые 256 пакетов и финальный `done=true`. `--max-memory-mib N` / `CopyOptions.max_controlled_bytes` — admission по оценке decoder DPB (`video_delay+1`, cap 32) + encoder hold + Fvid scratch для decode/lossless; для remux/trim/concat оценка равна потолку `max_packet_bytes`. `--max-rss-mib N` / `CopyOptions.max_rss_bytes` периодически (каждые 256 пакетов и на старте) сравнивает process Working Set / VmRSS с лимитом и abort’ит без публикации. Probe ограничен 5 MiB/5 сек анализа; mux interleave delta — 1 сек. **Это не полный RSS guarantee:** demuxer мог выделить пакет до проверки; внутренние libav thread/lookahead queues сверх оценки не учитываются побайтно.

## Lossless-экспорт, кроп и отражения

Native decoder → optional `av_frame_apply_cropping` → optional vertical row view → optional transpose (`libavfilter`) → optional rotate (`libavfilter` `rotate=a=DEG*PI/180:ow=…:oh=…:c=black`) → optional pad → optional neighbor scale → optional `epx=` → optional burn (`libavfilter` `subtitles=`) → optional overlay (`movie=` + `overlay=x:y`) → optional `tblend=` → optional `hqdn3d=` → optional `gblur=` → optional `eq=` → optional `unsharp=` → optional `hue=` → optional `boxblur=` → optional `negate` → optional `edgedetect=` → optional `atadenoise=` → optional `nlmeans=` → optional `smartblur=` → optional `cas=` → optional `vignette=` → optional `curves=` → optional `colorbalance=` → optional `colorlevels=` → optional `colorchannelmixer=` → optional `deflicker=` → optional `photosensitivity=` → optional `drawbox=` → optional `drawgrid=` → optional `colorspace=` → optional `tonemap=` → optional `format=` (`--pix-fmt`) → optional `minterpolate=` (`--minterpolate`) → optional `fps=` (`--fps`) → FFV1 level 3 → Matroska. `scale`+`format` without burn/overlay/tblend/hqdn3d/gblur/eq/unsharp/hue/boxblur/negate/edgedetect/atadenoise/nlmeans/smartblur/cas/vignette/curves/colorbalance/colorlevels/colorchannelmixer/deflicker/photosensitivity/drawbox/drawgrid/colorspace/tonemap/minterpolate/fps merges into one neighbor swscale (FFmpeg `scale=flags=neighbor,format=`); standalone `--pix-fmt` uses bicubic like FFmpeg `format=`. `transcode-lossless` без флагов сохраняет весь декодированный кадр. `--vflip` меняет начало planes и знак stride; альфа-плоскость отражается на полной высоте, chroma — на своей. Отдельный промежуточный pixel buffer не создаётся для crop/vflip; transpose, rotate, pad, scale, burn, overlay, tblend, hqdn3d, gblur, eq, unsharp, hue, boxblur, negate, edgedetect, atadenoise, nlmeans, smartblur, cas, vignette, curves, colorbalance, colorlevels, colorchannelmixer, deflicker, photosensitivity, drawbox, drawgrid, colorspace, tonemap, format, minterpolate и fps материализуют выходной кадр. Порядок совместной операции: crop, затем hflip, затем vflip, затем transpose, затем rotate, затем pad, затем scale, затем burn, затем overlay, затем tblend, затем hqdn3d, затем gblur, затем eq, затем unsharp, затем hue, затем boxblur, затем negate, затем edgedetect, затем atadenoise, затем nlmeans, затем smartblur, затем cas, затем vignette, затем curves, затем colorbalance, затем colorlevels, затем colorchannelmixer, затем deflicker, затем photosensitivity, затем drawbox, затем drawgrid, затем colorspace, затем tonemap, затем format, затем minterpolate, затем fps. Fvid меняет представление planes и не копирует pixel payload при crop; encoder может иметь собственные копии/allocations. GPU здесь не используется. Codec surface interop остаётся отдельным этапом.

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

[Зафиксированные end-to-end проверки](../benchmarks/media-validation.json) включают rich probe, standalone и mapped subtitle passthrough, точные decode intervals для reordered/no-reorder потоков, сравнения packet hashes, decoded pixel/audio samples, 8/10-bit crop, neighbor scale, B-frame drain, сохранение и retiming глав, несовместимые границы и сохранение существующего output. JSON фиксирует результаты сравнений, хеш release-бинарника и исходников; временные fixtures удаляются, их генерация воспроизводима скриптом. В production нет subprocess; FFmpeg/ffprobe CLI используются в тестах как генератор и oracle.

## Источники контрактов

- [FFmpeg remux example](https://www.ffmpeg.org/doxygen/7.1/remux_8c-example.html): packet ownership, timebase и muxing.
- [AVPacket API](https://ffmpeg.org/doxygen/trunk/group__lavc__packet.html): reference-counted packet buffers и timestamp rescale.
- [FFV1 specification](https://github.com/FFmpeg/FFV1/blob/master/ffv1.md): keyframes и entropy contexts.
- [Предметно изученные проекты](../research/FINDINGS.md): идеи типов времени, владения buffers и явных capabilities.

Лицензия собственного адаптера — MIT; она не изменяет лицензию подключённой сборки FFmpeg и её codec libraries.

## Измерения

`python3 scripts/benchmark_media.py` сравнивает remux, full-frame FFV1 и crop+vflip+FFV1 с FFmpeg default и one-thread. Перед замером каждого варианта сверяются все декодированные пиксели, для remux также packet hashes. [Результаты](../benchmarks/media-benchmark.json) содержат команды, каждое измерение, размеры файлов и provenance. Это CPU codec benchmark; GPU-путь здесь не измеряется.

`python3 scripts/benchmark_cpu_media_gate.py --binary target-media/release/fvid` запускает общий строгий native CPU gate после `validate_media.py`. Порядок одиннадцати операций и порядок Fvid/FFmpeg внутри каждой пары перемешиваются в каждом из 21 раундов, чтобы убрать систематический thermal/order bias. [Зафиксированный результат](../benchmarks/cpu-media-gate.json) требует для каждой пары медиану строго выше +15%.

`python3 scripts/benchmark_gpu_media_gate.py --binary target-media-cuda/release/fvid` применяет randomized/interleaved протокол к семи CUDA fair pairs, считает медиану внутрипарных deltas и требует совпадающий по hash отчёт `validate_hw_cuda.py`. Encode-heavy timelines (≥20s) используют multi-session NVENC на RTX 5090; soft vflip и fused остаются single-session. [Текущий 11-round результат](../benchmarks/gpu-media-gate.json): все семь пар проходят строго >15%.

## Lossless-обрезка внутри GOP

`transcode-lossless --from A --to B` декодирует исходный поток, выбирает кадры по presentation timestamp в `[A,B)` и кодирует их в FFV1. B-frames и отсутствие IDR на границе допустимы. Crop/vflip применяются после выбора. Сжатые пакеты изменяются, декодированные пиксели выбранных кадров сохраняются.

Времена отсчитываются от начала контейнера; из PTS результата вычитается A. Если A находится между кадрами, выбирается первый кадр с PTS >= A и сохраняется соответствующий начальный временной зазор. Длительность последнего выбранного кадра не подрезается: это выбор кадров, а не синтез дробного кадра. Границы должны точно представляться во входном time base. Matroska может округлять timestamps до своего time base; точность 25 fps проверена отдельно, универсальная субмиллисекундная точность не заявлена.

Требуется ровно одна выбранная **primary** видеодорожка (первая выбранная video); дополнительные выбранные video streams в full-duration `transcode-lossless` remux’ятся без decode. Interval (`--from/--to`) копирует secondary non-reordered video на точных packet boundaries и closed-GOP H.264/HEVC secondary тем же RAP/mid-GOP stream-copy контрактом, что и `trim` (отдельный demuxer); `--seek` с несколькими video допустим: primary demuxer seek’ает, secondary и compressed audio идут через dedicated demuxers. Дополнительные выбранные аудиодорожки могут быть packed PCM или AAC/MP3/FLAC. Главы обрезаются по интервалу, сдвигаются к его началу и сохраняют metadata; главы вне интервала удаляются. PCM режется внутри пакетов на точных границах сэмплов. AAC/MP3/FLAC в интервале декодируются в PCM (float для AAC/MP3, s16 для FLAC) с той же sample-window семантикой, что и `decode-audio --from/--to`. `--seek` ускоряет video demux/decode; compressed audio при `--seek` декодируется отдельным demuxer’ом с начала файла (contiguous sample window), чтобы сохранить sample-exact равенство с путём без seek — mid-stream AAC state после seek не sample-identical. Времена глав должны точно представляться в микросекундах; неподходящий time base возвращает ошибку. По умолчанию обрабатывается весь вход с drain декодера. Явный `--seek` использует `avformat_seek_file` с верхней границей не позже начала интервала; декодирование видео начинается с выбранной demuxer точки. После seek оставшаяся часть файла пока читается до EOF. Проверены внутренний рез H.264 B-frame, комбинация с crop/vflip, последний интервал перед EOF, PTS результата, отказ без частичного файла.

### Нечётные размеры

Lossless-путь допускает нечётные ширину и высоту полного кадра и области crop. Начало crop должно быть выровнено по chroma subsampling; размеры области такого требования не имеют. Проверяются 129×73 и crop 65×49 для YUV420p, YUV422p и YUV420p10le, включая vflip. Высота chroma-плоскостей округляется вверх; последний ряд не отбрасывается.

### Seek: проверенная граница

`--seek` проверен на H.264 MP4 с закрытыми GOP и B-frames: выбранные пиксели и PTS совпадают с полным декодированием. Общая корректность random access для open-GOP, повреждённых индексов и всех demuxers не квалифицирована, поэтому ускорение явно включается флагом. Ошибка seek возвращается без публикации результата. `decoded_frames` показывает все декодированные кадры, `video_frames` — отправленные в encoder, `seek_used` — выбранный режим. [Воспроизводимый замер](../benchmarks/seek-benchmark.json), скрипт `scripts/benchmark_seek.py`.

## PCM-аудио при lossless-обрезке

`transcode-lossless --from 0.125 --to 0.525` может сохранять выбранные PCM-дорожки вместе с видео. Поддерживается packed PCM integer 8/16/24/32-bit и float32/64 (для многобайтных форматов LE/BE). Проверены mono s16le и stereo s24le/s32le/f32le, 48 kHz, включая рез внутри пакета. Пакетный указатель и размер ограничиваются нужными sample frames; `AVPacket.buf` остаётся владельцем исходной памяти, копии payload не создаются. `trimmed_audio_sample_frames` суммирует сохранённые sample frames по аудиодорожкам, без умножения на число каналов.

Положительные duration, целые sample frames и согласованность payload с временем проверяются. Границы и новые packet timestamps должны точно представляться во входной временной шкале; иначе отказ без output. Это ограничивает точность контейнерами с грубым audio time base. Packet side data на video не принимается. AAC/MP3/FLAC в том же интервале декодируются в PCM (см. выше). `--seek` с PCM audio квалифицирован (demuxer seek + sample-exact `pcm::trim`); с compressed audio — video demuxer seek + отдельный from-start audio demux. Видео сохраняет начальный зазор относительно выбранного времени, а не сдвигается независимо от аудио. Проверочный pixel oracle использует passthrough frame rate, чтобы не создавать повторные кадры при таком зазоре.

### Отдельные PCM-файлы

`trim-pcm` принимает выбранные PCM-дорожки без видео. Он использует тот же packet slicing, сохраняет bit depth и channels, пересчитывает главы, проверяет точность rescale в выходной контейнер. Decoder/encoder не создаются; WAV→WAV и MKV→WAV проверены по всем сэмплам. По умолчанию выбираются все дорожки: наличие видео или сжатого аудио вызывает ошибку, для извлечения используется `--streams`. Пока читается весь вход. [CPU-бенчмарк](../benchmarks/pcm-benchmark.json) сравнивает с FFmpeg seek и atrim и проверяет stereo 24-bit PCM перед замерами.

### Горизонтальное отражение

`transcode-lossless input.mp4 output.mkv --hflip` отражает software-плоскости по горизонтали и сохраняет FFV1. `--crop` и `--vflip` можно сочетать с `--hflip`, включая lossless interval. На каждой строке переставляются целые pixel groups, чтобы не менять порядок байтов многобайтного сэмпла или компонентов packed RGB. Проверены planar YUV420/422, 10-bit, RGB10, alpha и нечётная ширина.

Hflip пишет отражённые строки в повторно используемый writable frame, не изменяя decoder reference surfaces. Для byte-planar YUV420p применяется plane-level AVX2 reverse-copy с одним runtime dispatch на плоскость; остальные форматы используют проверенные pixel groups. Это не операция с нулём копий. Packed subsampled форматы с неоднородными pixel groups и аппаратные плоскости отвергаются. Y4M CPU/GPU-реализация по-прежнему отдельная.

## Явный выбор видеокодека

Команда `transcode` требует `--encoder NAME`; её качество определяется выбранным энкодером и параметрами. Пример H.264:

```sh
fvid media transcode input.mp4 output.mp4 --encoder libx264 --encoder-option crf=23 --encoder-option preset=fast
fvid media transcode input.mp4 exact.mp4 --encoder libx264 --encoder-option crf=0 --crop 2:2:640:360 --hflip
fvid media transcode input.mp4 exact.webm --encoder libvpx-vp9 --encoder-option lossless=1
```

`transcode-lossless` фиксирует FFV1/Matroska и отвергает подмену энкодера. Identity-запрос с единственной уже FFV1-видеодорожкой планируется как stream copy: packets и VFR timeline сохраняются без decode/re-encode, а `encoder` сообщает `ffv1 (stream copy)`. В остальных случаях доступны существующие crop/hflip/vflip, interval, обработка PCM и глав; ограничения соответствующих операций сохраняются. Дополнительные выбранные потоки remux’ятся. Контейнер определяется расширением выходного файла. Encoder parameters передаются библиотеке напрямую; до 64 параметров, неиспользованные ключи отвергаются. Формат пикселей не преобразуется автоматически; несовместимость с энкодером вызывает ошибку. Видеокадры сохраняют исходные PTS и duration в time base входного stream; interval только вычитает точное начало. VFR gaps не превращаются скрыто в CFR.

Проверены libx264 CRF0, libx265 lossless и libvpx-vp9 lossless=1 с точным сравнением пикселей после crop/hflip, а также lossy H.264 CRF28 с сохранением количества кадров/геометрии. Для CRF28 равенство пикселей не заявляется. HEVC/AV1/ProRes квалифицированы как decode inputs для crop-lossless. Наличие остальных энкодеров в capabilities не доказывает их end-to-end поддержку. Hardware encoder selection не создаёт hardware decoder/filter interop и не означает zero-copy GPU. [H.264 lossless бенчмарк](../benchmarks/encoder-benchmark.json) сравнивает одинаковое преобразование и качество на коротком корпусе.

Rust API: `EncoderSettings { name, options }` и `transcode(...)`; тип результата исторически называется `LosslessStats`, но в общем режиме это статистика обработки, а не гарантия lossless. Поле `encoder` указывает фактически запрошенный энкодер.

## Декодирование сжатого аудио в PCM

```sh
fvid media decode-audio input.m4a decoded.wav
fvid media decode-audio video.mp4 decoded.wav --streams 1
fvid media decode-audio input.mp3 interval.wav --from 1.25 --to 3.75
fvid media decode-audio stereo.m4a mono.wav --channels 1
fvid media mix-audio mixed.wav track-a.m4a track-b.m4a
fvid media mix-audio summed.wav track-a.m4a track-b.m4a --no-normalize
fvid media mix-audio trio.wav a.m4a b.m4a c.m4a --weights 1,2,0.5
fvid media merge-audio stereo.wav left.m4a right.m4a
```

`decode-audio` выбирает ровно одну аудиодорожку. Сохраняются sample rate (или `--rate` через `libswresample`, fair-pair с FFmpeg `-ar`), optional `--channels N` rematrix на default layout (fair-pair с FFmpeg `-ac N`), optional `--volume` linear gain на float PCM (fair-pair с FFmpeg `-af volume=`), и точность **декодированного** sample format: AAC/MP3 float не квантуется скрыто до 16-bit. `mix-audio OUTPUT a b [c...]` смешивает 2..=16 float-потоков с семантикой FFmpeg `amix=inputs=N:duration=shortest:dropout_transition=0` (`--normalize` по умолчанию, `--no-normalize` для суммы; `--weights` как FFmpeg `weights`, короткий список повторяет последний вес). `merge-audio OUTPUT a b` склеивает каналы двух float-потоков (FFmpeg `amerge=inputs=2`, shortest, сумма channel count). Для packed frame packet удерживает ссылку на буфер декодера. Для многоканального planar frame нужен interleave в PCM-пакет; mono передаётся по ссылке, как packed. PCM-буферы берутся из refcounted пула и возвращаются туда только после освобождения всех ссылок. Stereo 32-bit interleave на aarch64 использует NEON с сохранением всех битов; остальные размеры/платформы используют переносимый путь. число переставленных байтов записывается в `planar_interleave_bytes`. Формат, layout и rate не могут меняться посередине потока. Размер сформированного блока ограничивается packet budget; это не полный бюджет памяти декодера.

Экспорт формирует непрерывную последовательность сэмплов с начала WAV, без синтеза тишины для разрывов исходных PTS. `--from/--to` задаёт полуоткрытый интервал этой последовательности; обе границы округляются вверх до первого сэмпла, чьё время не меньше границы. Источники с главами пока отвергаются до реализации их отображения на такую шкалу. Проверены полные и интервальные AAC→float32, MP3→float32 с codec delay и FLAC→s16, побайтное равенство с PCM FFmpeg, а также AAC resample 48 kHz→44.1 kHz. Тот же AAC sample-window доступен внутри `transcode-lossless --from/--to` рядом с видео (без `--seek`).

[Аудиобенчмарк](../benchmarks/audio-benchmark.json) сравнивает одинаковую точность PCM на 30-секундных stereo 48 kHz fixtures. Production-код вызывает библиотеку напрямую; внешние FFmpeg-команды используются только для проверки и измерений.

### Проверка аудиооптимизаций

Добавлены тесты unaligned buffers, SIMD tails, byte guards, разных размеров сэмплов/числа каналов, удержания ссылок и padding пула. Отдельный stereo AAC fixture содержит разные сигналы в каналах. Скрипт `benchmark_audio.py --baseline PATH --runs 21 --duration 30` перемешивает новый Fvid, старый бинарник и FFmpeg в каждом раунде; полный и интервальный выход каждого варианта сверяется по PCM-сэмплам перед измерениями. Интервальные пары имеют строгий gate >15%. JSON сохраняет оба binary hashes и исходные замеры.
