#!/usr/bin/env bash
# Slopgate SessionStart hook — capture the session-start model for stats attribution.
# Only SessionStart receives a `model` field; mid-session /model switches are invisible.
# Fail-open: any error leaves no file -> stats resolves model to 'unknown'.
ROOT=$(realpath "$(git rev-parse --show-toplevel 2>/dev/null || pwd)" 2>/dev/null || git rev-parse --show-toplevel 2>/dev/null || pwd)
exec node -e '
const crypto = require("crypto"), fs = require("fs"), path = require("path"), os = require("os");
const root = process.argv[1];
let d = "";
process.stdin.on("data", (c) => (d += c)).on("end", () => {
  let m;
  try {
    const j = JSON.parse(d);
    m = { model: j.model || "unknown", sessionId: j.session_id || null, startedAt: new Date().toISOString(), cwd: j.cwd || root };
  } catch {
    m = { model: "unknown", sessionId: null, startedAt: new Date().toISOString(), cwd: root };
  }
  try {
    const key = crypto.createHash("sha256").update(root).digest("hex").slice(0, 16);
    const dir = path.join(process.env.HOME || os.homedir(), ".slopgate", "sessions");
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(dir, key + ".json"), JSON.stringify(m));
  } catch { /* fail-open */ }
});
' "$ROOT"
