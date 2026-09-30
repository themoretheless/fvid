import Foundation
import CoreMedia
import CoreVideo

@main struct CameraProducerTests {
    static func main() throws {
        var image: CVPixelBuffer?
        precondition(CVPixelBufferCreate(kCFAllocatorDefault, 2, 2, kCVPixelFormatType_32BGRA, nil, &image) == kCVReturnSuccess)
        var format: CMVideoFormatDescription?
        precondition(CMVideoFormatDescriptionCreateForImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: image!, formatDescriptionOut: &format) == noErr)
        var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: CameraFormat.framesPerSecond), presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        precondition(CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: image!, formatDescription: format!, sampleTiming: &timing, sampleBufferOut: &sample) == noErr)
        var created: CMSimpleQueue?
        precondition(CMSimpleQueueCreate(allocator: kCFAllocatorDefault, capacity: 3, queueOut: &created) == noErr)
        let queue = created!
        let buffer = sample!
        try CameraProducer.validate(buffer, format: format!, after: nil)
        func rejected(_ sample: CMSampleBuffer, format: CMFormatDescription, after: CMTime?) {
            do {
                try CameraProducer.validate(sample, format: format, after: after)
                preconditionFailure("Invalid camera sample accepted")
            } catch ProducerError.invalidFrame {} catch { preconditionFailure("Unexpected error: \(error)") }
        }
        rejected(buffer, format: format!, after: .zero)
        rejected(buffer, format: format!, after: CMTime(value: 1, timescale: 60))
        var wrongSize: CMVideoFormatDescription?
        precondition(CMVideoFormatDescriptionCreate(allocator: kCFAllocatorDefault,
            codecType: kCVPixelFormatType_32BGRA, width: 4, height: 2,
            extensions: nil, formatDescriptionOut: &wrongSize) == noErr)
        rejected(buffer, format: wrongSize!, after: nil)
        var unready: CMSampleBuffer?
        precondition(CMSampleBufferCreateForImageBuffer(allocator: kCFAllocatorDefault,
            imageBuffer: image!, dataReady: false, makeDataReadyCallback: nil, refcon: nil,
            formatDescription: format!, sampleTiming: &timing, sampleBufferOut: &unready) == noErr)
        rejected(unready!, format: format!, after: nil)
        var otherImage: CVPixelBuffer?
        precondition(CVPixelBufferCreate(kCFAllocatorDefault, 2, 2, kCVPixelFormatType_32ARGB,
            nil, &otherImage) == kCVReturnSuccess)
        var otherFormat: CMVideoFormatDescription?
        precondition(CMVideoFormatDescriptionCreateForImageBuffer(allocator: kCFAllocatorDefault,
            imageBuffer: otherImage!, formatDescriptionOut: &otherFormat) == noErr)
        var otherSample: CMSampleBuffer?
        precondition(CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault,
            imageBuffer: otherImage!, formatDescription: otherFormat!, sampleTiming: &timing,
            sampleBufferOut: &otherSample) == noErr)
        rejected(otherSample!, format: format!, after: nil)
        for stamp in [CMTime.invalid, CMTime.indefinite, CMTime(value: -1, timescale: 60)] {
            timing.presentationTimeStamp = stamp
            var bad: CMSampleBuffer?
            precondition(CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault,
                imageBuffer: image!, formatDescription: format!, sampleTiming: &timing,
                sampleBufferOut: &bad) == noErr)
            rejected(bad!, format: format!, after: nil)
        }
        let pointer = Unmanaged.passUnretained(buffer).toOpaque()
        for _ in 0..<3 { precondition(CameraProducer.enqueueRetained(buffer, into: queue)) }
        for _ in 0..<1000 { precondition(!CameraProducer.enqueueRetained(buffer, into: queue)) }
        precondition(CMSimpleQueueGetCount(queue) == 3)
        for _ in 0..<3 {
            let entry = CMSimpleQueueDequeue(queue)!
            precondition(entry == UnsafeRawPointer(pointer))
            let owned = Unmanaged<CMSampleBuffer>.fromOpaque(entry).takeRetainedValue()
            precondition(CMSampleBufferIsValid(owned))
        }
        precondition(CMSimpleQueueGetCount(queue) == 0)
        precondition(CameraProducer.enqueueRetained(buffer, into: queue))
        Unmanaged<CMSampleBuffer>.fromOpaque(CMSimpleQueueDequeue(queue)!).release()
        precondition(CMSampleBufferIsValid(buffer))
        precondition(CMSampleBufferInvalidate(buffer) == noErr)
        rejected(buffer, format: format!, after: nil)
        print("Camera producer: frame admission, monotonic timestamps, three-buffer capacity, backpressure, retained transfer and reuse passed")
    }
}
