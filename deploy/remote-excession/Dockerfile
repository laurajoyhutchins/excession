# Build from a reviewed Excession Git checkout, including .git.
# Each deployment serves only the checked-out Git objects in this image.
FROM rust:1.99.1-bookworm AS rust-build
WORKDIR /build
COPY . .
RUN cargo fetch --locked --manifest-path core/Cargo.toml && \
    cargo build --offline --locked --manifest-path core/Cargo.toml --release --bin excession

FROM node:24-bookworm-slim AS typescript
WORKDIR /build
COPY package.json package-lock.json ./
RUN npm ci --ignore-scripts --no-audit --no-fund

FROM node:24-bookworm-slim
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
    git python3 ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=rust-build /build/core/target/release/excession /usr/local/bin/excession
COPY --from=typescript /build/node_modules/typescript/lib/typescript.js /usr/local/bin/lib/typescript.js
COPY deploy/remote-excession/server.mjs /opt/excession/server.mjs
COPY --chown=node:node . /srv/excession
ENV NODE_ENV=production
ENV PORT=8080
ENV EXCESSION_BIN=/usr/local/bin/excession
ENV EXCESSION_REPOSITORIES='{"excession":"/srv/excession"}'
WORKDIR /opt/excession
USER node
# Git refuses source owned by a different UID; validate under the runtime
# identity rather than marking untrusted paths globally safe.
RUN test -d /srv/excession/.git && git -C /srv/excession rev-parse --verify HEAD
EXPOSE 8080
CMD ["node","/opt/excession/server.mjs"]
