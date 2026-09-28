import Foundation

@main
struct CameraClockTests {
    static func main() {
        let clock = fvid_camera_clock_open(100)!
        defer { fvid_camera_clock_close(clock) }
        var tick = FVidCameraTick()
        assert(fvid_camera_clock_poll(clock, 100, &tick) == 1)
        assert(tick.media_ns == 0 && tick.host_ns == 100)
        assert(fvid_camera_clock_poll(clock, 101, &tick) == 0)
        assert(fvid_camera_clock_control(clock, 1, 1, 50_000_100) == 1)
        assert(fvid_camera_clock_poll(clock, 100_000_100, &tick) == 1)
        assert(tick.media_ns == 50_000_000 && tick.sequence == 3)
        assert(fvid_camera_clock_poll(clock, 200_000_100, &tick) == 1)
        assert(tick.media_ns == 50_000_000 && tick.host_ns == 200_000_100)
        assert(fvid_camera_clock_control(clock, 0, 0, 200_000_100) == 1)
        assert(fvid_camera_clock_poll(clock, 300_000_100, &tick) == 1)
        assert(tick.media_ns == 0 && tick.sequence == 9)
        assert(fvid_camera_clock_control(clock, 1, 0, 300_000_100) == 1)
        assert(fvid_camera_clock_control(clock, 2, 150_000_000, 300_000_100) == 1)
        assert(fvid_camera_clock_poll(clock, 500_000_100, &tick) == 1)
        assert(tick.media_ns == 50_000_000 && tick.sequence == 15)
        assert(fvid_camera_clock_poll(clock, 400_000_100, &tick) == -1)
        assert(fvid_camera_clock_control(clock, 99, 0, 500_000_100) == -1)
        assert(fvid_camera_clock_poll(clock, 600_000_100, &tick) == 1)
        assert(tick.media_ns == 0 && tick.host_ns == 600_000_100)
        assert(fvid_camera_clock_open_rate(0, 1, 0) == nil)
        assert(fvid_camera_clock_open_rate(60, 0, 0) == nil)
        let fast = fvid_camera_clock_open_rate(UInt32(CameraFormat.framesPerSecond), 1, 0)!
        defer { fvid_camera_clock_close(fast) }
        for sequence in UInt64(0)...60 {
            let now = (sequence * 1_000_000_000 + 59) / 60
            assert(fvid_camera_clock_poll(fast, now, &tick) == 1)
            assert(tick.sequence == sequence && tick.host_ns == now)
            assert(fvid_camera_clock_poll(fast, now, &tick) == 0)
        }
        print("Camera clock C/Swift/Rust tests passed")
    }
}
