// Prints the CoreGraphics window number of the main window owned by a PID,
// for `screencapture -l` (scripts/screenshots.sh).
import CoreGraphics
import Foundation

let pid = Int32(CommandLine.arguments[1]) ?? 0
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list where (w[kCGWindowOwnerPID as String] as? Int32) == pid && (w[kCGWindowLayer as String] as? Int) == 0 {
    print(w[kCGWindowNumber as String]!)
    exit(0)
}
exit(1)
