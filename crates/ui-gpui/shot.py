import ctypes, ctypes.wintypes as wt, time, subprocess, os, sys
from PIL import ImageGrab
u = ctypes.windll.user32; k = ctypes.windll.kernel32
try: ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception: u.SetProcessDPIAware()

def find():
    out = subprocess.run(["tasklist","/FI","IMAGENAME eq figcraft-ui-gpui.exe","/FO","CSV"],capture_output=True).stdout.decode("gbk","ignore")
    pid = int(out.strip().splitlines()[-1].split('","')[1])
    found=[]
    @ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)
    def cb(h,l):
        p=wt.DWORD(); u.GetWindowThreadProcessId(h, ctypes.byref(p))
        if p.value==pid and u.IsWindowVisible(h): found.append(h)
        return True
    u.EnumWindows(cb,0)
    return found[0]

def front(h):
    fg=u.GetForegroundWindow(); tf=u.GetWindowThreadProcessId(fg,None); tm=k.GetCurrentThreadId()
    u.AttachThreadInput(tm,tf,True); u.ShowWindow(h,9); u.SetForegroundWindow(h); u.BringWindowToTop(h)
    u.AttachThreadInput(tm,tf,False)

def client_box(h):
    pt=wt.POINT(0,0); u.ClientToScreen(h, ctypes.byref(pt))
    cr=wt.RECT(); u.GetClientRect(h, ctypes.byref(cr))
    return pt.x, pt.y, pt.x+cr.right, pt.y+cr.bottom

if __name__ == "__main__":
    h = find(); front(h); time.sleep(1.0)
    box = client_box(h)
    ImageGrab.grab(all_screens=True).crop(box).save(sys.argv[1] if len(sys.argv)>1 else "shot_client.png")
    print("client", box, "size", box[2]-box[0], box[3]-box[1])
