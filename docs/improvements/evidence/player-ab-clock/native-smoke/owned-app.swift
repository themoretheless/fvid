import AppKit
import Foundation
let wanted=URL(fileURLWithPath:CommandLine.arguments[1]).standardizedFileURL
let result=NSWorkspace.shared.runningApplications.filter { $0.bundleURL?.standardizedFileURL == wanted }.map { app -> [String:Any] in
 ["pid":Int(app.processIdentifier),"bundle_path":app.bundleURL!.path,"launch_date":app.launchDate?.timeIntervalSince1970 ?? 0,"terminated":app.isTerminated]
}
print(String(data:try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys]),encoding:.utf8)!)
