# Data Retention Policy

## Default Behavior

- Generated clips and artifacts are stored in `./output` unless configured otherwise.
- Security audit events are written to `./output/security_audit.log` by default.
- Temporary workspaces are cleaned automatically unless `--secure-temp-cleanup false`.

## Recommended Retention Baseline

- Generated clips: 30 to 90 days unless business need requires longer
- Benchmark and storyboard JSON: 30 to 180 days
- Security audit logs: 90 to 365 days
- Temporary files: immediate cleanup after run completion

## Deletion

Operators should implement periodic cleanup jobs for:

- stale outputs
- stale audit logs
- stale temporary workspaces (if preserved intentionally)

## Incident Retention Exception

If a security investigation is active, preserve relevant logs/artifacts under controlled access until the incident is resolved.
