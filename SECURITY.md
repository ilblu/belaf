# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| latest  | :white_check_mark: |
| < latest | :x:               |

We recommend always using the latest version of belaf CLI to ensure you have the most recent security patches.

## Reporting a Vulnerability

We take security vulnerabilities seriously. If you discover a security issue, please report it responsibly.

### How to Report

1. **Do NOT** open a public GitHub issue for security vulnerabilities
2. Email security concerns to: security@ilblu.dev
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Any suggested fixes (optional)

### Response Timeline

- **Initial Response**: Within 48 hours
- **Status Update**: Within 7 days
- **Resolution Target**: Within 30 days for critical issues

### What to Expect

- Acknowledgment of your report
- Regular updates on progress
- Credit in release notes (unless you prefer anonymity)
- No legal action for responsible disclosure

## Security Measures

### Input Validation

- **File Size Limits**: Configuration files (TOML, JSON) are limited to 10MB to prevent resource exhaustion attacks
- **Path Traversal Protection**: Paths containing `..`, null bytes, or Windows reserved names are rejected
- **Package Name Validation**: Package names are validated to prevent injection attacks

### Path Security

- Symlink traversal outside repository boundaries is blocked
- Windows-specific protections:
  - Reserved device names (CON, PRN, AUX, NUL, etc.) are rejected
  - UNC paths (\\server\share) are rejected
  - Drive letter prefixes (C:\) outside expected paths are rejected
  - Both forward and backward slashes are normalized

### Git Operations

- Git commands use `--` separator to prevent option injection
- Repository paths are validated before operations
- Commit metadata is sanitized

### Dependency Security

- Dependencies are regularly audited using `cargo audit`
- Minimal dependency footprint to reduce attack surface
- No network operations without explicit user consent

## Security Best Practices for Users

1. **Verify Downloads**: Always download belaf from official sources
2. **Review Changes**: Inspect release changelogs before version updates
3. **Environment Variables**: Avoid storing secrets in environment variables accessible to the CLI
4. **Repository Trust**: Only run `belaf` commands in trusted repositories

## Threat Model

### In Scope

- Local privilege escalation through CLI
- Arbitrary file read/write outside repository
- Command injection through user input
- Resource exhaustion (memory, disk, CPU)
- Information disclosure of sensitive data

### Out of Scope

- Physical access attacks
- Social engineering
- Denial of service through legitimate operations
- Attacks requiring pre-existing system compromise

## Security Audits

We welcome security audits and penetration testing. Please coordinate with us before conducting any testing to ensure it's performed safely and legally.

## Acknowledgments

We thank the following individuals for responsibly disclosing security issues:

*No vulnerabilities reported yet.*
