//! End-to-end coverage for the five distribution verbs.
//!
//! Every test runs the real built binary through `CARGO_BIN_EXE_axiom-cli` against a fixture
//! install root, release set and artifact cache in the process temp directory. Nothing here
//! reaches a network, a remote, a tag or a branch tip: the channel manifest is written by this
//! file, the artifact bytes are bytes this file creates, and the fixture is removed when the
//! test ends.
//!
//! The suite is organised around the distribution contract's own rule: every verb answers a real
//! result, a mutating verb needs an approval bound to the canonical plan digest, a refusal states
//! its class and reason, and a refusal never mutates the host. The `--json` envelope is checked on
//! every path because section 2 rule 2 makes it a public surface.
//!
//! `tests/argv_surface.rs` owns the frozen argv/usage surface; this file owns the behaviour behind
//! the verbs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Canonical exit vocabulary, owned by `axiom-specs/docs/16-CLI-AND-CONTROL-API.md` section 6.
const SUCCESS: i32 = 0;
const VALIDATION: i32 = 2;
const NOT_FOUND: i32 = 3;
const NOT_READY: i32 = 4;
const CONFLICT: i32 = 6;
const TIMEOUT: i32 = 7;
const PARTIAL: i32 = 20;

const COMPONENT: &str = "axiom-cli";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const REVISION: &str = "3333333333333333333333333333333333333333";
const TRUST_ROOT: &str = "23e4845c8e76c7cf8642b2c47f759d401345424309aec6b3bf4ad8413b40ee7a";
const SIGNATURE_ARTIFACT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const METADATA_EXPIRY: &str = "2027-09-20T00:00:00Z";
const SEED_GENERATION: &str = "g-20260920t000000z-00000001";
const SEED_PLAN_DIGEST: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A self-contained SHA-256, so the fixture can record real digests without a dependency.
mod sha256 {
    #[allow(clippy::needless_range_loop)]
    pub fn hex(input: &[u8]) -> String {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut state: [u32; 8] = [
            0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
            0x5be0cd19,
        ];
        let mut message = input.to_vec();
        let bit_len = (input.len() as u64).wrapping_mul(8);
        message.push(0x80);
        while message.len() % 64 != 56 {
            message.push(0);
        }
        message.extend_from_slice(&bit_len.to_be_bytes());
        for chunk in message.chunks(64) {
            let mut w = [0u32; 64];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[i * 4],
                    chunk[i * 4 + 1],
                    chunk[i * 4 + 2],
                    chunk[i * 4 + 3],
                ]);
            }
            for i in 16..64 {
                let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
                let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
                w[i] = w[i - 16]
                    .wrapping_add(s0)
                    .wrapping_add(w[i - 7])
                    .wrapping_add(s1);
            }
            let mut a = state[0];
            let mut b = state[1];
            let mut c = state[2];
            let mut d = state[3];
            let mut e = state[4];
            let mut f = state[5];
            let mut g = state[6];
            let mut h = state[7];
            for i in 0..64 {
                let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
                let ch = (e & f) ^ ((!e) & g);
                let t1 = h
                    .wrapping_add(s1)
                    .wrapping_add(ch)
                    .wrapping_add(K[i])
                    .wrapping_add(w[i]);
                let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
                let maj = (a & b) ^ (a & c) ^ (b & c);
                let t2 = s0.wrapping_add(maj);
                h = g;
                g = f;
                f = e;
                e = d.wrapping_add(t1);
                d = c;
                c = b;
                b = a;
                a = t1.wrapping_add(t2);
            }
            state[0] = state[0].wrapping_add(a);
            state[1] = state[1].wrapping_add(b);
            state[2] = state[2].wrapping_add(c);
            state[3] = state[3].wrapping_add(d);
            state[4] = state[4].wrapping_add(e);
            state[5] = state[5].wrapping_add(f);
            state[6] = state[6].wrapping_add(g);
            state[7] = state[7].wrapping_add(h);
        }
        {
            use std::fmt::Write;
            state.iter().fold(String::new(), |mut output, value| {
                write!(output, "{value:08x}").expect("writing to String cannot fail");
                output
            })
        }
    }
}

/// The canonical host id of the running test process (contract section 3).
fn host() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x64",
        ("macos", "x86_64") => "macos-x64",
        ("macos", "aarch64") => "macos-arm64",
        ("linux", "x86_64") => "linux-x64",
        other => panic!("this test does not declare the platform {other:?}"),
    }
}

/// A host this fixture declares an artifact for instead of the running one.
fn foreign_host() -> &'static str {
    if host() == "windows-x64" {
        "linux-x64"
    } else {
        "windows-x64"
    }
}

/// How the fixture's release set is deliberately shaped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Declared for this host, bytes present and matching the recorded digest.
    Present,
    /// Declared for this host, bytes present but not matching the recorded digest.
    Tampered,
    /// Declared for this host, no local bytes at all.
    Absent,
    /// Declared for a different host only, so this host has no artifact.
    ForeignHost,
}

fn payload() -> Vec<u8> {
    format!("axiom-cli 0.1.0 distribution fixture payload for {COMPONENT}\n")
        .repeat(2)
        .into_bytes()
}

fn artifact_name() -> String {
    format!("{COMPONENT}-{VERSION}.zip")
}

/// The fixture channel manifest, in the schema the channel parser accepts.
fn manifest_text(mode: Mode) -> String {
    let bytes = payload();
    let digest = sha256::hex(&bytes);
    let platform = if mode == Mode::ForeignHost {
        foreign_host()
    } else {
        host()
    };
    let component = format!(
        "{{\"component\":\"{COMPONENT}\",\"status\":\"declared\",\"version\":\"{VERSION}\",\
         \"revision\":\"{REVISION}\",\"declared_source\":\"axiom-cli/VERSION = {VERSION} (fixture)\",\
         \"needs_restart\":false,\"artifacts\":[{{\"platform\":\"{platform}\",\
         \"class\":\"per-user-installer\",\
         \"url\":\"https://fixtures.invalid/verbs/{}\",\
         \"sha256\":\"{digest}\",\"size_bytes\":{}}}]}}",
        artifact_name(),
        bytes.len(),
    );
    format!(
        "{{\"manifest_version\":1,\"channel\":\"stable\",\"updated_at\":\"2026-09-20T00:00:00Z\",\
         \"published\":false,\"note\":\"axiom-cli verb fixture; never published\",\
         \"trust\":{{\"metadata_version\":1,\"metadata_expiry\":\"{METADATA_EXPIRY}\",\
         \"trust_root\":\"{TRUST_ROOT}\",\"signature_artifact\":\"{SIGNATURE_ARTIFACT}\"}},\
         \"components\":[{component}]}}"
    )
}

// ---------------------------------------------------------------------------
// A release set the engine's ecosystem plan can actually consume
//
// `Mode::Present` declares only the distribution component `axiom-cli`, which is *not* one of the
// engine's core components: `install --apply` can therefore only ever reach the honesty boundary
// (`engine_bundle_components_missing`). These helpers write the shape the engine really consumes -
// the two core components plus an engine-format skills bundle - so the invoke path is provable.
// ---------------------------------------------------------------------------

/// The two components the engine's ecosystem plan requires in `bundle.json` (`INSTALL_ORDER`).
const GRAPHD: &str = "axiom-graphd";
const MCP: &str = "axiom-mcp";
const GRAPHD_VERSION: &str = "0.0.0-dev";
const MCP_VERSION: &str = "0.1.0";
/// The channel calls the skills pack `skills`; the engine's ecosystem calls it `axiom-skills`.
const SKILLS: &str = "skills";
const SKILLS_VERSION: &str = "0.1.0";
/// Pinned revisions, 40 lowercase hex, as the engine's `skills/bundle.json` requires.
const SKILLS_REVISION: &str = "4444444444444444444444444444444444444444";
const SPEC_REVISION: &str = "5555555555555555555555555555555555555555";

/// The digest the engine stand-in reports for its *own* plan.
///
/// The engine injects a random plan id and wall clock, so its plan digest is not reproducible.
/// This layer can never recompute that digest; it must carry the one the engine printed straight
/// into the apply step. The stand-in makes that carried value observable.
const ENGINE_PLAN_DIGEST: &str = "8abd32ab8abd32ab8abd32ab8abd32ab8abd32ab8abd32ab8abd32ab8abd32ab";

fn core_payload(component: &str, version: &str) -> Vec<u8> {
    format!("{component} {version} engine fixture payload\n")
        .repeat(2)
        .into_bytes()
}

/// The artifact file name, matching the basename of the manifest `url` (how bytes are resolved).
fn core_artifact(component: &str, version: &str) -> String {
    if component == MCP {
        format!("{component}-{version}-py3-none-any.whl")
    } else {
        format!("{component}-{version}.bin")
    }
}

fn component_row(component: &str, version: &str, bytes: &[u8]) -> String {
    let digest = sha256::hex(bytes);
    format!(
        "{{\"component\":\"{component}\",\"status\":\"declared\",\"version\":\"{version}\",\
         \"revision\":\"{REVISION}\",\"declared_source\":\"fixture\",\"needs_restart\":false,\
         \"artifacts\":[{{\"platform\":\"{}\",\"class\":\"per-user-installer\",\
         \"url\":\"https://fixtures.invalid/engine/{}\",\"sha256\":\"{digest}\",\
         \"size_bytes\":{}}}]}}",
        host(),
        core_artifact(component, version),
        bytes.len(),
    )
}

/// The channel row for the skills pack: no artifact (a skills bundle is not a channel artifact),
/// only the pinned revision the converter needs.
fn skills_channel_row() -> String {
    format!(
        "{{\"component\":\"{SKILLS}\",\"status\":\"declared\",\"version\":\"{SKILLS_VERSION}\",\
         \"revision\":\"{SKILLS_REVISION}\",\"declared_source\":\"fixture\",\
         \"needs_restart\":false,\"artifacts\":[]}}"
    )
}

fn core_manifest_text(rows: &[String]) -> String {
    format!(
        "{{\"manifest_version\":1,\"channel\":\"stable\",\"updated_at\":\"2026-09-20T00:00:00Z\",\
         \"published\":false,\"note\":\"engine fixture; never published\",\
         \"trust\":{{\"metadata_version\":1,\"metadata_expiry\":\"{METADATA_EXPIRY}\",\
         \"trust_root\":\"{TRUST_ROOT}\",\"signature_artifact\":\"{SIGNATURE_ARTIFACT}\"}},\
         \"components\":[{}]}}",
        rows.join(",")
    )
}

/// One payload file plus the engine-format `skills/bundle.json` that declares it.
fn skills_tree() -> (String, Vec<u8>) {
    let body = b"# axiom\n\nfixture skill payload for the engine bundle\n".to_vec();
    let digest = sha256::hex(&body);
    let manifest = format!(
        "{{\"schema_version\":1,\"component\":\"skills\",\"version\":\"{SKILLS_VERSION}\",\
         \"revision\":\"{SKILLS_REVISION}\",\"spec_revision\":\"{SPEC_REVISION}\",\
         \"entries\":[{{\"path\":\"instructions/axiom.md\",\"kind\":\"instruction\",\
         \"sha256\":\"{digest}\",\"size_bytes\":{}}}]}}",
        body.len()
    );
    (manifest, body)
}

/// An engine stand-in that answers both argv steps the way the real engine does.
///
/// `install plan --bundle <dir> --out <file>` writes the plan file the engine re-reads and prints
/// the engine's four-key summary; `install apply --plan <file> --approve-digest <d>` answers with
/// the engine's own `status`/`components` shape. Everything else is an unexpected argv, and says so.
const ENGINE_ACCEPTING: &str = r#"case "$1 $2" in
  "install plan")
    out=""
    prev=""
    for arg in "$@"; do
      if [ "$prev" = "--out" ]; then out="$arg"; fi
      prev="$arg"
    done
    printf '{"plan_version":1,"verb":"install","target":{"host":"macos-x64"}}\n' > "$out"
    printf '{"plan_digest":"@DIGEST@","plan_file":"%s","plan_id":"install-standin","status":"planned"}\n' "$out"
    exit 0
    ;;
  "install apply")
    printf '{"status":"installed","components":[{"position":1,"component":"@GRAPHD@","version":"@GV@","status":"installed","sha256":"0","destination":"bin/@GRAPHD@"},{"position":2,"component":"@MCP@","version":"@MV@","status":"installed","sha256":"0","destination":"python/@MCP@"},{"position":3,"component":"axiom-skills","version":"@SV@","status":"installed","sha256":"0","destination":"skills"}]}\n'
    exit 0
    ;;
