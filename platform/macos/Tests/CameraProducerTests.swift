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
        print("Camera producer: three-buffer capacity, backpressure, retained transfer and reuse passed")
    }
}
