// macOS-only helper for a currently running, marked GameHub research copy.
// Usage: game-window EXPERIMENT_JSON info|capture|click|move X_FRACTION Y_FRACTION
//        game-window EXPERIMENT_JSON scroll X_FRACTION Y_FRACTION WHEEL_DELTA
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
guard args.count >= 3 else { fail("expected experiment.json and info, capture, click x y, or move x y") }
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
func focusGame(_ app: NSRunningApplication) throws {
    let deadline = Date().addingTimeInterval(2)
    while !app.isActive && Date() < deadline { RunLoop.current.run(until: Date().addingTimeInterval(0.05)) }
    if !app.isActive {
        let bridge=URL(fileURLWithPath:CommandLine.arguments[0]).standardizedFileURL.deletingLastPathComponent().appendingPathComponent("game-window-focus")
        if FileManager.default.isExecutableFile(atPath:bridge.path) {
            _ = try run(bridge.path,[String(pidNumber),executable.path])
        } else {
            _ = try run("/usr/bin/osascript", ["-e", "tell application \"System Events\" to set frontmost of first process whose unix id is \(pidNumber) to true"])
        }
        RunLoop.current.run(until:Date().addingTimeInterval(0.15))
    }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else {fail("research client did not become active")}
}
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
case "click", "move", "scroll":
    guard args.count == (args[2] == "scroll" ? 6 : 5), let rx = Double(args[3]), let ry = Double(args[4]),
          rx.isFinite, ry.isFinite, (0...1).contains(rx), (0...1).contains(ry)
    else { fail("click coordinates must be fractions from 0 to 1") }
    var wheelDelta: Int32 = 0
    if args[2] == "scroll" {
        guard let delta = Int32(args[5]), (-20...20).contains(delta), delta != 0
        else { fail("scroll requires a nonzero wheel delta from -20 to 20") }
        wheelDelta = delta
    }
    guard let app = NSRunningApplication(processIdentifier: pid),
          app.activate(options: [.activateAllWindows])
    else { fail("unable to activate the research client") }
    // Activation is asynchronous. Pump AppKit's run loop so the CLI observes
    // the focus change instead of reading its cached isActive value.
    try focusGame(app)
    let point = CGPoint(x: x + width * rx, y: y + height * ry)
    // Wine games may poll GetCursorPos instead of using an event's coordinates.
    guard CGPreflightPostEventAccess(), CGWarpMouseCursorPosition(point) == .success
    else { fail("mouse event access unavailable; no click sent") }
    let kinds: [CGEventType] = args[2] == "click" ? [.mouseMoved, .leftMouseDown, .leftMouseUp] : [.mouseMoved]
    for kind in kinds {
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
    if args[2] == "scroll" {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid,
              let event = CGEvent(scrollWheelEvent2Source: nil, units: .line, wheelCount: 1,
                                  wheel1: wheelDelta, wheel2: 0, wheel3: 0)
        else { fail("research client lost focus or scroll event unavailable") }
        event.post(tap: .cghidEventTap)
        result["wheel_delta"] = wheelDelta
    }
    result["fraction"] = [rx, ry]
case "look":
    guard args.count == 5, let dx = Double(args[3]), let dy = Double(args[4]),
          dx.isFinite, dy.isFinite, abs(dx) <= 300, abs(dy) <= 300,
          let app = NSRunningApplication(processIdentifier: pid),
          app.activate(options: [.activateAllWindows]), CGPreflightPostEventAccess()
    else { fail("look requires bounded mouse deltas and an active research window") }
    try focusGame(app)
    let point = CGPoint(x: x + width / 2 + dx, y: y + height / 2 + dy)
    guard point.x > x && point.x < x + width && point.y > y && point.y < y + height,
          CGWarpMouseCursorPosition(point) == .success,
          let event = CGEvent(mouseEventSource: nil, mouseType: .mouseMoved, mouseCursorPosition: point, mouseButton: .left)
    else { fail("look would leave the game window") }
    guard NSWorkspace.shared.frontmostApplication?.processIdentifier == pid else { fail("research client lost focus") }
    event.post(tap: .cghidEventTap)
    result["delta"] = [dx, dy]
case "hold":
    let keys: [String: CGKeyCode] = ["w":13,"s":1,"a":0,"d":2,"q":12,"e":14,"r":15,"f":3,"g":5,"space":49,"shift":56,"ctrl":59,"c":8,"1":18,"2":19,"escape":53]
    let sprintChord = args.count == 5 && args[3] == "w+shift"
    guard args.count == 5, let seconds = Double(args[4]), seconds.isFinite,
          (0.03...(sprintChord ? 8.0 : args[3] == "g" ? 6.0 : 3.0)).contains(seconds),
          sprintChord || keys[args[3]] != nil || ["fire","ads"].contains(args[3])
    else { fail("hold requires an allowlisted gameplay key and a bounded duration (sprint chord:8 seconds; grenade:6; others:3)") }
    guard let app = NSRunningApplication(processIdentifier:pid), app.activate(options:[.activateAllWindows]), CGPreflightPostEventAccess()
    else { fail("unable to focus game or post gameplay key") }
    try focusGame(app)
    let pointer = CGEvent(source: nil)?.location ?? CGPoint(x: x + width / 2, y: y + height / 2)
    if ["fire", "ads"].contains(args[3]) {
        guard pointer.x >= x && pointer.x < x + width && pointer.y >= y && pointer.y < y + height
        else { fail("gameplay mouse button requires the pointer inside the game") }
    }
    func gameplayEvent(_ down: Bool) -> CGEvent? {
        if let code=keys[args[3]] {return CGEvent(keyboardEventSource:nil,virtualKey:code,keyDown:down)}
        let right=args[3]=="ads"
        return CGEvent(mouseEventSource:nil,mouseType:right ? (down ? .rightMouseDown : .rightMouseUp) : (down ? .leftMouseDown : .leftMouseUp),mouseCursorPosition:pointer,mouseButton:right ? .right : .left)
    }
    if sprintChord {
        guard let forwardDown=CGEvent(keyboardEventSource:nil,virtualKey:13,keyDown:true),
              let forwardUp=CGEvent(keyboardEventSource:nil,virtualKey:13,keyDown:false),
              let sprintDown=CGEvent(keyboardEventSource:nil,virtualKey:56,keyDown:true),
              let sprintUp=CGEvent(keyboardEventSource:nil,virtualKey:56,keyDown:false)
        else {fail("cannot construct sprint chord")}
        forwardDown.post(tap:.cghidEventTap)
        sprintDown.post(tap:.cghidEventTap)
        defer {
            sprintUp.post(tap:.cghidEventTap)
            forwardUp.post(tap:.cghidEventTap)
        }
        let release=Date().addingTimeInterval(seconds)
        while Date()<release {
            if NSWorkspace.shared.frontmostApplication?.processIdentifier != pid {break}
            Thread.sleep(forTimeInterval:min(0.05,max(0,release.timeIntervalSinceNow)))
        }
    } else {
        guard let down=gameplayEvent(true), let up=gameplayEvent(false) else {fail("cannot construct gameplay event")}
        down.post(tap:.cghidEventTap)
        Thread.sleep(forTimeInterval:seconds)
        up.post(tap:.cghidEventTap)
    }
    result["key"]=args[3];result["seconds"]=seconds
default:
    fail("unknown window action")
}
let data = try JSONSerialization.data(withJSONObject: result, options: [.sortedKeys])
FileHandle.standardOutput.write(data + Data([10]))
