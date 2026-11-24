#!/usr/bin/env python3
import sys

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

STATES = ("checked", "checkable", "expanded", "expandable", "selected", "focusable", "focused")


def nodes(o, depth=0):
    yield o, depth
    for i in range(o.get_child_count()):
        child = o.get_child_at_index(i)
        if child is not None:
            yield from nodes(child, depth + 1)


def line(o):
    states = o.get_state_set()
    flags = [s for s in STATES if states.contains(getattr(Atspi.StateType, s.upper()))]
    action = o.get_action_iface()
    actions = [action.get_action_name(i) for i in range(action.get_n_actions())] if action else []
    text = f"[{o.get_role_name()}] {o.get_name()!r}"
    if o.get_description():
        text += f" desc={o.get_description()!r}"
    if flags:
        text += f" {flags}"
    if actions:
        text += f" actions={actions}"
    return text


desktop = Atspi.get_desktop(0)
apps = [desktop.get_child_at_index(i) for i in range(desktop.get_child_count())]
apps = [a for a in apps if a is not None and a.get_name() == "endeavor"]
if not apps:
    sys.exit("atspi-tree: Endeavor isn't on the accessibility bus. Is it running with the bus on?")
if len(sys.argv) > 1:
    want = sys.argv[1]
    for app in apps:
        for o, _ in nodes(app):
            if o.get_name() == want and o.get_action_iface():
                print("clicked" if o.get_action_iface().do_action(0) else "click failed", line(o))
                sys.exit(0)
    sys.exit(f"atspi-tree: no node named {want!r} that can be clicked")
for app in apps:
    for o, depth in nodes(app):
        print("  " * depth + line(o))
