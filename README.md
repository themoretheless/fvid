# Fvid

Исследовательский медиадвижок на Rust и архитектурный проект на основе обзора 633 медиарепозиториев.

- [Архитектура и решения](docs/ARCHITECTURE.md)
- [Матрица реализованных функций и roadmap](docs/FEATURE_MATRIX.md)
- [MP4/MKV: remux, обрезка, склейка и lossless-кроп](docs/MEDIA.md)
- [Приоритет: обрезка, кроп и склейка без потерь и лишних копий](docs/LOSSLESS_EDITING.md)
- [Обзор 632 проектов помимо FFmpeg: методика и ограничения](research/REPORT.md)
- [Каталог со ссылками](research/CATALOG.md) · [CSV](research/catalog-final.csv)
- [Что перенять из 18 предметно изученных проектов](research/FINDINGS.md)
- [GPU: Metal, CUDA, DirectX 12, Vulkan, OpenGL/GLES](docs/GPU.md)
- [Цепочки GPU-фильтров без промежуточного возврата в RAM](docs/GPU_RESIDENT.md)
- [Бенчмарки resident-цепочки против FFmpeg](benchmarks/RESIDENT_BENCHMARK.md)
- [Новые GPU-бенчмарки](benchmarks/GPU_REPORT.md) · [исходный CPU-бенчмарк](benchmarks/REPORT.md)

## Что уже работает

Самостоятельный Rust CLI и библиотека для Y4M без зависимости от FFmpeg. CPU-ядро дополнено GPU-backend через wgpu и отдельный CUDA-адаптер: 8-bit planar YUV420/422/444, crop, горизонтальное и вертикальное отражение. Один план объединяет операции, входной и выходной буферы переиспользуются. Лимит памяти (по умолчанию 256 MiB) учитывает CPU frame buffers и, при выборе GPU, текстуры и upload/readback buffers; это не строгий лимит RSS. Дополнительно используются I/O-буферы, заголовки и служебные структуры.

После первого GPU-снимка добавлен `FrameView`: CPU identity/crop/vflip записывают строки прямо из входного кадра без второго кадрового buffer. CPU hflip без сужения кадра разворачивает строки во входном буфере без второго кадра. Для hflip с сужением кропа сохранён более быстрый путь с выходным буфером; GPU также материализует выход. Адреса исходных строк и бюджет одного кадра проверяются тестами. Все бенчмарки относятся к явно зафиксированным версиям кода.

Необязательный `--features media` добавляет Rust-адаптер к native-библиотекам FFmpeg: probe, remux, выбор дорожек, строгую обрезку/склейку сжатых потоков и FFV1 lossless-экспорт полного кадра или crop+vflip с сохранением аудио. `fvid play` (то же, что `fvid media play`) открывает окно и проигрывает локальный файл: software decode, звук устройства вывода, пауза по Space, выход по Esc. Production-код не запускает FFmpeg CLI; кодеки здесь предоставлены библиотекой, а не переписаны на Rust. Проверены MP4/MKV, H.264/FFV1, PCM/AAC, 8/10/16-bit samples и alpha planes. Подробные ограничения находятся в docs/MEDIA.md.

```sh
cargo build --release --features media
./target/release/fvid media trim input.mp4 clip.mp4 --from 1 --to 2 --streams 0
./target/release/fvid media concat joined.mp4 first.mp4 second.mp4 --streams 0
./target/release/fvid media crop-lossless input.mp4 cropped.mkv --crop 2:2:640:360
./target/release/fvid media transcode-lossless input.mp4 flipped.mkv --vflip
./target/release/fvid play input.mp4
```

Добавлена resident GPU-цепочка: `--backend metal --hflip --then --crop 2:2:1280:720 --then --vflip`. Между этапами кадр остаётся на GPU, загрузка и выгрузка происходят только на границах Y4M. В Rust API выгрузка явная и необязательная. Реализован также CUDA-путь; на Windows + NVIDIA RTX 5090 проверены DX12/CUDA Y4M и resident CUDA. Codec-surface interop: вертикальный срез `cargo build --release --features media-cuda` → `fvid media hw-filter` (NVDEC→NV12 filter→NVENC, без host frame copies).

```sh
cargo build --release
./target/release/fvid --list-devices
./target/release/fvid input.y4m gpu-output.y4m --backend metal --hflip
./target/release/fvid input.y4m output.y4m --crop 0:0:1280:720 --hflip
./target/release/fvid input.y4m - --vflip > output.y4m
cargo test
cargo clippy --all-targets -- -D warnings
python3 scripts/benchmark.py
```

Crop задаётся `X:Y:WIDTH:HEIGHT` и применяется до отражений. Размеры и смещения должны соответствовать chroma subsampling. `-` обозначает stdin/stdout. Существующий output file никогда не перезаписывается. Для файлов результат сначала пишется во временный файл и публикуется через hard link в том же каталоге; если hard link недоступен (другой том, FAT и т.п.), используется rename. Это атомарная видимость на поддерживающих FS, без гарантии crash durability/fsync. При stdout ошибочный ввод может оставить частичный поток.

