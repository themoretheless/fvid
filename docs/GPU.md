# GPU backends Fvid

Реализовано GPU-исполнение существующих Y4M-операций: crop, hflip, vflip и их объединённого прохода. Методы работают с 8-bit planar YUV420/422/444. Это слой обработки кадров; кодирование/декодирование H.264/HEVC/AV1, NVENC, VideoToolbox и VAAPI сюда ещё не входят.

## Пользовательские шейдеры

`--shader FILE.wgsl` добавляет программируемый planar-byte фильтр к GPU-этапу;
`--then` соединяет несколько таких этапов без промежуточного readback.
Плеер принимает отдельный RGB display shader, а также явный выбор GPU API
через `--backend` и адаптера через `--device`. Контракты функций, примеры и
проверки: [SHADERS.md](SHADERS.md).

## API и платформы

| Backend | Реализация | Целевая платформа | Установленный факт |
|---|---|---|---|
| `metal` | wgpu → Metal, integer-texture render pipeline | macOS | Выполнение на Apple M4 Max; 34 дифференциальных случая |
| `dx12` / `directx` / `d3d12` | wgpu → Direct3D 12, тот же shader | Windows | Выполнение на Windows + NVIDIA (RTX); см. benchmarks |
| `vulkan` | wgpu → Vulkan, тот же shader | Linux, Windows; Apple через MoltenVK | Cross-target compilation; нужен запуск на Vulkan GPU |
| `gl` / `opengl` / `gles` | wgpu → OpenGL/GLES, render pipeline без compute/storage buffers | Linux, Windows; Apple через ANGLE | Cross-target compilation; нужен запуск на GL/GLES GPU |
| `cuda` | cudarc → CUDA Driver + NVRTC 13, CUDA kernel | Linux, Windows с NVIDIA | Выполнение на Windows + NVIDIA (RTX 5090 / sm_120); Linux — компиляция + тот же код |
| `cpu` | Независимый safe-Rust проход | Проверен на macOS | Контрольный путь и default |
| `auto` | Выбор доступного backend при подготовке задания | Те же платформы | На M4 Max выбирает Metal; fallback CPU выводится в диагностике |

**Cross-target compilation означает проверку компилятором, а не запуск на целевой ОС.** DirectX в этой реализации — D3D12; D3D11 не реализован. OpenCL и браузерные WebGPU/WebGL в CLI не добавлены. Vulkan — дополнительный backend к перечисленным в запросе. Нельзя запустить CUDA на M4 Max и выдать результат Metal за CUDA-проверку.

`--list-devices` показывает обнаруженные аппаратные адаптеры. `status=available` означает обнаружение, а не полную квалификацию shader/driver или наличие NVRTC: создание конкретного задания дополнительно проверяет формат, лимиты, компилятор и устройство. Software adapters с DeviceType::Cpu исключены из списка wgpu GPU. Номер устройства относится к выбранному backend; выбирайте его из текущего списка, не переносите порядковый номер между машинами.

## Запуск

```sh
cargo build --release
./target/release/fvid --list-devices
./target/release/fvid input.y4m output.y4m --backend metal --device 0 --hflip
./target/release/fvid input.y4m output.y4m --backend cuda --crop 0:0:1280:720 --vflip
./target/release/fvid input.y4m output.y4m --backend directx
./target/release/fvid input.y4m output.y4m --backend opengl
./target/release/fvid input.y4m output.y4m --backend vulkan
```

Стандартная сборка включает `gpu` и `cuda`, с платформенными ограничениями. `cargo build --release --no-default-features` создаёт CPU-only вариант. Выборочно: `--no-default-features --features gpu` или `--features cuda`.

На Apple OpenGL-путь wgpu требует **ANGLE/EGL** и feature `angle`; это не обёртка над системным legacy OpenGL. Vulkan на Apple требует **MoltenVK** и feature `vulkan-portability`. Одного Cargo feature недостаточно без доступного runtime. Эти библиотеки автоматически не устанавливаются.

