import socket, json, time

HOST, PORT = "127.0.0.1", 55425
def call(method, params, timeout=15.0):
    s = socket.create_connection((HOST, PORT), timeout=timeout)
    f = s.makefile("rwb")
    f.write((json.dumps({"id":1,"method":method,"params":params})+"\n").encode()); f.flush()
    s.settimeout(timeout)
    try: return json.loads(f.readline().decode("utf-8","replace"))
    finally: s.close()

# select a group WITH children, then screenshot
print("select Color:", call("ui.set", {"selectGroup": "Color"}))
time.sleep(0.7)
print("shot group:", call("ui.screenshot", {"path": r"D:/Projektdateien/Monet/.workbuddy-ai/shots/v_group.png"}))
