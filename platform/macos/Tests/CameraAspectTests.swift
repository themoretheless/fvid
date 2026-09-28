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
        print("Camera pixel-aspect bridge tests passed")
    }
}
