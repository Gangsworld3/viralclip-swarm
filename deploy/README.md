# Reverse Proxy and TLS

This folder contains the internet-facing deployment scaffold for ViralClip Swarm.

## Goal

Expose only the reverse proxy to the internet and keep the Rust API bound to loopback behind it.

## Files

- `docker-compose.yml`: app plus Nginx reverse proxy
- `nginx.conf`: HTTPS redirect, TLS termination, secure headers, and proxy rules
- `certs/`: place your TLS certificate files here

Expected certificate file names:

- `certs/fullchain.pem`
- `certs/privkey.pem`

## Deployment Rules

1. Do not expose the Rust API port directly to the internet.
2. Keep `--api-bind` on `127.0.0.1:8787`.
3. Put valid TLS certificates in `deploy/certs/`.
4. Set either `VIRALCLIP_API_TOKEN_SHA256` or `VIRALCLIP_API_KEY`.
5. Use a firewall so only ports `80` and `443` are reachable.

## Recommended Production Extras

- use ACME/Let's Encrypt automation outside this repo
- restrict ingress by IP if the API is private
- add centralized logs for Nginx and the app
- store secrets outside the repo and outside compose files

## Start

From the repo root:

```powershell
docker compose -f deploy/docker-compose.yml up --build -d
```

## Verify

- `https://your-domain/health` should proxy to the backend
- the backend should not be reachable directly on `8787` from outside the container host