CUDA подключает драйвер и NVRTC динамически; SDK не требуется при сборке Rust. Во время работы нужны NVIDIA GPU и совместимый драйвер. Для встроенного multi-arch PTX NVRTC не требуется; **CUDA 13 NVRTC** (`nvrtc64_130_0.dll` / `libnvrtc.so.13`) нужен для fallback-компиляции, когда встроенный PTX не подходит. На Windows добавьте `CUDA\v13.*\bin` в `PATH`. Ошибки отсутствующего runtime, несовместимой версии compiler/driver и неизвестного device возвращаются пользователю. [Детали CUDA](../crates/fvid-cuda/README.md).

В Rust API старый `process` сохраняет CPU-семантику. Явный выбор:

```rust,ignore
let stats = fvid::process_with_options(
    reader, writer, transform, 256 * 1024 * 1024,
    fvid::ExecutionOptions { backend: fvid::Backend::Metal, device: 0 },
)?;
```

Результат содержит фактический backend, имя устройства и `controlled_memory_bytes`. Явный запрос `metal`/`cuda`/другого ускорителя никогда не переключается на CPU. Только `auto` пробует другие backend при подготовке, затем может выбрать CPU с записанными причинами. Ошибка во время уже начавшейся GPU-обработки прерывает задание; смешивания кадров разных backend и скрытого повторного запуска нет. Default остаётся `cpu`: наличие GPU не гарантирует выигрыш.

## Контракт реализации

Валидированный CPU Plan переводится в 32 значения u32: три plane descriptors, флаги отражений, входная/выходная длина и служебные поля. Каждая входная адресация проверяется до исполнения; GPU-индексы должны помещаться в 32 бита. CUDA повторно проверяет ABI на своей границе.

**wgpu:** байты YUV упакованы в RGBA8Uint textures, четыре независимых байта в texel. Fragment shader вычисляет четыре адреса исходных байтов и возвращает целочисленные значения. Здесь нет интерполяции, нормализации, sRGB-преобразований или float-округления данных пикселей. Один fullscreen triangle выполняет fused crop/отражения. GL не требует compute shaders: запрашиваются downlevel WebGL2-style limits и проверяется поддержка нужных RGBA8Uint usages.

Texture width увеличивается при необходимости, чтобы вместить крупный кадр в фактический `max_texture_dimension_2d`. Строки staging выровнены на 256 bytes. Padding обрабатывается явно, в выходной Y4M попадают только исходные payload bytes. Нечётные 444 размеры и границы byte lanes включены в проверки.

Device, queue, pipeline, bindings, input/output textures и upload/readback buffers создаются один раз на поток. В каждом кадре upload mapping → copy to texture → GPU render → copy to readback → CPU read. Map/submit/poll упорядочивают доступ. Буфер не читается до завершения GPU; mapped view уничтожается до unmap. Ошибки validation/OOM/internal и device-lost возвращаются через диагностику; map/poll имеет timeout. Таймаут ожидания не является обещанием принудительно остановить зависший драйвер.

**CUDA:** input/output/parameter device buffers and pinned host staging are reused. A process-wide pool keeps one `CudaContext` and loaded PTX modules per device ordinal (warm for MCP / repeated jobs in-process; one-shot CLI still pays driver/context once). One CUDA stream orders pinned→device upload → kernel → device→pinned download; synchronize completes work before returning. Each kernel thread writes its output byte. Unsafe is limited to the adapter (driver probes, pinned alloc, kernel launch); the root crate still forbids unsafe.

Y4M streaming API возвращает CPU-байты после каждого обработанного кадра. Новый `GpuPipeline` и CLI `--then` удерживают промежуточные кадры между отдельными фильтрами на GPU; в Rust API финальный download явный и необязательный. [Контракт resident-цепочки и проверки](GPU_RESIDENT.md). Resident API пока не импортирует поверхности декодера. Отдельный `media hw-filter` с feature `media-cuda` реализует NVDEC→CUDA NV12 crop/flip→NVENC без host frame copies; это не общий shader/codec граф. CUDA streaming использует два слота для перекрытия кадров. Metal decode→shader→encode без копирования поверхностей через CPU ещё не соединён.

