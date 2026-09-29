import Foundation

// Owns the Rust handle. All calls, including destruction, belong to the serial
// camera producer queue. Returned Data uses Swift copy-on-write ownership.
final class NativeVideoSource {
    private let handle: OpaquePointer
    let width: Int
    let height: Int
    let durationNS: UInt64
    private var scaled = Data()
    private var pixels: Data
    init(url: URL, budget: Int = 256 << 20) throws {
        guard url.isFileURL, budget > 0 else { throw NativeVideoError.openFailed("Expected a local file and a positive decoder budget") }
        let path = Array(url.path.utf8)
        let opened = path.withUnsafeBufferPointer { bytes in
            fvid_camera_open(bytes.baseAddress, bytes.count, budget)
        }
        guard let opened else { throw NativeVideoError.openFailed(nativeCameraDiagnostic("Cannot open video")) }
        let size = fvid_camera_size(opened)
        guard size.width > 0, size.height > 0,
              UInt64(size.width) * UInt64(size.height) * 4 <= 64 << 20 else {
            fvid_camera_close(opened)
            throw NativeVideoError.invalidDimensions
        }
        durationNS = fvid_camera_duration(opened)
        handle = opened
        width = Int(size.width); height = Int(size.height)
        pixels = Data(count: width * height * 4)
    }
    func cameraFrame(mediaTime: UInt64, hostTime: UInt64, sequence: UInt64, targetWidth: Int, targetHeight: Int) throws -> Data {
        guard targetWidth > 0, targetHeight > 0, targetWidth <= 4096, targetHeight <= 4096 else { throw NativeVideoError.invalidDimensions }
        let input = try frame(mediaTime: mediaTime, hostTime: hostTime, sequence: sequence)
        let count = targetWidth * targetHeight * 4
        if scaled.count != count { scaled = Data(count: count) }
        let result = input.withUnsafeBytes { source in
            scaled.withUnsafeMutableBytes { destination in
                fvid_camera_fit_source(handle, source.bindMemory(to: UInt8.self).baseAddress, input.count,
                    destination.bindMemory(to: UInt8.self).baseAddress, count,
                    FVidCameraSize(width: UInt32(targetWidth), height: UInt32(targetHeight)))
            }
        }
        guard result == 1 else { throw NativeVideoError.decodeFailed(nativeCameraDiagnostic("Cannot decode or convert video frame")) }
        return scaled
    }
    deinit { fvid_camera_close(handle) }
    func frame(mediaTime: UInt64, hostTime: UInt64, sequence: UInt64) throws -> Data {
        let length = pixels.count
        let result = pixels.withUnsafeMutableBytes { bytes in
            fvid_camera_frame(handle, mediaTime, hostTime, sequence,
                bytes.bindMemory(to: UInt8.self).baseAddress, length)
        }
        guard result == 1 else { throw NativeVideoError.decodeFailed(nativeCameraDiagnostic("Cannot decode or convert video frame")) }
        return pixels
    }
}
private func nativeCameraDiagnostic(_ fallback: String) -> String {
    let count = fvid_camera_error(nil, 0)
    guard count > 0 && count <= 4096 else { return fallback }
    var bytes = [UInt8](repeating: 0, count: count)
    let actual = bytes.withUnsafeMutableBufferPointer { buffer in
        fvid_camera_error(buffer.baseAddress, buffer.count)
    }
    guard actual == count else { return fallback }
    return String(bytes: bytes, encoding: .utf8) ?? fallback
}
enum NativeVideoError: LocalizedError, CustomStringConvertible {
    case openFailed(String), invalidDimensions, decodeFailed(String)
    var description: String {
        switch self {
        case .openFailed(let reason): return "Cannot open video: \(reason)"
        case .invalidDimensions: return "Invalid or oversized video dimensions"
        case .decodeFailed(let reason): return "Cannot read video frame: \(reason)"
        }
    }
    var errorDescription: String? { description }
}
