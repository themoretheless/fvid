import Foundation
import CoreMediaIO
import CoreMedia
import CoreVideo

// Call lifecycle and enqueue methods from one serial queue. The CMIO consumer
// may remove samples concurrently; the producer never overwrites a full queue.
final class CameraProducer {
    private let device: CMIODeviceID
    private let stream: CMIOStreamID
    private let buffers: CMSimpleQueue
    private let format: CMFormatDescription
    private var running = false
    private var lastTimestamp: CMTime?
    static let deviceUID = "5181C706-ED2D-4E69-B81B-53B50228E48B"

    init() throws {
        var found: CMIODeviceID?
        for device in try Self.objects(CMIOObjectID(kCMIOObjectSystemObject), selector: kCMIOHardwarePropertyDevices) {
            var address = Self.address(kCMIODevicePropertyDeviceUID)
            var uid: Unmanaged<CFString>?
            var size = UInt32(MemoryLayout.size(ofValue: uid))
            let status = CMIOObjectGetPropertyData(device, &address, 0, nil, size, &size, &uid)
            if status == noErr, let uid {
                let value = uid.takeRetainedValue() as String
                if value.caseInsensitiveCompare(Self.deviceUID) == .orderedSame { found = device; break }
            }
        }
        guard let device = found else { throw ProducerError.cameraNotInstalled }
        let streams = try Self.objects(device, selector: kCMIODevicePropertyStreams, scope: kCMIODevicePropertyScopeOutput)
        guard streams.count == 1, let stream = streams.first else { throw ProducerError.invalidStream }
        var queue: Unmanaged<CMSimpleQueue>?
        try Self.check(CMIOStreamCopyBufferQueue(stream, nil, nil, &queue))
        guard let queue else { throw ProducerError.invalidStream }
        let buffers = queue.takeRetainedValue()
        guard CMSimpleQueueGetCapacity(buffers) > 0, CMSimpleQueueGetCapacity(buffers) <= 3 else { throw ProducerError.invalidStream }
        var address = Self.address(kCMIOStreamPropertyFormatDescription)
        var description: Unmanaged<CMFormatDescription>?
        var size = UInt32(MemoryLayout.size(ofValue: description))
        try Self.check(CMIOObjectGetPropertyData(stream, &address, 0, nil, size, &size, &description))
        guard let description else { throw ProducerError.invalidStream }
        let format = description.takeRetainedValue()
        guard CMFormatDescriptionGetMediaSubType(format) == kCVPixelFormatType_32BGRA else { throw ProducerError.invalidStream }
        self.device = device; self.stream = stream; self.buffers = buffers; self.format = format
    }
    var dimensions: (Int, Int) {
        let size = CMVideoFormatDescriptionGetDimensions(format)
        return (Int(size.width), Int(size.height))
    }
    var canEnqueue: Bool { running && CMSimpleQueueGetCount(buffers) < CMSimpleQueueGetCapacity(buffers) }
    func start() throws {
        guard !running else { return }
        try Self.check(CMIODeviceStartStream(device, stream))
        running = true; lastTimestamp = nil
    }
    func stop() throws {
        guard running else { return }
        try Self.check(CMIODeviceStopStream(device, stream))
        running = false; lastTimestamp = nil
        // Once stopped, the producer owns any entries that CMIO did not consume.
        while let pointer = CMSimpleQueueDequeue(buffers) {
            Unmanaged<CMSampleBuffer>.fromOpaque(pointer).release()
        }
    }
    deinit { try? stop() }
    /// Returns false for backpressure. Success transfers one retained sample to CMIO.
    func enqueue(_ sample: CMSampleBuffer) throws -> Bool {
        guard running else { throw ProducerError.notRunning }
        try Self.validate(sample, format: format, after: lastTimestamp)
        let time = CMSampleBufferGetPresentationTimeStamp(sample)
        if !Self.enqueueRetained(sample, into: buffers) { return false }
        lastTimestamp = time
        return true
    }
    /// Shared admission check; testable without an installed CMIO device.
    static func validate(_ sample: CMSampleBuffer, format: CMFormatDescription, after lastTimestamp: CMTime?) throws {
        guard CMSampleBufferIsValid(sample), CMSampleBufferDataIsReady(sample),
              let image = CMSampleBufferGetImageBuffer(sample),
              CVPixelBufferGetPixelFormatType(image) == kCVPixelFormatType_32BGRA else { throw ProducerError.invalidFrame }
        let dimensions = CMVideoFormatDescriptionGetDimensions(format)
        guard CVPixelBufferGetWidth(image) == Int(dimensions.width),
              CVPixelBufferGetHeight(image) == Int(dimensions.height) else { throw ProducerError.invalidFrame }
        let time = CMSampleBufferGetPresentationTimeStamp(sample)
        guard time.isNumeric, time.value >= 0,
              lastTimestamp.map({ CMTimeCompare(time, $0) > 0 }) ?? true else { throw ProducerError.invalidFrame }
    }
    // CMSimpleQueue itself does not retain/release its opaque elements.
    static func enqueueRetained(_ sample: CMSampleBuffer, into queue: CMSimpleQueue) -> Bool {
        guard CMSimpleQueueGetCount(queue) < CMSimpleQueueGetCapacity(queue) else { return false }
        let owned = Unmanaged.passRetained(sample)
        if CMSimpleQueueEnqueue(queue, element: owned.toOpaque()) != noErr {
            owned.release()
            return false
        }
        return true
    }
    private static func address(_ selector: Int, scope: Int = kCMIOObjectPropertyScopeGlobal) -> CMIOObjectPropertyAddress {
        CMIOObjectPropertyAddress(mSelector: UInt32(selector), mScope: UInt32(scope), mElement: UInt32(kCMIOObjectPropertyElementMain))
    }
    private static func objects(_ object: CMIOObjectID, selector: Int, scope: Int = kCMIOObjectPropertyScopeGlobal) throws -> [CMIOObjectID] {
        var address = address(selector, scope: scope)
        var size: UInt32 = 0
        try check(CMIOObjectGetPropertyDataSize(object, &address, 0, nil, &size))
        guard size % 4 == 0, size <= 4096 * 4 else { throw ProducerError.invalidStream }
        var result = [CMIOObjectID](repeating: 0, count: Int(size) / 4)
        if size == 0 { return [] }
        let capacity = size
        try result.withUnsafeMutableBytes { bytes in
            try check(CMIOObjectGetPropertyData(object, &address, 0, nil, capacity, &size, bytes.baseAddress!))
        }
        guard size <= capacity, size % 4 == 0 else { throw ProducerError.invalidStream }
        return Array(result.prefix(Int(size) / 4))
    }
    private static func check(_ status: OSStatus) throws { if status != noErr { throw ProducerError.media(status) } }
}
enum ProducerError: Error { case cameraNotInstalled, invalidStream, invalidFrame, notRunning, media(OSStatus) }
