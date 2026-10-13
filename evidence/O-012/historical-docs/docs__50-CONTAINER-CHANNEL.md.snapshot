# Container channel — axiom-cli

## K-011 final local lifecycle revalidation

`evidence/K-011/` records a fresh owned-volume installation of the pinned A
kit, real C# query and watcher runs with a foreground daemon, eight B-kit
refusal or injected failure cases, OCI-driven B apply/check and rollback,
fresh daemon starts after each transition, and digest-approved owned removal.
The final rebuilt image has the same CLI binary SHA as the exercised B image.
After removal the source, bindings and user-data SHA values match the last
pre-removal query; owned entrypoints and runtime pointers are absent.

Graphd and CLI retain verified, inactive B generation files after rollback
to permit a retry. An unchanged generation-directory listing is therefore not
the rollback invariant. The active pointers, entrypoint, user content and
absence of an uncommitted distribution receipt are checked separately, and
the retained B binaries must hash to the pinned kit. The original strict
directory-listing failure is preserved as diagnostic evidence.

The local image identity is an OCI image ID, not a published registry digest.
Foreground SIGINT drain was exercised. This Docker host has no `systemd --user`
manager, so no supervisor registration is claimed. The local kit is unsigned;
native Linux and the other release lanes remain separate release obligations.

## K-406 local update kit

The K-406 kit contains a pinned Linux candidate, its runtime receipt, the
Linux per-user installer and the update coordinator. A container using the
owned volume and managed Python may name the extracted kit with
`AXIOM_CLI_COMPOSITE_KIT` and run the public `axiom-cli update` verbs. Plan
approval binds the kit manifest, all 49 release files and installed A state.
The graphd-owned engine transaction activates core/MCP/skills; the coordinator
then installs the CLI entrypoint and records a rollback receipt. An injected
failure after engine activation restores A through the engine's rollback API.
The kit is unsigned and unpublished; it cannot certify any native lane.

## K-405 local candidate

`evidence/K-405/` records an unsigned Linux x64 candidate built from the
exact K-401 skills, K-402 core, K-403 wheel and K-404 runtime inputs. The
candidate archive contains the distribution files and input receipt; the
large CPython runtime and wheelhouse remain pinned by SHA in the K-404
handoff. The local OCI image was exercised as uid/gid 10001 with an owned
volume. Per-user CLI install, engine install, idempotent rerun, installed
foreground daemon, corrupt and unowned conflict refusals, injected
entrypoint-write failure, and owned removal were checked in Debian 12 x86_64
containers. `systemd --user` was unavailable, so the foreground policy was
used. No release image was published.

Owner: axiom-cli. Normative rules live in `axiom-specs`
`contracts/axiom-cli-distribution-contract.md` section 5 ("Container channel") and
`compatibility/platform-matrix.json` (`distribution.container`). This guide is the owner-side
runbook for the `container-linux-x64` delivery channel: it neither restates the contract nor
certifies any platform.

## What the channel is

| Field | Value |
|---|---|
| Registry | `ghcr.io` |
| Image | `ghcr.io/orchex006/axiom-cli` |
| Platform id | `container-linux-x64` |
| Artifact class | `oci-image` |
| Tier | `finish-first` |
| Native target | none |
| Immutable reference | `ghcr.io/orchex006/axiom-cli@sha256:<64-hex>` |

The image is an **additional delivery channel** for Linux-family automation. Running it is **not
native runtime evidence** for any mandatory native target (`windows-x64`, `linux-x64`,
`macos-arm64`, `macos-x64`), and it is never a prerequisite for a native installation path. A
container run must not be recorded against the `install`, `queue`, `guard`, `watcher`, `path`,
`migration` or `update` evidence of a native target.

## Base image pins

Both stages are pinned by digest in `containers/Dockerfile`; no stage uses a floating tag.

| Stage | Tag the digest was resolved from | Pinned digest |
|---|---|---|
| builder | `rust:1.85-slim-bookworm` | `sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36` |
| runtime | `debian:bookworm-slim` | `sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251` |

Resolved 2026-09-20 with `docker buildx imagetools inspect <tag>`. Because the pin is the
multi-architecture index digest, one pin serves both `linux/amd64` and `linux/arm64`: enabling the
arm64 leg needs no pin change. Bumping a pin is a deliberate edit to `containers/Dockerfile`, and
`tests/container_distribution.rs` fails when a pin changes unnoticed.

## Runtime identity

