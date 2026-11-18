"""Scripts for AOSP's `hid` command: a virtual USB game pad, or a two-finger touch screen, driven by a
timed sequence — the S9 checks that need hardware nobody has at the desk (2026-10-24).

    python -I tools/hid_input.py pad   out.json  wait:3000 btn:1:150 hat:ne:120 stick:100:-100:200 ...
    python -I tools/hid_input.py pinch out.json  X1 Y1 X2 Y2 SPREAD [STEPS [TAP_X TAP_Y]]
    adb -s <serial> push out.json /data/local/tmp/ && adb -s <serial> shell hid /data/local/tmp/out.json

The device lives while `hid` runs (send it to the background to act meanwhile); its reports follow
the script's delays. The shell may open /dev/uhid (group uhid) even where SELinux refuses writes to
/dev/input/* (S9: `sendevent` = Permission denied). `register` names its bus: Android 16's `hid`
(S25) throws a NullPointerException in Event.getBus without one (2026-10-27).

pad — 16 buttons, a hat, X/Y (−127..127), two analog triggers (0..255). Linux hid-input + Android
Generic.kl give buttons 1 A, 2 B, 3 C, 4 X, 5 Y, 6 Z, 7 L1, 8 R1, 9 L2, 10 R2, 11 SELECT, 12 START,
13 MODE, 14 L3, 15 R3; the hat is AXIS_HAT_X/Y (a corner = one sample with both); the triggers are
Simulation Brake / Accelerator = AXIS_BRAKE / AXIS_GAS, the L2/R2 of a pad that has no trigger
buttons (the app reads max(LTRIGGER, BRAKE), 2026-10-25). Steps: wait:MS | btn:N:MS | hat:DIR:MS
(n ne e se s sw w nw) | stick:X:Y:MS | trig:L:R:MS — each press is followed by the idle report.

pinch — hid-multitouch, two contacts, 0..4095. Coordinates are the app's landscape screen
(2220x1080 on the S9, SurfaceOrientation 1): raw = (1080 − y, x) on the natural portrait panel.
The driver queues reports while it probes the new device, so the script waits 9 s after registering
(2.5 s delivered every report in the same millisecond). An optional tap first selects a control in
the layout editor; then both fingers go down and move apart to SPREAD x their distance (< 1 pinches in).
"""
import json
import sys

PAD = [
    0x05, 0x01, 0x09, 0x05, 0xA1, 0x01,
    0x05, 0x09, 0x19, 0x01, 0x29, 0x10, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x10, 0x81, 0x02,
    0x05, 0x01, 0x09, 0x39, 0x15, 0x00, 0x25, 0x07, 0x35, 0x00, 0x46, 0x3B, 0x01, 0x65, 0x14,
    0x75, 0x04, 0x95, 0x01, 0x81, 0x42,
    0x65, 0x00, 0x75, 0x04, 0x95, 0x01, 0x81, 0x03,
    0x09, 0x30, 0x09, 0x31, 0x15, 0x81, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x02, 0x81, 0x02,
    0x05, 0x02, 0x09, 0xC5, 0x09, 0xC4, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x75, 0x08, 0x95, 0x02, 0x81, 0x02,
    0xC0,
]
HAT = {"n": 0, "ne": 1, "e": 2, "se": 3, "s": 4, "sw": 5, "w": 6, "nw": 7}

FINGER = [
    0x05, 0x0D, 0x09, 0x22, 0xA1, 0x02,
    0x09, 0x42, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95, 0x01, 0x81, 0x02,
    0x95, 0x07, 0x81, 0x03,
    0x09, 0x51, 0x25, 0x7F, 0x75, 0x08, 0x95, 0x01, 0x81, 0x02,
    0x05, 0x01, 0x26, 0xFF, 0x0F, 0x75, 0x10, 0x95, 0x01,
    0x09, 0x30, 0x81, 0x02, 0x09, 0x31, 0x81, 0x02,
    0xC0,
]
TOUCH = (
    [0x05, 0x0D, 0x09, 0x04, 0xA1, 0x01, 0x85, 0x01]
    + FINGER + FINGER
    + [0x05, 0x0D, 0x09, 0x54, 0x25, 0x7F, 0x95, 0x01, 0x75, 0x08, 0x81, 0x02,
       0x85, 0x02, 0x09, 0x55, 0x25, 0x02, 0xB1, 0x02, 0xC0]
)
PANEL_W = 1080  # the natural (portrait) panel's short side, in screen px


