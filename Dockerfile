# Climax headless server (SERVER-PLAN.md Milestone C).
#
# Multi-stage: the SvelteKit SPA builds in a node stage, the Rust binary builds
# in a rust stage (rust-embed bakes the SPA in - release mode embeds bytes, so
# the final binary is fully self-contained), and the runtime stage is a slim
# Debian with just the binary + CA roots. reqwest uses rustls (no OpenSSL) and
# sqlx bundles SQLite, so no other system libraries are needed.
#
# Build:  docker build -t climax-server .
# Run:    docker run -d -p 9998:9998 -v climax-data:/data \
#           -e CLIMAX_TOKEN=change-me climax-server
# See deploy/README.md for compose + bare-Linux + full env reference.

# ---- Stage 1: the web UI ----
FROM node:22-slim AS web
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY svelte.config.js vite.config.js tsconfig.json ./
COPY static ./static
COPY src ./src
RUN npm run build

# ---- Stage 2: the server binary ----
FROM rust:1-bookworm AS build
WORKDIR /app
# The whole workspace manifest set is needed for cargo to parse the workspace,
# but only climax-server (+ climax-core) actually compiles - the Tauri desktop
# shell is parsed, never built.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src-tauri ./src-tauri
# The SPA build lands at repo-root build/ - exactly where climax-server's
# rust-embed expects it (its build.rs would create an empty one otherwise, which
# compiles but serves no UI; copying the real build BEFORE cargo runs means the
# release binary embeds the actual assets).
COPY --from=web /app/build ./build
RUN cargo build --release -p climax-server

# ---- Stage 3: runtime ----
FROM debian:bookworm-slim
# ca-certificates for rustls trust; tzdata so a named TZ (e.g. Europe/Berlin)
# resolves - without it the server's chrono::Local falls back to UTC and day
# boundaries (assigned_day) drift for clients in other timezones. Set the actual
# zone with the TZ env (see docker-compose.yml).
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends ca-certificates tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --system --create-home --uid 1000 climax \
    && mkdir -p /data \
    && chown climax:climax /data
COPY --from=build /app/target/release/climax-server /usr/local/bin/climax-server
USER climax
# 0.0.0.0 is correct INSIDE a container (the container's localhost isn't the
# host's); publish/firewall decides actual exposure. Set CLIMAX_TOKEN - the
# server warns loudly if reachable beyond loopback without one.
ENV CLIMAX_DATA_DIR=/data \
    CLIMAX_BIND=0.0.0.0 \
    CLIMAX_PORT=9998
EXPOSE 9998
# The database lives here - mount a volume or a restart loses everything.
VOLUME /data
ENTRYPOINT ["climax-server"]
