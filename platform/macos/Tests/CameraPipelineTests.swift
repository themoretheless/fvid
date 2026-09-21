import Foundation
import CoreMedia

@main struct CameraPipelineTests {
    static func main() throws {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent("fvid-pipeline-\(UUID().uuidString).y4m")
        defer { try? FileManager.default.removeItem(at: url) }
        var file = Data("YUV4MPEG2 W2 H2 F25:1 Ip C420jpeg\nFRAME\n".utf8)
        file.append(contentsOf: [235,235,235,235,128,128]); try file.write(to: url)
        let source = try NativeVideoSource(url: url)
        let pixels = try source.cameraFrame(mediaTime: 0, hostTime: 1, sequence: 0, targetWidth: 4, targetHeight: 2)
        let row: [UInt8] = [0,0,0,255,255,255,255,255,255,255,255,255,0,0,0,255]
        precondition(pixels == Data(row + row))
        let builder = try CameraFrameBuilder(width: 4, height: 2)
        var samples: [CMSampleBuffer] = []
        for time in UInt64(1)...3 { samples.append(try builder.sample(pixels, hostTime: time)!) }
        for _ in 0..<100 { let extra = try builder.sample(pixels, hostTime: 4); precondition(extra == nil) }
        let unpacked = try CameraSink.copyBGRA(samples[0], width: 4, height: 2)
        precondition(unpacked == pixels)
        precondition(CMTimeCompare(CMSampleBufferGetPresentationTimeStamp(samples[0]), CMTime(value: 1, timescale: 1_000_000_000)) == 0)
        samples.removeLast()
        let reused = try builder.sample(pixels, hostTime: 5)
        precondition(reused != nil)
        print("Camera pipeline: Rust decode/letterbox -> Swift sample -> sink BGRA, timestamps and bounded pool passed")
    }
}