esac
echo "engine stand-in: unexpected argv: $*" >&2
exit 90
"#;

fn engine_accepting_body() -> String {
    ENGINE_ACCEPTING
        .replace("@DIGEST@", ENGINE_PLAN_DIGEST)
        .replace("@GRAPHD@", GRAPHD)
        .replace("@GV@", GRAPHD_VERSION)
        .replace("@MCP@", MCP)
        .replace("@MV@", MCP_VERSION)
        .replace("@SV@", SKILLS_VERSION)
}

/// The same stand-in, but its `apply` step exits 0 without writing a status object.
///
/// The engine's exit code is its answer; what it *did* is named only by its status object. This
/// variant proves the boundary does not invent `installed` when that object is absent.
fn engine_silent_at_apply_body() -> String {
    let body = engine_accepting_body();
    let line = body
        .lines()
        .find(|line| line.contains("\"status\":\"installed\",\"components\""))
        .expect("the stand-in's apply step has a status line")
        .to_string();
    body.replacen(&line, "    :", 1)
}

/// An engine stand-in that refuses the way the real engine does: the envelope on stdout, one
/// `<CODE>: <message>` line on stderr, and the engine's own exit code.
fn engine_refusing_body(exit: i32, code: &str, rule: &str) -> String {
    format!(
        "printf '{{\"code\":\"{code}\",\"message\":\"the engine refused this bundle\",\
         \"retryable\":false,\"details\":{{\"rule\":\"{rule}\"}},\"request_id\":null}}\\n'\n\
         printf '{code}: the engine refused this bundle\\n' >&2\n\
         exit {exit}\n"
    )
}

/// One local, on-disk fixture: a release set, an install root and an artifact cache.
struct Fixture {
    dir: PathBuf,
    release: PathBuf,
    root: PathBuf,
    cache: PathBuf,
    manifest: PathBuf,
}

impl Fixture {
    fn new(name: &str, mode: Mode) -> Fixture {
        let ordinal = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "axiom-cli-verbs-{}-{ordinal}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let release = dir.join("release");
        let root = dir.join("root");
        let cache = dir.join("cache");
        std::fs::create_dir_all(&release).expect("the release set must be creatable");
        std::fs::create_dir_all(&root).expect("the install root must be creatable");
        std::fs::create_dir_all(&cache).expect("the artifact cache must be creatable");
        let manifest = release.join("channel.json");
        let fixture = Fixture {
            dir,
            release,
            root,
            cache,
            manifest,
        };
        std::fs::write(&fixture.manifest, manifest_text(mode).as_bytes())
            .expect("the fixture manifest must be writable");
        if mode == Mode::Present || mode == Mode::Tampered {
            let bytes = if mode == Mode::Tampered {
                format!("tampered bytes for {COMPONENT}\n").into_bytes()
            } else {
                payload()
            };
            std::fs::write(fixture.release.join(artifact_name()), &bytes)
                .expect("the release-set artifact must be writable");
            std::fs::write(fixture.cache.join(artifact_name()), &bytes)
                .expect("the cached artifact must be writable");
        }
        fixture
    }

    /// Seed an installed release whose recorded manifest is this fixture's manifest.
    fn seed_installed(&self, generation: &str, artifact_action: &str) {
        let manifest = std::fs::read(&self.manifest).expect("the fixture manifest must exist");
        let manifest_digest = sha256::hex(&manifest);
        std::fs::write(self.root.join("recorded-manifest.json"), &manifest)
            .expect("the recorded manifest must be writable");
        std::fs::write(
            self.root.join("installed.json"),
            format!(
                "{{\"schema_version\":1,\"channel\":\"stable\",\
                 \"channel_manifest_sha256\":\"{manifest_digest}\",\
                 \"current_generation\":\"{generation}\",\"previous_generation\":null,\
                 \"installed_at\":\"2026-09-20T00:00:00Z\"}}"
            ),
        )
        .expect("the installed record must be writable");
        let generation_dir = self.root.join("generations").join(generation);
        let bytes = payload();
        let digest = sha256::hex(&bytes);
        let relative = format!("payload/{COMPONENT}/{}", artifact_name());
        let path = generation_dir.join(&relative);
        std::fs::create_dir_all(path.parent().expect("the payload path has a parent"))
            .expect("the payload directory must be creatable");
        std::fs::write(&path, &bytes).expect("the payload must be writable");
        let entry = format!(
            "{{\"component\":\"{COMPONENT}\",\"action\":\"{artifact_action}\",\
             \"version\":\"{VERSION}\",\"revision\":\"{REVISION}\",\
             \"artifact_sha256\":\"{digest}\",\"payload\":\"{relative}\",\
             \"needs_restart\":false,\"probe\":null}}"
        );
        std::fs::write(
            generation_dir.join("generation.json"),
            format!(
                "{{\"schema_version\":1,\"generation_id\":\"{generation}\",\
                 \"transaction_id\":\"t-20260920t000000z-00000001\",\
                 \"plan_id\":\"update-stable-axiom-cli\",\"plan_digest\":\"{SEED_PLAN_DIGEST}\",\
                 \"channel\":\"stable\",\"host\":\"{}\",\"install_root\":\"{}\",\
                 \"created_at\":\"2026-09-20T00:00:00Z\",\"components\":[{entry}]}}",
                host(),
                self.root_text(),
            ),
        )
        .expect("the generation record must be writable");
    }

    /// Rewrite this fixture's release set into the shape the engine's ecosystem plan consumes.
    ///
    /// The two core components are written with real, verifiable bytes, and the skills pack is
    /// written as an engine-format pass-through tree (`skills/bundle.json` + `skills/payload/**`).
    fn seed_engine_release(&self) {
        let graphd = core_payload(GRAPHD, GRAPHD_VERSION);
        let mcp = core_payload(MCP, MCP_VERSION);
        std::fs::write(
            self.release.join(core_artifact(GRAPHD, GRAPHD_VERSION)),
            &graphd,
        )
        .expect("the graphd payload must be writable");
        std::fs::write(self.release.join(core_artifact(MCP, MCP_VERSION)), &mcp)
            .expect("the mcp payload must be writable");
        std::fs::write(
            &self.manifest,
            core_manifest_text(&[
                component_row(GRAPHD, GRAPHD_VERSION, &graphd),
                component_row(MCP, MCP_VERSION, &mcp),
                skills_channel_row(),
            ]),
        )
        .expect("the engine release manifest must be writable");
        let (manifest, body) = skills_tree();
        let skills = self.release.join("skills");
        let payload = skills.join("payload").join("instructions");
        std::fs::create_dir_all(&payload).expect("the skills payload directory must be creatable");
        std::fs::write(payload.join("axiom.md"), &body)
            .expect("the skills payload must be writable");
        std::fs::write(skills.join("bundle.json"), manifest)
            .expect("the skills bundle manifest must be writable");
    }

    /// Rewrite this fixture's release set to carry the *source* skills manifest instead.
    ///
    /// This is the `axiom-skills` repository schema (`skills-manifest.json` with `role`, `bytes`
    /// and a `capabilities` review only where a file can run code), which the engine does not
    /// accept as-is: the layer must convert it into `skills/bundle.json` + `skills/payload/**`.
    fn seed_engine_release_from_source_skills(&self) {
        let graphd = core_payload(GRAPHD, GRAPHD_VERSION);
        let mcp = core_payload(MCP, MCP_VERSION);
        std::fs::write(
            self.release.join(core_artifact(GRAPHD, GRAPHD_VERSION)),
            &graphd,
        )
        .expect("the graphd payload must be writable");
        std::fs::write(self.release.join(core_artifact(MCP, MCP_VERSION)), &mcp)
            .expect("the mcp payload must be writable");
        std::fs::write(
            &self.manifest,
            core_manifest_text(&[
                component_row(GRAPHD, GRAPHD_VERSION, &graphd),
                component_row(MCP, MCP_VERSION, &mcp),
                skills_channel_row(),
            ]),
        )
        .expect("the engine release manifest must be writable");
        let hook = b"# hook runtime\n\nfixture hook\n".to_vec();
        let skill = b"# skill\n\nfixture skill\n".to_vec();
        for (path, body) in [
            ("adapters/common/hook_runtime.py", &hook),
            ("skills/axiom-cli-install/SKILL.md", &skill),
        ] {
            let file = self.release.join(path);
            std::fs::create_dir_all(file.parent().expect("a parent directory"))
                .expect("the payload directory must be creatable");
            std::fs::write(&file, body).expect("the payload must be writable");
        }
        let manifest = format!(
            "{{\"manifest_version\":1,\"component_version\":\"{SKILLS_VERSION}\",\
             \"spec_revision\":\"{SPEC_REVISION}\",\"files\":[\
             {{\"path\":\"adapters/common/hook_runtime.py\",\"role\":\"host-hook-runtime\",\
             \"sha256\":\"{hook_digest}\",\"bytes\":{hook_len},\
             \"capabilities\":[\"read\",\"execute\"]}},\
             {{\"path\":\"skills/axiom-cli-install/SKILL.md\",\"role\":\"skill\",\
             \"sha256\":\"{skill_digest}\",\"bytes\":{skill_len}}}]}}",
            hook_digest = sha256::hex(&hook),
            hook_len = hook.len(),
            skill_digest = sha256::hex(&skill),
            skill_len = skill.len(),
        );
        std::fs::write(self.release.join("skills-manifest.json"), manifest)
            .expect("the source skills manifest must be writable");
    }

    /// Drop the skills tree, keeping the core components: the P1 gap, made visible.
    fn without_skills(&self) {
        let _ = std::fs::remove_dir_all(self.release.join("skills"));
    }

    fn root_text(&self) -> String {
        self.root.to_string_lossy().replace('\\', "/")
    }

    /// The active generation's payload file, for integrity tampering.
    fn payload_path(&self, generation: &str) -> PathBuf {
        self.root
            .join("generations")
            .join(generation)
            .join("payload")
            .join(COMPONENT)
            .join(artifact_name())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Outcome {
    code: i32,
    stdout: String,
    stderr: String,
}

/// How a test invocation is wired to the fixture.
struct Run<'a> {
    root: &'a Path,
    manifest: Option<&'a Path>,
    cache: Option<&'a Path>,
    engine: Option<&'a Path>,
    engine_bin_absent: bool,
    empty_path: bool,
    extra_env: Vec<(&'static str, &'static str)>,
}

impl<'a> Run<'a> {
    fn new(root: &'a Path) -> Run<'a> {
        Run {
            root,
            manifest: None,
            cache: None,
            engine: None,
            engine_bin_absent: false,
            empty_path: false,
            extra_env: Vec::new(),
        }
    }

    fn with_env(mut self, key: &'static str, value: &'static str) -> Run<'a> {
        self.extra_env.push((key, value));
        self
    }

    fn with_candidate_manifest(mut self, manifest: &'a Path, cache: &'a Path) -> Run<'a> {
        self.manifest = Some(manifest);
        self.cache = Some(cache);
        self
    }

    fn with_engine(mut self, engine: &'a Path) -> Run<'a> {
        self.engine = Some(engine);
        self
    }

    /// Force engine discovery to fail: no override, no candidate on PATH.
    fn without_engine(mut self) -> Run<'a> {
        self.engine_bin_absent = true;
        self.empty_path = true;
        self
    }