- Non-root runtime user `axiom`, uid/gid `10001`, `nologin` shell, home `/home/axiom` (also the `WORKDIR`).
- The runtime stage installs no package, no helper and no toolchain; the binary links nothing beyond the base C library at run time.
- The entrypoint is `#!/bin/sh` (the base image's POSIX `dash`). Bash, WSL, elevation and a container-orchestration agent are not required in order to run the binary.

## OCI labels

`containers/Dockerfile` declares `org.opencontainers.image.title`, `.description`, `.source`,
`.url`, `.documentation`, `.version`, `.revision`, `.created` and `.licenses`, plus
`io.orchex006.axiom.channel="container-linux-x64"` and
`io.orchex006.axiom.native-evidence="false"`. A bare local build defaults `AXIOM_REVISION` to the
literal `unknown` rather than inventing a commit; the publication workflow always passes the real
one.

## Entrypoint and verbs

`containers/entrypoint.sh` forwards argv verbatim — `exec /usr/local/bin/axiom-cli "$@"`. The
container adds no verb, removes none, renames none and re-implements none, so the argv surface and
the exit codes are the native ones by construction.

| Verb | Purpose | Exit code in this slice |
|---|---|---|
| `install` | Install or repair this platform's pinned component set | `2` bare verb (an explicit `--dry-run` or `--apply` is required); `0` `--dry-run` when a release set resolves; `4` `no_release_set`; `3`/`4` engine handoff on `--apply` |
| `update` | `check` / `plan` / `apply` / `rollback` through the update channel | see `docs/50-UPDATE-CHANNEL.md` |
| `doctor` | Report prerequisites, installed versions, digests and target state | `0` clean; `4` an unverified required check (for example no engine); `2` a failed installed-generation integrity check |
| `version` | Report installed and available versions per component and per target | `0` always, even with nothing installed |
| `uninstall` | Remove binaries and service registration; preserve user data | `2` bare verb; `0` `--dry-run`; `3` `nothing_installed` / `engine_not_found` on `--apply`; `4` `engine_removal_unavailable` |

Canonical exit vocabulary: `0 2 3 4 5 6 7 8 9 10 20`, owned by
`axiom-specs/docs/16-CLI-AND-CONTROL-API.md` section 6. All five verbs now dispatch to real work and
answer with their own typed exit code; `NotReady` (exit `4`) is a conditional refusal at the engine
handoff, not the universal answer. The container forwards argv verbatim, so it preserves those codes
exactly and never turns a refusal into a success. `--help` exits `0`; a missing verb and an unknown
verb exit `2`. The per-verb behaviour is tabulated in `docs/40-CLI-ARGV-SURFACE.md`.

## Build and run locally

```sh
# 1. build. Needs only the two pinned base images: axiom-cli has no third-party
#    dependencies, so the release build is `--locked --offline`.
docker build -f containers/Dockerfile \
  --build-arg SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)" \
  --build-arg AXIOM_VERSION="$(tr -d '[:space:]' < VERSION)" \
  --build-arg AXIOM_REVISION="$(git rev-parse HEAD)" \
  --build-arg AXIOM_CREATED="$(git show -s --format=%cI HEAD)" \
  -t axiom-cli:local .

# 2. inspect identity, labels and the pinned digests actually used
docker image inspect axiom-cli:local --format '{{.Id}} user={{.Config.User}} {{.Architecture}}/{{.Os}}'
docker image inspect axiom-cli:local --format '{{json .Config.Labels}}'
docker image inspect axiom-cli:local --format '{{json .RepoDigests}}'

# 3. exercise the verbs and read the real exit codes
docker run --rm axiom-cli:local --help         ; echo "exit=$?"   # 0
docker run --rm axiom-cli:local version        ; echo "exit=$?"   # 0
docker run --rm axiom-cli:local doctor         ; echo "exit=$?"   # 4 (engine unverified in a bare image)
docker run --rm axiom-cli:local --json doctor  ; echo "exit=$?"   # 4, one JSON object on stdout
docker run --rm axiom-cli:local install --apply --approve-digest "$(printf 'a%.0s' 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59 60 61 62 63 64)" ; echo "exit=$?"   # 4 no_release_set in a bare image
docker run --rm axiom-cli:local frobnicate     ; echo "exit=$?"   # 2 validation

# 4. the no-verb path must reach the binary, not a shell default
docker run --rm --entrypoint /usr/local/bin/axiom-cli axiom-cli:local \
  ; echo "exit=$?"                                                # 2, "no verb supplied"
```

## Publication

`.github/workflows/publish-container.yml` builds with buildx and publishes to
`ghcr.io/orchex006/axiom-cli`.

- `linux/amd64` is built by default (`finish-first`). `linux/arm64` is built only when the
  `publish_arm64` input is enabled, so an unverified arm64 image is never published as a placeholder.
- Tags are immutable: the `VERSION` value plus `git-<short-sha>`. No floating `latest` tag is published.
- Pushing is opt-in: a manual run defaults to build-and-record only, and a tag push publishes.
- The resolved digest is written to `axiom-cli-container-digest.txt`, uploaded as a build artifact
  and printed in the run summary as `ghcr.io/orchex006/axiom-cli@<digest>`.
- The workflow never overrides the base-image pins, never tags and never creates a release.
- The job certifies no platform and reports no native evidence category.

The digest belongs in `compatibility/platform-matrix.json`
(`distribution.container.published_image_digest`) once a real publication has happened. That field
is `null` today: this slice defines and locally proves the channel, and publication stays closed.

## Offline or air-gapped variant

The channel works without an internet path: the build fetches nothing beyond the two pinned base
images, and the binary needs no package index at run time.

Digests an air-gapped host needs:

| Need | Digest |
|---|---|
| builder base image | `sha256:9f841bbe9e7d8e37ceb96ed907265a3a0df7f44e3737d0b100e7907a679acb36` |
| runtime base image | `sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251` |
| the published `axiom-cli` image | unrecorded while `published_image_digest` is `null` |

Transport the image as a tarball between a connected host and the air-gapped host:

```sh
# connected host
docker pull ghcr.io/orchex006/axiom-cli@sha256:<recorded-digest>
docker save ghcr.io/orchex006/axiom-cli@sha256:<recorded-digest> -o axiom-cli-container.tar

# air-gapped host
docker load -i axiom-cli-container.tar
docker run --rm ghcr.io/orchex006/axiom-cli@sha256:<recorded-digest> version
```

To build inside the air gap, seed both pinned base images by `docker save` / `docker load` from a
connected host and build with `--pull=false`; the crate build is `cargo build --release --locked
--offline`. The verification rule does not relax: verify a digest before the image is used or
recorded, and never report an unverified image as working.

## Reproducibility

Pinned inputs: both base images by index digest, the dependency graph through `Cargo.lock` and
`cargo build --locked --offline`, the image version through `VERSION`, and the image config
timestamp through `SOURCE_DATE_EPOCH` when it is supplied (the workflow passes the commit time).

Measured on the J-006 host (Docker Desktop 29.6.2, engine `linux/amd64`):

| Property | Result |
|---|---|
| application payload across two clean `--no-cache` builds | bit-identical: `axiom-cli` sha256 `494a88903d13da47fa94ca8876bd611f8f9ef4bb4dd6ce8808d29b1aeb00c8d9`, entrypoint sha256 `a68fdce7ea99fbecb149ff0c65c29e10661299ae53e81a5bd3a5fc2eba20b2d2` |
| image config `created` with `SOURCE_DATE_EPOCH` set | deterministic: both clean builds reported `2026-09-20T07:43:20Z` |
| image digest across two clean `--no-cache` builds | **not** bit-identical on this builder |

The last row is a builder property, not a defect in this Dockerfile. The minimal counterexample is
the pinned runtime base image plus one `RUN`: two clean builds produced different layer `diffID`s (recorded instance: `evidence/J-006/j006-reproducibility.txt`, experiment `e1-pinned-base-plus-one-run`),
`sha256:0e76f239da4119bcdfde0ef33845a80edc8c853634526a87d22d04b39357fcfe` against
`sha256:d1953abe622922ed7cb08ec3f6d33da7623775c3475652bd07b8bf8322873794`. A `COPY` from the build
context and a `COPY --from` a stage were likewise non-deterministic, while the base image layer
itself was stable. Bit-identical image digests are therefore not a property this channel can promise
on an arbitrary builder.

Consequence: treat the digest as an output to record, never as a value to predict. The publication
workflow records the digest buildx actually produced, and `published_image_digest` must be filled
from that record. Re-running a publication cannot be assumed to reproduce an earlier digest, so an
image is identified and verified only by the digest recorded for it.

## Verification status

Recorded for task `J-006` on Windows 11 x64 with Docker Desktop 29.6.2 (engine `linux/amd64`) at
revision `7ac376b5e5e0f52f4fe3af7fb97971d220277d38`:

| Check | Result |
|---|---|
| local image build | exit `0`; labels, user, revision and platform verified with `docker image inspect` |
| image built and exercised | `axiom-cli:j006-local`, id `sha256:843e184865ccb2f161036a149330526a4952750708caa14a4528a8fa49c54d56`, `linux/amd64`, 75218565 bytes |
| runtime identity | `uid=10001(axiom) gid=10001(axiom)`, `HOME=/home/axiom`, `PWD=/home/axiom` |
| verb parity against the native binary | 18 of 18 argv cases matched, 0 mismatches, including `--json doctor` exiting `4` with exactly one JSON object on stdout. **Superseded for verb behaviour:** the matrix was captured at `J-006` against the pre-lifecycle argv surface; the exit codes changed when the five verbs were wired to real work. The argv forwarding below is unchanged. |
| no-verb path | explicit empty argv (CMD bypassed) exits `2` with the native `no verb supplied` message |
| image config `created` with `SOURCE_DATE_EPOCH` | `2026-09-20T07:43:20Z` in both clean builds |

A rebuild that reuses the build cache reproduces the image digest; a clean `--no-cache` rebuild does
not, for the reason measured in "Reproducibility" above. The digest above is the digest of the image
that was actually built and exercised.

Not executed, and therefore not claimed:

- no push to `ghcr.io`, no image tag and no release: publication is closed for this wave, so
  `published_image_digest` stays `null` and no image digest is recorded as published;
- `linux/arm64` was not built or verified;
- no signature or attestation was produced or verified;
- no native target is certified, and this channel contributes no native evidence.
