# Матрица функциональности Fvid

Цель — расширение медиадвижка по проверяемым вертикальным сценариям. «Весь FFmpeg и все 632 проекта» не является достигнутым или конечным фиксированным набором функций: многие найденные проекты — кодеки, редакторы, streaming servers или компоненты. Ни metadata-инвентарь, ни наличие функции в подключённой библиотеке не считаются готовой функцией Fvid.

## Реализовано и проверено

| Возможность | Исполнитель | Проверенная граница |
|---|---|---|
| Y4M 420/422/444 8-bit | Rust | parse, crop, hflip, vflip, malformed input |
| CPU views без промежуточного кадра | Rust | crop/vflip; hflip CLI пока материализуется |
| Metal GPU-фильтры | Rust + wgpu | Реальный M4 Max, byte equality, 4K |
| Цепочки GPU-фильтров без промежуточного RAM | Rust + wgpu | Metal, lifetime/reuse/counters; без codec interop |
| CUDA / DX12 (Windows) | CUDA / wgpu adapters | RTX 5090: byte equality vs CPU/FFmpeg; resident CUDA chains |
| CUDA / Vulkan / DX12 / GL (прочее) | CUDA / wgpu adapters | Реализация; Linux/прочие GPU — отдельная квалификация |
| Цепочки GPU без промежуточного RAM (CUDA) | Rust + CUDA | Windows RTX 5090; lifetime/reuse/counters; без codec interop |
| NVDEC → CUDA NV12 filter → NVENC | FFmpeg CUDA + fvid-cuda | Windows RTX 5090; `media hw-filter`; host_frame_copies=0 |
| MCP-модуль | rmcp + Rust handlers | 11 tools, stdio/локальный HTTP, CPU/media/Metal; без публичного deployment |
| Probe обычного файла | native libavformat adapter | MP4/MKV, streams, codec, dimensions, exact timebase |
| Remux без encoder/decoder | native libavformat adapter | MP4→MKV, MP4+AAC→MP4, packet equality |
| Выбор/удаление дорожек | native adapter | `--streams`; PCM audio extraction to WAV |
| Lossless-обрезка внутри GOP | decode → выбор PTS → FFV1 | H.264 B-frames, crop/vflip, retiming глав, packed PCM packet slicing; opt-in seek только video-only |
| Обрезка по времени | native adapter + строгий Rust-контракт | H.264 IDR без B-frames; FFV1 keyframe; aligned PCM |
| Склейка | native adapter + строгий Rust-контракт | Совместимые H.264 / FFV1+PCM, точные seams |
| Декодирование H.264 / FFV1 | native libavcodec adapter | Для crop-lossless; включая B-frame drain |
| Явный видеокодек и параметры | native libavcodec | transcode: libx264 CRF0/28, VP9 lossless; прочие энкодеры ещё не квалифицированы |
| Lossless-экспорт | native libavcodec + Fvid plane views | FFV1/Matroska, полный кадр или crop+hflip+vflip; 8/10/16-bit samples и alpha |
| Декодирование аудио в PCM | native decoder + PCM packet adapter | AAC/MP3 float32, FLAC s16; codec delay, sample equality; без глав и синтеза gaps |
| Отдельная PCM-обрезка и извлечение | Native demux/mux + Rust packet view | trim-pcm WAV→WAV, MKV→WAV, строгий timestamp rescale |
| Обрезка PCM внутри пакета | Rust packet view | Mono s16le, stereo s24le/s32le/f32le, exact samples; без seek |
| Сохранение аудио при crop | packet remux | PCM samples и AAC payload/decoded audio |
| Главы и базовая metadata | native adapter | Remux и full-duration crop; lossless trim clip/rebase; stream-copy trim/concat глав пока reject |
| Отказ без частичного output | Rust + native adapter | Temporary file → no-clobber publication |
| Воспроизводимые бенчмарки | scripts + fixtures | Y4M CPU/Metal против FFmpeg; native remux/FFV1/crop+vflip отдельно в media-benchmark.json |

