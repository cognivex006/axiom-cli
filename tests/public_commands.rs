//! Execute the canonical native entrypoints, preserving output and failure semantics.
use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("public-command-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn alias(root: &std::path::Path, name: &str) -> PathBuf {
    let path = root.join(if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    });
    std::fs::copy(env!("CARGO_BIN_EXE_axiom-cli"), &path).unwrap();
    path
}

#[test]
fn native_alias_help_names_the_real_component_without_legacy_command_labels() {
    for (name, role) in [
        ("axm", "Axiom"),
        ("axsk", "Skills Manager"),
        ("axmp", "MCP Server"),
    ] {
        let root = fixture(name);
        let output = Command::new(alias(&root, name))
            .arg("--help")
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains(name) && text.contains(role), "{text}");
        assert!(!text.contains("axiom-cli"), "{text}");
        assert!(output.stderr.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn absent_mcp_runtime_is_not_success_and_json_stays_one_object() {
    let root = fixture("mcp-missing");
    let output = Command::new(alias(&root, "axmp"))
        .args(["version", "--json"])
        .env("AXIOM_CLI_INSTALL_ROOT", root.join("missing"))
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let text = String::from_utf8(output.stdout).unwrap();
    assert_eq!(text.lines().count(), 1);
    assert!(text.contains("\"code\":4"), "{text}");
    assert!(output.stderr.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}
