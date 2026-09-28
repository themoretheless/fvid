import Foundation

@main
struct CameraRepeatTests {
    static func main() throws {
        precondition(CommandLine.arguments.count == 2, "expected MP4 fixture")
        let source = try NativeVideoSource(url: URL(fileURLWithPath: CommandLine.arguments[1]))
        precondition(source.durationNS > 0)
        let clock = fvid_camera_clock_open(100)!
        defer { fvid_camera_clock_close(clock) }
        precondition(fvid_camera_clock_control(clock, 2, source.durationNS, 100) == 1)
        var tick = FVidCameraTick()
        func read(_ now: UInt64) throws -> Data {
            precondition(fvid_camera_clock_poll(clock, now, &tick) == 1)
            return try source.frame(mediaTime: tick.media_ns, hostTime: tick.host_ns, sequence: tick.sequence)
        }
        let first = try read(100)
        let middle = try read(100 + source.durationNS / 2)
        precondition(first != middle, "fixture must contain motion")
        let repeated = try read(100 + source.durationNS)
        precondition(tick.media_ns == 0)
        precondition(tick.host_ns == 100 + source.durationNS)
        precondition(first == repeated, "loop must return exact first-frame pixels")
        precondition(fvid_camera_clock_control(clock, 2, 0, 100 + source.durationNS) == 1)
        let held = try read(100 + source.durationNS * 3)
        let heldAgain = try read(100 + source.durationNS * 4)
        precondition(held == heldAgain, "disabled repeat must hold EOF")
        print("Native camera repeat/EOF pixel tests passed")
    }
}
