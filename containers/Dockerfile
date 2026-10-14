# axiom-cli - container delivery channel (`container-linux-x64`).
#
# Owner: axiom-cli. Normative rules: `axiom-specs`
# `contracts/axiom-cli-distribution-contract.md` section 5 "Container channel" and
# `compatibility/platform-matrix.json` `distribution.container`.
#
# Two rules this file exists to satisfy:
#   1. every base image is pinned by digest, never by a floating tag;
#   2. the runtime user is non-root.
#
# Running this image is NOT native runtime evidence for any mandatory native target
# (windows-x64, linux-x64, macos-arm64, macos-x64). The container is an additional
# delivery channel for Linux-family automation; it certifies nothing and is never a
# prerequisite for a native installation path.
#
# The build reaches no network beyond the two pinned base images: `axiom-cli` has no
# third-party dependencies, so the release build is `--locked --offline`.

FROM rust:1.85-slim-bookworm@sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36 AS builder

WORKDIR /src

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release --locked --offline --bin axiom-cli

FROM debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251 AS runtime

# `AXIOM_VERSION` is kept equal to the repository `VERSION` file by
# tests/container_distribution.rs. `AXIOM_REVISION` defaults to the literal
# `unknown` on purpose: a bare local build must not invent a revision. The
# publication workflow always passes the real commit.
# Declaring `SOURCE_DATE_EPOCH` lets BuildKit consume it, which makes the image config
# timestamp deterministic instead of `whenever the build happened to run`. The
# publication workflow passes the commit time; a local build may omit it.
ARG SOURCE_DATE_EPOCH
ARG AXIOM_VERSION=0.1.6
ARG AXIOM_REVISION=unknown
# Deterministic default keeps a bare local build reproducible; the workflow passes
# the real commit timestamp.
ARG AXIOM_CREATED=1970-01-01T00:00:00Z

LABEL org.opencontainers.image.title="axiom-cli" \
      org.opencontainers.image.description="Axiom ecosystem distribution entrypoint: install, update, doctor, version, uninstall. Additional container delivery channel; not native runtime evidence for any target." \
      org.opencontainers.image.source="https://github.com/orchex006/axiom-cli" \
      org.opencontainers.image.url="https://github.com/orchex006/axiom-cli" \
      org.opencontainers.image.documentation="https://github.com/orchex006/axiom-cli/blob/main/docs/50-CONTAINER-CHANNEL.md" \
      org.opencontainers.image.version="${AXIOM_VERSION}" \
      org.opencontainers.image.revision="${AXIOM_REVISION}" \
      org.opencontainers.image.created="${AXIOM_CREATED}" \
      org.opencontainers.image.licenses="UNLICENSED" \
      io.orchex006.axiom.channel="container-linux-x64" \
      io.orchex006.axiom.native-evidence="false"

RUN groupadd --gid 10001 axiom \
 && useradd --uid 10001 --gid 10001 --no-create-home --home-dir /home/axiom --shell /usr/sbin/nologin axiom \
 && install -d -o 10001 -g 10001 -m 0755 /home/axiom

COPY --from=builder --chown=0:0 --chmod=0755 /src/target/release/axiom-cli /usr/local/bin/axm
COPY --chown=0:0 --chmod=0755 containers/entrypoint.sh /usr/local/libexec/axm-entrypoint

USER 10001:10001
WORKDIR /home/axiom

# The entrypoint forwards argv verbatim to the binary, so the container exposes the
# same five verbs with the same exit codes as the native binary. No verb is added,
# removed or shimmed here.
ENTRYPOINT ["/usr/local/libexec/axm-entrypoint"]
CMD ["--help"]
