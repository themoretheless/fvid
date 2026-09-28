import Foundation
import CoreMedia
import CoreVideo

final class CameraFrameBuilder {
    let width: Int
    let height: Int
    private let pool: PixelPool
    private let format: CMVideoFormatDescription
    init(width: Int, height: Int) throws {
        self.width = width; self.height = height
        pool = try PixelPool(width: width, height: height)
        var description: CMVideoFormatDescription?
        let status = CMVideoFormatDescriptionCreate(allocator: kCFAllocatorDefault,
            codecType: kCVPixelFormatType_32BGRA, width: Int32(width), height: Int32(height),
            extensions: nil, formatDescriptionOut: &description)
        guard status == noErr, let description else { throw CameraError.media(status) }
        format = description
    }
    func sample(_ pixels: Data, hostTime: UInt64) throws -> CMSampleBuffer? {
        guard pixels.count == width * height * 4, hostTime <= UInt64(Int64.max) else { throw CameraError.invalidFrame }
        guard let image = try pool.acquire() else { return nil }
        guard CVPixelBufferLockBaseAddress(image, []) == kCVReturnSuccess else { throw CameraError.invalidFrame }
        guard let base = CVPixelBufferGetBaseAddress(image) else {
            CVPixelBufferUnlockBaseAddress(image, []); throw CameraError.invalidFrame
        }
        pixels.withUnsafeBytes { bytes in
            for row in 0..<height {
                base.advanced(by: row * CVPixelBufferGetBytesPerRow(image)).copyMemory(
                    from: bytes.baseAddress!.advanced(by: row * width * 4), byteCount: width * 4)
            }
        }
        CVPixelBufferUnlockBaseAddress(image, [])
        var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30),
            presentationTimeStamp: CMTime(value: Int64(hostTime), timescale: 1_000_000_000), decodeTimeStamp: .invalid)
        var result: CMSampleBuffer?
        let status = CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: image,
            formatDescription: format, sampleTiming: &timing, sampleBufferOut: &result)
        guard status == noErr, let result else { throw CameraError.media(status) }
        return result
    }
}

final class CameraSession {
    private let queue = DispatchQueue(label: "fvid.camera.playback")
    private var source: NativeVideoSource?
    private var producer: CameraProducer?
    private var builder: CameraFrameBuilder?
    private var timer: DispatchSourceTimer?
    private var clock: OpaquePointer?
    private var paused = false
    var onStatus: ((String) -> Void)?
    func start(url: URL) {
        queue.async { [weak self] in
            guard let self else { return }
            do {
                try self.stopOnQueue()
                let source = try NativeVideoSource(url: url)
                let producer = try CameraProducer()
                let (width,height) = producer.dimensions
                let builder = try CameraFrameBuilder(width: width, height: height)
                try producer.start()
                self.source = source; self.producer = producer; self.builder = builder
                self.clock = fvid_camera_clock_open(try Self.hostTime())
                guard self.clock != nil else { throw CameraError.invalidFrame }
                self.paused = false
                let timer = DispatchSource.makeTimerSource(queue: self.queue)
                timer.schedule(deadline: .now(), repeating: .nanoseconds(1_000_000_000 / 30))
                timer.setEventHandler { [weak self] in self?.tick() }
                self.timer = timer; timer.resume()
                self.report("Sending \(url.lastPathComponent) to FVid Camera. Select FVid Camera in your video application.")
            } catch {
                try? self.stopOnQueue()
                self.report("Cannot start camera video: \(error)")
            }
        }
    }
    func stop() {
        queue.async { [weak self] in
            guard let self else { return }
            do { try self.stopOnQueue(); self.report("Camera video stopped.") }
            catch { self.report("Cannot stop camera stream: \(error)") }
        }
    }
    func togglePause() {
        queue.async { [weak self] in
            guard let self, let clock = self.clock else { return }
            do {
                guard fvid_camera_clock_control(clock, 1, self.paused ? 0 : 1, try Self.hostTime()) == 1 else { throw CameraError.invalidFrame }
                self.paused.toggle()
                self.report(self.paused ? "Video paused; camera holds the current frame." : "Camera video resumed.")
            } catch { self.report("Cannot pause camera video: \(error)") }
        }
    }
    func restart() {
        queue.async { [weak self] in
            guard let self, let clock = self.clock else { return }
            do {
                guard fvid_camera_clock_control(clock, 0, 0, try Self.hostTime()) == 1 else { throw CameraError.invalidFrame }
                self.report("Camera video restarted.")
            } catch { self.report("Cannot restart camera video: \(error)") }
        }
    }
    func shutdown() { queue.sync { try? stopOnQueue() } }
    private func stopOnQueue() throws {
        timer?.cancel(); timer = nil
        fvid_camera_clock_close(clock); clock = nil
        try producer?.stop()
        producer = nil; source = nil; builder = nil
    }
    private func tick() {
        guard let source, let producer, let builder, let clock, producer.canEnqueue else { return }
        do {
            let now = try Self.hostTime()
            var tick = FVidCameraTick()
            let result = fvid_camera_clock_poll(clock, now, &tick)
            guard result >= 0 else { throw CameraError.invalidFrame }
            guard result == 1 else { return }
            let pixels = try source.cameraFrame(mediaTime: tick.media_ns, hostTime: tick.host_ns,
                sequence: tick.sequence, targetWidth: builder.width, targetHeight: builder.height)
            if let sample = try builder.sample(pixels, hostTime: now) { _ = try producer.enqueue(sample) }
        } catch {
            try? stopOnQueue(); report("Camera video failed: \(error)")
        }
    }
    private static func hostTime() throws -> UInt64 {
        let value = CMTimeConvertScale(CMClockGetTime(CMClockGetHostTimeClock()), timescale: 1_000_000_000, method: .default).value
        guard value >= 0 else { throw CameraError.invalidFrame }
        return UInt64(value)
    }
    private func report(_ text: String) { DispatchQueue.main.async { [weak self] in self?.onStatus?(text) } }
}
