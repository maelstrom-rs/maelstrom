# Multi-stage build for Maelstrom Matrix homeserver.
# Also used as the Complement test image (exposes 8008 + 8448).

# -- Stage 1: Build --
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Cache dependencies: copy manifests first, build a dummy, then copy source
COPY Cargo.toml Cargo.lock ./
COPY crates/maelstrom-core/Cargo.toml crates/maelstrom-core/Cargo.toml
COPY crates/maelstrom-storage/Cargo.toml crates/maelstrom-storage/Cargo.toml
COPY crates/maelstrom-media/Cargo.toml crates/maelstrom-media/Cargo.toml
COPY crates/maelstrom-api/Cargo.toml crates/maelstrom-api/Cargo.toml
COPY crates/maelstrom-federation/Cargo.toml crates/maelstrom-federation/Cargo.toml
COPY crates/maelstrom-admin/Cargo.toml crates/maelstrom-admin/Cargo.toml

# Create dummy source files for dependency caching
RUN mkdir -p src && echo 'fn main() {}' > src/main.rs && \
    for crate in maelstrom-core maelstrom-storage maelstrom-media maelstrom-api maelstrom-federation maelstrom-admin; do \
        mkdir -p crates/$crate/src && echo '' > crates/$crate/src/lib.rs; \
    done

RUN cargo build --release 2>/dev/null || true

# Copy real source and build
COPY src/ src/
COPY crates/ crates/

# Touch source files to invalidate cache
RUN find src crates -name "*.rs" -exec touch {} +
RUN cargo build --release

# -- Stage 2: Runtime --
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/maelstrom /usr/local/bin/maelstrom
COPY config/example.toml /etc/maelstrom/config.toml

ENV MAELSTROM_CONFIG=/etc/maelstrom/config.toml

EXPOSE 8008 8448

HEALTHCHECK --interval=10s --timeout=5s --retries=3 \
    CMD curl -f http://localhost:8008/_health/live || exit 1

ENTRYPOINT ["maelstrom"]
