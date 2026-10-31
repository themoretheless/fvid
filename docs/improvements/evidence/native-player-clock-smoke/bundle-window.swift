import Foundation
import AppKit
import CoreGraphics
let apps=NSRunningApplication.runningApplications(withBundleIdentifier:"com.fvid.codex.clocksmoke")
var result:[[String:Any]]=[]
for app in apps {
 let pid=Int(app.processIdentifier)
 let list=CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
 let windows=list.filter {($0[kCGWindowOwnerPID as String] as? Int)==pid && ($0[kCGWindowLayer as String] as? Int)==0}.map {w -> [String:Any] in ["window_id":w[kCGWindowNumber as String] ?? 0,"title":w[kCGWindowName as String] ?? "","bounds":w[kCGWindowBounds as String] ?? [:]]}
 result.append(["pid":pid,"launch_time":app.launchDate?.timeIntervalSince1970 ?? -1,"executable":app.executableURL?.path ?? "","windows":windows])
}
print(String(data:try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys]),encoding:.utf8)!)
