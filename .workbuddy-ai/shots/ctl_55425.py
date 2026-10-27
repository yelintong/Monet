import socket, json, time, sys

HOST, PORT = "127.0.0.1", 55425

def call(mid, method, params, timeout=10.0):
    s = socket.create_connection((HOST, PORT), timeout=timeout)
    f = s.makefile("rwb")
    req = json.dumps({"id": mid, "method": method, "params": params})
    f.write((req + "\n").encode("utf-8"))
    f.flush()
    s.settimeout(timeout)
    try:
        line = f.readline()
        print("<-", line.decode("utf-8", "replace").strip()[:600])
    except Exception as e:
        print("(no response for", method, ":", e, ")")
    finally:
        s.close()

cmd = sys.argv[1] if len(sys.argv) > 1 else "inspect"
if cmd == "inspect":
    call(1, "ui.inspect", {})
elif cmd == "vars":
    call(1, "ui.set", {"variables": True})
    time.sleep(0.6)
    call(2, "ui.inspect", {})
