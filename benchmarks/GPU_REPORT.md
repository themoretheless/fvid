# GPU: измеренные результаты

Исторический снимок до добавления CPU FrameView и прямой записи строк. Сохранённые числа и хеши не обновлялись под новый код; они не являются замером текущей версии CPU-конвейера.

Apple M4 Max, Metal, macOS. Доступ к настоящему GPU подтверждён вне sandbox. Снимок выполнен 2026-09-04T22:46:27.507656+00:00.

**Корректность:** 34 transform cases: 24 малых комбинации и 10 случаев больших кадров/границ текстур. Payload каждого кадра совпал у CPU, Metal и FFmpeg; `auto` выбирал Metal. Дополнительно прошли 14 проверок отказов, бюджета и непубликации частичного файла.

**Доступность (исторический снимок):** CUDA, D3D12, Vulkan и GL на той машине не выполнялись.

## Windows NVIDIA qualification (2026-09-14)

Host: Windows + NVIDIA GeForce RTX 5090, CUDA Toolkit 13.4 NVRTC (`nvrtc64_130_0.dll`), MSVC, Rust 1.98.1.

**Корректность:** `scripts/validate_gpu.py --backends dx12 cuda` — payload equality CPU / DX12 / CUDA / FFmpeg на 34 transform cases + failure/atomic publication checks. Report: [gpu-results-windows.json](gpu-results-windows.json).

**Resident:** `scripts/validate_resident.py --backend cuda` — 8 CLI cases vs CPU/FFmpeg + API ignored test. Report: [resident-results-windows.json](resident-results-windows.json).

Перед запуском: `scripts/use_cuda_windows.ps1` или CUDA `v13.*\bin` в `PATH`.

## Время полной обработки (исторический Metal)

См. прежнюю таблицу в git history / Metal snapshot в [gpu-results.json](gpu-results.json). GPU не включён по умолчанию из-за CPU→GPU→CPU overhead на дешёвых фильтрах.

## Воспроизведение

```sh
cargo build --release
python scripts/validate_gpu.py --backends dx12 cuda
python scripts/validate_resident.py --backend cuda
```

- [Windows DX12/CUDA](gpu-results-windows.json) · [Windows resident CUDA](resident-results-windows.json)
- [Сырые результаты Metal](gpu-results.json)
