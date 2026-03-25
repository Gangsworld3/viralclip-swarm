# 12-Stage Security Roadmap

This roadmap breaks internet-facing security work into concrete delivery stages for this repository.

## Stage 1: Production Scaffolding

Deliverables:

- `.env.example`
- `deploy/docker-compose.yml`
- `deploy/nginx.conf`
- security roadmap and security policy docs

Status:

- implemented

## Stage 2: Reverse Proxy and TLS

Deliverables:

- HTTPS termination in front of the app
- secure proxy headers
- localhost-only backend exposure
- deployment instructions for certificates

Status:

- implemented in deployment scaffolding

## Stage 3: Stronger Authentication

Deliverables:

- shared-token auth
- hashed token verification
- scoped API client registry (`read`, `run`, `admin`)

Status:

- implemented (service-token model)

## Stage 4: Request Validation

Deliverables:

- JSON content-type enforcement
- body size limit
- header count limit
- config validation
- API job constraints (output path, clip count, duration, nested mode restrictions)

Status:

- implemented (current HTTP API surface)

## Stage 5: Abuse Controls

Deliverables:

- rate limiting
- endpoint protection against trivial abuse
- queue limits and job caps

Status:

- implemented (single-node/in-memory)

## Stage 6: Security Logging

Deliverables:

- structured audit logs for auth failures, job submission, and admin-relevant events

Status:

- implemented (JSONL local audit log)

## Stage 7: File and Media Isolation

Deliverables:

- stricter file validation
- worker isolation guidance
- safer temp-file lifecycle
- malware scanning integration point

Status:

- implemented (policy controls + deployment guidance)

## Stage 8: Secret Management

Deliverables:

- env template
- secret handling guidance
- recommended rotation policy

Status:

- implemented (docs + env template baseline)

## Stage 9: Supply Chain Security

Deliverables:

- dependency audit workflow
- pinned build checks
- CI security checks

Status:

- implemented (CI/dependabot scaffolding)

## Stage 10: Privacy and Retention

Deliverables:

- privacy policy
- retention policy
- deletion expectations

Status:

- implemented (policy docs)

## Stage 11: Vulnerability Disclosure

Deliverables:

- reporting path
- response expectations
- disclosure guidance

Status:

- implemented (disclosure doc/process)

## Stage 12: Release and Deployment Gate

Deliverables:

- deployment hardening checklist
- release security gate
- production rollout checklist

Status:

- implemented (gate checklist + CI scaffold)
