# Deployment Hardening Checklist

Use this checklist before exposing the API to the internet.

- [ ] TLS certificates are valid and auto-renewed
- [ ] Backend service is not publicly exposed directly
- [ ] Firewall only allows required ingress (`80`, `443`)
- [ ] `VIRALCLIP_API_CLIENTS_JSON` or token auth is configured
- [ ] Raw shared token fallback is disabled in production
- [ ] Rate limits are set for expected traffic profile
- [ ] Queue limits are set to prevent resource exhaustion
- [ ] Security audit log path is configured and monitored
- [ ] Secrets are loaded from managed secret storage
- [ ] Outputs/log retention jobs are configured
- [ ] Dependency scanning is enabled in CI
- [ ] Incident response contacts/process are documented
