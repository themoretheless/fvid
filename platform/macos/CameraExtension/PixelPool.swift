import CoreVideo
import Foundation

/// All allocations use the threshold, including the first one. Pool minimum
/// buffer count is a reuse hint; it is not used as a memory cap.
final class PixelPool {
    private let pool: CVPixelBufferPool
    private let auxiliary: CFDictionary
    init(width: Int, height: Int, capacity: Int = 3) throws {
        guard width > 0, height > 0, width <= 4096, height <= 4096,
              capacity > 0, capacity <= 8 else { throw CameraError.invalidFormat }
        var created: CVPixelBufferPool?
        let attributes: [String: Any] = [
            kCVPixelBufferWidthKey as String: width,
            kCVPixelBufferHeightKey as String: height,
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA,
            kCVPixelBufferIOSurfacePropertiesKey as String: [:]
        ]
        let status = CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attributes as CFDictionary, &created)
        guard status == kCVReturnSuccess, let created else { throw CameraError.media(status) }
        pool = created
        auxiliary = [kCVPixelBufferPoolAllocationThresholdKey: capacity] as CFDictionary
    }
    /// Exhaustion drops this frame; it must not fall back to unbounded allocation.
    func acquire() throws -> CVPixelBuffer? {
        var pixel: CVPixelBuffer?
        let status = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(kCFAllocatorDefault, pool, auxiliary, &pixel)
        if status == kCVReturnWouldExceedAllocationThreshold { return nil }
        guard status == kCVReturnSuccess, let pixel else { throw CameraError.media(status) }
        return pixel
    }
}

enum CameraError: Error { case invalidFormat, invalidFrame, media(OSStatus) }
