# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| 0.8.x | Yes |
| 0.7.x | Security fixes only |
| < 0.7 | No |

## Reporting a vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

Please report security issues by emailing the maintainers directly. We aim to
acknowledge reports within 48 hours and to produce a fix within 14 days for
critical issues.

When reporting, please include:

- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Any suggested mitigation

## Scope

Issues in scope:

- Credential leakage through logs, IPC messages, or commit metadata
- Secret scanner bypass — a pattern that should be detected but isn't
- Privilege escalation through the daemon's Unix socket
- Arbitrary command execution through hook configuration injection
- Dependency vulnerabilities (`cargo audit`)

Issues out of scope:

- Denial of service against the local daemon
- Issues requiring physical access to the machine
