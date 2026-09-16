"""Protocol fixture executable. Only tests invoke this file with synthetic input."""
import json
import os
import subprocess
import sqlite3
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
    # A transaction protects the observation itself across independent processes.
    # Directory-delete/recreate locks have platform-specific pending-delete
    # behavior; the fixture must not confuse that with checker correctness.
    database = sqlite3.connect(state / "concurrency.sqlite", timeout=5, isolation_level=None)
    try:
        database.execute("BEGIN IMMEDIATE")
        database.execute("CREATE TABLE IF NOT EXISTS active (pid INTEGER PRIMARY KEY)")
        database.execute("CREATE TABLE IF NOT EXISTS counters (id INTEGER PRIMARY KEY, maximum INTEGER NOT NULL)")
        database.execute("INSERT OR IGNORE INTO counters VALUES (1, 0)")
        database.execute("INSERT INTO active VALUES (?)", (os.getpid(),))
        current = database.execute("SELECT COUNT(*) FROM active").fetchone()[0]
        database.execute("UPDATE counters SET maximum = MAX(maximum, ?) WHERE id = 1", (current,))
        database.execute("COMMIT")
        time.sleep(0.10)
        database.execute("BEGIN IMMEDIATE")
        database.execute("DELETE FROM active WHERE pid = ?", (os.getpid(),))
        database.execute("COMMIT")
    finally:
        database.close()

json.dump(response, sys.stdout)
sys.stdout.flush()
if behavior == "nonzero":
    sys.exit(7)