## Следующие блоки и критерии готовности

| Приоритет | Блок | Что требуется доказать | Источники подхода |
|---|---|---|---|
| P0 | Точные резы H.264/HEVC с B-frames | Decode dependencies, pre-roll, display order, независимые границы; no silent lossy | FFmpeg, Smelter, Vireo |
| P0 | Audio delay/padding и произвольные швы | Ни потерянных/повторных samples, ни drift; явный выбор режима | FFmpeg, Symphonia |
| P0 | Полный budget native backend | DPB/lookahead, packet queues, cancellation, long-file RSS | GStreamer, Membrane |
| P0 | VideoToolbox ↔ Metal surfaces | Реальный decode→filter→encode без host readback, ownership/fences | MetalPetal, Smelter |
| P1 | NVDEC/CUDA/NVENC | Вертикальный срез `media hw-filter` (H.264 CUDA decode → NV12 crop/flip → h264_nvenc); расширять профили/аудио | cros-codecs, BMF, NVIDIA APIs |
| P1 | Прочие decode profiles/форматы | Корпус HEVC/AV1/VP9/ProRes, 10/12-bit, alpha | FFmpeg, rust-av |
| P1 | Scale/rotate/transpose/pad | Точные геометрия/SAR/chroma/color contracts, CPU/GPU parity | libvips, Halide, FFmpeg |
| P1 | Audio filters/resample/mix | Format conversion, clipping policy, sample accuracy | Symphonia, Firewheel |
| P1 | Streaming encode и presets | Несколько encode backends; явные lossless/lossy quality gates | FFmpeg, Mediabunny |
| P1 | План задания и explain | Показ copy/reencode/transfer/materialization до исполнения | GStreamer, MetalPetal |
| P1 | Seek/cancel/progress | Неперепутанные поколения кадров, bounded cancellation | VapourSynth, Membrane |
| P2 | Temporal filters, denoise, interpolation | Lookahead, frame dependencies и bounded caches | VapourSynth, Halide |
| P2 | Overlay/compositing/transitions | Alpha/color space correctness, graph scheduling | MetalPetal, MLT |
| P2 | Subtitles: render/convert/timing | Fonts, shaping, timestamps, mixed encodings | FFmpeg, MLT |
| P2 | HDR/tone mapping/Dolby metadata | Color/metadata conformance corpus | FFmpeg, OpenColorIO |
| P2 | Timeline interchange / OTIO | Rational time roundtrip и media linking | OpenTimelineIO |
| P2 | Loudness/quality analysis | Эталонные измерения, воспроизводимость | FFmpeg, audio projects |
| P3 | HLS/DASH/CMAF, RTP/RTSP/SRT/WebRTC | Clock, reconnect, discontinuity, live drop policies | GStreamer, Pion, OvenMediaEngine |
| P3 | Capture/devices/hardware ingest | Платформенные устройства и clocks | GStreamer, FFmpeg |
| P3 | Thumbnail/preview/editor API | Demand-driven frames и UI cache policies | VapourSynth, Mediabunny |
| P3 | Distribution/services/batch DAG | Scheduling, persistence, retries, multi-job resource budget | Av1an, BMF |

P0 сохраняет текущий приоритет пользователя: обрезка/кроп/склейка без потерь и лишних копий в памяти. Таблица — roadmap, не созданные фоновые задания и не гарантия сроков.

## Как читать capabilities

`fvid media capabilities` перечисляет компоненты фактически установленного FFmpeg. В текущем снимке: 359 demuxers, 184 muxers, 527 decoders, 190 encoders, 481 filters. Это доступность **библиотеки**, а не 1741 реализованный и протестированный workflow Fvid. Наоборот, публичные операции Fvid перечислены в первой таблице, с их фактическими ограничениями.

[Native media](MEDIA.md) · [GPU resident](GPU_RESIDENT.md) · [Исследование 633 репозиториев](../research/REPORT.md) · [18 предметных разборов](../research/FINDINGS.md).
