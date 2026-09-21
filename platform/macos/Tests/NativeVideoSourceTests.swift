import Foundation

@main struct NativeVideoSourceTests {
    static func main() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("fvid-ffi-\(UUID().uuidString).y4m")
        defer { try? FileManager.default.removeItem(at: url) }
        var file = Data("YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\n".utf8)
        for y: UInt8 in [16,235] {
            file.append(Data("FRAME\n".utf8)); file.append(contentsOf: [y,y,y,y,128,128])
        }
        try file.write(to: url)
        let source = try NativeVideoSource(url: url)
        precondition(source.width == 2 && source.height == 2)
        for (sequence, pair) in [(UInt64(0),UInt8(0)),(40_000_000,255),(400_000_000,255),(0,0)].enumerated() {
            let pixels = try source.frame(mediaTime: pair.0, hostTime: UInt64(sequence+1), sequence: UInt64(sequence))
            let expected = Data(Array(repeating: [pair.1,pair.1,pair.1,255], count: 4).flatMap { $0 })
            precondition(pixels == expected)
        }
        do { _ = try source.frame(mediaTime: 0, hostTime: 1, sequence: 0); fatalError("stale timestamp accepted") }
        catch NativeVideoError.decodeFailed {}
        do { _ = try source.frame(mediaTime: 0, hostTime: 10, sequence: 10); fatalError("failed handle reused") }
        catch NativeVideoError.decodeFailed {}
        let missing = url.appendingPathExtension("missing")
        do { _ = try NativeVideoSource(url: missing); fatalError("missing input opened") }
        catch NativeVideoError.openFailed {}
        if CommandLine.arguments.count == 3 {
            let compressed = try NativeVideoSource(url: URL(fileURLWithPath: CommandLine.arguments[1]))
            let expectedRGB = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2]))
            let frameSize = compressed.width * compressed.height * 3
            for (sequence, frameIndex) in [0,1,10,0].enumerated() {
                let frame = try compressed.frame(mediaTime: UInt64(frameIndex) * 40_000_000,
                    hostTime: UInt64(sequence+1), sequence: UInt64(sequence))
                let rgb = expectedRGB.subdata(in: frameIndex*frameSize..<(frameIndex+1)*frameSize)
                var expected = Data()
                for index in stride(from: 0, to: rgb.count, by: 3) {
                    expected.append(contentsOf: [rgb[index+2],rgb[index+1],rgb[index],255])
                }
                precondition(frame == expected)
            }
            print("Swift/Rust MP4 bridge: frames 0/1/10/0 match native RGB exactly")
        }
        print("Swift/Rust camera bridge: BGRA, seek, EOF, timestamp errors and failed-handle rejection passed")
    }
}
