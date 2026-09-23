# Анализ производительности кодеков

## Текущее состояние (2026-09-22)

### Недавние оптимизации (последние 10 коммитов)

**H.264 (AVC):**
- ✅ VideoToolbox hardware decoding на macOS
- ✅ Streaming reconstruction: парсер + рабочие потоки по рядам
- ✅ Parallel row bands для inter macroblocks
- ✅ Оптимизированная интерполяция движения с кэшированием
- ✅ YUV→RGB в fragment shader через GPU
- ✅ Zero-copy передача hardware-decoded planes на GPU

**HEVC:**
- ✅ Многопоточная reconstruction: N worker threads с row-level синхронизацией (добавлено 2026-09-22)
- ✅ WPP (Wavefront Parallel Processing) поддержка
- ✅ VideoToolbox hardware decoding на macOS (добавлено 2026-09-22)
- ✅ Row-based parallelism: row N ждет row N-1 для intra prediction dependencies
- ✅ Scratch buffer reuse: горизонтальные промежуточные буферы переиспользуются между блоками (добавлено 2026-09-22)
- ✅ Zero-residual оптимизация: пустые residual буферы не аллоцируются (добавлено 2026-09-22)
- ⚠️ Ограниченное распараллеливание: только при count >= 128*96

**VP9:**
- ⚠️ Однопоточный декодер (tile parallelism требует сложной рефакторизации)
- ❌ Нет tile-based parallelism
- ✅ YUV→RGB через GPU shader (Planar8 path)
- ✅ Scratch buffer reuse: промежуточные буферы интерполяции переиспользуются между блоками (добавлено 2026-09-22)
- ✅ Zero-residual оптимизация: пустые residual буферы не аллоцируются (добавлено 2026-09-22)

**AV1:**
- ⚠️ Однопоточный декодер (tile parallelism требует сложной рефакторизации)
- ❌ Тайлы реализованы, но не распараллелены
- ❌ Нет SIMD оптимизаций
- ✅ YUV→RGB через GPU shader (Planar8 path)
- ✅ Scratch buffer reuse: промежуточные буферы интерполяции переиспользуются между блоками (добавлено 2026-09-22)

**Y4M:**
- ✅ YUV→RGB через GPU shader (Planar8 path, добавлено 2026-09-22)

### Архитектура плеера

```
Decode Thread → Converter Thread → GPU Render
     ↓              ↓
  RawFrame      Planar8/RGB
```

- Decode thread: производит сырые кадры
- Converter thread: конвертирует в RGB пока декодируется следующий
- Throughput = slower(decode, convert), не сумма
- Очередь: 24 кадра (~400ms при 60Hz)

## Узкие места

### 1. ~~HEVC Decoder~~ ✅ Реализовано
**Было:** Один worker thread для reconstruction
**Решение:** Multiple worker threads (до N = available_parallelism) с row-level синхронизацией
**Результат:** Row N ждет row N-1 для intra dependencies, но N workers могут обрабатывать разные кадры

### 2. ~~GPU Pipeline для WebM~~ ✅ Реализовано
**Было:** VP9/AV1 и Y4M конвертировались в RGB на CPU
**Решение:** Все форматы теперь используют Planar8 → GPU shader path
**Результат:** Y4M → `RawFrame::Planar8` с BT.601 limited-range; WebM уже использовал Planar8

### 3. ~~VideoToolbox для HEVC~~ ✅ Реализовано
**Было:** Hardware decoding только для H.264
**Решение:** Добавлена HEVC поддержка в fvid-vt и интеграция в playback pipeline
**Результат:** `Session::new_hevc()` + `open_hardware_hevc()` в playback_mp4.rs

### 4. ~~HEVC Motion Compensation Allocations~~ ✅ Реализовано
**Было:** Per-block heap allocations в motion compensation hot path
**Решение:** Scratch buffer reuse pattern - горизонтальные промежуточные буферы переиспользуются между вызовами predict
**Результат:** Vec<i32> scratch persists across all predict calls within a row, growing to max needed size

### 5. ~~HEVC Zero-Residual Optimization~~ ✅ Реализовано
**Было:** `vec![0; n*n]` аллокация для блоков без residual данных
**Решение:** `reconstruct_intra` принимает пустые residual slices как special case (all zeros)
**Результат:** Избегаем unnecessary allocations для skip mode и intra blocks без coefficients

### 6. ~~VP9 Motion Compensation Allocations~~ ✅ Реализовано
**Было:** Per-block heap allocations в motion compensation hot path
**Решение:** Scratch buffer reuse pattern - промежуточные буферы интерполяции переиспользуются между вызовами
**Результат:** Vec<i32> scratch persists across all interpolate calls within a frame, growing to max needed size

### 7. ~~VP9 Zero-Residual Optimization~~ ✅ Реализовано
**Было:** `vec![0; size*size]` аллокация для блоков без residual данных
**Решение:** Empty Vec для skip blocks, проверка `residual.is_empty()` при применении
**Результат:** Избегаем unnecessary allocations для skip mode блоков

### 8. ~~AV1 Motion Compensation Allocations~~ ✅ Реализовано
**Было:** Per-block heap allocations в motion compensation hot path
**Решение:** Scratch buffer reuse pattern - промежуточные буферы интерполяции переиспользуются между вызовами
**Результат:** Vec<i32> scratch persists across all motion_samples calls within a frame, growing to max needed size

### 9. VP9/AV1 Tile Parallelism (Future Work)
**Проблема:** Тайлы декодируются последовательно
**Влияние:** Не используются доступные ядра CPU
**Сложность:** Высокая - требуется рефакторизация Decoder struct для изоляции состояния тайлов
**Решение:** Параллельное декодирование независимых тайлов с правильной синхронизацией

## Приоритеты оптимизации

### P0: Немедленное влияние
1. ~~**HEVC multi-threaded reconstruction**~~ ✅ - N worker threads с row synchronization
2. ~~**GPU YUV→RGB для всех форматов**~~ ✅ - все форматы через Planar8/GPU shader
3. ~~**HEVC allocation reduction**~~ ✅ - scratch buffer reuse + zero-residual optimization
4. ~~**VP9 allocation reduction**~~ ✅ - scratch buffer reuse + zero-residual optimization
5. ~~**AV1 allocation reduction**~~ ✅ - scratch buffer reuse for interpolation

### P1: Среднесрочные улучшения
6. ~~**VideoToolbox HEVC**~~ ✅ - hardware decoding на macOS (реализовано)
7. **VP9 tile parallelism** - параллельные тайлы
8. **AV1 tile parallelism** - параллельные тайлы

### P2: Долгосрочные оптимизации
9. **SIMD оптимизации** - для intra prediction, transform, interpolation

## Метрики успеха

- HEVC: 2-3x ускорение на 8+ ядерных CPU
- WebM: 30-50% снижение CPU usage через GPU conversion
- VideoToolbox HEVC: 5-10x ускорение vs software decode
- Tile parallelism: линейное ускорение до числа тайлов
