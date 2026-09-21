import AppKit
import SystemExtensions

final class CameraHost: NSObject, NSApplicationDelegate, OSSystemExtensionRequestDelegate {
    private var window: NSWindow!
    private var status: NSTextField!
    private var install: NSButton!
    private let session = CameraSession()
    private var request: OSSystemExtensionRequest?
    private let extensionID = "org.fvid.camera.extension"

    func applicationDidFinishLaunching(_ notification: Notification) {
        window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 560, height: 220),
                          styleMask: [.titled, .closable, .miniaturizable], backing: .buffered, defer: false)
        window.title = "FVid Camera"
        let explanation = NSTextField(wrappingLabelWithString: "Install the FVid virtual camera for use in other applications. macOS may ask you to approve the camera extension in System Settings.")
        explanation.frame = NSRect(x: 24, y: 132, width: 512, height: 64)
        status = NSTextField(wrappingLabelWithString: "Camera installation has not been requested.")
        status.frame = NSRect(x: 24, y: 62, width: 512, height: 60)
        install = NSButton(title: "Install camera", target: self, action: #selector(activate))
        install.frame = NSRect(x: 24, y: 20, width: 160, height: 32)
        for view in [explanation, status, install] as [NSView] { window.contentView?.addSubview(view) }
        let open = NSButton(title: "Open video…", target: self, action: #selector(openVideo))
        open.frame = NSRect(x: 196, y: 20, width: 150, height: 32)
        let stop = NSButton(title: "Stop video", target: self, action: #selector(stopVideo))
        stop.frame = NSRect(x: 358, y: 20, width: 150, height: 32)
        window.contentView?.addSubview(open); window.contentView?.addSubview(stop)
        session.onStatus = { [weak self] text in self?.status.stringValue = text }
        window.center(); window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
    }
    @objc private func openVideo() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false; panel.allowsMultipleSelection = false
        panel.message = "Choose a Y4M or supported MP4/AVC video. The final frame is held at the end."
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let url = panel.url else { return }
            self?.status.stringValue = "Opening video…"
            self?.session.start(url: url)
        }
    }
    @objc private func stopVideo() { session.stop() }
    func applicationWillTerminate(_ notification: Notification) { session.shutdown() }
    @objc private func activate() {
        guard request == nil else { return }
        let embedded = Bundle.main.bundleURL.appendingPathComponent("Contents/Library/SystemExtensions/\(extensionID).systemextension")
        guard FileManager.default.fileExists(atPath: embedded.path) else {
            status.stringValue = "The camera extension is missing from this application bundle."
            return
        }
        let activation = OSSystemExtensionRequest.activationRequest(forExtensionWithIdentifier: extensionID, queue: .main)
        activation.delegate = self
        request = activation
        install.isEnabled = false
        status.stringValue = "Requesting camera activation…"
        OSSystemExtensionManager.shared.submitRequest(activation)
    }
    func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        status.stringValue = "macOS approval is required. Allow FVid Camera in System Settings, then return here."
    }
    func request(_ request: OSSystemExtensionRequest, actionForReplacingExtension existing: OSSystemExtensionProperties,
                 withExtension ext: OSSystemExtensionProperties) -> OSSystemExtensionRequest.ReplacementAction { .replace }
    func request(_ request: OSSystemExtensionRequest, didFinishWithResult result: OSSystemExtensionRequest.Result) {
        status.stringValue = result == .completed ? "Camera extension activated. Open a video to start sending frames." : "Restart macOS to finish activating the camera extension."
        self.request = nil; install.isEnabled = true
    }
    func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        status.stringValue = "Camera activation failed: \(error.localizedDescription)"
        self.request = nil; install.isEnabled = true
    }
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
}
let app = NSApplication.shared
let delegate = CameraHost()
app.delegate = delegate
app.setActivationPolicy(.regular)
app.run()
