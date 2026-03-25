# Release Gate

A release candidate should pass the following gate before tagging:

1. `cargo test`
2. `cargo fmt -- --check`
3. `cargo clippy --all-targets -- -D warnings`
4. `cargo audit` (or equivalent dependency vulnerability scan)
5. security docs are up to date (`SECURITY.md`, `PRIVACY.md`, `DATA_RETENTION.md`)
6. deployment hardening checklist reviewed
7. release notes include security-relevant changes

## Minimum Stop Conditions

- failing tests
- critical dependency vulnerabilities without accepted exception
- auth/rate-limit protections disabled for internet deployment target
