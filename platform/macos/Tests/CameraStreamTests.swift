import Foundation
import CoreMedia
import CoreMediaIO

@main struct CameraStreamTests {
    static func main() throws {
        var held: [CMSampleBuffer] = []
        var flags: [CMIOExtensionStream.DiscontinuityFlags] = []
        var times: [UInt64] = []
        let output = try CameraStream(width: 2, height: 2, fps: 60) { sample, discontinuity, time in
            held.append(sample); flags.append(discontinuity); times.append(time)
        }
        let pixels = Data(repeating: 0, count: 16)
        precondition(trySubmit(output, pixels, 1) == false)
        try output.startStream()
        precondition(trySubmit(output, pixels, 2))
        precondition(flags.last == .time)
        precondition(trySubmit(output, pixels, 3))
        precondition(flags.last == [])
        precondition(trySubmit(output, pixels, 4))
        for time in UInt64(5)...100 {
            precondition(trySubmit(output, pixels, time) == false)
        }
        precondition(held.count == 3)
        held.removeFirst()
        precondition(trySubmit(output, pixels, 101))
        precondition(flags.last == .sampleDropped)
        held.removeAll()
        let propagated = try output.submit(bgra: pixels, hostTime: 102, discontinuity: .unknown)
        precondition(propagated)
        precondition(flags.last == .unknown)
        held.removeAll()
        try output.startStream() // A second consumer keeps the stream alive.
        try output.stopStream()
        precondition(trySubmit(output, pixels, 103))
        precondition(flags.last == [])
        held.removeAll()
        try output.stopStream()
        precondition(trySubmit(output, pixels, 104) == false)
        try output.startStream()
        precondition(trySubmit(output, pixels, 105))
        precondition(flags.last == .time)
        do {
            _ = try output.submit(bgra: pixels, hostTime: 103)
            fatalError("Restart accepted a reversed timestamp")
        } catch CameraError.invalidFrame {}
        precondition(times == [2, 3, 4, 101, 102, 103, 105])
        precondition(CMTimeCompare(CMSampleBufferGetDuration(held[0]), CMTime(value: 1, timescale: 60)) == 0)
        print("Camera stream: bounded backpressure, propagated discontinuities, multiple consumers and monotonic reconnect passed")
    }
    static func trySubmit(_ stream: CameraStream, _ pixels: Data, _ time: UInt64) -> Bool {
        try! stream.submit(bgra: pixels, hostTime: time)
    }
}
