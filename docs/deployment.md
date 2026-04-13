# Deployment Guide

This document covers production deployment of `context-cutter-mcp`.

## Environment Variables

- `CONTEXT_CUTTER_MAX_HANDLES`: Maximum cached handles in memory (default: `1000`).
- `CONTEXT_CUTTER_TTL_SECS`: Time-to-live in seconds for idle handles (default: `3600`).
- `CONTEXT_CUTTER_MAX_PAYLOAD_BYTES`: Max HTTP response size accepted by `fetch_json_cutted` (default: `10485760`, 10 MB).
- `CONTEXT_CUTTER_LOG_FORMAT`: `plain` or `json` (default: `plain`).
- `RUST_LOG`: tracing level filter (default: `info`).

## Memory Sizing

Memory usage depends on payload size and active handle count.

Approximate upper bound:

`max_handles * average_payload_size`

Example:

- `CONTEXT_CUTTER_MAX_HANDLES=1000`
- average payload `50 KB`
- upper cache memory around `50 MB` plus map/index overhead

Recommended strategy:

- Start with `MAX_HANDLES=1000`
- Set `TTL_SECS=900..3600` depending on query locality
- Lower payload limit if upstream APIs are noisy

## systemd Service

`/etc/systemd/system/context-cutter-mcp.service`:

```ini
[Unit]
Description=ContextCutter MCP server
After=network.target

[Service]
Type=simple
User=app
Group=app
ExecStart=/usr/local/bin/context-cutter-mcp
Environment=RUST_LOG=info
Environment=CONTEXT_CUTTER_LOG_FORMAT=json
Environment=CONTEXT_CUTTER_MAX_HANDLES=1000
Environment=CONTEXT_CUTTER_TTL_SECS=3600
Environment=CONTEXT_CUTTER_MAX_PAYLOAD_BYTES=10485760
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true

[Install]
WantedBy=multi-user.target
```

Enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable context-cutter-mcp
sudo systemctl start context-cutter-mcp
sudo systemctl status context-cutter-mcp
```

## Docker Compose

```yaml
services:
  context-cutter-mcp:
    image: ghcr.io/nikitaclicks/context-cutter-mcp:latest
    container_name: context-cutter-mcp
    stdin_open: true
    tty: true
    environment:
      RUST_LOG: info
      CONTEXT_CUTTER_LOG_FORMAT: json
      CONTEXT_CUTTER_MAX_HANDLES: "1000"
      CONTEXT_CUTTER_TTL_SECS: "3600"
      CONTEXT_CUTTER_MAX_PAYLOAD_BYTES: "10485760"
```

Run:

```bash
docker compose up -d
```
