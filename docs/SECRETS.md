# Secrets Management

## Scope

This project uses environment variables for API tokens and credentials.

## Rules

- Never commit live secrets into the repository.
- Prefer hashed API tokens (`VIRALCLIP_API_TOKEN_SHA256`) over raw shared tokens.
- Use per-client scoped tokens through `VIRALCLIP_API_CLIENTS_JSON` for API mode.
- Keep separate credentials for development, staging, and production.
- Rotate cloud provider keys regularly.

## Local Setup

Use `.env.example` as the template and keep real values in local environment or secret storage.

Recommended variables:

- `VIRALCLIP_API_TOKEN_SHA256`
- `VIRALCLIP_API_CLIENTS_JSON`
- `HF_TOKEN`
- `OPENROUTER_API_KEY`
- `GROQ_API_KEY`
- `OPENAI_API_KEY`
- `ANTHROPIC_API_KEY`
- `GEMINI_API_KEY`

## Rotation Policy

- rotate internet-facing API tokens at least every 90 days
- rotate cloud model provider keys immediately if exposure is suspected
- remove stale client tokens from `VIRALCLIP_API_CLIENTS_JSON`

## Production Storage

Store secrets in a managed secret store (cloud secret manager, vault, or orchestrator secrets), not in compose files or plaintext docs.
