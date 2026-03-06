#!/usr/bin/env node

const path = require("node:path");
const { spawn } = require("node:child_process");

const binary = process.platform === "win32"
  ? path.join(__dirname, "bin", "context-cutter-mcp.exe")
  : path.join(__dirname, "bin", "context-cutter-mcp");

const child = spawn(binary, process.argv.slice(2), {
  stdio: "inherit",
});

child.on("exit", (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
    return;
  }
  process.exit(code ?? 1);
});

child.on("error", (err) => {
  console.error(`[context-cutter-mcp] failed to start binary: ${err.message}`);
  process.exit(1);
});
