# Deploy the Excession agent gateway

This container runs the existing Rust Excession CLI against pinned Git objects. The HTTP layer only transports four read-only operations. It does not reimplement the analysis, scan arbitrary URLs, run tests or run user-supplied commands.

## Build and run

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

## Connect the Hatchable MCP project

Hatchable MCP endpoint: https://excession-mcp.hatchable.site/mcp

Hatchable console: https://hatchable.com/console/projects/excession-mcp

Configure the **backend_url** with the HTTPS origin of the running container. In Setup, configure the **EXCESSION_SERVICE_TOKEN** secret to match the value injected into the container. In the ChatGPT custom MCP integration, authorize this endpoint through Hatchable OAuth; all four MCP tools are gated to the Hatchable project admin.

The four tools are excession_repositories, excession_locate, excession_context, and excession_verify. Until the Rust backend is deployed and configured, the published MCP gateway responds NOT_CONFIGURED or BACKEND_UNREACHABLE. A deployed MCP gateway alone is not a working Excession analysis engine.

## Limits and verification

The adapter admits two concurrent analyses, kills child processes after 40 seconds, caps CLI output at 2 MiB, and caps request bodies at 16 KiB. There is no arbitrary shell, remote selection, test execution or write endpoint. HEAD becomes stale until a trusted clone is deliberately refreshed.

The bootstrap currently supports public GitHub HTTPS clones only. Private repositories need a separately authenticated source-ingestion path. Large repositories can incur costly cold starts; for them, use persistent storage and separate refresh/index workers. Hosting never upgrades the evidence limits stated in the existing Excession agent CLI documentation.

Run the adapter tests separately:

    node --test deploy/service/test/server.test.mjs
