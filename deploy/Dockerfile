# syntax=docker/dockerfile:1
# Builds the SolidCraft web app from source and serves it with Caddy: no Rust or Trunk needed on
# the host.   docker compose -f deploy/compose.yaml up --build
#
# The WASM link wants roughly 8-12 GB of free RAM. On a smaller machine, lower the parallelism:
#   docker compose -f deploy/compose.yaml build --build-arg JOBS=2
# Developing? deploy/Dockerfile.prebuilt packages a `trunk build --release` you did yourself (much faster).
FROM rust:1.95-bookworm AS build
ARG TRUNK_VERSION=v0.21.14
ARG TARGETARCH
ARG JOBS=
RUN set -eu; \
    case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
      amd64) arch=x86_64 ;; arm64) arch=aarch64 ;; \
      *) echo "unsupported architecture: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    rustup target add wasm32-unknown-unknown; \
    curl -fsSL "https://github.com/trunk-rs/trunk/releases/download/${TRUNK_VERSION}/trunk-${arch}-unknown-linux-gnu.tar.gz" \
      | tar -xz -C /usr/local/bin
WORKDIR /src
COPY . .
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    set -eu; \
    if [ -n "${JOBS}" ]; then export CARGO_BUILD_JOBS="${JOBS}"; fi; \
    cd apps/solidcraft-web && trunk build --release && cp -r /src/dist/web /out

FROM caddy:2-alpine
COPY deploy/Caddyfile /etc/caddy/Caddyfile
COPY --from=build /out /srv
EXPOSE 8080
HEALTHCHECK CMD wget -qO- http://127.0.0.1:8080/ >/dev/null || exit 1
