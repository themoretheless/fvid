import Foundation
import CoreGraphics
let wanted=Int(CommandLine.arguments[1])!
let list=CGWindowListCopyWindowInfo([.optionOnScreenOnly,.excludeDesktopElements],kCGNullWindowID) as? [[String:Any]] ?? []
let windows=list.filter {($0[kCGWindowOwnerPID as String] as? Int)==wanted && ($0[kCGWindowLayer as String] as? Int)==0}.map { w -> [String:Any] in
 ["pid":wanted,"window_id":w[kCGWindowNumber as String] ?? 0,"title":w[kCGWindowName as String] ?? "","bounds":w[kCGWindowBounds as String] ?? [:]]
}
print(String(data:try JSONSerialization.data(withJSONObject:windows,options:[.sortedKeys]),encoding:.utf8)!)
