#!/usr/bin/env swift
// ax-tree.swift PID [MAX_DEPTH]: print the macOS accessibility tree of the
// process with the given pid, as VoiceOver would see it. Walks
// AXUIElementCreateApplication(pid) -> AXChildren, printing each node's role,
// identifier, label, description, value and supported actions, indented by
// depth. MAX_DEPTH defaults to 14 (deep enough for a dialog or a menu; the
// notebook's web view can nest much deeper, so pass a smaller depth there).
//
// Needs Accessibility permission for whatever process runs this (Terminal,
// Ghostty, or the like) -- the same permission target/ui-tools needs.
//
// Run directly: scripts/ax-tree.swift PID
// or: swift scripts/ax-tree.swift PID

import ApplicationServices
import Foundation

func attr(_ el: AXUIElement, _ name: String) -> CFTypeRef? {
    var value: CFTypeRef?
    let err = AXUIElementCopyAttributeValue(el, name as CFString, &value)
    return err == .success ? value : nil
}

func str(_ v: CFTypeRef?) -> String? {
    guard let v = v else { return nil }
    if let s = v as? String, !s.isEmpty { return s }
    return nil
}

func actionNames(_ el: AXUIElement) -> [String] {
    var names: CFArray?
    guard AXUIElementCopyActionNames(el, &names) == .success, let arr = names as? [String] else { return [] }
    return arr
}

func describe(_ el: AXUIElement) -> String {
    var parts: [String] = []
    parts.append(str(attr(el, kAXRoleAttribute as String)) ?? "?")
    if let sub = str(attr(el, kAXSubroleAttribute as String)) { parts.append("(\(sub))") }
    if let id = str(attr(el, "AXIdentifier")) { parts.append("id=\(id)") }
    if let title = str(attr(el, kAXTitleAttribute as String)) { parts.append("title=\"\(title)\"") }
    if let desc = str(attr(el, kAXDescriptionAttribute as String)) { parts.append("desc=\"\(desc)\"") }
    if let value = str(attr(el, kAXValueAttribute as String)) { parts.append("value=\"\(value)\"") }
    if let help = str(attr(el, kAXHelpAttribute as String)) { parts.append("help=\"\(help)\"") }
    let actions = actionNames(el).filter { $0 != "AXShowMenu" }
    if !actions.isEmpty { parts.append("actions=\(actions.joined(separator: ","))") }
    return parts.joined(separator: " ")
}

func walk(_ el: AXUIElement, depth: Int, maxDepth: Int, out: inout String) {
    guard depth <= maxDepth else {
        out += String(repeating: "  ", count: depth) + "...\n"
        return
    }
    out += String(repeating: "  ", count: depth) + describe(el) + "\n"
    guard let childrenRef = attr(el, kAXChildrenAttribute as String), let children = childrenRef as? [AXUIElement] else { return }
    for c in children { walk(c, depth: depth + 1, maxDepth: maxDepth, out: &out) }
}

let args = CommandLine.arguments
guard args.count >= 2, let pid = pid_t(args[1]) else {
    FileHandle.standardError.write("usage: ax-tree.swift PID [MAX_DEPTH]\n".data(using: .utf8)!)
    exit(1)
}
let maxDepth = args.count >= 3 ? (Int(args[2]) ?? 14) : 14

if !AXIsProcessTrusted() {
    FileHandle.standardError.write("Accessibility permission not granted to this process. Grant it in System Settings > Privacy & Security > Accessibility.\n".data(using: .utf8)!)
    exit(2)
}

let app = AXUIElementCreateApplication(pid)
var out = ""
walk(app, depth: 0, maxDepth: maxDepth, out: &out)
print(out, terminator: "")
