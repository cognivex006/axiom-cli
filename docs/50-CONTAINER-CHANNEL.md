# Container distribution

The container is a separate Linux delivery path. Start with the [native installation guide](INSTALL.md) for a workstation installation.

## Base images

The build and runtime images are pinned in [Dockerfile](../containers/Dockerfile):

| Role | Image | Digest |
|---|---|---|
| Build | `rust:1.85-slim-bookworm` | `sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36` |
| Runtime | `debian:bookworm-slim` | `sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251` |

## Image and execution

`containers/Dockerfile` builds the CLI from digest-pinned base images with locked, offline Rust dependencies. It runs as non-root uid/gid `10001` and forwards arguments to the CLI. An owned writable volume holds installation/runtime state; user files and bindings must survive removal.

A bare CLI image is not a complete installed ecosystem. Missing engine, channel or runtime inputs produce a refusal; a container entrypoint must preserve the CLI's exit code. A container run is not native runtime evidence. Native Windows, macOS and WSL behavior require their own host checks.

## Build locally

```sh
docker build -f containers/Dockerfile \
  --build-arg SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
  --build-arg AXIOM_VERSION="$(tr -d '[:space:]' < VERSION)" \
  --build-arg AXIOM_REVISION="$(git rev-parse HEAD)" \
  --build-arg AXIOM_CREATED="$(git show -s --format=%cI HEAD)" \
  -t axiom-cli:local .
docker run --rm axiom-cli:local --help
docker image inspect axiom-cli:local
```

Local full-ecosystem candidates add verified core, MCP, skills and Python inputs. Their update kits bind both generations and use the installed engine transaction. They retain inactive verified payloads for retry after rollback. Building or testing such a candidate does not publish an image.

## Publication

`.github/workflows/publish-container.yml` has a separate opt-in publication path for `ghcr.io/orchex006/axiom-cli`. It records the resolved image digest. Version and source tags identify builds; no floating `latest` tag is used. An arm64 image requires its explicit build input and separate verification.

Consume an actually published image by its recorded digest. This guide does not supply a verified public image digest or claim container publication from the native archive release.

## Offline or air-gapped delivery

On a connected host, save a digest-verified image with `docker save`, record the archive's SHA-256 and transfer it with its checksum record. On the offline host, verify the transfer before `docker load`. Runtime provisioning likewise uses verified local Python, wheelhouse, wheel and lock inputs rather than downloading dependencies during activation.

Local validation covers non-root execution, query/watcher behavior, updates, rollback, interrupted operations and owned removal with user-data retention. Native targets and public registry publication remain separate claims.
