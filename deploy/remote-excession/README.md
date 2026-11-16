# Remote Excession backend for the existing Hatchable MCP gateway

This service is a fixed, authenticated HTTP adapter over the **real Rust
Excession CLI**, not a second code-intelligence implementation. It supplies
the four routes expected by the existing `excession-mcp` Hatchable project:

| Route | Rust authority |
| --- | --- |
| `GET /v1/repositories` | `git rev-parse HEAD` and `HEAD^{tree}` for allowlisted checkouts |
| `POST /v1/agent/locate` | `excession agent locate` |
| `POST /v1/agent/context` | `excession agent context` |
| `POST /v1/agent/verify` | `excession agent verify` |

The backend accepts **repository aliases**, never an arbitrary path, URL,
command, Git remote, or untrusted checkout. It requires full 40-character
Git commit IDs when a request specifies an exact revision, limits the size of
inputs and outputs, and refuses on CLI errors. For source queries, Git remains
authoritative; dirty working-tree edits are not included. There are no test
execution, filesystem write, ref update, merge, or user-supplied receipt
methods. Excession may maintain its normal external analysis cache.

## Build and test

Node.js 24 and Git are sufficient to run the gateway contract tests:

```sh
node --test deploy/remote-excession/server.test.mjs
```

Build the container from an **actual pinned Excession Git checkout**. The
image contains that checkout, its real Git objects, the Rust CLI, and the
TypeScript sidecar. It does not manufacture replacement Git history:

```sh
docker build -f deploy/remote-excession/Dockerfile -t excession-remote:local .
```

The demo image exposes the alias `excession` for the exact Git objects
included in the Docker build context. For other repositories, supply an
approved, read-only Git object checkout in the runtime image or mounted
volume and set `EXCESSION_REPOSITORIES` to a JSON object mapping 1–32
aliases to its absolute paths. Treat bundled private Git objects as sensitive
build artifacts, including in container registries.

The service **will not start** without `EXCESSION_SERVICE_TOKEN` of at
least 32 characters and an allowlisted repository. A caller must send
`Authorization: Bearer <secret>`. Use a separately generated random secret
stored in the deployment platform and in Hatchable's project Setup. Never
commit it or put it in a command-line argument. The gateway does not provide
TLS itself. Run it only behind an HTTPS endpoint, such as Cloud Run.

### GCP deployment

Use the existing Google Cloud project, service identities, private build
pipeline and region where possible. A deployer with the necessary permissions
can build and push the image using an existing approved artifact registry
and deploy that image to Cloud Run. The container listens on `PORT`
(default 8080). Give the service **no writable source mount or repository
write permissions**. Provide the bearer token through a secret manager and
configure Cloud Run runtime identity for only the resources it needs.

The current Hatchable client uses a shared backend bearer token, **not a
Cloud Run OIDC identity token**. Consequently a Cloud Run service protected by
the standard IAM invoker check is *not directly callable* by this adapter.
There are two deployment choices requiring review:

1. Reuse an existing private authenticated identity bridge that can reach
   the service, preserving Cloud Run IAM authentication.
2. Expose only this fixed HTTPS bearer-authenticated endpoint using Cloud Run
   ingress, protected by a high-entropy token and appropriate network controls.

Do not quietly switch an existing private Cloud Run service to public
invocation just to satisfy the proxy. No GCP deployment, IAM change, secret
creation, or network exposure is performed by committing these files.

### Connect the existing Hatchable project

The existing project is **Excession MCP Gateway** at
`https://excession-mcp.hatchable.site`. Its MCP tools use
`lib/excession-client.js`, which already allows only these fixed routes,
and are restricted to `admin` access.

In its Configure screen, set `backend_url` to the reviewed HTTPS origin
of the deployed service. In Setup, set `EXCESSION_SERVICE_TOKEN` to the
matching secret. The project's `GET /api/status` (admin) must then return
`ready: true` and a list of real aliases and Git commit IDs.

Before considering this complete, test that:

- Anonymous access to Hatchable's status API returns 401.
- Missing/wrong backend bearer token returns 401.
- `/v1/repositories` exposes only approved aliases and exact 40-hex Git
  commit and tree IDs.
- Every MCP tool round trips through the backend into a real
  `excession.agent-*.v1` report without invoking a shell.
- Invalid repository alias, path traversal, wrong revision, malformed body,
  and oversized requests are rejected.

**A deployed Hatchable project without a configured backend is not a
working Excession integration.** The project currently reports
`NOT_CONFIGURED` until a valid backend URL and secret are supplied.

## Limits

This is a single-repository-image demonstration and allowlisted multi-repo
HTTP backend, not a multi-tenant workspace indexer. Building a container for
each source snapshot or supplying read-only volume content is a deployment
decision. Native analysis coverage can be incomplete. Passing Excession
verification does not establish behavioral correctness or authorize merge.