    fn out(&self, args: &[&str]) -> Outcome {
        let mut command = Command::new(env!("CARGO_BIN_EXE_axiom-cli"));
        command.env_remove("AXIOM_CLI_INSTALL_ROOT");
        command.env_remove("AXIOM_CLI_CHANNEL_MANIFEST");
        command.env_remove("AXIOM_CLI_ARTIFACT_CACHE");
        command.env_remove("AXIOM_ENGINE_BIN");
        command.env_remove("AXIOM_INSTALL_YES");
        // CI runners (GitHub Windows) run elevated; the refusal itself is unit-tested.
        command.env("AXIOM_CLI_TEST_ALLOW_ELEVATED", "1");
        command.env("AXIOM_CLI_INSTALL_ROOT", self.root);
        // Never let a test reach the real user PATH: Windows writes below a per-fixture
        // HKCU\Software\AxiomCliTest key, POSIX writes the profile of a per-fixture HOME.
        let tag = self
            .root
            .to_string_lossy()
            .bytes()
            .fold(0u64, |hash, byte| {
                hash.wrapping_mul(31).wrapping_add(u64::from(byte))
            });
        command.env(
            "AXIOM_CLI_TEST_USER_ENV_KEY",
            format!("Software\\AxiomCliTest\\{tag:x}"),
        );
        if let Some(parent) = self.root.parent() {
            let home = parent.join("home");
            let _ = std::fs::create_dir_all(&home);
            command.env("HOME", &home);
            command.env("SHELL", "/bin/sh");
        }
        for (key, value) in &self.extra_env {
            command.env(key, value);
        }
        if let Some(manifest) = self.manifest {
            command.env("AXIOM_CLI_CHANNEL_MANIFEST", manifest);
        }
        if let Some(cache) = self.cache {
            command.env("AXIOM_CLI_ARTIFACT_CACHE", cache);
        }
        if let Some(engine) = self.engine {
            command.env("AXIOM_ENGINE_BIN", engine);
        }
        if self.engine_bin_absent {
            command.env("AXIOM_ENGINE_BIN", "");
        }
        if self.empty_path {
            command.env("PATH", "");
        }
        let output = command
            .args(args)
            .output()
            .expect("the built axiom-cli binary must be runnable");
        Outcome {
            code: output
                .status
                .code()
                .expect("axiom-cli must exit with a code, not a signal"),
            stdout: String::from_utf8(output.stdout).expect("stdout must be UTF-8"),
            stderr: String::from_utf8(output.stderr).expect("stderr must be UTF-8"),
        }
    }
}

/// The distribution executable itself, used as a stand-in "engine binary that exists and runs".
///
/// `install --apply` and `uninstall --apply` locate the engine and then refuse because the
/// engine-owned placement/removal is not consumable from here, so any runnable executable proves
/// the *locate-then-refuse* boundary without depending on `axiom-graphd` being built.
fn engine_stand_in() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_axiom-cli"))
}

/// A stand-in engine script that can be made to exit a chosen code or write chosen stdout.
///
/// `doctor` invokes a located engine with `version --json`, so the failure boundary needs a real
/// process that exits non-zero, or exits zero with non-JSON stdout. A shell script is the
/// smallest such process and needs no new dependency; the product itself never shells out.
/// Returns `None` (and prints why) when this host offers no usable `sh`, so a host without one
/// skips the test instead of failing it.
fn shell_engine(dir: &Path, name: &str, body: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        static NATIVE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        let native = NATIVE.get_or_init(|| {
            let output = std::env::temp_dir().join(format!(
                "axiom-cli-native-engine-fixture-{}.exe",
                std::process::id()
            ));
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/native_engine_stub.rs");
            let status = Command::new("rustc")
                .args(["+1.85.0", "--edition=2021"])
                .arg(source)
                .arg("-o")
                .arg(&output)
                .status()
                .expect("native fixture compiler");
            assert!(status.success(), "native engine fixture must compile");
            output
        });
        std::fs::create_dir_all(dir).expect("fixture directory");
        let path = dir.join(format!("{name}.exe"));
        std::fs::copy(native, &path).expect("native fixture executable");
        std::fs::write(path.with_extension("fixture"), body).expect("fixture response body");
        Some(path)
    }
    #[cfg(not(windows))]
    {
        let sh_works = Command::new("sh")
            .arg("-c")
            .arg("exit 0")
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !sh_works {
            eprintln!("skipping: this host has no usable `sh` to build an engine stand-in");
            return None;
        }
        std::fs::create_dir_all(dir).expect("the engine stand-in directory must be creatable");
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n"))
            .expect("the engine stand-in script must be writable");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                .expect("the engine stand-in must be executable");
        }
        Some(path)
    }
}

/// Assert `text` is exactly one JSON object and nothing else.
fn assert_single_json_object(text: &str) {
    let trimmed = text.trim();
    assert!(
        trimmed.starts_with('{'),
        "stdout must be one JSON object, got: {text:?}"
    );
    assert!(
        trimmed.ends_with('}'),
        "stdout must be one JSON object, got: {text:?}"
    );
    let mut depth = 0i32;
    let mut top_level = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for character in trimmed.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    top_level += 1;
                }
                depth += 1;
            }
            '}' => depth -= 1,
            _ => {}
        }
    }
    assert_eq!(depth, 0, "unbalanced braces in: {text:?}");
    assert_eq!(
        top_level, 1,
        "expected exactly one JSON object, got {top_level}: {text:?}"
    );
}

fn json_string_field(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = json.find(&needle)? + needle.len();
    let mut out = String::new();
    let mut chars = json[start..].chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => return Some(out),
            '\\' => match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some(other) => out.push(other),
                None => return None,
            },
            other => out.push(other),
        }
    }
    None
}

fn json_number_field(json: &str, key: &str) -> Option<i64> {
    let needle = format!("\"{key}\":");
    let start = json.find(&needle)? + needle.len();
    let digits: String = json[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '-')
        .collect();
    digits.parse().ok()
}

/// The object literal that follows `"key":` in the canonical envelope.
fn json_object_field<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\":");
    let mut search = json;
    loop {
        let start = search.find(&needle)? + needle.len();
        let rest = &search[start..];
        if let Some(body) = rest.strip_prefix('{') {
            let mut depth = 1i32;
            let mut in_string = false;
            let mut escaped = false;
            for (index, character) in body.char_indices() {
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if character == '\\' {
                        escaped = true;
                    } else if character == '"' {
                        in_string = false;
                    }
                    continue;
                }
                match character {
                    '"' => in_string = true,
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(&body[..index]);
                        }
                    }
                    _ => {}
                }
            }
            return None;
        }
        search = &search[start..];
    }
}

/// The `findings[]` object whose `check` equals `check`.
///
/// Doctor findings are emitted with alphabetically sorted keys (`check`, then `message`, then
/// `status`, then any details), so no two fields are ever adjacent in the raw text. Matching on
/// `"check":"X","status":"Y"` would therefore never succeed; this walks the enclosing object and
/// returns its body so callers can read fields independently.
fn finding_object<'a>(json: &'a str, check: &str) -> Option<&'a str> {
    let needle = format!("\"check\":\"{check}\"");
    let position = json.find(&needle)?;
    let start = json[..position].rfind('{')?;
    let body = &json[start + 1..];
    let mut depth = 1i32;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in body.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&body[..index]);
                }
            }
            _ => {}
        }
    }
    None
}

/// The `status` of the `findings[]` object whose `check` equals `check`.
fn finding_status(json: &str, check: &str) -> Option<String> {
    json_string_field(finding_object(json, check)?, "status")
}

/// Read a string field of the *top-level* envelope, ignoring the same-named field nested inside
/// `details`.
///
/// The canonical envelope sorts its keys, so `details` always precedes `status`; a naive `find`
/// would return the first nested `status` (a doctor finding, for example) instead of the
/// envelope's own class token.
fn envelope_string_field(json: &str, key: &str) -> Option<String> {
    let (skip_start, skip_end) = match json_object_field(json, "details") {
        Some(body) => {
            let start = body.as_ptr() as usize - json.as_ptr() as usize;
            (start.saturating_sub(1), start + body.len())
        }
        None => (0, 0),
    };
    let needle = format!("\"{key}\":\"");
    let mut search_from = 0usize;
    while let Some(relative) = json[search_from..].find(&needle) {
        let at = search_from + relative;
        if at < skip_start || at > skip_end {
            return json_string_field(&json[at..], key);
        }
        search_from = at + needle.len();
    }
    None
}

/// Recursively count entries under `dir`.
fn entries(dir: &Path) -> usize {
    let mut total = 0usize;
    let Ok(read) = std::fs::read_dir(dir) else {
        return 0;
    };
    for item in read.flatten() {
        total += 1;
        if item.path().is_dir() {
            total += entries(&item.path());
        }
    }
    total
}

/// Assert the install root is exactly as it was: nothing installed, nothing written.
fn assert_root_untouched(fx: &Fixture, context: &str) {
    assert!(
        !fx.root.join("installed.json").exists(),
        "{context} must not record an installed release"
    );
    assert_eq!(
        entries(&fx.root),
        0,
        "{context} must not write anything under the install root"
    );
}

