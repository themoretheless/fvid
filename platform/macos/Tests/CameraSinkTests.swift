import Foundation
import CoreMedia
import CoreVideo

@main struct CameraSinkTests {
    static func main() throws {
        var image: CVPixelBuffer?
        let attributes = [kCVPixelBufferBytesPerRowAlignmentKey: 64] as CFDictionary
        let status = CVPixelBufferCreate(kCFAllocatorDefault, 3, 2, kCVPixelFormatType_32BGRA, attributes, &image)
        guard status == kCVReturnSuccess, let image else { fatalError("create pixel buffer: \(status)") }
        precondition(CVPixelBufferGetBytesPerRow(image) > 12)
        precondition(CVPixelBufferLockBaseAddress(image, []) == kCVReturnSuccess)
        let base = CVPixelBufferGetBaseAddress(image)!
        for row in 0..<2 {
            memset(base.advanced(by: row * CVPixelBufferGetBytesPerRow(image)), 0xee, CVPixelBufferGetBytesPerRow(image))
            for x in 0..<12 {
                base.storeBytes(of: UInt8(row * 12 + x), toByteOffset: row * CVPixelBufferGetBytesPerRow(image) + x, as: UInt8.self)
            }
        }
        CVPixelBufferUnlockBaseAddress(image, [])
        var format: CMVideoFormatDescription?
        precondition(CMVideoFormatDescriptionCreateForImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: image,
            formatDescriptionOut: &format) == noErr)
        var timing = CMSampleTimingInfo(duration: CMTime(value: 1, timescale: 30),
            presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        precondition(CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: image,
            formatDescription: format!, sampleTiming: &timing, sampleBufferOut: &sample) == noErr)
        let data = try CameraSink.copyBGRA(sample!, width: 3, height: 2)
        precondition(data == Data((0..<24).map(UInt8.init)))
        do { _ = try CameraSink.copyBGRA(sample!, width: 4, height: 2); fatalError("wrong geometry accepted") }
        catch CameraError.invalidFrame {}
        var argb: CVPixelBuffer?
        precondition(CVPixelBufferCreate(kCFAllocatorDefault, 3, 2, kCVPixelFormatType_32ARGB, attributes, &argb) == kCVReturnSuccess)
        var argbFormat: CMVideoFormatDescription?
        precondition(CMVideoFormatDescriptionCreateForImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: argb!,
            formatDescriptionOut: &argbFormat) == noErr)
        var argbSample: CMSampleBuffer?
        precondition(CMSampleBufferCreateReadyWithImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: argb!,
            formatDescription: argbFormat!, sampleTiming: &timing, sampleBufferOut: &argbSample) == noErr)
        do { _ = try CameraSink.copyBGRA(argbSample!, width: 3, height: 2); fatalError("ARGB accepted as BGRA") }
        catch CameraError.invalidFrame {}
        CMSampleBufferInvalidate(sample!)
        do { _ = try CameraSink.copyBGRA(sample!, width: 3, height: 2); fatalError("invalid sample accepted") }
        catch CameraError.invalidFrame {}
        print("Camera sink: padded BGRA rows, geometry, format and invalid sample checks passed")
    }
}
