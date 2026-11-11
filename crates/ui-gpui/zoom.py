import ctypes, time, sys
u = ctypes.windll.user32
try: ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception: u.SetProcessDPIAware()
x, y, clicks = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
u.SetCursorPos(x, y); time.sleep(0.3)
u.keybd_event(0x11, 0, 0, 0)  # ctrl down
time.sleep(0.1)
for _ in range(abs(clicks)):
    u.mouse_event(0x0800, 0, 0, 120 if clicks > 0 else -120, 0)
    time.sleep(0.12)
u.keybd_event(0x11, 0, 2, 0)  # ctrl up
time.sleep(0.8)
print("wheel", clicks, "at", x, y)
