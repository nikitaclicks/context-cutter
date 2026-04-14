#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");
const { spawn, spawnSync } = require("node:child_process");

const binary = process.platform === "win32"
  ? path.join(__dirname, "bin", "context-cutter-mcp.exe")
  : path.join(__dirname, "bin", "context-cutter-mcp");

// npx skips postinstall by default, so the binary may not have been downloaded.
// Self-heal: run install.js on first use if the binary is missing.
if (!fs.existsSync(binary)) {
  process.stderr.write("[context-cutter-mcp] binary not found, downloading...\n");
  const pkg = require("./package.json");
  const result = spawnSync(process.execPath, [path.join(__dirname, "install.js")], {
    stdio: "inherit",
    env: { ...process.env, npm_package_version: pkg.version },
  });
  if (result.status !== 0) {
    process.stderr.write("[context-cutter-mcp] download failed\n");
    process.exit(1);
  }
}

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
  process.stderr.write(`[context-cutter-mcp] failed to start binary: ${err.message}\n`);
  process.exit(1);
});
