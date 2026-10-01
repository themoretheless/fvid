import Foundation
import CoreMediaIO
import CoreMedia
import CoreVideo

// Platform bridge only. Container parsing and decoding belong to the Rust library.
// All lifecycle and submission calls must run on the provider's serial queue.
final class CameraStream: NSObject, CMIOExtensionStreamSource {
    private(set) var stream: CMIOExtensionStream!
    let formats: [CMIOExtensionStreamFormat]
    let width: Int
    let height: Int
    private let videoDescription: CMVideoFormatDescription
    private let duration: CMTime
    private let pixelPool: PixelPool
    private var clients = 0
    private var lastTime: UInt64?
    private var pendingDiscontinuity: CMIOExtensionStream.DiscontinuityFlags = []
    private let onSend: ((CMSampleBuffer, CMIOExtensionStream.DiscontinuityFlags, UInt64) -> Void)?

    init(width: Int, height: Int, fps: Int32,
         onSend: ((CMSampleBuffer, CMIOExtensionStream.DiscontinuityFlags, UInt64) -> Void)? = nil) throws {
        self.onSend = onSend
        guard width > 0, height > 0, width <= 4096, height <= 4096, fps > 0, fps <= 240 else {
            throw CameraError.invalidFormat
        }
        pixelPool = try PixelPool(width: width, height: height)
        self.width = width
        self.height = height
        duration = CMTime(value: 1, timescale: fps)
        var format: CMVideoFormatDescription?
        let status = CMVideoFormatDescriptionCreate(allocator: kCFAllocatorDefault,
            codecType: kCVPixelFormatType_32BGRA, width: Int32(width), height: Int32(height),
            extensions: nil, formatDescriptionOut: &format)
        guard status == noErr, let format else { throw CameraError.media(status) }
        videoDescription = format
        formats = [CMIOExtensionStreamFormat(formatDescription: format,
            maxFrameDuration: duration, minFrameDuration: duration, validFrameDurations: nil)]
        super.init()
        stream = CMIOExtensionStream(localizedName: "FVid Video", streamID: UUID(uuidString: "DEFA791E-FE00-4CC7-B92C-C88BF8C30141")!,
            direction: .source, clockType: .hostTime, source: self)
    }
    var availableProperties: Set<CMIOExtensionProperty> { [.streamActiveFormatIndex, .streamFrameDuration] }
    func streamProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionStreamProperties {
        let result = CMIOExtensionStreamProperties(dictionary: [:])
        result.activeFormatIndex = 0
        result.frameDuration = duration
        return result
    }
    func setStreamProperties(_ properties: CMIOExtensionStreamProperties) throws {
        if let index = properties.activeFormatIndex, index != 0 { throw CameraError.invalidFormat }
        if let value = properties.frameDuration, CMTimeCompare(value, duration) != 0 { throw CameraError.invalidFormat }
    }
    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool { true }
    func startStream() throws {
        if clients == 0 { pendingDiscontinuity.insert(.time) }
        clients += 1
    }
    func stopStream() throws { clients = max(0, clients - 1) }

    // Input hostTime must use CMClockGetHostTimeClock's nanosecond epoch.
    // No file timestamps or process-relative Instant values may be passed here.
    func submit(bgra: Data, hostTime: UInt64,
                discontinuity: CMIOExtensionStream.DiscontinuityFlags = []) throws -> Bool {
        guard bgra.count == width * height * 4, hostTime <= UInt64(Int64.max),
              lastTime.map({ hostTime > $0 }) ?? true else { throw CameraError.invalidFrame }
        guard clients > 0 else { return false }
        pendingDiscontinuity.formUnion(discontinuity)
        guard let image = try pixelPool.acquire() else {
            pendingDiscontinuity.insert(.sampleDropped)
            return false
        }
        guard CVPixelBufferLockBaseAddress(image, []) == kCVReturnSuccess else { throw CameraError.invalidFrame }
        guard let base = CVPixelBufferGetBaseAddress(image) else {
            CVPixelBufferUnlockBaseAddress(image, [])
            throw CameraError.invalidFrame
        }
        bgra.withUnsafeBytes { bytes in
            for row in 0..<height {
                base.advanced(by: row * CVPixelBufferGetBytesPerRow(image)).copyMemory(
                    from: bytes.baseAddress!.advanced(by: row * width * 4), byteCount: width * 4)
            }
        }
        CVPixelBufferUnlockBaseAddress(image, [])
        var timing = CMSampleTimingInfo(duration: duration,
            presentationTimeStamp: CMTime(value: Int64(hostTime), timescale: 1_000_000_000), decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        let sampleStatus = CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault,
            imageBuffer: image, formatDescription: videoDescription, sampleTiming: &timing, sampleBufferOut: &sample)
        guard sampleStatus == noErr, let sample else { throw CameraError.media(sampleStatus) }
        if let onSend { onSend(sample, pendingDiscontinuity, hostTime) }
        else { stream.send(sample, discontinuity: pendingDiscontinuity, hostTimeInNanoseconds: hostTime) }
        pendingDiscontinuity = []
        lastTime = hostTime
        return true
    }
}

final class CameraDevice: NSObject, CMIOExtensionDeviceSource {
    private(set) var device: CMIOExtensionDevice!
    let video: CameraStream
    let input: CameraSink
    init(width: Int, height: Int, fps: Int32, queue: DispatchQueue) throws {
        video = try CameraStream(width: width, height: height, fps: fps)
        input = CameraSink(output: video, fps: fps, queue: queue)
        super.init()
        device = CMIOExtensionDevice(localizedName: "FVid Camera",
            deviceID: UUID(uuidString: "5181C706-ED2D-4E69-B81B-53B50228E48B")!, legacyDeviceID: nil, source: self)
        try device.addStream(video.stream)
        try device.addStream(input.stream)
    }
    var availableProperties: Set<CMIOExtensionProperty> { [.deviceModel] }
    func deviceProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionDeviceProperties {
        let result = CMIOExtensionDeviceProperties(dictionary: [:]); result.model = "FVid Virtual Camera"; return result
    }
    func setDeviceProperties(_ properties: CMIOExtensionDeviceProperties) throws {}
}
final class CameraProvider: NSObject, CMIOExtensionProviderSource {
    private(set) var provider: CMIOExtensionProvider!
    let camera: CameraDevice
    let queue: DispatchQueue
    init(width: Int, height: Int, fps: Int32) throws {
        let queue = DispatchQueue(label: "fvid.camera.provider")
        self.queue = queue
        camera = try CameraDevice(width: width, height: height, fps: fps, queue: queue)
        super.init()
        provider = CMIOExtensionProvider(source: self, clientQueue: queue)
        try provider.addDevice(camera.device)
    }
    var availableProperties: Set<CMIOExtensionProperty> { [.providerManufacturer] }
    func providerProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionProviderProperties {
        let result = CMIOExtensionProviderProperties(dictionary: [:]); result.manufacturer = "FVid"; return result
    }
    func setProviderProperties(_ properties: CMIOExtensionProviderProperties) throws {}
    func connect(to client: CMIOExtensionClient) throws {}
    func disconnect(from client: CMIOExtensionClient) { camera.input.disconnected(client) }
}
