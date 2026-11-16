# Deploy the Excession agent gateway

This container runs the existing Rust Excession CLI against pinned Git objects. The HTTP layer only transports four read-only operations. It does not reimplement the analysis, scan arbitrary URLs, run tests or run user-supplied commands.

## One-command Docker Compose deployment

On a host with Docker Compose, set up a private token once and start the service:

```sh
cp deploy/.env.example deploy/.env
# Fill EXCESSION_SERVICE_TOKEN in deploy/.env with `openssl rand -hex 32` output.
docker compose -f deploy/compose.yml up --build -d
curl --fail http://127.0.0.1:8080/healthz
```

The Compose configuration binds only to localhost, never exposes the analysis process publicly, and uses a tmpfs for transient Git clones and SQLite caches. Put it behind your own HTTPS reverse proxy with authentication or an existing private ingress when remote access is required. Do not commit `deploy/.env` or log the token. Check `docker compose -f deploy/compose.yml logs excession` for clone/bootstrap errors. Clones are read-only sources and the runtime's Git history is refreshed only on service restart.

## Build and run without Compose

From the repository root, build with:

    docker build -f deploy/Dockerfile -t excession-service .

Provide an operator-owned GitHub source allowlist and a persistent random secret (at least 32 characters) through the deployment environment, not Git. Example:

    export EXCESSION_SERVICE_TOKEN="$(openssl rand -hex 32)"
    docker run --rm -p 8080:8080 \
      -e EXCESSION_SOURCES='{"excession":"https://github.com/laurajoyhutchins/excession.git"}' \
      -e EXCESSION_SERVICE_TOKEN="$EXCESSION_SERVICE_TOKEN" \
      excession-service

At startup, start.mjs clones complete history of the allowlisted public GitHub HTTPS repositories into the ephemeral filesystem. No request may change the list. Complete history is needed to compare base and candidate commits.

For a hosted service such as Cloud Run: push the built image to a registry; configure EXCESSION_SOURCES; inject EXCESSION_SERVICE_TOKEN from the host's secret manager; and allow for Git clone startup time and disk use. Keep Cloud Run IAM enforced where a gateway identity has invoker permission; the service-token check is independent.

For a mounted trusted local checkout, skip startup cloning and invoke node deploy/service/server.mjs with EXCESSION_REPOSITORIES containing {"alias":"/absolute/checkout"}, EXCESSION_SERVICE_TOKEN and optionally EXCESSION_BINARY.

## API

All routes except GET /healthz require Authorization: Bearer followed by the shared token.

| Route | Input | Action |
| --- | --- | --- |
| GET /v1/repositories | none | List permitted repository aliases and exact HEAD commits |
| POST /v1/agent/locate | repository, task, optional revision/limit | Run Excession agent locate |
| POST /v1/agent/context | repository, focus paths, optional revision/task/limit | Run Excession agent context |
| POST /v1/agent/verify | repository, base SHA, candidate SHA, optional limit | Run Excession agent verify |

Locate and context resolve HEAD to an immutable 40-character Git commit when no revision is supplied. Verify requires full SHAs. Each successful response preserves Excession's native JSON report, including coverage gaps and unresolved obligations. A result is never merge authorization.

## Owner-approved Google Cloud deployment experiment

A no-build Cloud Run **experiment** is staged in `deploy/cloudrun-bootstrap.mjs`. It downloads the pinned v0.2.0 Linux musl release, validates the full archive SHA-256, and starts the reviewed Rust adapter with a fixed public GitHub source allowlist. It assumes Git and tar are present in the chosen runtime image, a fact that has **not** been verified on Cloud Run. Its startup-generated token is intentionally not externally recoverable: no gateway connection is possible until a proper secret or private identity path is configured. Prefer the source-built Docker image for production.

The existing engineering GCP gateway accepted an exact request but returned `approval_required`; **nothing has been deployed**. The pending, secret-free request is captured as [`cloudrun-owner-request.json`](cloudrun-owner-request.json). Its verified approval digest is `ef9ab64cd430049bad2460b6ac5d3c940a533185fc9b3ff22e456dfefa134872`.

The existing [owner-approval workflow](https://github.com/laurajoyhutchins/engineering-infra/actions/workflows/mcp-tool-call-owner.yml) requires the owner to dispatch from `engineering-infra/main` with the JSON in `call_request` and the digest in `approved_digest`. This approval is not a grant of cloud permissions. The isolated tool identity is ordinarily a Cloud Run viewer, so the attempted deployment may still fail on IAM and must never be reported as ready until a new revision is independently verified. Do not bypass its identity or approval checks.

## Use the existing Hatchable connection from ChatGPT

No custom MCP server installation is required. The Hatchable project already exposes an **admin-only** `POST /api/run` endpoint. ChatGPT's connected Hatchable `run_function` operation can call it directly:

- `project_id`: `proj_NXfZ0HkcEsNG`
- `path`: `/api/run`
- `method`: `POST`
- `body`: `{"operation":"repositories"}`

For analysis, send `{"operation":"locate","arguments":{"repository":"excession","task":"Find the agent verification code","limit":10}}`. The other operations are `context` (requires `repository` and an array of exact `focus` paths) and `verify` (requires `repository`, full `base` SHA and full `candidate` SHA). The gateway accepts only these four operations. It rejects unknown commands, paths, repositories, and unprivileged callers; it cannot execute shell commands through this interface.

Hatchable console: https://hatchable.com/console/projects/excession-mcp

The separately running Rust container still needs configuring: set **backend_url** to its HTTPS origin and the `EXCESSION_SERVICE_TOKEN` secret in Hatchable Setup to the same value injected into the container. Until the Rust backend is deployed and these settings are configured, valid calls return `NOT_CONFIGURED` or `BACKEND_UNREACHABLE`. Hatchable is a dispatcher and auth boundary, not a Rust execution host.

Hatchable also publishes an optional MCP endpoint, but **ChatGPT need not register it as a custom MCP server** to use Excession through the existing Hatchable connector.

## Limits and verification

The adapter admits two concurrent analyses, kills child processes after 40 seconds, caps CLI output at 2 MiB, and caps request bodies at 16 KiB. There is no arbitrary shell, remote selection, test execution or write endpoint. HEAD becomes stale until a trusted clone is deliberately refreshed.

The bootstrap currently supports public GitHub HTTPS clones only. Private repositories need a separately authenticated source-ingestion path. Large repositories can incur costly cold starts; for them, use persistent storage and separate refresh/index workers. Hosting never upgrades the evidence limits stated in the existing Excession agent CLI documentation.

Run the adapter tests separately:

    node --test deploy/service/test/server.test.mjs
