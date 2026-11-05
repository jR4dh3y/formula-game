"""Send MCP tool calls to the running PhotoCraft through its agent bridge.

Usage: python3 -I pc.py '[{"tool": "doc_inspect", "args": {}}, ...]'
Image results are written next to this script as shot-<n>.png.
"""
import base64, json, os, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
CLI = os.path.expanduser("~/livery-projects/_photocraft/squashfs-root/usr/bin/photocraft-cli")
PORT = os.environ.get("PC_PORT", "38711")
calls = json.loads(sys.argv[1] if len(sys.argv) > 1 else sys.stdin.read())

proc = subprocess.Popen(
    [CLI, "mcp", "--bridge", PORT, "--control-token-file", os.path.join(HERE, "pc.token")],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
)

def send(message):
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()

def reply(want):
    for line in proc.stdout:
        message = json.loads(line)
        if message.get("id") == want:
            return message
    raise SystemExit("bridge closed")

send({"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
    "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "livery", "version": "1"}}})
reply(0)
send({"jsonrpc": "2.0", "method": "notifications/initialized"})
shot = 0
for n, call in enumerate(calls, 1):
    send({"jsonrpc": "2.0", "id": n, "method": "tools/call",
          "params": {"name": call["tool"], "arguments": call.get("args", {})}})
    message = reply(n)
    result = message.get("result", message.get("error"))
    for item in (result or {}).get("content", []):
        if item.get("type") == "image":
            shot += 1
            path = os.path.join(HERE, f"shot-{shot}.png")
            with open(path, "wb") as f:
                f.write(base64.b64decode(item["data"]))
            print(f"[{n}] {call['tool']}: image -> {path}")
        elif item.get("type") == "text":
            text = item["text"]
            limit = call.get("limit", 1500)
            print(f"[{n}] {call['tool']}: {text[:limit]}")
    if result and result.get("isError") and "no document open" in str(result) and call.get("args", {}).get("id") == "file.closeAll":
        continue
    if result and result.get("isError"):
        print(f"[{n}] stopping after error")
        break
proc.stdin.close()
proc.wait(timeout=10)