def pad(steps):
    def report(buttons=0, hat=8, x=0, y=0, lt=0, rt=0):
        return [buttons & 0xFF, (buttons >> 8) & 0xFF, hat & 0x0F, x & 0xFF, y & 0xFF, lt & 0xFF, rt & 0xFF]

    ev = [{"id": 1, "command": "register", "name": "HZ test pad", "vid": 0x2345, "pid": 0x6789, "bus": "usb",
           "descriptor": PAD}]
    for s in steps:
        kind, *a = s.split(":")
        if kind == "wait":
            ev.append({"id": 1, "command": "delay", "duration": int(a[0])})
            continue
        r = {"btn": lambda: report(buttons=1 << (int(a[0]) - 1)), "hat": lambda: report(hat=HAT[a[0]]),
             "stick": lambda: report(x=int(a[0]), y=int(a[1])), "trig": lambda: report(lt=int(a[0]), rt=int(a[1]))}[kind]()
        ev += [{"id": 1, "command": "report", "report": r}, {"id": 1, "command": "delay", "duration": int(a[-1])},
               {"id": 1, "command": "report", "report": report()}]
    return ev


def pinch(args):
    x1, y1, x2, y2, spread = map(float, args[:5])
    steps = int(args[5]) if len(args) > 5 else 10

    def contact(tip, cid, sx=0.0, sy=0.0):
        x, y = (int((PANEL_W - sy) * 4095 / PANEL_W), int(sx * 4095 / 2220)) if tip else (0, 0)
        return [1 if tip else 0, cid, x & 0xFF, x >> 8, y & 0xFF, y >> 8]

    ev = [{"id": 2, "command": "register", "name": "HZ test touch", "vid": 0x2345, "pid": 0x6790, "bus": "usb",
           "descriptor": TOUCH, "feature_reports": [{"id": 2, "data": [2, 2]}]},
          {"id": 2, "command": "delay", "duration": 9000}]

    def rep(c0, c1, n):
        ev.append({"id": 2, "command": "report", "report": [1] + c0 + c1 + [n]})
        ev.append({"id": 2, "command": "delay", "duration": 40})

    if len(args) > 7:
        tx, ty = float(args[6]), float(args[7])
        rep(contact(True, 1, tx, ty), contact(False, 0), 1)
        rep(contact(False, 1), contact(False, 0), 0)
        ev.append({"id": 2, "command": "delay", "duration": 600})
    mx, my = (x1 + x2) / 2, (y1 + y2) / 2
    rep(contact(True, 3, x1, y1), contact(False, 0), 1)
    rep(contact(True, 3, x1, y1), contact(True, 4, x2, y2), 2)
    for i in range(1, steps + 1):
        k = 1 + (spread - 1) * i / steps
        rep(contact(True, 3, mx + (x1 - mx) * k, my + (y1 - my) * k), contact(True, 4, mx + (x2 - mx) * k, my + (y2 - my) * k), 2)
    rep(contact(False, 3), contact(False, 4), 0)
    ev.append({"id": 2, "command": "delay", "duration": 1500})
    return ev


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("pad", "pinch"):
        print(__doc__)
        return 2
    ev = pad(sys.argv[3:]) if sys.argv[1] == "pad" else pinch(sys.argv[3:])
    with open(sys.argv[2], "w") as f:
        f.write("\n".join(json.dumps(e) for e in ev) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
