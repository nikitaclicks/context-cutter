# Security Policy

## Supported Versions

| Version | Supported |
| ------- | --------- |
| 0.1.x   | ✅ Yes    |

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Report vulnerabilities via [GitHub Security Advisories](https://github.com/nikitaclicks/context-cutter/security/advisories/new).

You can expect:

- **Acknowledgement** within 48 hours.
- **Status update** within 5 business days.
- **Coordinated disclosure** — we will publish a CVE and release a patch before any public disclosure.

## Scope

In-scope:

- Remote code execution in the MCP binary or Python SDK.
- SSRF via the `fetch_json_cutted` tool fetching internal network resources.
- Handle enumeration or unauthorized access to stored payloads.
- JSONPath injection leading to unexpected data exposure.
- Denial of service via memory exhaustion or unbounded computation.

Out of scope:

- Security issues in dependencies (report upstream; we will track and update).
- Theoretical attacks with no practical impact.

## Security Defaults

The binary enforces the following protections out of the box:

| Protection | Default | Override |
|---|---|---|
| Max payload size | 10 MB | `CONTEXT_CUTTER_MAX_PAYLOAD_BYTES` env var |
| Max stored handles | 1 000 | `CONTEXT_CUTTER_MAX_HANDLES` env var |
| Handle TTL | 1 hour | `CONTEXT_CUTTER_TTL_SECS` env var |
| Allowed URL schemes | `https` only | Not configurable |
| HTTP request timeout | 45 s | `timeout_seconds` parameter |
