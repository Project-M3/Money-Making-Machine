# Security Policy

## Supported Versions

Only the latest release in the active `0.6.x` line receives security fixes and signed updates:

| Version | Supported |
| :--- | :--- |
| `0.6.x` (`main` / `beta`) | Yes |
| `< 0.6.0` | No (upgrade to latest via `m3 update` or [Releases](https://github.com/thinksmart-systems/m3/releases)) |

## Local-First Security Posture

- **No Brokerage Credentials or Execution**: M3 does not connect to brokerage accounts, store trading API keys, or submit orders.
- **Verified Self-Updates**: `m3 update` downloads public release archives over HTTPS, verifies downloaded byte sizes and SHA-256 checksums before extraction, and atomically swaps `/Applications/M3.app`.
- **Protected Personal Workbench**: Storage cleanup (`Settings -> Storage` and `m3 storage clean`) explicitly refuses to delete `personal_workbench` files (`watchlist.json`, dossiers, memos, industries, decision journal, and budgets).

## Reporting a Vulnerability

Please **do not open a public GitHub issue** for security vulnerabilities.

Instead, report potential security issues privately through GitHub Security Advisories:

- **Private Advisory**: <https://github.com/thinksmart-systems/m3/security/advisories/new>

When reporting, please include:
- A clear description of the vulnerability and potential impact
- Steps to reproduce or a minimal proof of concept
- Your `m3 --version` output and macOS version

We will acknowledge receipt promptly, investigate, and coordinate disclosure once a fix is verified and released.
