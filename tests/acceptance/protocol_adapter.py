"""Protocol fixture executable. Only tests invoke this file with synthetic input."""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

request = json.load(sys.stdin)
assert request["protocolVersion"] == 1
settings = request["settings"] or {}
behavior = settings.get("behavior", "pass")
if settings.get("record"):
    Path(settings["record"]).write_text(json.dumps(request), encoding="utf-8")
response = {"protocolVersion": 1, "status": "complete", "violations": [], "errors": []}
if behavior == "fail":
    response["violations"] = [{"id": "example/no-slop", "severity": "high", "category": "policy", "file": "src/source.data", "line": 1, "text": "policy violation", "resolution": "remove prohibited behavior", "engine": "untrusted", "fullLine": "forged baseline content"}]
elif behavior == "error":
    response.update(status="error", errors=["checker could not analyze source"])
elif behavior == "malformed":
    sys.stderr.write("fixture diagnostic: malformed response\n")
    sys.stdout.write("{not json")
    sys.exit(0)
elif behavior == "empty":
    sys.exit(0)
elif behavior == "version":
    response["protocolVersion"] = 99
elif behavior == "field":
    response["unexpected"] = True
elif behavior == "contradict":
    response["errors"] = ["cannot also claim completion"]
elif behavior == "sleep":
    time.sleep(60)
elif behavior == "flood":
    while True:
        os.write(1, b"x" * 8192)
        os.write(2, b"y" * 8192)
elif behavior == "cleanup-proof":
    subprocess.Popen([sys.executable, "-c", "import pathlib,sys,time; time.sleep(1); pathlib.Path(sys.argv[1]).write_text('survived')", settings["survivorMarker"]])
elif behavior == "descendant":
    child = subprocess.Popen([sys.executable, "-c", "import time;time.sleep(60)"])
    if settings.get("childPid"):
        Path(settings["childPid"]).write_text(str(child.pid), encoding="utf-8")
elif behavior == "traversal":
    response["violations"] = [{"id": "bad/path", "severity": "high", "category": "policy", "file": "../outside", "line": 1, "text": "bad", "resolution": "fix"}]
elif behavior == "location":
    response["violations"] = [{"id": "bad/line", "severity": "high", "category": "policy", "file": "src/source.data", "line": 999999, "text": "bad", "resolution": "fix"}]
elif behavior == "severity":
    response["violations"] = [{"id": "bad/severity", "severity": "hgh", "category": "policy", "file": "src/source.data", "line": 1, "text": "bad", "resolution": "fix"}]
elif behavior == "mutate-index":
    Path("src/source.data").write_text("modified and staged during checking\n", encoding="utf-8")
    subprocess.run(["git", "add", "src/source.data"], check=True, timeout=5)
elif behavior == "concurrency":
    state = Path(settings["state"])
    state.mkdir(exist_ok=True)
    lock = state / "lock"
    deadline = time.monotonic() + 10
    while True:
        try:
            lock.mkdir()
            break
        except FileExistsError:
            if time.monotonic() > deadline:
                raise RuntimeError("fixture lock timed out")
            time.sleep(0.002)
    marker = state / ("active-" + str(os.getpid()))
    marker.write_text("active", encoding="utf-8")
    current = len(list(state.glob("active-*")))
    maximum = state / "maximum"
    previous = int(maximum.read_text()) if maximum.exists() else 0
    maximum.write_text(str(max(current, previous)), encoding="utf-8")
    lock.rmdir()
    time.sleep(0.10)
    marker.unlink()
json.dump(response, sys.stdout)
sys.stdout.flush()
if behavior == "nonzero":
    sys.exit(7)
