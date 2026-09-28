import Foundation
import CoreMediaIO
import CoreMedia
import CoreVideo

// CMIO owns the cross-process sample queue. Only one producer may feed this
// device; at most one asynchronous consumption request is outstanding.
final class CameraSink: NSObject, CMIOExtensionStreamSource {
    private(set) var stream: CMIOExtensionStream!
    private let output: CameraStream
    private let queue: DispatchQueue
    private var client: CMIOExtensionClient?
    private var timer: DispatchSourceTimer?
    private var generation: UInt64 = 0
    private var pending = false
    private var running = false
    private let fps: Int32
    var formats: [CMIOExtensionStreamFormat] { output.formats }

    init(output: CameraStream, fps: Int32, queue: DispatchQueue) {
        self.output = output; self.fps = fps; self.queue = queue
        super.init()
        stream = CMIOExtensionStream(localizedName: "FVid Input",
            streamID: UUID(uuidString: "5F7C9644-4C14-4BF0-9858-F82FC39DF449")!,
            direction: .sink, clockType: .hostTime, source: self)
    }
    var availableProperties: Set<CMIOExtensionProperty> {
        [.streamActiveFormatIndex, .streamFrameDuration, .streamSinkBufferQueueSize, .streamSinkBuffersRequiredForStartup]
    }
    func streamProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionStreamProperties {
        let result = CMIOExtensionStreamProperties(dictionary: [:])
        result.activeFormatIndex = 0
        result.frameDuration = CMTime(value: 1, timescale: fps)
        result.sinkBufferQueueSize = 3
        result.sinkBuffersRequiredForStartup = 1
        return result
    }
    func setStreamProperties(_ properties: CMIOExtensionStreamProperties) throws {
        if let index = properties.activeFormatIndex, index != 0 { throw CameraError.invalidFormat }
        if let duration = properties.frameDuration,
            CMTimeCompare(duration, CMTime(value: 1, timescale: fps)) != 0 { throw CameraError.invalidFormat }
        if let size = properties.sinkBufferQueueSize, size != 3 { throw CameraError.invalidFormat }
        if let count = properties.sinkBuffersRequiredForStartup, count != 1 { throw CameraError.invalidFormat }
    }
    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool {
        if let current = self.client { return current == client }
        self.client = client
        return true
    }
    func startStream() throws {
        guard client != nil else { throw CameraError.invalidFrame }
        guard !running else { return }
        running = true
        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(deadline: .now(), repeating: .nanoseconds((1_000_000_000 + Int(fps) - 1) / Int(fps)))
        timer.setEventHandler { [weak self] in self?.consume() }
        self.timer = timer
        timer.resume()
    }
    func stopStream() throws { stop() }
    func disconnected(_ client: CMIOExtensionClient) {
        if self.client == client { stop() }
    }
    private func stop() {
        running = false; generation &+= 1
        timer?.cancel(); timer = nil; client = nil
    }
    private func consume() {
        guard running, !pending, let client else { return }
        pending = true
        let requestGeneration = generation
        stream.consumeSampleBuffer(from: client) { [weak self] sample, sequence, _, _, error in
            guard let self else { return }
            self.queue.async {
                self.pending = false
                guard self.running, self.generation == requestGeneration else { return }
                guard error == nil, let sample else { return }
                do {
                    let bytes = try Self.copyBGRA(sample, width: self.output.width, height: self.output.height)
                    let now = CMClockGetTime(CMClockGetHostTimeClock())
                    let ns = CMTimeConvertScale(now, timescale: 1_000_000_000, method: .default).value
                    guard ns >= 0 else { throw CameraError.invalidFrame }
                    if try self.output.submit(bgra: bytes, hostTime: UInt64(ns)) {
                        self.stream.notifyScheduledOutputChanged(CMIOExtensionScheduledOutput(
                            sequenceNumber: sequence, hostTimeInNanoseconds: UInt64(ns)))
                    }
                } catch {
                    NSLog("FVid rejected input frame: %@", String(describing: error))
                }
            }
        }
    }
    static func copyBGRA(_ sample: CMSampleBuffer, width: Int, height: Int) throws -> Data {
        guard CMSampleBufferIsValid(sample), CMSampleBufferDataIsReady(sample),
              let image = CMSampleBufferGetImageBuffer(sample),
              CVPixelBufferGetPixelFormatType(image) == kCVPixelFormatType_32BGRA,
              CVPixelBufferGetWidth(image) == width, CVPixelBufferGetHeight(image) == height,
              CVPixelBufferGetBytesPerRow(image) >= width * 4 else { throw CameraError.invalidFrame }
        guard CVPixelBufferLockBaseAddress(image, .readOnly) == kCVReturnSuccess else { throw CameraError.invalidFrame }
        defer { CVPixelBufferUnlockBaseAddress(image, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(image) else { throw CameraError.invalidFrame }
        var bytes = Data(count: width * height * 4)
        bytes.withUnsafeMutableBytes { output in
            for row in 0..<height {
                output.baseAddress!.advanced(by: row * width * 4).copyMemory(
                    from: base.advanced(by: row * CVPixelBufferGetBytesPerRow(image)), byteCount: width * 4)
            }
        }
        return bytes
    }
}
