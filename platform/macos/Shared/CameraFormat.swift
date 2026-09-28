// Shared by the host and extension so scheduling and advertised format agree.
enum CameraFormat {
    static let framesPerSecond: Int32 = 60
    static let timerNanoseconds = (1_000_000_000 + Int(framesPerSecond) - 1) / Int(framesPerSecond)
}