## Память и завершение

`--memory-mib` проверяет контролируемые frame/staging allocations **до их создания**:

- CPU после добавления FrameView: `input_len`; при hflip — `input_len + output_len`.
- CUDA: `5 × (input_len + output_len) + 128` (caller host + two pinned/device slots for depth-2 overlap).
- wgpu: CPU input/output + две padded input-sized allocations (texture/upload) + две padded output-sized allocations (texture/readback) + 128 bytes uniform.

Эти формулы GPU относятся к одному этапу. Для `--then` учитываются также все промежуточные device frames и uniforms, без промежуточных host/staging кадров; полная формула приведена в GPU_RESIDENT.md.

Это payload budget, не гарантированный RSS/VRAM limit: driver allocations, pipeline compilation, internal command buffers и I/O overhead отдельно. Размеры проверяются также против adapter limits. На unified-memory GPU модель намеренно учитывает логические allocations, даже если физические страницы могут иметь особенности размещения.

GPU-ошибка не публикует частичный output file; сохраняется существующее поведение временного файла и no-clobber публикации (hard link, при недоступности — rename). Для stdout частичный поток возможен. Crash durability и произвольное восстановление GPU-контекста не обещаются.

## Проверки и воспроизведение

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo test --no-default-features
cargo test --test gpu_shader
cargo test --manifest-path crates/fvid-cuda/Cargo.toml
python3 scripts/validate_gpu.py --backends metal vulkan dx12 gl cuda auto --benchmark
```

Шесть offline-тестов проверяют WGSL без optional capabilities и перевод обеих стадий в GLSL 330, GLES 300, HLSL 5.1, SPIR-V 1.0 и MSL 1.2 с явными resource bindings. Они также запрещают compute/storage resources в этом shader. Это проверка трансляции, а не компиляция драйвером или выполнение на GPU.

Корневой crate со стандартными features прошёл `cargo check --all-targets` для `x86_64-unknown-linux-gnu` и `x86_64-pc-windows-msvc`; CUDA-адаптер отдельно прошёл check и clippy для обеих целей. На macOS дополнительно проверена сборка с `--all-features`, включая ANGLE и Vulkan portability. Эти проверки не запускают целевые runtime.

GPU validation должно выполняться в среде с доступом к физическому GPU. В sandbox этого хоста Metal не обнаруживался; настоящий запуск выполнялся вне sandbox. CPU-fallback не считается GPU-проверкой.

Скрипт сравнивает frame payload hashes с CPU и FFmpeg для 34 transform cases, включая повторные кадры, 420/422/444, нечётные 444, границы texture rows и 4K. Проверяет недоступный backend/device, недостаточный бюджет и непубликацию частичного результата. Дополнительные ignored CUDA-тесты нужно явно запускать на NVIDIA host, они не заменяются успешными macOS tests.

[Новые GPU-результаты](../benchmarks/GPU_REPORT.md) · [сырые измерения](../benchmarks/gpu-results.json). Старые CPU-снимки сохранены отдельно. CPU↔GPU transfers, setup и readback включены в новый CLI-бенчмарк; он не доказывает скорость resident-GPU pipeline или аппаратного кодирования.

## Источники API

- [wgpu 30.0.1: backends и optional translation layers](https://docs.rs/wgpu/30.0.1/wgpu/).
- [cudarc 0.19.9](https://docs.rs/cudarc/0.19.9/cudarc/).

Использование общего API даёт одну реализацию фильтра и платформенные адаптеры. Для заявления о проверенной поддержке каждой платформы нужны отдельные реальные runner/devices; таблица выше различает реализацию, компиляцию и выполненные проверки.