/// Assert a refusal envelope: canonical code, canonical status, one JSON object, a reason.
fn assert_refusal(out: &Outcome, code: i32, status: &str, reason_contains: &str) {
    assert_single_json_object(&out.stdout);
    assert_eq!(
        out.code, code,
        "expected exit {code}, got {}; stdout={} stderr={}",
        out.code, out.stdout, out.stderr
    );
    assert_eq!(
        json_number_field(&out.stdout, "code"),
        Some(i64::from(code)),
        "the envelope code must equal the process exit code"
    );
    assert_eq!(
        envelope_string_field(&out.stdout, "status").as_deref(),
        Some(status),
        "envelope status must match the canonical class"
    );
    let reason = json_string_field(&out.stdout, "reason_code").unwrap_or_default();
    assert!(
        reason.contains(reason_contains),
        "reason_code must contain `{reason_contains}`, got `{reason}`; stdout={}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------
// version
// ---------------------------------------------------------------------------

#[test]
fn version_succeeds_with_nothing_installed() {
    let fx = Fixture::new("version-clean", Mode::Present);
    let out = Run::new(&fx.root).out(&["version", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        out.code, SUCCESS,
        "`version` must succeed on a clean host: {}",
        out.stdout
    );
    assert_eq!(json_number_field(&out.stdout, "code"), Some(0));
    assert_eq!(
        json_string_field(&out.stdout, "status").as_deref(),
        Some("ok")
    );
    assert!(
        out.stdout.contains("\"installed\":false"),
        "a clean host must be reported as not installed: {}",
        out.stdout
    );
    assert_eq!(
        json_string_field(&out.stdout, "available_source").as_deref(),
        Some("unresolved"),
        "with no manifest reachable the available set must be reported unresolved"
    );
    assert_eq!(
        json_string_field(&out.stdout, "target").as_deref(),
        Some(host()),
        "the host block must report the contract target id"
    );
}

#[test]
fn version_plain_distinguishes_aliases_and_recorded_versions_from_updates() {
    let fx = Fixture::new("version-plain", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let out = Run::new(&fx.root).out(&["version"]);
    assert_eq!(out.code, SUCCESS, "{}", out.stdout);
    assert!(out
        .stdout
        .contains(&format!("{:<19} {:<8} {}", "Axiom CLI", "axm", VERSION)));
    assert!(out
        .stdout
        .contains("Axiom Graph Engine  axge     not recorded"));
    assert!(out.stdout.contains("Axiom MCP"));
    assert!(!out.stdout.contains("installed:"));
    assert!(!out.stdout.contains("axiom-cli"));
    assert!(!out.stdout.contains("Recorded channel"));
    assert!(!out.stdout.contains("reported installed"));
    assert!(!out.stdout.contains(SEED_GENERATION));
    assert!(!out.stdout.contains("up to date"));
    assert!(!out.stdout.contains("action="));
    let verbose = Run::new(&fx.root).out(&["version", "--verbose"]);
    assert_eq!(verbose.code, SUCCESS);
    assert!(verbose.stdout.contains(SEED_GENERATION));
    assert!(verbose.stdout.contains("action="));
}

#[test]
fn version_all_reports_the_cli_host_and_engine_blocks() {
    let fx = Fixture::new("version-all", Mode::Present);
    let out = Run::new(&fx.root).out(&["version", "--all", "--json"]);
    assert_eq!(out.code, SUCCESS, "stdout={}", out.stdout);
    assert_single_json_object(&out.stdout);
    assert!(out.stdout.contains("\"all\":true"));
    for block in ["cli", "host", "engine"] {
        assert!(
            json_object_field(&out.stdout, block).is_some(),
            "`version --all` must report the `{block}` block: {}",
            out.stdout
        );
    }
    assert_eq!(
        json_string_field(&out.stdout, "program").as_deref(),
        Some("axm")
    );
    assert_eq!(
        json_string_field(&out.stdout, "version").as_deref(),
        Some(VERSION)
    );
}

#[test]
fn version_reads_the_recorded_manifest_when_a_release_is_installed() {
    let fx = Fixture::new("version-installed", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let out = Run::new(&fx.root).out(&["version", "--json"]);
    assert_eq!(out.code, SUCCESS, "stdout={}", out.stdout);
    assert_single_json_object(&out.stdout);
    assert!(
        json_object_field(&out.stdout, "installed").is_some(),
        "an installed host must report the installed block: {}",
        out.stdout
    );
    let source = json_string_field(&out.stdout, "available_source").unwrap_or_default();
    assert!(
        source.starts_with("recorded:"),
        "the available set must come from the recorded manifest, got `{source}`"
    );
}

#[test]
fn version_reports_the_installed_generation_and_its_components() {
    // `version` is the operator's answer to "what is installed here?". With a seeded generation it
    // must report the installed block, name the active generation and channel, and list the
    // component the generation actually holds - never claim a clean host.
    let fx = Fixture::new("version-installed-generation", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let out = Run::new(&fx.root).out(&["version", "--json"]);
    assert_eq!(out.code, SUCCESS, "stdout={}", out.stdout);
    assert_single_json_object(&out.stdout);
    assert!(
        !out.stdout.contains("\"installed\":false"),
        "an installed host must not be reported clean: {}",
        out.stdout
    );
    let installed = json_object_field(&out.stdout, "installed")
        .expect("an installed host must report the installed block");
    assert_eq!(
        json_string_field(installed, "generation").as_deref(),
        Some(SEED_GENERATION),
        "the active generation must be named: {installed}"
    );
    assert_eq!(
        json_string_field(installed, "channel").as_deref(),
        Some("stable"),
        "the installed channel must be named: {installed}"
    );
    assert!(
        installed.contains(&format!("\"component\":\"{COMPONENT}\"")),
        "the component list must include the distributed CLI: {installed}"
    );
}

#[test]
fn version_all_sets_the_all_detail_with_an_installed_generation() {
    // `--all` widens the component report to the whole channel. The flag must be echoed in
    // `details.all` so a consumer can tell the widened shape apart from the default one.
    let fx = Fixture::new("version-all-installed", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let out = Run::new(&fx.root).out(&["version", "--all", "--json"]);
    assert_eq!(out.code, SUCCESS, "stdout={}", out.stdout);
    assert_single_json_object(&out.stdout);
    let details =
        json_object_field(&out.stdout, "details").expect("the envelope must carry details");
    assert!(
        details.contains("\"all\":true"),
        "`--all` must be recorded in details.all: {details}"
    );
}

#[test]
fn version_reports_the_candidate_manifest_when_nothing_is_installed() {
    let fx = Fixture::new("version-candidate", Mode::Present);
    let out = Run::new(&fx.root)
        .with_candidate_manifest(&fx.manifest, &fx.cache)
        .out(&["version", "--json"]);
    assert_eq!(out.code, SUCCESS, "stdout={}", out.stdout);
    let source = json_string_field(&out.stdout, "available_source").unwrap_or_default();
    assert!(
        source.starts_with("AXIOM_CLI_CHANNEL_MANIFEST:"),
        "an explicit candidate manifest must be reported as such, got `{source}`"
    );
    assert!(
        source.contains(&fx.manifest.display().to_string()),
        "the reported candidate must name the manifest that was supplied, got `{source}`"
    );
    assert!(
        out.stdout.contains("\"installed\":false"),
        "naming a candidate manifest must not claim anything is installed"
    );
}

#[test]
fn version_json_is_one_object_for_every_shape() {
    let fx = Fixture::new("version-shapes", Mode::Present);
    for args in [
        vec!["version", "--json"],
        vec!["--json", "version"],
        vec!["version", "--all", "--json"],
        vec!["version", "--json", "--verbose"],
    ] {
        let out = Run::new(&fx.root).out(&args);
        assert_single_json_object(&out.stdout);
        assert_eq!(out.code, SUCCESS, "argv={args:?} stdout={}", out.stdout);
        assert_eq!(
            json_number_field(&out.stdout, "code"),
            Some(0),
            "argv={args:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// doctor
// ---------------------------------------------------------------------------

#[test]
fn doctor_reports_structural_findings_and_never_fails_silently() {
    let fx = Fixture::new("doctor-clean", Mode::Present);
    let out = Run::new(&fx.root).out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    assert!(
        out.code == SUCCESS || out.code == NOT_READY,
        "`doctor` must report success or not-ready, got {}; stdout={}",
        out.code,
        out.stdout
    );
    assert_eq!(
        json_number_field(&out.stdout, "code"),
        Some(i64::from(out.code))
    );
    for check in ["install_root", "target", "prerequisites", "engine"] {
        assert!(
            out.stdout.contains(&format!("\"check\":\"{check}\"")),
            "`doctor` must report the `{check}` check: {}",
            out.stdout
        );
    }
    assert_eq!(
        json_string_field(&out.stdout, "target").as_deref(),
        Some(host())
    );
    // A clean host must not claim an installed release.
    assert!(out.stdout.contains("\"installed\":false"));
    // The anti-false-pass invariant: an unverified engine check must not exit 0.
    if finding_status(&out.stdout, "engine").as_deref() == Some("unverified") {
        assert_ne!(
            out.code, SUCCESS,
            "an unverified required check must not be reported as success: {}",
            out.stdout
        );
        assert_eq!(out.code, NOT_READY);
    }
}

#[test]
fn doctor_exits_four_when_the_engine_is_absent_and_states_the_searched_places() {
    let fx = Fixture::new("doctor-no-engine", Mode::Present);
    let out = Run::new(&fx.root)
        .without_engine()
        .out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    if finding_status(&out.stdout, "engine").as_deref() == Some("unverified") {
        assert_eq!(out.code, NOT_READY, "stdout={}", out.stdout);
        assert_eq!(
            envelope_string_field(&out.stdout, "status").as_deref(),
            Some("not_ready")
        );
        assert!(
            out.stdout.contains("searched"),
            "a missing engine must name the places that were searched: {}",
            out.stdout
        );
    } else {
        // A sibling `axiom` binary exists on this host; the engine check then owns the verdict.
        let status = finding_status(&out.stdout, "engine");
        assert!(
            matches!(status.as_deref(), Some("ok") | Some("fail")),
            "the engine finding must be present: {}",
            out.stdout
        );
    }
}

#[test]
fn doctor_plain_names_not_ready_and_the_missing_engine() {
    let fx = Fixture::new("doctor-plain-no-engine", Mode::Absent);
    let out = Run::new(&fx.root).without_engine().out(&["doctor"]);
    assert_eq!(out.code, NOT_READY);
    assert!(out.stdout.is_empty());
    assert!(out.stderr.contains("NotReady"));
    assert!(out.stderr.contains("not installed"));
    assert!(out.stderr.contains("engine"));
    assert!(out.stderr.contains("Next: axm install"));
}

#[test]
fn doctor_detects_a_tampered_installed_generation() {
    let fx = Fixture::new("doctor-tampered", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    std::fs::write(
        fx.payload_path(SEED_GENERATION),
        b"tampered payload bytes\n",
    )
    .expect("the payload must be rewritable");

    let out = Run::new(&fx.root).out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_ne!(
        out.code, SUCCESS,
        "a tampered generation must not exit 0: {}",
        out.stdout
    );
    // An integrity failure keeps its canonical class (validation, 2) and is surfaced as a
    // top-level reason rather than folded into the "not ready" aggregate.
    assert_eq!(out.code, VALIDATION, "stdout={}", out.stdout);
    assert!(
        finding_status(&out.stdout, "installed_release").as_deref() == Some("fail"),
        "the integrity failure must be reported as a failed check: {}",
        out.stdout
    );
    let reason = json_string_field(&out.stdout, "reason_code").unwrap_or_default();
    assert!(
        reason.contains("payload_digest_mismatch"),
        "an integrity failure must carry a payload digest reason, got `{reason}`: {}",
        out.stdout
    );
}

#[test]
fn doctor_reports_a_healthy_installed_generation_as_ok() {
    let fx = Fixture::new("doctor-healthy", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let out = Run::new(&fx.root)
        .with_candidate_manifest(&fx.manifest, &fx.cache)
        .out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        finding_status(&out.stdout, "installed_release").as_deref(),
        Some("ok"),
        "a verified generation must be reported ok: {}",
        out.stdout
    );
    assert_ne!(
        out.code, VALIDATION,
        "`doctor` must not answer a validation code for a healthy host"
    );
}

#[test]
fn doctor_reports_a_missing_engine_sqlite_probe_as_a_failure() {
    let fx = Fixture::new("doctor-sqlite-driver", Mode::Present);
    let out = Run::new(&fx.root)
        .without_engine()
        .out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        finding_status(&out.stdout, "sqlite-driver").as_deref(),
        Some("fail"),
        "the sqlite driver must be reported unverified, never ok: {}",
        out.stdout
    );
    assert_ne!(
        out.code, SUCCESS,
        "an unverified mandatory prerequisite must never exit 0: {}",
        out.stdout
    );
}

#[test]
fn doctor_fails_when_the_engine_binary_exits_non_zero() {
    // An engine that exists but cannot answer a version query has not proven the delegated
    // prerequisite. `doctor` must report the engine as `fail` and refuse to claim readiness,
    // rather than treating "a binary is present" as a pass.
    let fx = Fixture::new("doctor-engine-exit", Mode::Present);
    let Some(engine) = shell_engine(
        &fx.dir.join("engine"),
        "exit-non-zero.sh",
        "echo boom >&2\nexit 7",
    ) else {
        return;
    };
    let out = Run::new(&fx.root)
        .with_engine(&engine)
        .out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        finding_status(&out.stdout, "engine").as_deref(),
        Some("fail"),
        "an engine that exits non-zero must be reported fail: {}",
        out.stdout
    );
    assert_eq!(out.code, NOT_READY, "stdout={}", out.stdout);
    let finding = finding_object(&out.stdout, "engine").expect("an engine finding must be present");
    assert_eq!(
        json_number_field(finding, "exit_code"),
        Some(7),
        "the failing engine exit code must be reported: {finding}"
    );
}

#[test]
fn doctor_fails_when_the_engine_exits_zero_without_a_json_object() {
    // Exit zero alone is not proof: the contract forbids reporting a leg as passing when it did
    // not run. An engine whose stdout is not a JSON object has not answered a version query, so
    // the finding must be `fail` even though the process succeeded.
    let fx = Fixture::new("doctor-engine-non-json", Mode::Present);
    let Some(engine) = shell_engine(
        &fx.dir.join("engine"),
        "non-json.sh",
        "echo not-a-json-object\nexit 0",
    ) else {
        return;
    };
    let out = Run::new(&fx.root)
        .with_engine(&engine)
        .out(&["doctor", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        finding_status(&out.stdout, "engine").as_deref(),
        Some("fail"),
        "an engine that answers exit 0 without JSON must be reported fail: {}",
        out.stdout
    );
    assert_eq!(out.code, NOT_READY, "stdout={}", out.stdout);
    let finding = finding_object(&out.stdout, "engine").expect("an engine finding must be present");
    assert!(
        finding.contains("did not answer a JSON version report"),
        "the finding must explain why exit 0 was not accepted: {finding}"
    );
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

/// The plan digest a refusal names in its message, so a refused plan still has an address.
///
/// The refusal envelope carries no `plan_digest` member of its own, so the digest is read from
/// the sentence that states which plan was refused.
fn refused_plan_digest(out: &Outcome) -> String {
    const LEAD: &str = "the install plan digest is ";
    let start = out
        .stdout
        .find(LEAD)
        .unwrap_or_else(|| panic!("the refusal must name its plan: {}", out.stdout))
        + LEAD.len();
    let digest: String = out.stdout[start..]
        .chars()
        .take_while(char::is_ascii_hexdigit)
        .collect();
    assert_eq!(
        digest.len(),
        64,
        "a plan digest must be 64 hex chars: {}",
        out.stdout
    );
    digest
}

/// The canonical plan digest reported by an install run.
fn install_plan_digest(out: &Outcome) -> String {
    let digest = json_string_field(&out.stdout, "plan_digest").unwrap_or_default();
    assert_eq!(
        digest.len(),
        64,
        "a plan digest must be 64 hex chars: {}",
        out.stdout
    );
    assert!(
        digest.chars().all(|c| c.is_ascii_hexdigit()),
        "a plan digest must be hex: {digest}"
    );
    digest
}

#[test]
fn install_without_a_mode_or_terminal_prints_the_plan_and_changes_nothing() {
    // ADR-0033 decision 4: a bare `install` is the interactive single confirmation. Without a
    // terminal (and `--json` never prompts) and without `--yes` it prints the plan, changes nothing
    // and exits `4`; it never answers success for an install that did not run.
    let fx = Fixture::new("install-no-mode", Mode::Present);
    let out =
        Run::new(&fx.root).out(&["install", "--from", fx.release.to_str().unwrap(), "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        out.code, NOT_READY,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert_eq!(
        json_string_field(&out.stdout, "reason_code").as_deref(),
        Some("confirmation_required")
    );
    assert!(
        json_string_field(&out.stdout, "message")
            .unwrap_or_default()
            .contains("--yes"),
        "the refusal must teach `--yes`: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"plan_digest\""),
        "the plan is reported: {}",
        out.stdout
    );
    assert_root_untouched(&fx, "`install` without a mode or terminal");
}

#[test]
fn install_yes_applies_the_printed_plan_and_announces_the_path_change() {
    let fx = Fixture::new("install-yes", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };
    let release = fx.release.to_str().unwrap().to_string();
    let out = Run::new(&fx.root).with_engine(&engine).out(&[
        "install",
        "--yes",
        "--from",
        release.as_str(),
        "--json",
    ]);
    assert_eq!(
        out.code, SUCCESS,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert_eq!(
        json_string_field(&out.stdout, "confirmation").as_deref(),
        Some("pre-approved")
    );
    assert!(
        out.stdout.contains("\"path_change\"") && out.stdout.contains("\"action\":\"add\""),
        "the PATH change is part of the approved plan: {}",
        out.stdout
    );
    assert!(fx.root.join("installed.json").is_file());
    assert!(
        fx.root.join("path-change.json").is_file(),
        "the applied PATH change is recorded for uninstall"
    );
}

#[test]
fn install_adopts_a_manifest_owned_legacy_cli_store() {
    let fx = Fixture::new("install-legacy-manifest", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };
    let name = if cfg!(windows) {
        "axiom-cli.exe"
    } else {
        "axiom-cli"
    };
    let bin = fx.root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir(fx.root.join("cli")).unwrap();
    let old = b"owned old CLI";
    let exe = bin.join(name);
    std::fs::write(&exe, old).unwrap();
    std::fs::write(fx.root.join("cli/state.json"), b"legacy state retained").unwrap();
    let root_path = fx.root.display().to_string().replace('\\', "/");
    let exe_path = exe.display().to_string().replace('\\', "/");
    let manifest = format!(
        "{{\"schema_version\":1,\"document_kind\":\"axiom-cli-install-manifest\",\"release_version\":\"0.1.2\",\
         \"components\":[{{\"component\":\"axiom-cli\",\"installed_version\":\"0.1.2\",\
         \"install_root\":\"{root_path}\",\"artifacts\":[{{\"name\":\"{name}\",\
         \"installed_path\":\"{exe_path}\",\"size_bytes\":{},\"sha256\":\"{}\"}}]}}]}}",
        old.len(), sha256::hex(old)
    );
    std::fs::write(fx.root.join("install-manifest.json"), &manifest).unwrap();
    let out = Run::new(&fx.root).with_engine(&engine).out(&[
        "install",
        "--yes",
        "--no-modify-path",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        out.code, SUCCESS,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert!(fx.root.join("installed.json").is_file());
    assert_eq!(
        std::fs::read(fx.root.join("legacy/cli-store-0.1.2").join(name)).unwrap(),
        old
    );
    assert_eq!(
        std::fs::read(bin.join(".internal").join(if cfg!(windows) {
            "retired-axiom-cli.exe"
        } else {
            "retired-axiom-cli"
        }))
        .unwrap(),
        old
    );
    assert!(!exe.exists());
    assert_eq!(
        std::fs::read_to_string(fx.root.join("install-manifest.json")).unwrap(),
        manifest
    );
    assert_eq!(
        std::fs::read(fx.root.join("cli/state.json")).unwrap(),
        b"legacy state retained"
    );
}

#[test]
fn install_yes_with_no_modify_path_leaves_the_path_alone() {
    let fx = Fixture::new("install-yes-no-path", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };
    let release = fx.release.to_str().unwrap().to_string();
    let out = Run::new(&fx.root)
        .with_engine(&engine)
        .with_env("AXIOM_INSTALL_YES", "1")
        .out(&[
            "install",
            "--no-modify-path",
            "--from",
            release.as_str(),
            "--json",
        ]);
    assert_eq!(
        out.code, SUCCESS,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert!(
        out.stdout.contains("\"action\":\"skipped\""),
        "the skipped PATH change is shown in the plan: {}",
        out.stdout
    );
    assert!(!fx.root.join("path-change.json").exists());
}

#[test]
fn the_interactive_plan_digest_differs_from_the_automation_plan() {
    // `--dry-run` / `--apply --approve-digest` behave as before: their plan carries no PATH
    // change, so an approval for one can never be replayed as the other.
    let fx = Fixture::new("install-digests", Mode::Present);
    let release = fx.release.to_str().unwrap().to_string();
    let dry =
        Run::new(&fx.root).out(&["install", "--dry-run", "--from", release.as_str(), "--json"]);
    let bare = Run::new(&fx.root).out(&["install", "--from", release.as_str(), "--json"]);
    assert_eq!(dry.code, SUCCESS, "stdout={}", dry.stdout);
    assert!(!dry.stdout.contains("\"path_change\""));
    assert!(bare.stdout.contains("\"path_change\""));
    assert_ne!(
        json_string_field(&dry.stdout, "plan_digest"),
        json_string_field(&bare.stdout, "plan_digest")
    );
}

#[test]
fn install_dry_run_verifies_the_artifacts_and_changes_nothing() {
    let fx = Fixture::new("install-dry-run", Mode::Present);
    let out = Run::new(&fx.root).out(&[
        "install",
        "--dry-run",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        out.code, SUCCESS,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert_single_json_object(&out.stdout);
    assert_eq!(
        json_string_field(&out.stdout, "mode").as_deref(),
        Some("dry-run")
    );
    let verified = json_object_field(&out.stdout, "verified_artifacts");
    // `verified_artifacts` is an array, so locate it textually instead.
    assert!(
        out.stdout.contains("\"verified_artifacts\":[{"),
        "a verified artifact must be reported: {}",
        out.stdout
    );
    let _ = verified;
    install_plan_digest(&out);
    assert_root_untouched(&fx, "`install --dry-run`");
}

#[test]
fn install_plan_is_sealed_and_carries_no_volatile_timestamp() {
    // Regression guard. The canonical approval boundary accepts a plan only when the plan carries
    // its own digest, and a digest that covers the observation time can never be re-derived on a
    // later `--apply`. Both defects made `install --apply` impossible to approve.
    let fx = Fixture::new("install-seal", Mode::Present);
    let argv = [
        "install",
        "--dry-run",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ];
    let first = Run::new(&fx.root).out(&argv);
    assert_eq!(first.code, SUCCESS, "stdout={}", first.stdout);
    let digest = install_plan_digest(&first);

    let plan = json_object_field(&first.stdout, "plan").expect("the report must include the plan");
    assert!(
        plan.contains(&format!("\"plan_digest\":\"{digest}\"")),
        "the plan must be sealed with its own digest: {plan}"
    );
    assert!(
        !plan.contains("generated_at"),
        "the plan body must not carry a volatile timestamp: {plan}"
    );
    assert!(
        first.stdout.contains("\"generated_at\":\""),
        "the observation time must still be reported, outside the plan"
    );

    std::thread::sleep(std::time::Duration::from_millis(1200));
    let second = Run::new(&fx.root).out(&argv);
    assert_eq!(second.code, SUCCESS, "stdout={}", second.stdout);
    assert_eq!(
        install_plan_digest(&second),
        digest,
        "the plan digest must be reproducible across time"
    );
}

#[test]
fn install_apply_without_approval_is_validation_and_changes_nothing() {
    let fx = Fixture::new("install-apply-no-approval", Mode::Present);
    let out = Run::new(&fx.root).out(&[
        "install",
        "--apply",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(&out, VALIDATION, "validation_error", "");
    assert_root_untouched(&fx, "`install --apply` without approval");
}

#[test]
fn install_apply_with_a_malformed_digest_is_validation() {
    let fx = Fixture::new("install-apply-bad-digest", Mode::Present);
    let out = Run::new(&fx.root).out(&[
        "install",
        "--apply",
        "--approve-digest",
        "deadbeef",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(out.code, VALIDATION, "stdout={}", out.stdout);
    assert_single_json_object(&out.stdout);
    assert_root_untouched(&fx, "`install --apply` with a malformed digest");
}

#[test]
fn install_from_without_a_value_is_validation_and_one_json_object() {
    // A flag that promises a value must fail closed when the value is missing: parsing must not
    // silently treat the next flag as the path, and `--json` must still yield exactly one object
    // because the envelope is the public surface even on a validation failure.
    let fx = Fixture::new("install-from-no-value", Mode::Present);
    let out = Run::new(&fx.root).out(&["install", "--from", "--json"]);
    assert_refusal(&out, VALIDATION, "validation_error", "");
    assert_root_untouched(&fx, "`install --from` without a value");
}

#[test]
fn install_apply_with_a_wrong_digest_is_a_conflict_and_changes_nothing() {
    let fx = Fixture::new("install-apply-wrong-digest", Mode::Present);
    let out = Run::new(&fx.root).out(&[
        "install",
        "--apply",
        "--approve-digest",
        ZERO_DIGEST,
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(&out, CONFLICT, "conflict", "approval_required");
    assert_root_untouched(&fx, "`install --apply` with a wrong digest");
}

#[test]
fn install_apply_with_the_correct_digest_reaches_the_engine_boundary() {
    // The end-to-end proof that the approval path is reachable: a digest taken from a *separate*
    // `--dry-run` invocation is accepted, and the run then refuses at the honest assembly
    // boundary - naming the core components this fixture does not carry - instead of a bogus
    // approval conflict.
    let fx = Fixture::new("install-apply-approved", Mode::Present);
    let dry = Run::new(&fx.root).out(&[
        "install",
        "--dry-run",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(dry.code, SUCCESS, "stdout={}", dry.stdout);
    let digest = install_plan_digest(&dry);

    let ignored = ZERO_DIGEST; // keep the unapproved comparison explicit below
    let approved = Run::new(&fx.root).with_engine(&engine_stand_in()).out(&[
        "install",
        "--apply",
        "--approve-digest",
        digest.as_str(),
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(
        &approved,
        NOT_READY,
        "not_ready",
        "engine_bundle_components_missing",
    );
    assert_ne!(
        approved.stdout, "",
        "the refusal must still carry the plan evidence"
    );
    assert!(
        approved.stdout.contains(&digest),
        "the refusal must name the approved plan digest: {}",
        approved.stdout
    );
    assert_root_untouched(&fx, "`install --apply` that reached the engine boundary");
    let _ = ignored;
}

#[test]
fn install_apply_with_a_report_digest_is_not_refused_for_a_stale_plan_digest() {
    // Locks the `seal()` fix end to end. A digest taken from a *report-mode* run must satisfy the
    // approval contract on a later `--apply`, so the run reaches the engine ownership boundary
    // instead of being refused for `plan_digest_not_approved` - the defect that made approval
    // impossible and is only proven fixed by crossing the two invocations.
    let fx = Fixture::new("install-seal-end-to-end", Mode::Present);
    let release = fx.release.display().to_string();
    let report =
        Run::new(&fx.root).out(&["install", "--dry-run", "--from", release.as_str(), "--json"]);
    assert_eq!(
        report.code, SUCCESS,
        "stdout={} stderr={}",
        report.stdout, report.stderr
    );
    let digest = install_plan_digest(&report);

    let applied = Run::new(&fx.root).with_engine(&engine_stand_in()).out(&[
        "install",
        "--from",
        release.as_str(),
        "--apply",
        "--approve-digest",
        digest.as_str(),
        "--json",
    ]);
    let reason = json_string_field(&applied.stdout, "reason_code").unwrap_or_default();
    assert!(
        !reason.contains("plan_digest_not_approved"),
        "a report-mode digest must be approvable, got `{reason}`: {}",
        applied.stdout
    );
    assert!(
        applied.code == NOT_READY || applied.code == NOT_FOUND,
        "the run must reach the engine boundary, got {}: {}",
        applied.code,
        applied.stdout
    );
    if applied.code == NOT_READY {
        assert_eq!(
            reason, "engine_bundle_components_missing",
            "stdout={}",
            applied.stdout
        );
    } else {
        assert_eq!(reason, "engine_not_found", "stdout={}", applied.stdout);
    }

    // A host with no engine must not resurrect the stale-digest refusal either: the boundary
    // answer changes, the approval verdict must not.
    let no_engine = Run::new(&fx.root).without_engine().out(&[
        "install",
        "--from",
        release.as_str(),
        "--apply",
        "--approve-digest",
        digest.as_str(),
        "--json",
    ]);
    let no_engine_reason = json_string_field(&no_engine.stdout, "reason_code").unwrap_or_default();
    assert!(
        !no_engine_reason.contains("plan_digest_not_approved"),
        "stdout={} stderr={}",
        no_engine.stdout,
        no_engine.stderr
    );
}

#[test]
fn install_apply_with_the_correct_digest_reaches_the_boundary_from_the_recorded_manifest() {
    let fx = Fixture::new("install-recorded-apply", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let run = Run::new(&fx.root).with_candidate_manifest(&fx.manifest, &fx.cache);

    let dry = run.out(&["install", "--dry-run", "--json"]);
    assert_eq!(
        dry.code, SUCCESS,
        "stdout={} stderr={}",
        dry.stdout, dry.stderr
    );
    let digest = install_plan_digest(&dry);
    let source = json_string_field(&dry.stdout, "manifest_source").unwrap_or_default();
    assert!(
        source.contains("recorded-manifest.json"),
        "an installed host must resolve the recorded manifest, got `{source}`"
    );

    let approved = Run::new(&fx.root)
        .with_candidate_manifest(&fx.manifest, &fx.cache)
        .with_engine(&engine_stand_in())
        .out(&[
            "install",
            "--apply",
            "--approve-digest",
            digest.as_str(),
            "--json",
        ]);
    assert_refusal(
        &approved,
        NOT_READY,
        "not_ready",
        "engine_bundle_components_missing",
    );
}

#[test]
fn install_without_any_release_set_is_not_ready() {
    let fx = Fixture::new("install-no-set", Mode::Present);
    let out = Run::new(&fx.root).out(&["install", "--dry-run", "--json"]);
    assert_refusal(&out, NOT_READY, "not_ready", "no_release_set");
    assert_root_untouched(&fx, "`install --dry-run` with no release set");
}

#[test]
fn install_refuses_a_release_set_that_has_no_manifest() {
    let fx = Fixture::new("install-missing-set", Mode::Present);
    let empty = fx.dir.join("empty-set");
    std::fs::create_dir_all(&empty).expect("the empty set must be creatable");
    let out = Run::new(&fx.root).out(&[
        "install",
        "--dry-run",
        "--from",
        empty.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(&out, NOT_FOUND, "not_found", "release_set_manifest_missing");
    assert_root_untouched(&fx, "`install` against a release set with no manifest");
}

#[test]
fn install_refuses_an_artifact_whose_bytes_do_not_match() {
    let fx = Fixture::new("install-tampered", Mode::Tampered);
    let out = Run::new(&fx.root).out(&[
        "install",
        "--dry-run",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(&out, VALIDATION, "validation_error", "artifact_unverified");
    assert_root_untouched(&fx, "`install` with a tampered artifact");
}

#[test]
fn install_refuses_an_artifact_that_is_not_available_locally() {
    let fx = Fixture::new("install-absent", Mode::Absent);
    let out = Run::new(&fx.root).out(&[
        "install",
        "--dry-run",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(&out, NOT_READY, "not_ready", "artifact_unreachable");
    assert_root_untouched(&fx, "`install` with an absent artifact");
}

#[test]
fn install_reports_nothing_to_install_when_this_host_has_no_artifact() {
    let fx = Fixture::new("install-foreign", Mode::ForeignHost);
    let dry = Run::new(&fx.root).out(&[
        "install",
        "--dry-run",
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    // A release set this host installs nothing from is refused the same way in both modes: there
    // is no plan worth approving, and `--dry-run` must not report a happy plan that `--apply`
    // then refuses. The message states the fact, not the mode.
    assert_refusal(&dry, NOT_READY, "not_ready", "nothing_to_install");
    assert!(
        dry.stdout.contains("declares no artifact for host"),
        "the refusal must name what is missing: {}",
        dry.stdout
    );

    let digest = refused_plan_digest(&dry);

    let applied = Run::new(&fx.root).with_engine(&engine_stand_in()).out(&[
        "install",
        "--apply",
        "--approve-digest",
        digest.as_str(),
        "--from",
        fx.release.to_str().unwrap(),
        "--json",
    ]);
    assert_refusal(&applied, NOT_READY, "not_ready", "nothing_to_install");
    assert_root_untouched(&fx, "`install --apply` with no artifact for this host");
}

#[test]
fn install_json_is_one_object_on_every_path() {
    let fx = Fixture::new("install-json", Mode::Present);
    let release = fx.release.to_str().unwrap();
    let paths: Vec<Vec<&str>> = vec![
        vec!["install", "--json"],
        vec!["install", "--dry-run", "--from", release, "--json"],
        vec!["install", "--apply", "--json"],
        vec![
            "install",
            "--apply",
            "--approve-digest",
            ZERO_DIGEST,
            "--from",
            release,
            "--json",
        ],
        vec![
            "install",
            "--apply",
            "--approve-digest",
            "not-a-digest",
            "--json",
        ],
        vec!["install", "--from", "/does/not/exist", "--json"],
    ];
    for argv in paths {
        let out = Run::new(&fx.root).out(&argv);
        assert_single_json_object(&out.stdout);
        assert_eq!(
            json_number_field(&out.stdout, "code"),
            Some(i64::from(out.code)),
            "argv={argv:?} stdout={}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("\"details\":{}"),
            "a verb must never emit an empty success envelope; argv={argv:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// uninstall
// ---------------------------------------------------------------------------

#[test]
fn uninstall_dry_run_changes_nothing() {
    let fx = Fixture::new("uninstall-report", Mode::Present);
    let out = Run::new(&fx.root).out(&["uninstall", "--dry-run", "--json"]);
    assert_eq!(out.code, SUCCESS, "stdout={}", out.stdout);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        json_string_field(&out.stdout, "mode").as_deref(),
        Some("dry-run")
    );
    assert_eq!(json_string_field(&out.stdout, "installed").as_deref(), None);
    assert!(out.stdout.contains("\"installed\":false"));
    install_plan_digest(&out);
    assert_root_untouched(&fx, "`uninstall --dry-run`");
}

#[test]
fn uninstall_without_a_mode_or_terminal_prints_the_plan_and_changes_nothing() {
    let fx = Fixture::new("uninstall-no-mode", Mode::Present);
    let out = Run::new(&fx.root).out(&["uninstall", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        out.code, NOT_READY,
        "stdout={} stderr={}",
        out.stdout, out.stderr
    );
    assert_eq!(
        json_string_field(&out.stdout, "reason_code").as_deref(),
        Some("confirmation_required")
    );
    assert_root_untouched(&fx, "`uninstall` without a mode or terminal");
}

#[test]
fn uninstall_purge_data_without_apply_is_validation() {
    let fx = Fixture::new("uninstall-purge", Mode::Present);
    let out = Run::new(&fx.root).out(&["uninstall", "--purge-data", "--json"]);
    assert_refusal(&out, VALIDATION, "validation_error", "");
    assert_root_untouched(&fx, "`uninstall --purge-data` without apply");
}

#[test]
fn uninstall_apply_without_approval_is_validation() {
    let fx = Fixture::new("uninstall-no-approval", Mode::Present);
    let out = Run::new(&fx.root).out(&["uninstall", "--apply", "--json"]);
    assert_refusal(&out, VALIDATION, "validation_error", "");
    assert_root_untouched(&fx, "`uninstall --apply` without approval");
}

#[test]
fn uninstall_apply_flag_combinations_are_validation() {
    // Removing owned binaries needs an explicit digest, and `--dry-run` promises not to change the
    // host even when `--apply` is also present. Both contradictions must fail closed at argv
    // validation with the canonical single-object refusal, and neither may touch the root.
    let fx = Fixture::new("uninstall-flag-combinations", Mode::Present);
    for argv in [
        vec!["uninstall", "--apply", "--json"],
        vec!["uninstall", "--dry-run", "--apply", "--json"],
    ] {
        let out = Run::new(&fx.root).out(&argv);
        assert_refusal(&out, VALIDATION, "validation_error", "");
        assert_root_untouched(&fx, &format!("`{}`", argv.join(" ")));
    }
}

#[test]
fn uninstall_apply_with_a_wrong_digest_is_a_conflict() {
    let fx = Fixture::new("uninstall-wrong-digest", Mode::Present);
    let out = Run::new(&fx.root).out(&[
        "uninstall",
        "--apply",
        "--approve-digest",
        ZERO_DIGEST,
        "--json",
    ]);
    assert_refusal(&out, CONFLICT, "conflict", "approval_required");
    assert_root_untouched(&fx, "`uninstall --apply` with a wrong digest");
}

#[test]
fn uninstall_plan_digest_is_reproducible_and_sealed() {
    let fx = Fixture::new("uninstall-seal", Mode::Present);
    let first = Run::new(&fx.root).out(&["uninstall", "--dry-run", "--json"]);
    assert_eq!(first.code, SUCCESS, "stdout={}", first.stdout);
    let digest = install_plan_digest(&first);
    let plan = json_object_field(&first.stdout, "plan").expect("the report must include the plan");
    assert!(plan.contains(&format!("\"plan_digest\":\"{digest}\"")));
    assert!(!plan.contains("generated_at"));

    std::thread::sleep(std::time::Duration::from_millis(1200));
    let second = Run::new(&fx.root).out(&["uninstall", "--dry-run", "--json"]);
    assert_eq!(install_plan_digest(&second), digest);
}

#[test]
fn uninstall_apply_with_nothing_installed_is_not_found() {
    let fx = Fixture::new("uninstall-empty", Mode::Present);
    let dry = Run::new(&fx.root).out(&["uninstall", "--dry-run", "--json"]);
    let digest = install_plan_digest(&dry);
    let out = Run::new(&fx.root).out(&[
        "uninstall",
        "--apply",
        "--approve-digest",
        digest.as_str(),
        "--json",
    ]);
    assert_refusal(&out, NOT_FOUND, "not_found", "nothing_installed");
    assert_root_untouched(&fx, "`uninstall --apply` with nothing installed");
}

#[test]
fn uninstall_apply_with_an_installed_release_refuses_honestly_and_preserves_data() {
    let fx = Fixture::new("uninstall-installed", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let data = fx.root.join("user-workspace-data.txt");
    std::fs::write(&data, b"user data that a removal must not touch\n")
        .expect("user data must be creatable");

    let run = Run::new(&fx.root)
        .without_engine()
        .with_candidate_manifest(&fx.manifest, &fx.cache);
    let dry = run.out(&["uninstall", "--dry-run", "--json"]);
    assert_refusal(&dry, NOT_FOUND, "not_found", "engine_not_found");
    assert!(
        data.is_file(),
        "an uninstall refusal must not delete user data"
    );
    assert!(
        fx.root.join("installed.json").is_file(),
        "an uninstall refusal must not remove the installed record"
    );
}

#[test]
fn uninstall_without_an_engine_reports_not_found_and_removes_nothing() {
    let fx = Fixture::new("uninstall-no-engine", Mode::Present);
    fx.seed_installed(SEED_GENERATION, "install");
    let run = Run::new(&fx.root)
        .without_engine()
        .with_candidate_manifest(&fx.manifest, &fx.cache);
    let dry = run.out(&["uninstall", "--dry-run", "--json"]);
    assert_refusal(&dry, NOT_FOUND, "not_found", "engine_not_found");
    assert!(fx.root.join("installed.json").is_file());
}

// ---------------------------------------------------------------------------
// cross-cutting: JSON discipline on failure and under --verbose
// ---------------------------------------------------------------------------

#[test]
fn failure_envelopes_are_one_object_with_matching_code_and_status() {
    // The `--json` envelope is a public surface on failure as much as on success. Every refusal
    // must be exactly one JSON object, its `code` must equal the process exit, its `status` must
    // be the canonical class token, and diagnostics must stay off stderr because the object on
    // stdout already carries the reason.
    let fx = Fixture::new("failure-json-discipline", Mode::Present);
    let missing = fx.dir.join("missing-release").display().to_string();
    let cases: [(Vec<&str>, i32, &str); 3] = [
        (
            vec!["--json", "install", "--dry-run"],
            NOT_READY,
            "not_ready",
        ),
        (
            vec!["--json", "install", "--dry-run", "--from", missing.as_str()],
            NOT_READY,
            "not_ready",
        ),
        (
            vec![
                "--json",
                "uninstall",
                "--apply",
                "--approve-digest",
                ZERO_DIGEST,
            ],
            CONFLICT,
            "conflict",
        ),
    ];
    for (argv, code, status) in cases {
        let out = Run::new(&fx.root).out(&argv);
        assert_single_json_object(&out.stdout);
        assert_eq!(
            out.code, code,
            "argv={argv:?} stdout={} stderr={}",
            out.stdout, out.stderr
        );
        assert_eq!(
            json_number_field(&out.stdout, "code"),
            Some(i64::from(code)),
            "the envelope code must equal the process exit; argv={argv:?}"
        );
        assert_eq!(
            envelope_string_field(&out.stdout, "status").as_deref(),
            Some(status),
            "argv={argv:?} stdout={}",
            out.stdout
        );
        assert!(
            out.stderr.is_empty(),
            "diagnostics must never be written to stderr on a JSON path; argv={argv:?} stderr={}",
            out.stderr
        );
    }
}

#[test]
fn verbose_json_keeps_stdout_to_one_json_object() {
    // `--verbose` adds diagnostics, never a second document. In JSON mode stdout must stay exactly
    // one object and any diagnostic must land on stderr, which the harness captures separately.
    let fx = Fixture::new("verbose-json", Mode::Present);
    let release = fx.release.display().to_string();
    for argv in [
        vec!["version", "--json", "--verbose"],
        vec!["doctor", "--json", "--verbose"],
        vec!["install", "--from", release.as_str(), "--json", "--verbose"],
        vec!["uninstall", "--json", "--verbose"],
    ] {
        let out = Run::new(&fx.root).out(&argv);
        assert_single_json_object(&out.stdout);
        assert!(
            !out.stdout.contains(": diagnostic:"),
            "a diagnostic must never be written to stdout; argv={argv:?} stdout={}",
            out.stdout
        );
    }
}

// ---------------------------------------------------------------------------
// update (regression cross-check: the pre-existing verb is unchanged)
// ---------------------------------------------------------------------------

#[test]
fn update_check_still_answers_a_real_result() {
    let fx = Fixture::new("update-check", Mode::Present);
    let out = Run::new(&fx.root).out(&["update", "check", "--json"]);
    assert_single_json_object(&out.stdout);
    assert_eq!(
        json_number_field(&out.stdout, "code"),
        Some(i64::from(out.code)),
        "stdout={}",
        out.stdout
    );
    assert!(
        out.code == SUCCESS || out.code == NOT_READY,
        "`update check` on a clean host must be not-ready, got {}; stdout={}",
        out.code,
        out.stdout
    );
    if out.code == NOT_READY {
        assert_eq!(
            json_string_field(&out.stdout, "status").as_deref(),
            Some("not_ready")
        );
    }
}

// ---------------------------------------------------------------------------
// install --apply: the engine boundary, with a release set the engine consumes
//
// These are the invoke-path proofs. The engine stand-in is a shell script that answers the two
// argv steps the real engine publishes, so what is under test is *this layer*: that it assembles
// a bundle from verified bytes, hands the engine that bundle (not its own plan document), carries
// the engine's own plan digest into the apply step instead of recomputing it, and reports the
// engine's own exit code and raw evidence. The end-to-end run against the real engine binary is
// recorded in the handoff, not re-run here, because it depends on another repository's build.
// ---------------------------------------------------------------------------

/// An engine stand-in whose *plan* step succeeds and whose *apply* step refuses.
const ENGINE_REFUSING_AT_APPLY: &str = r#"case "$1 $2" in
  "install plan")
    out=""
    prev=""
    for arg in "$@"; do
      if [ "$prev" = "--out" ]; then out="$arg"; fi
      prev="$arg"
    done
    printf '{"plan_version":1,"verb":"install","target":{"host":"macos-x64"}}\n' > "$out"
    printf '{"plan_digest":"@DIGEST@","plan_file":"%s","plan_id":"install-standin","status":"planned"}\n' "$out"
    exit 0
    ;;
  "install apply")
    printf '{"code":"@CODE@","message":"the engine refused this bundle","retryable":@RETRYABLE@,"details":{"rule":"@RULE@"},"request_id":null}\n'
    printf '@CODE@: the engine refused this bundle\n' >&2
    exit @EXIT@
    ;;
esac
echo "engine stand-in: unexpected argv: $*" >&2
exit 90
"#;

fn engine_refusing_at_apply_body(exit: i32, code: &str, rule: &str, retryable: bool) -> String {
    ENGINE_REFUSING_AT_APPLY
        .replace("@DIGEST@", ENGINE_PLAN_DIGEST)
        .replace("@CODE@", code)
        .replace("@RULE@", rule)
        .replace("@RETRYABLE@", if retryable { "true" } else { "false" })
        .replace("@EXIT@", &exit.to_string())
}

/// Run `install --dry-run` then `install --apply` against a seeded engine release set.
///
/// Returns the apply outcome. The digest is taken from the dry-run report, i.e. from a *separate*
/// invocation, which is what the approval contract makes a caller do.
fn apply_engine_release(fx: &Fixture, engine: &Path) -> Outcome {
    let release = fx.release.to_str().unwrap().to_string();
    let dry =
        Run::new(&fx.root).out(&["install", "--dry-run", "--from", release.as_str(), "--json"]);
    assert_eq!(
        dry.code, SUCCESS,
        "the fixture release set must resolve: stdout={} stderr={}",
        dry.stdout, dry.stderr
    );
    let digest = install_plan_digest(&dry);
    Run::new(&fx.root).with_engine(engine).out(&[
        "install",
        "--apply",
        "--approve-digest",
        digest.as_str(),
        "--from",
        release.as_str(),
        "--json",
    ])
}

#[test]
fn install_apply_installs_through_the_engine_from_a_local_release_set() {
    let fx = Fixture::new("install-engine-happy", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_single_json_object(&applied.stdout);
    assert_eq!(
        applied.code, SUCCESS,
        "an engine that placed the bundle must be reported as success: stdout={} stderr={}",
        applied.stdout, applied.stderr
    );
    assert_eq!(
        json_number_field(&applied.stdout, "code"),
        Some(0),
        "the envelope code must equal the process exit code"
    );
    assert_eq!(
        envelope_string_field(&applied.stdout, "status").as_deref(),
        Some("ok"),
        "the envelope status, not an engine step's status: {}",
        applied.stdout
    );
    assert_eq!(
        json_string_field(&applied.stdout, "engine_status").as_deref(),
        Some("installed"),
        "the report must carry the engine's own status: {}",
        applied.stdout
    );
    assert_eq!(
        json_string_field(&applied.stdout, "engine_plan_digest").as_deref(),
        Some(ENGINE_PLAN_DIGEST),
        "the engine's own plan digest must be carried, never recomputed: {}",
        applied.stdout
    );
    assert!(
        applied.stdout.contains(GRAPHD) && applied.stdout.contains(MCP),
        "the placed components must be reported: {}",
        applied.stdout
    );

    // The bundle this layer assembled, not its own plan document, is what the engine was handed.
    let staging = fx.root.join("staging").join("engine-bundle");
    let manifest = std::fs::read_to_string(staging.join("bundle.json"))
        .expect("the assembled bundle manifest must exist");
    assert!(
        manifest.contains("\"component\":\"axiom-graphd\"")
            && manifest.contains("\"component\":\"axiom-mcp\""),
        "the core components must be declared: {manifest}"
    );
    assert!(
        staging.join("bin").join(GRAPHD).is_file(),
        "the graphd payload must be placed at the bundle-relative artifact path"
    );
    assert!(
        manifest.contains("\"permissions\":[\"read\",\"execute\"]"),
        "a binary artifact is executed, so its permissions must announce `execute`, not `read` \
         alone: {manifest}"
    );
    assert!(
        staging.join("skills").join("bundle.json").is_file()
            && staging
                .join("skills")
                .join("payload")
                .join("instructions")
                .join("axiom.md")
                .is_file(),
        "the skills bundle and its payload must be carried"
    );

    // Both steps' raw evidence is carried, at the engine's own exit codes.
    assert!(
        applied.stdout.contains("\"step\":\"plan\"")
            && applied.stdout.contains("\"step\":\"apply\""),
        "both engine steps must be evidenced: {}",
        applied.stdout
    );
}

#[test]
fn install_apply_reports_a_missing_engine_status_as_unreported_not_installed() {
    // The engine exited 0, so the install succeeded - but the engine never said what it did. The
    // report carries `unreported` and says so, instead of substituting a status the engine never
    // sent. This is the honesty half of "the engine's answer is the answer".
    let fx = Fixture::new("install-engine-silent-apply", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_silent_at_apply_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_single_json_object(&applied.stdout);
    assert_eq!(
        applied.code, SUCCESS,
        "stdout={} stderr={}",
        applied.stdout, applied.stderr
    );
    assert_eq!(
        json_string_field(&applied.stdout, "engine_status").as_deref(),
        Some("unreported"),
        "this layer must not invent an engine status: {}",
        applied.stdout
    );
    assert!(
        applied.stdout.contains("\"engine_status_reported\":false"),
        "the report must say the engine sent no status: {}",
        applied.stdout
    );
    assert_eq!(
        envelope_string_field(&applied.stdout, "status").as_deref(),
        Some("ok"),
        "exit 0 is still the engine's own answer: {}",
        applied.stdout
    );
}

#[test]
fn install_apply_keeps_a_conflicting_engine_at_its_own_exit_code() {
    let fx = Fixture::new("install-engine-conflict", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(
        &fx.dir,
        "engine",
        &engine_refusing_at_apply_body(6, "CONFLICT", "staging_incomplete", true),
    ) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_single_json_object(&applied.stdout);
    assert_eq!(
        applied.code, CONFLICT,
        "the engine's own exit code is authoritative: stdout={} stderr={}",
        applied.stdout, applied.stderr
    );
    assert_eq!(
        envelope_string_field(&applied.stdout, "status").as_deref(),
        Some("conflict"),
        "the envelope status, not the plan step's status: {}",
        applied.stdout
    );
    assert_eq!(
        json_string_field(&applied.stdout, "reason_code").as_deref(),
        Some("engine_conflict")
    );
    assert_eq!(
        json_number_field(&applied.stdout, "engine_exit_code"),
        Some(6)
    );
    assert_eq!(
        json_string_field(&applied.stdout, "engine_code").as_deref(),
        Some("CONFLICT")
    );
    assert!(
        applied.stdout.contains("staging_incomplete"),
        "the engine's own rule must survive into the report: {}",
        applied.stdout
    );
    assert!(
        applied.stdout.contains("\"step\":\"plan\"") && applied.stdout.contains("\"exit_code\":0"),
        "the successful plan step must still be evidenced: {}",
        applied.stdout
    );
    assert!(
        !fx.root.join("installed.json").exists(),
        "an engine refusal must not be reported as an installed release"
    );
}

#[test]
fn install_apply_reports_not_ready_when_the_engine_is_not_ready() {
    let fx = Fixture::new("install-engine-not-ready", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(
        &fx.dir,
        "engine",
        &engine_refusing_at_apply_body(4, "NOT_READY", "prerequisite-unsatisfied", false),
    ) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(&applied, NOT_READY, "not_ready", "engine_not_ready");
    assert_eq!(
        json_number_field(&applied.stdout, "engine_exit_code"),
        Some(4)
    );
    assert!(
        applied.stdout.contains("prerequisite-unsatisfied"),
        "the engine's own rule must survive: {}",
        applied.stdout
    );
}

#[test]
fn install_apply_reports_authorization_error_when_the_engine_forbids() {
    // Exit 5 had no emitter at all before this path existed; the engine's `FORBIDDEN` is where the
    // CLI's `authorization_error` class comes from. `Class` has no Authorization variant, so this
    // also proves the raw report constructor is used rather than folding 5 into another class.
    let fx = Fixture::new("install-engine-forbidden", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(
        &fx.dir,
        "engine",
        &engine_refusing_at_apply_body(5, "FORBIDDEN", "approval-required", false),
    ) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(&applied, 5, "authorization_error", "engine_forbidden");
    assert_eq!(
        json_number_field(&applied.stdout, "engine_exit_code"),
        Some(5)
    );
    assert_eq!(
        json_number_field(&applied.stdout, "code"),
        Some(5),
        "the envelope code must equal the process exit code"
    );
}

#[test]
fn install_apply_refuses_when_the_release_set_carries_no_skills_bundle() {
    // The P1 gap, made visible at the boundary: core components alone are not installable, and
    // the layer says exactly that instead of handing the engine a bundle it would reject.
    let fx = Fixture::new("install-engine-no-skills", Mode::Present);
    fx.seed_engine_release();
    fx.without_skills();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(
        &applied,
        NOT_READY,
        "not_ready",
        "engine_bundle_skills_missing",
    );
    assert!(
        !fx.root
            .join("staging")
            .join("engine-bundle")
            .join("bundle.json")
            .exists(),
        "a bundle that cannot be completed must not be left behind for the engine to read"
    );
    assert!(
        !fx.root.join("installed.json").exists(),
        "nothing was installed"
    );
}

#[test]
fn install_apply_refuses_a_skills_payload_that_does_not_match_its_manifest() {
    // The pass-through skills tree is not exempt from the "verified bytes only" rule: a payload
    // that disagrees with the digest its manifest declares is refused here, by name, before the
    // engine is invoked - not discovered later by the engine's own `plan` step.
    let fx = Fixture::new("install-engine-skills-tampered", Mode::Present);
    fx.seed_engine_release();
    std::fs::write(
        fx.release
            .join("skills")
            .join("payload")
            .join("instructions")
            .join("axiom.md"),
        b"# axiom\n\nTAMPERED after the manifest was written\n",
    )
    .expect("the payload must be writable");
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(
        &applied,
        VALIDATION,
        "validation_error",
        "skills_payload_unverified:instructions/axiom.md",
    );
    assert!(
        !fx.root
            .join("staging")
            .join("engine-bundle")
            .join("bundle.json")
            .exists(),
        "a bundle whose bytes do not match its declaration must not be left behind"
    );
    assert!(
        !fx.root.join("installed.json").exists(),
        "nothing was installed"
    );
}

#[test]
fn install_apply_refuses_a_skills_payload_file_its_manifest_does_not_declare() {
    // `copy_tree` carries the release set's whole skills tree. A file under `payload/**` that the
    // manifest does not declare would reach the engine unhashed, so the layer refuses it here and
    // names it - the "verified bytes only" claim is about every byte, not just declared ones.
    let fx = Fixture::new("install-engine-skills-undeclared", Mode::Present);
    fx.seed_engine_release();
    std::fs::write(
        fx.release
            .join("skills")
            .join("payload")
            .join("instructions")
            .join("smuggled.md"),
        b"# not declared by skills/bundle.json\n",
    )
    .expect("the undeclared payload must be writable");
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(
        &applied,
        VALIDATION,
        "validation_error",
        "skills_payload_undeclared",
    );
    assert!(
        applied.stderr.contains("instructions/smuggled.md")
            || applied.stdout.contains("instructions/smuggled.md"),
        "the refusal must name the undeclared file: stdout={} stderr={}",
        applied.stdout,
        applied.stderr
    );
    assert!(
        !fx.root
            .join("staging")
            .join("engine-bundle")
            .join("bundle.json")
            .exists(),
        "a bundle carrying an unverified byte must not be left behind"
    );
    assert!(
        !fx.root.join("installed.json").exists(),
        "nothing was installed"
    );
}

#[test]
fn install_apply_converts_the_source_skills_manifest_into_an_engine_bundle() {
    // The engine accepts `skills/bundle.json` only when every entry carries `capabilities`, and
    // only when the bundle's own `component` is `skills` (not the ecosystem's `axiom-skills`).
    // The converter is checked against exactly those two requirements, because the engine's own
    // refusal would otherwise be the first place a caller learned of them.
    let fx = Fixture::new("install-engine-convert-skills", Mode::Present);
    fx.seed_engine_release_from_source_skills();
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_single_json_object(&applied.stdout);
    assert_eq!(
        applied.code, SUCCESS,
        "the converted bundle must reach the engine: stdout={} stderr={}",
        applied.stdout, applied.stderr
    );
    assert!(
        applied.stdout.contains("skills_manifest_converted"),
        "the report must say which source the skills bundle came from: {}",
        applied.stdout
    );
    let manifest = std::fs::read_to_string(
        fx.root
            .join("staging")
            .join("engine-bundle")
            .join("skills")
            .join("bundle.json"),
    )
    .expect("the converted skills manifest must exist");
    assert!(
        manifest.contains("\"component\":\"skills\"") && !manifest.contains("axiom-skills"),
        "the declared component must be the one the engine's skills installer requires: {manifest}"
    );
    assert_eq!(
        manifest.matches("\"capabilities\":").count(),
        2,
        "every entry needs the `capabilities` field the engine requires: {manifest}"
    );
    assert!(
        manifest.contains("\"kind\":\"script\"")
            && manifest.contains("\"capabilities\":[\"read\",\"execute\"]"),
        "the reviewed executable entry must be declared as a script with its review: {manifest}"
    );
    assert!(
        fx.root
            .join("staging")
            .join("engine-bundle")
            .join("skills")
            .join("payload")
            .join("adapters")
            .join("common")
            .join("hook_runtime.py")
            .is_file(),
        "the converted payload must be placed under the bundle's payload root"
    );
}

#[test]
fn install_apply_refuses_a_core_component_that_declares_no_version() {
    // Verified bytes with no declared version: the layer refuses rather than inventing `0.0.0-dev`.
    let fx = Fixture::new("install-engine-undeclared", Mode::Present);
    fx.seed_engine_release();
    let manifest = std::fs::read_to_string(&fx.manifest).expect("the manifest must be readable");
    let stripped = manifest.replace(
        &format!(
            "\"component\":\"{GRAPHD}\",\"status\":\"declared\",\"version\":\"{GRAPHD_VERSION}\",\"revision\":\"{REVISION}\""
        ),
        &format!(
            "\"component\":\"{GRAPHD}\",\"status\":\"undeclared\",\"version\":null,\"revision\":null"
        ),
    );
    assert_ne!(
        stripped, manifest,
        "the graphd version row must have been rewritten"
    );
    std::fs::write(&fx.manifest, stripped).expect("the manifest must be writable");
    let Some(engine) = shell_engine(&fx.dir, "engine", &engine_accepting_body()) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(
        &applied,
        NOT_READY,
        "not_ready",
        "engine_bundle_component_undeclared:axiom-graphd",
    );
}

#[test]
fn install_apply_reports_the_engines_timeout_exit_code() {
    // A busy engine is retryable by its own account of itself, and the operator needs to be able
    // to tell "come back later" apart from "this request is wrong".
    let fx = Fixture::new("install-engine-timeout", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(
        &fx.dir,
        "engine",
        &engine_refusing_body(7, "TIMEOUT_BUSY", "lock_unavailable"),
    ) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_single_json_object(&applied.stdout);
    assert_eq!(
        applied.code, TIMEOUT,
        "a busy engine is its own exit code: stdout={} stderr={}",
        applied.stdout, applied.stderr
    );
    assert!(
        applied.stdout.contains("\"status\":\"timeout_busy\""),
        "the busy status must be reported: {}",
        applied.stdout
    );
    assert_eq!(
        json_string_field(&applied.stdout, "reason_code").as_deref(),
        Some("engine_timeout_busy")
    );
}

#[test]
fn install_apply_reports_the_engines_partial_exit_code() {
    // A partial placement is *not* success: the operator must not read exit 0 out of an install
    // that placed some components and not others.
    let fx = Fixture::new("install-engine-partial", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(
        &fx.dir,
        "engine",
        &engine_refusing_body(20, "PARTIAL", "rollback_incomplete"),
    ) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_single_json_object(&applied.stdout);
    assert_ne!(
        applied.code, SUCCESS,
        "a partial placement must never be reported as success: {}",
        applied.stdout
    );
    assert_eq!(
        applied.code, PARTIAL,
        "the engine's partial code is carried: stdout={} stderr={}",
        applied.stdout, applied.stderr
    );
    assert!(
        applied.stdout.contains("\"status\":\"partial\""),
        "the partial status must be reported: {}",
        applied.stdout
    );
    assert_eq!(
        json_string_field(&applied.stdout, "reason_code").as_deref(),
        Some("engine_partial")
    );
}

#[test]
fn install_apply_reports_the_engines_plan_step_refusal() {
    // A refusal at the *plan* step must be reported as the plan step, with the engine's own code
    // and rule, and must not be retried against the apply step.
    let fx = Fixture::new("install-engine-plan-refusal", Mode::Present);
    fx.seed_engine_release();
    let Some(engine) = shell_engine(
        &fx.dir,
        "engine",
        &engine_refusing_body(3, "NOT_FOUND", "component-missing"),
    ) else {
        return;
    };

    let applied = apply_engine_release(&fx, &engine);

    assert_refusal(&applied, NOT_FOUND, "not_found", "engine_not_found");
    assert_eq!(
        json_string_field(&applied.stdout, "engine_step").as_deref(),
        Some("plan")
    );
    assert_eq!(
        json_number_field(&applied.stdout, "engine_exit_code"),
        Some(3)
    );
    assert!(
        applied.stdout.contains("component-missing"),
        "the engine's own rule must survive: {}",
        applied.stdout
    );
    assert!(
        !applied.stdout.contains("\"step\":\"apply\""),
        "the apply step must not run after a failed plan: {}",
        applied.stdout
    );
}
