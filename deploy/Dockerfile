FROM rust:1.99.1-bookworm AS rust-build
WORKDIR /src
COPY core/ ./core/
RUN cargo build --manifest-path core/Cargo.toml --release --locked --bin excession

FROM node:22-bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=rust-build /src/core/target/release/excession /usr/local/bin/excession
COPY deploy/service/ ./service/
ENV PORT=8080
EXPOSE 8080
USER node
CMD ["node", "/app/service/start.mjs"]
