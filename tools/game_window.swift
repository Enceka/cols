// macOS-only helper for a currently running, marked GameHub research copy.
// Usage: game-window EXPERIMENT_JSON info|capture|click X_FRACTION Y_FRACTION
// Targets the recorded PID; never searches or clicks unrelated application windows.
import AppKit
import CoreGraphics
import Foundation

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(2)
}
func run(_ executable: String, _ arguments: [String]) throws -> String {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: executable)
    process.arguments = arguments
    let pipe = Pipe()
    process.standardOutput = pipe
    try process.run()
    let output = pipe.fileHandleForReading.readDataToEndOfFile()
    process.waitUntilExit()
    guard process.terminationStatus == 0 else { fail("helper subprocess failed") }
    return String(decoding: output, as: UTF8.self)
}
let args = CommandLine.arguments
guard args.count >= 3 else { fail("expected experiment.json and info, capture, or click x y") }
let experiment = URL(fileURLWithPath: args[1]).standardizedFileURL
guard let state = try JSONSerialization.jsonObject(with: Data(contentsOf: experiment)) as? [String: Any],
      state["finished_unix"] == nil,
      let pidNumber = state["pid"] as? Int,
      pidNumber > 0,
      let argv = state["argv"] as? [String], argv.count > 1
else { fail("experiment must describe a running client") }
let executable = URL(fileURLWithPath: argv[1]).standardizedFileURL
guard executable.lastPathComponent == "codoMP_client_shipRetail.exe",
      FileManager.default.fileExists(atPath: executable.deletingLastPathComponent().appendingPathComponent(".codol-research-copy.json").path)
else { fail("client must be a marked research copy") }
let actual = try run("/bin/ps", ["-p", String(pidNumber), "-o", "comm="]).trimmingCharacters(in: .whitespacesAndNewlines)
guard actual == executable.path else { fail("recorded PID no longer belongs to the research client") }
let pid = pid_t(pidNumber)
let windows = CGWindowListCopyWindowInfo(.optionOnScreenOnly, kCGNullWindowID) as? [[String: Any]] ?? []
guard let window = windows.first(where: {
    ($0[kCGWindowOwnerPID as String] as? Int) == pidNumber && ($0[kCGWindowLayer as String] as? Int) == 0
}), let number = window[kCGWindowNumber as String] as? Int,
    let rawBounds = window[kCGWindowBounds as String] as? [String: NSNumber],
    let x = rawBounds["X"]?.doubleValue, let y = rawBounds["Y"]?.doubleValue,
    let width = rawBounds["Width"]?.doubleValue, let height = rawBounds["Height"]?.doubleValue,
    width > 0, height > 0
else { fail("research client has no visible window") }
var result: [String: Any] = ["pid": pidNumber, "window": number, "bounds": rawBounds, "action": args[2]]
switch args[2] {
case "info":
    guard args.count == 3 else { fail("info takes no extra arguments") }
case "capture":
    guard args.count == 3 else { fail("capture takes no extra arguments") }
    let output = experiment.deletingLastPathComponent().appendingPathComponent("ui-\(Int(Date().timeIntervalSince1970 * 1000)).png")
    _ = try run("/usr/sbin/screencapture", ["-x", "-o", "-l", String(number), output.path])
    result["path"] = output.path
case "click":
    guard args.count == 5, let rx = Double(args[3]), let ry = Double(args[4]),
          rx.isFinite, ry.isFinite, (0...1).contains(rx), (0...1).contains(ry)
    else { fail("click coordinates must be fractions from 0 to 1") }
    guard let app = NSRunningApplication(processIdentifier: pid),
          app.activate(options: [.activateAllWindows])
    else { fail("unable to activate the research client") }
    // Activation is asynchronous. Pump AppKit's run loop so the CLI observes
    // the focus change instead of reading its cached isActive value.
    let deadline = Date().addingTimeInterval(2)
    while !app.isActive && Date() < deadline {
        RunLoop.current.run(until: Date().addingTimeInterval(0.05))
    }
    if !app.isActive {
        // Wine's unbundled executable may require System Events activation.
        // Still target only the validated experiment PID; never click globally.
        _ = try run("/usr/bin/osascript", ["-e", "tell application \"System Events\" to set frontmost of first process whose unix id is \(pidNumber) to true"])
        RunLoop.current.run(until: Date().addingTimeInterval(0.15))
    }
    guard app.isActive else { fail("research client did not become active; no click sent") }
    let point = CGPoint(x: x + width * rx, y: y + height * ry)
    // Wine games may poll GetCursorPos instead of using an event's coordinates.
    guard CGPreflightPostEventAccess(), CGWarpMouseCursorPosition(point) == .success
    else { fail("mouse event access unavailable; no click sent") }
    for kind: CGEventType in [.mouseMoved, .leftMouseDown, .leftMouseUp] {
        guard let event = CGEvent(mouseEventSource: nil, mouseType: kind, mouseCursorPosition: point, mouseButton: .left)
        else { fail("unable to create mouse event") }
        if kind != .mouseMoved { event.setIntegerValueField(.mouseEventClickState, value: 1) }
        // Wine's game mouse path consumes HID events. postToPid can move the
        // pointer while silently losing button presses. Check focus immediately
        // before every event so a focus change aborts this guarded action.
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid
        else { fail("research client lost focus; click aborted") }
        event.post(tap: .cghidEventTap)
        Thread.sleep(forTimeInterval: 0.08)
    }
    result["fraction"] = [rx, ry]
default:
    fail("unknown window action")
}
let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
FileHandle.standardOutput.write(data + Data([10]))
