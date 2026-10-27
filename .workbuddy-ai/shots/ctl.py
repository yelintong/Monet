import socket, json, sys, time

HOST, PORT = "127.0.0.1", 55813

def rpc(method, params=None, timeout=5.0):
    s = socket.create_connection((HOST, PORT), timeout=timeout)
    s.settimeout(timeout)
    msg = json.dumps({"id": 1, "method": method, "params": params or {}}) + "\n"
    s.sendall(msg.encode())
    buf = b""
    # read until we get a line that parses and has the same id (or any json line)
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            data = s.recv(65536)
        except socket.timeout:
            break
        if not data:
            break
        buf += data
        # try to find a complete json line
        if b"\n" in buf:
            line = buf.split(b"\n", 1)[0]
            try:
                return json.loads(line.decode())
            except Exception:
                pass
    s.close()
    return None

if __name__ == "__main__":
    method = sys.argv[1] if len(sys.argv) > 1 else "ui.inspect"
    params = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}
    print(json.dumps(rpc(method, params), indent=2, ensure_ascii=False))
