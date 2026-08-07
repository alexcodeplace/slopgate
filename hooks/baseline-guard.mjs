// Slopgate baseline-guard — PreToolUse hook blocking agent bypass of the gate.
// Covers: Edit/Write to baseline.json/suppressions.json, Bash rm/mv on baseline*,
// Bash `slopgate baseline` in any form.
// Exit 2 = block the tool use (PreToolUse protocol). Exit 0 = allow. Always fail-open on errors.

import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";

const BASELINE_FILE = /\.slopgate\/(baseline|suppressions)\.json$/;
const BASELINE_CMD =
  /(^|[;&|\s])(npx\s+|pnpm\s+exec\s+|yarn\s+)?(node\s+[^ ]+\/bin\/slopgate|slopgate)\s+baseline(\s|$)/;
const BASELINE_RM = /(rm|mv)\s+.*\.slopgate\/baseline/;

function payload() {
  try {
    return JSON.parse(fs.readFileSync(0, "utf8"));
  } catch {
    return {};
  }
}

function verdict(input) {
  const toolInput = input.tool_input || {};
  switch (input.tool_name) {
    case "Edit":
    case "Write": {
      const filePath = String(toolInput.file_path || "");
      if (BASELINE_FILE.test(filePath)) {
        return `direct write to ${filePath} blocked — edit baseline/suppressions via slopgate CLI only`;
      }
      return null;
    }
    case "Bash": {
      const command = String(toolInput.command || "");
      if (BASELINE_RM.test(command)) {
        return "rm/mv of .slopgate/baseline blocked — baseline integrity guard";
      }
      if (BASELINE_CMD.test(command)) {
        return "slopgate baseline blocked — agent cannot update/create baseline; run in your own terminal";
      }
      return null;
    }
    default:
      return null;
  }
}

function recordBypassAttempt(reason) {
  let root;
  try {
    root = execFileSync("git", ["rev-parse", "--show-toplevel"], { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    root = process.cwd();
  }
  try {
    const key = crypto.createHash("sha256").update(root).digest("hex").slice(0, 16);
    const sess = JSON.parse(fs.readFileSync(path.join(os.homedir(), ".slopgate", "sessions", `${key}.json`), "utf8"));
    const row = JSON.stringify({
      ts: new Date().toISOString(),
      project: path.basename(root),
      projectPath: root,
      model: sess.model || "unknown",
      sessionId: sess.sessionId || null,
      mode: "bypass-attempt",
      ruleId: "baseline-tamper",
      severity: "critical",
      category: "security",
      engine: "bypass-attempt",
      file: null,
      line: null,
      reason
    });
    fs.appendFileSync(path.join(os.homedir(), ".slopgate", "stats.jsonl"), `${row}\n`);
  } catch {
    // fail-open: the deny still stands even when stats cannot be written
  }
}

const reason = verdict(payload());
if (reason) {
  fs.writeSync(2, `⛔ SLOPGATE GUARD: ${reason}\n`);
  recordBypassAttempt(reason);
  process.exit(2);
}
process.exit(0);
