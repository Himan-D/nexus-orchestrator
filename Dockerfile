# Nexus core (Rust) — multi-stage build. Migrations are embedded in the binary.
FROM rust:1.95-slim AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/orchestrator-core /usr/local/bin/nexus-core
ENV ORCH_BIND=0.0.0.0:3002
EXPOSE 3002
ENTRYPOINT ["/usr/local/bin/nexus-core"]
