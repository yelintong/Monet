import ctypes, time, sys
u = ctypes.windll.user32
try: ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception: u.SetProcessDPIAware()
x0, y0 = int(sys.argv[1]), int(sys.argv[2])
dx, dy = int(sys.argv[3]), int(sys.argv[4])
u.SetCursorPos(x0, y0); time.sleep(0.2)
u.mouse_event(0x0002, 0, 0, 0, 0)  # left down
time.sleep(0.15)
steps = 12
for i in range(1, steps+1):
    u.SetCursorPos(x0 + dx*i//steps, y0 + dy*i//steps)
    time.sleep(0.03)
time.sleep(0.2)
u.mouse_event(0x0004, 0, 0, 0, 0)  # left up
time.sleep(0.5)
print("dragged from", x0, y0, "by", dx, dy)
