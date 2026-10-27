import socket, json, time, sys

HOST, PORT = "127.0.0.1", 55425

def call(method, params, timeout=15.0):
    s = socket.create_connection((HOST, PORT), timeout=timeout)
    f = s.makefile("rwb")
    req = json.dumps({"id": 1, "method": method, "params": params})
    f.write((req + "\n").encode("utf-8")); f.flush()
    s.settimeout(timeout)
    try:
        line = f.readline()
        return json.loads(line.decode("utf-8", "replace"))
    except Exception as e:
        return {"error": str(e)}
    finally:
        s.close()

def shot(path):
    return call("ui.screenshot", {"path": path})

# 1) open variables panel
print("open:", call("ui.set", {"variables": True}))
time.sleep(0.8)
# 2) dump full inspect to find group names
ins = call("ui.inspect", {})
with open("/tmp/inspect.json", "w", encoding="utf-8") as fh:
    json.dump(ins, fh, ensure_ascii=False)
print("inspect bytes:", len(json.dumps(ins)))
# 3) select ALL variables and screenshot
print("selectAll:", call("ui.set", {"selectGroup": "ALL"}))
time.sleep(0.6)
print("shot ALL:", shot(r"D:/Projektdateien/Monet/.workbuddy-ai/shots/v_all.png"))
