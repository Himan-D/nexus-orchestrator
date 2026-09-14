# Nexus core (Rust) — multi-stage build. Migrations are embedded in the binary.
FROM rust:1.95-slim AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
RUN cargo build --release

FROM debian:bookworm-slim
RUN groupadd --system nexus && useradd --system --gid nexus --create-home nexus
COPY --from=builder /build/target/release/orchestrator-core /usr/local/bin/nexus-core
# The SQLite volume lives under the user's home; pre-create so the mount
# lands on an owned directory.
RUN mkdir -p /home/nexus/data && chown -R nexus:nexus /home/nexus
USER nexus
WORKDIR /home/nexus
ENV ORCH_BIND=0.0.0.0:3002 ORCH_DB_URL=sqlite:///home/nexus/data/orchestrator.db?mode=rwc
EXPOSE 3002
# Runtime image ships without curl/wget on purpose; the binary self-probes.
HEALTHCHECK --interval=10s --timeout=3s --retries=5 \
  CMD ["nexus-core", "--probe"]
ENTRYPOINT ["/usr/local/bin/nexus-core"]
