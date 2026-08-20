# The headless Fiber MCP server, as a tiny static image for a container manager
# like ToolHive. No Tauri, no webview, no OpenSSL — reqwest already uses rustls,
# so the binary links no system TLS and can be built fully static (musl) and
# dropped onto distroless/static.
#
#   docker build -t fiber-mcp .
#
# See deploy/toolhive.md for the ToolHive recipe and the env contract.

# syntax=docker/dockerfile:1

# --- build: a static musl binary, GUI feature off -------------------------
# Alpine is musl-native, so this is a host build, not a cross-compile — which
# keeps aws-lc-rs (rustls' crypto backend) happy. Its C build needs cmake/perl
# and a compiler; the bundled SQLite needs the C toolchain too.
FROM rust:1.90-alpine AS build
RUN apk add --no-cache build-base cmake perl clang clang-dev linux-headers nasm
WORKDIR /src/src-tauri
COPY src-tauri/ ./
# --no-default-features drops Tauri entirely (see Cargo.toml [features]); the
# musl target links the CRT statically by default, so the result needs no libc.
# Naming the target explicitly keeps the output path deterministic even though
# it matches the Alpine host.
ARG TARGET=x86_64-unknown-linux-musl
RUN rustup target add "$TARGET" \
 && cargo build --release --locked --no-default-features --bin fiber --target "$TARGET"

# --- runtime: distroless/static — ca-certificates, nonroot, ~2 MB ---------
# rustls verifies against the platform trust store at runtime, so the image must
# carry CA roots; distroless/static ships them (bare scratch would need the
# bundle copied in and SSL_CERT_FILE set).
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /src/src-tauri/target/x86_64-unknown-linux-musl/release/fiber /usr/local/bin/fiber

# Collections are read from here — mount your `sections/` directory in. Loader
# caches, history and spilled bodies are written here too, so keep it writable
# (by uid 65532, the nonroot user) and persistent if you want query_response to
# work across calls.
ENV FIBER_DATA_DIR=/data
VOLUME ["/data"]

# MCP over stdio — ToolHive attaches to this. Absolute path: distroless has no shell.
ENTRYPOINT ["/usr/local/bin/fiber", "mcp"]