В собственном Y4M-движке явно interlaced-видео отвергается; progressive и unspecified обрабатываются как последовательность кадров. Заголовки ограничены 4096 bytes, plane sizes проверяются до allocation. Его форматы пока ограничены 8-bit planar YUV, без RGB, odd subsampled dimensions и аудио. Для обычных контейнеров/кодеков используется отдельный native media adapter. GPU codec interop и произвольный граф ещё не реализованы. Неизвестные Y4M header/frame tags сохраняются без понимания их семантики. Fvid пока не является полноценной заменой FFmpeg.

Исходный CPU-прототип проверен на 12 комбинациях; в его замерах были выигрыши и проигрыши относительно FFmpeg. Новый GPU-путь Metal сравнен побайтно с CPU и FFmpeg на 34 случаях. На этих простых фильтрах полный Metal-проход оказался в 2,11–4,71 раза медленнее CPU Fvid, поэтому CPU остаётся default. Другие GPU-backend прошли проверки компиляции, но ещё требуют запуска на соответствующем оборудовании. Полные команды, сырые измерения и ограничения находятся в benchmarks/ и docs/GPU.md.

## Структура

`src/lib.rs` — проверка Y4M, план и исполнение; `src/backend.rs` — выбор backend и устройств; `src/gpu/` — общий GPU shader и runtime; `crates/fvid-cuda/` — CUDA; `src/main.rs` — CLI и файловая публикация; `tests/` — контрактные проверки; `scripts/` — воспроизводимое исследование и бенчмарки. Большая архитектура из docs/ — следующий проектный этап, не описание уже реализованного графа.

Для воспроизведения исследования сначала восстановите raw-ответы по research/queries.json и исходный candidates.json snapshot. research_catalog.py использует индексы этого зафиксированного snapshot: нельзя применять его список исключений к заново отсортированному набору. fetch_research.py читает публичный GitHub через авторизованный `gh`, без запуска кода репозиториев. Скрипты fetch пропускают уже сохранённые README; для нового исследования используйте отдельный каталог, чтобы не смешивать снимки. finish_research.py и write_reports.py строят итоговые таблицы/отчёты из сохранённых данных.

Лицензия локального кода Fvid: MIT. Файлы research/sources и raw — материалы сторонних проектов со своими лицензиями, они не входят в сборку и не перелицензируются под MIT.

CPU-only сборка без GPU-зависимостей: `cargo build --release --no-default-features`. Default backend — CPU; `--backend auto` явно сообщает выбранный backend или причины fallback. Явный запрос GPU не подменяется CPU.

Опционально `--features airbug` подключает [airbug-err](https://github.com/themoretheless/airbug)/[airbug-otel](https://github.com/themoretheless/airbug) с ветки `release` (panic/error → локальный hub, OTLP). Отключить OTLP: `FVID_AIRBUG_OTEL=0`. Endpoint ошибок: `AIRBUG_ERR_ENDPOINT`.

CLI-бенч fvid vs FFmpeg через [airbug-bench](https://github.com/themoretheless/airbug) (`release`): **Y4M** (cpu / full / ffmpeg) + **media export** (fvid_cpu↔ffmpeg_cpu libx264 veryfast, fvid_gpu↔ffmpeg_gpu NVENC p1).

```sh
cargo build --release --features media-cuda
# bench may rebuild default features — pin the media-cuda binary:
cp target/release/fvid target/release/fvid-media-cuda   # Windows: Copy-Item
cargo build --release --no-default-features --target-dir target-cpu
FVID_BENCH_BIN_FULL=$PWD/target/release/fvid-media-cuda cargo bench --bench ffmpeg_compare --features media-cuda
```

Опционально: `FVID_BENCH_BIN_CPU`; `FVID_BENCH_SKIP_Y4M=1` / `FVID_BENCH_SKIP_MEDIA=1`. Y4M: 720p×180, 1080p×120, 4K×60. Media: 1080p×5s. Ops: copy/crop/hflip/vflip/fused.

## MCP

`cargo build --release --features mcp` добавляет `fvid mcp --root DIRECTORY`: 11 инструментов обработки через stdio или локальный Streamable HTTP с bearer-токеном. Прямые вызовы Rust API, новые выходные файлы без перезаписи, одна операция за раз. [Подключение, схемы и ограничения](docs/MCP.md) · [Пример конфигурации](examples/mcp/client.json). Публичный плагин ChatGPT и передача файлов из облака не настроены.

MCP поддерживает `--jobs 2` для ограниченной параллельной обработки независимых вызовов; по умолчанию одна операция. Транспорт остаётся асинхронным, лишние вызовы получают busy. Память и потоки кодеков расходуются на каждую операцию отдельно; подробности в [MCP.md](docs/MCP.md).
