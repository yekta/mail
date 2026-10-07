# The server (apps/server). The apps are built by their own workflows.

# Pinned so a new Rust release doesn't throw away the cached dependency layers.
FROM rust:1.99-bookworm AS chef
RUN curl -L --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh | bash \
    && cargo binstall -y cargo-chef@0.1.78
WORKDIR /app

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS server
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release -p mail-server --recipe-path recipe.json
COPY . .
RUN cargo build --release -p mail-server \
    && mkdir /out && mv target/release/mail-server /out/ \
    && rm -rf target

FROM debian:bookworm-slim
LABEL org.opencontainers.image.source=https://github.com/yekta/wonnet
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=server /out/mail-server /usr/local/bin/mail-server
ENV PORT=3000 RUST_LOG=info
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD curl -fsS http://127.0.0.1:3000/healthz || exit 1
CMD ["mail-server"]
