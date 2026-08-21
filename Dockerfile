# The headless Fiber MCP server, as a tiny static image for a container manager
# like ToolHive. No Tauri, no webview, no OpenSSL — reqwest already uses rustls,
# so the binary links no system TLS and can be built fully static (musl) and
# dropped onto distroless/static.
#
#   docker build -t fiber-mcp .
#
# Nobody should have to run that, though: the release workflow publishes
# ghcr.io/mathiaswp/fiber-mcp for both architectures. See deploy/toolhive.md.

# syntax=docker/dockerfile:1

# --- build: a static musl binary, GUI feature off -------------------------
# Alpine is musl-native, so this is a host build, not a cross-compile — which
# keeps aws-lc-rs (rustls' crypto backend) happy. Its C build needs cmake/perl
# and a compiler; the bundled SQLite needs the C toolchain too.
#
# --platform=$BUILDPLATFORM is deliberately absent: under `docker buildx
# --platform`, each architecture builds on an emulated runner of its own arch,
# which is slower than cross-compiling but is the only way aws-lc-rs' assembly
# builds without a cross toolchain to configure.
FROM rust:1.90-alpine AS build
RUN apk add --no-cache build-base cmake perl clang clang-dev linux-headers nasm
WORKDIR /src/src-tauri
COPY src-tauri/ ./
# TARGETARCH is set for us by buildx, and defaults to the host's arch on a plain
# `docker build`, so both paths land on the right target without a flag.
ARG TARGETARCH
# --no-default-features drops Tauri entirely (see Cargo.toml [features]) — and
# with it the Linux keychain, whose D-Bus backend needs libdbus here and a
# session bus at runtime, neither of which a container has. Secrets come from
# FIBER_SECRETS instead. The musl target links the CRT statically by default, so
# the result needs no libc.
#
# The binary is copied to a fixed path because the target triple is not known to
# the next stage.
RUN case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-musl ;; \
      arm64) target=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && rustup target add "$target" \
 && cargo build --release --locked --no-default-features --bin fiber --target "$target" \
 && cp "target/$target/release/fiber" /fiber

# --- runtime: distroless/static — ca-certificates, nonroot, ~2 MB ---------
# rustls verifies against the platform trust store at runtime, so the image must
# carry CA roots; distroless/static ships them (bare scratch would need the
# bundle copied in and SSL_CERT_FILE set).
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /fiber /usr/local/bin/fiber

# Collections are read from here — mount your `sections/` directory in. Loader
# caches, history and spilled bodies are written here too, so keep it writable
# (by uid 65532, the nonroot user) and persistent if you want query_response to
# work across calls.
ENV FIBER_DATA_DIR=/data
VOLUME ["/data"]

# MCP over stdio — ToolHive attaches to this. Absolute path: distroless has no shell.
ENTRYPOINT ["/usr/local/bin/fiber", "mcp"]
