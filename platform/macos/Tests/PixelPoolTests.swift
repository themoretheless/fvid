import Foundation
import CoreVideo

@main
struct PixelPoolTests {
    static func main() throws {
        let pool = try PixelPool(width: 16, height: 16, capacity: 3)
        var held: [CVPixelBuffer] = []
        for _ in 0..<3 {
            guard let frame = try pool.acquire() else { fatalError("early exhaustion") }
            precondition(CVPixelBufferGetWidth(frame) == 16)
            precondition(CVPixelBufferGetPixelFormatType(frame) == kCVPixelFormatType_32BGRA)
            held.append(frame)
        }
        for _ in 0..<100 { precondition(tryAcquire(pool) == nil) }
        held.removeLast()
        guard let reused = try pool.acquire() else { fatalError("released buffer was not reused") }
        held.append(reused)
        precondition(tryAcquire(pool) == nil)
        withExtendedLifetime(held) {}
        print("PixelPool: bounded allocation and reuse passed")
    }
    static func tryAcquire(_ pool: PixelPool) -> CVPixelBuffer? { try! pool.acquire() }
}
