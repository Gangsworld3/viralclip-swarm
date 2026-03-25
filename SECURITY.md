# Security

## Supported Use

The built-in API server is intended for local or trusted-network use.

Current protections:

- loopback-only bind enforcement for the built-in HTTP API
- request body size limit
- request header count limit
- in-memory rate limiting
- persistent per-client daily API run quotas
- optional `x-api-key` authentication
- optional SHA-256 token verification through environment variables
- optional multi-client token registry with scopes
- local input file validation for extension, file type, and size
- local-first processing by default
- security audit log output

## Recommended API Auth

Preferred option:

1. Generate a strong random token.
2. Compute its SHA-256 hex digest.
3. Store the digest in `VIRALCLIP_API_TOKEN_SHA256`.
4. Send the raw token as the `x-api-key` header.

Helper script:

```powershell
.\scripts\rotate-api-token.ps1
```

The server will hash the provided token and compare it to the stored digest.

Fallback option:

- Set `VIRALCLIP_API_KEY` to a raw shared token.

Scoped client option:

- Set `VIRALCLIP_API_CLIENTS_JSON` to a JSON array like:

```json
[
  {
    "client_id": "demo-reader",
    "token_sha256": "sha256hexhere",
    "scopes": ["read"]
  },
  {
    "client_id": "demo-runner",
    "token_sha256": "sha256hexhere",
    "scopes": ["read", "run"]
  }
]
```

Supported scopes:

- `read`
- `run`
- `admin`

## Security Limits

The built-in API is not a full production gateway. It does not provide:

- TLS termination
- user accounts
- role-based access control
- centralized tamper-resistant audit logs
- distributed rate limiting
- malware scanning or media sandbox isolation
- full persistent identity management

## Audit Logging

Security-relevant API events are written to the configured audit log path as JSON lines.

Examples:

- auth failures
- authorization failures
- rate-limit rejections
- queue-limit rejections
- daily-quota rejections
- accepted jobs
- completed or failed jobs

For internet-facing deployment, put the API behind:

- HTTPS/TLS
- a reverse proxy
- stronger auth
- request logging
- network-level firewall controls

The repo includes starter reverse-proxy/TLS deployment files in [deploy/README.md](/C:/Users/hp/Documents/New%20folder/viralclip-swarm/deploy/README.md).

## Reporting

If you find a security issue, report it privately to the project maintainer before opening a public issue.

Detailed disclosure workflow is documented in [docs/VULNERABILITY_DISCLOSURE.md](/C:/Users/hp/Documents/New%20folder/viralclip-swarm/docs/VULNERABILITY_DISCLOSURE.md).
