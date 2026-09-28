import Foundation
import CoreMediaIO

// The initial advertised format is fixed for the lifetime of this device.
// The sink stream receives BGRA samples; the host producer is connected separately.
do {
    let source = try CameraProvider(width: 1280, height: 720, fps: CameraFormat.framesPerSecond)
    CMIOExtensionProvider.startService(provider: source.provider)
    withExtendedLifetime(source) { dispatchMain() }
} catch {
    fputs("FVid camera initialization failed: \(error)\n", stderr)
    exit(1)
}
