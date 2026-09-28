import Foundation

@main
struct CameraAspectTests {
    static func main() throws {
        let path = FileManager.default.temporaryDirectory.appendingPathComponent("fvid-aspect-\(UUID().uuidString).y4m")
        defer { try? FileManager.default.removeItem(at: path) }
        var data = Data("YUV4MPEG2 W2 H2 F30:1 Ip A2:1 C420\nFRAME\n".utf8)
        data.append(contentsOf: [235, 235, 235, 235, 128, 128])
        try data.write(to: path, options: .withoutOverwriting)
        let source = try NativeVideoSource(url: path)
        let frame = try source.cameraFrame(mediaTime: 0, hostTime: 1, sequence: 0, targetWidth: 4, targetHeight: 4)
        for y in 0..<4 {
            for x in 0..<4 {
                let at = (y * 4 + x) * 4
                let expected: UInt8 = (y == 0 || y == 3) ? 0 : 255
                precondition(frame[at] == expected && frame[at+1] == expected && frame[at+2] == expected)
                precondition(frame[at+3] == 255)
            }
        }
        // Same coded/output dimensions must still apply nonsquare pixel aspect.
        let small = try source.cameraFrame(mediaTime: 0, hostTime: 2, sequence: 1, targetWidth: 2, targetHeight: 2)
        precondition(small[0] == 255 && small[8] == 0)
        precondition(CommandLine.arguments.count == 2, "expected MP4 pixel-aspect fixture")
        var movie = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
        let matrix = movie.range(of: Data("tkhd".utf8))!.lowerBound + 4 + 40
        for (offset, value): (Int, UInt32) in [(0, 0), (4, 0x0001_0000), (12, 0xFFFF_0000), (16, 0)] {
            movie.replaceSubrange((matrix+offset)..<(matrix+offset+4), with:
                [UInt8(truncatingIfNeeded: value >> 24), UInt8(truncatingIfNeeded: value >> 16),
                 UInt8(truncatingIfNeeded: value >> 8), UInt8(truncatingIfNeeded: value)])
        }
        let moviePath = path.deletingPathExtension().appendingPathExtension("mp4")
        defer { try? FileManager.default.removeItem(at: moviePath) }
        try movie.write(to: moviePath, options: .withoutOverwriting)
        let rotated = try NativeVideoSource(url: moviePath)
        precondition(rotated.width == 64 && rotated.height == 64)
        let raw = try rotated.frame(mediaTime: 0, hostTime: 1, sequence: 0)
        let fitted = try rotated.cameraFrame(mediaTime: 0, hostTime: 2, sequence: 1, targetWidth: 64, targetHeight: 64)
        for y in 0..<64 {
            for x in 0..<64 {
                let at = (y * 64 + x) * 4
                if x < 16 || x >= 48 {
                    precondition(Array(fitted[at..<(at+4)]) == [0, 0, 0, 255])
                } else {
                    let original = (y * 64 + (x - 16) * 2) * 4
                    precondition(fitted[at..<(at+4)] == raw[original..<(original+4)])
                }
            }
        }
        print("Camera pixel-aspect and rotated MP4 bridge tests passed")
    }
}
