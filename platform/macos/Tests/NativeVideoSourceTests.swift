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
        catch NativeVideoError.decodeFailed(let reason) { precondition(!reason.isEmpty) }
        do { _ = try source.frame(mediaTime: 0, hostTime: 10, sequence: 10); fatalError("failed handle reused") }
        catch NativeVideoError.decodeFailed(let reason) { precondition(!reason.isEmpty) }
        let missing = url.appendingPathExtension("missing")
        do { _ = try NativeVideoSource(url: missing); fatalError("missing input opened") }
        catch NativeVideoError.openFailed(let reason) { precondition(!reason.isEmpty); precondition(!reason.contains("openFailed")) }
        // A successful operation clears the thread's previous source diagnostic.
        let reopened = try NativeVideoSource(url: url)
        _ = try reopened.frame(mediaTime: 0, hostTime: 1, sequence: 0)
        precondition(fvid_camera_error(nil, 0) == 0)
        if CommandLine.arguments.count == 3 || CommandLine.arguments.count == 4 {
            let compressed = try NativeVideoSource(url: URL(fileURLWithPath: CommandLine.arguments[1]))
            let expectedRGB = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[2]))
            let frameSize = compressed.width * compressed.height * 3
            let frameCount = expectedRGB.count / frameSize
            precondition(frameCount >= 11 && expectedRGB.count % frameSize == 0)
            let timestamps: [UInt64]
            if CommandLine.arguments.count == 4 {
                timestamps = try JSONDecoder().decode([UInt64].self,
                    from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[3])))
                precondition(timestamps.count == frameCount)
            } else { timestamps = (0..<frameCount).map { UInt64($0) * 40_000_000 } }
            for (sequence, frameIndex) in [0,1,10,frameCount-1,0].enumerated() {
                let frame = try compressed.frame(mediaTime: timestamps[frameIndex],
                    hostTime: UInt64(sequence+1), sequence: UInt64(sequence))
                let rgb = expectedRGB.subdata(in: frameIndex*frameSize..<(frameIndex+1)*frameSize)
                var expected = Data()
                for index in stride(from: 0, to: rgb.count, by: 3) {
                    expected.append(contentsOf: [rgb[index+2],rgb[index+1],rgb[index],255])
                }
                precondition(frame == expected)
            }
            print("Swift/Rust MP4 bridge: first/next/middle/last/rewind frames match software RGB exactly")
        }
        print("Swift/Rust camera bridge: BGRA, seek, EOF, timestamp errors and failed-handle rejection passed")
    }
}
