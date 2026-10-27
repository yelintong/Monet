import socket, json, time, sys

HOST, PORT = "127.0.0.1", 55813
SHOT = r"D:\Projektdateien\Monet\.workbuddy-ai\shots\variables_after_fixes.png"

s = socket.create_connection((HOST, PORT), timeout=15)
f = s.makefile("rwb")

def call(mid, method, params):
    req = json.dumps({"id": mid, "method": method, "params": params})
    f.write((req + "\n").encode("utf-8"))
    f.flush()
    s.settimeout(15)
    try:
        line = f.readline()
        print("<-", line.decode("utf-8", "replace").strip())
    except Exception as e:
        print("(no response for", method, ":", e, ")")

# Open the Variables panel (empty collection = the doc's default).
call(1, "ui.set", {"variables": True})
time.sleep(0.8)
# Take a screenshot of the app window to the shots dir.
call(2, "ui.screenshot", {"path": SHOT})
time.sleep(0.5)
# Report panel state for confirmation.
call(3, "ui.inspect", {})
s.close()
print("SHOT_PATH", SHOT)
