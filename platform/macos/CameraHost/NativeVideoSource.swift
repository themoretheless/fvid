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
        guard url.isFileURL, budget > 0 else { throw NativeVideoError.openFailed }
        let path = Array(url.path.utf8)
        let opened = path.withUnsafeBufferPointer { bytes in
            fvid_camera_open(bytes.baseAddress, bytes.count, budget)
        }
        guard let opened else { throw NativeVideoError.openFailed }
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
        if targetWidth == width && targetHeight == height { return input }
        let count = targetWidth * targetHeight * 4
        if scaled.count != count { scaled = Data(count: count) }
        let result = input.withUnsafeBytes { source in
            scaled.withUnsafeMutableBytes { destination in
                fvid_camera_fit(source.bindMemory(to: UInt8.self).baseAddress, input.count,
                    FVidCameraSize(width: UInt32(width), height: UInt32(height)),
                    destination.bindMemory(to: UInt8.self).baseAddress, count,
                    FVidCameraSize(width: UInt32(targetWidth), height: UInt32(targetHeight)))
            }
        }
        guard result == 1 else { throw NativeVideoError.decodeFailed }
        return scaled
    }
    deinit { fvid_camera_close(handle) }
    func frame(mediaTime: UInt64, hostTime: UInt64, sequence: UInt64) throws -> Data {
        let length = pixels.count
        let result = pixels.withUnsafeMutableBytes { bytes in
            fvid_camera_frame(handle, mediaTime, hostTime, sequence,
                bytes.bindMemory(to: UInt8.self).baseAddress, length)
        }
        guard result == 1 else { throw NativeVideoError.decodeFailed }
        return pixels
    }
}
enum NativeVideoError: Error { case openFailed, invalidDimensions, decodeFailed }
