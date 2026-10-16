//! Native host protocol fixture; never calls a real agent or changes its configuration.
#![allow(dead_code)]
#[cfg(test)]
#[path = "../src/update/sha256.rs"]
pub mod fixture_sha;
#[path = "../src/update/json.rs"]
mod json;
use json::Json;

fn main() {
    let exe = std::env::current_exe().unwrap();
    let state = exe.with_extension("state.json");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut doc = if state.exists() {
        json::parse(&std::fs::read_to_string(&state).unwrap()).unwrap()
    } else {
        Json::from_pairs(vec![
            ("marketplaces", Json::array(vec![])),
            ("installed", Json::array(vec![])),
        ])
    };
    let words: Vec<_> = args.iter().map(String::as_str).collect();
    let claude = exe.file_stem().unwrap().to_string_lossy() == "claude";
    let query = words.starts_with(&["plugin", "list"])
        || words.starts_with(&["plugin", "marketplace", "list"]);
    if claude && !query {
        assert!(
            !words.contains(&"--json"),
            "older Claude mutations have no --json flag"
        );
        assert!(words.windows(2).any(|w| w == ["--scope", "user"]));
    }
    let mut changed = false;
    let result = if words.starts_with(&["plugin", "marketplace", "list"]) {
        if claude {
            Json::array(
                doc.get("marketplaces")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        Json::from_pairs(vec![
                            ("name", r.get("name").unwrap().clone()),
                            (
                                "source",
                                r.get("source").cloned().unwrap_or(Json::text("directory")),
                            ),
                            ("path", r.get("root").unwrap().clone()),
                        ])
                    })
                    .collect(),
            )
        } else {
            Json::from_pairs(vec![(
                "marketplaces",
                doc.get("marketplaces").unwrap().clone(),
            )])
        }
    } else if words.starts_with(&["plugin", "list"]) {
        if claude {
            assert!(!words.contains(&"--marketplace"));
            Json::array(
                doc.get("installed")
                    .unwrap()
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|r| {
                        Json::from_pairs(vec![
                            (
                                "id",
                                Json::text(&format!(
                                    "{}@{}",
                                    r.get("name").unwrap().as_text().unwrap(),
                                    r.get("marketplaceName").unwrap().as_text().unwrap()
                                )),
                            ),
                            (
                                "scope",
                                r.get("scope").cloned().unwrap_or(Json::text("user")),
                            ),
                            ("enabled", r.get("enabled").unwrap().clone()),
                            ("version", r.get("version").unwrap().clone()),
                            ("installPath", r.get("installPath").unwrap().clone()),
                        ])
                    })
                    .collect(),
            )
        } else {
            Json::from_pairs(vec![("installed", doc.get("installed").unwrap().clone())])
        }
    } else if words.starts_with(&["plugin", "marketplace", "add"]) {
        doc.set(
            "marketplaces",
            Json::array(vec![Json::from_pairs(vec![
                ("name", Json::text("axiom-skills")),
                ("root", Json::text(&args[3])),
            ])]),
        )
        .unwrap();
        changed = true;
        Json::Object(Default::default())
    } else if words.starts_with(&["plugin", "marketplace", "remove"]) {
        doc.set("marketplaces", Json::array(vec![])).unwrap();
        changed = true;
        Json::Null
    } else if words.starts_with(&["plugin", "add"])
        || (claude && words.starts_with(&["plugin", "install"]))
    {
        let fail = exe.with_extension("fail-add");
        if fail.exists() {
            std::fs::remove_file(fail).unwrap();
            std::process::exit(9);
        }
        let root = doc.get("marketplaces").unwrap().as_array().unwrap()[0]
            .get("root")
            .unwrap()
            .as_text()
            .unwrap();
        let plugin_root = std::path::Path::new(root).join("plugins/axiom");
        let manifest = plugin_root.join(if claude {
            ".claude-plugin/plugin.json"
        } else {
            ".codex-plugin/plugin.json"
        });
        let plugin = json::parse(&std::fs::read_to_string(manifest).unwrap()).unwrap();
        let cache = exe.with_extension("cache");
        if claude {
            copy_owned_fixture(&plugin_root, &cache);
        }
        doc.set(
            "installed",
            Json::array(vec![Json::from_pairs(vec![
                ("name", Json::text("axiom")),
                ("marketplaceName", Json::text("axiom-skills")),
                ("installed", Json::bool(true)),
                ("enabled", Json::bool(true)),
                ("version", plugin.get("version").unwrap().clone()),
                ("installPath", Json::text(&cache.display().to_string())),
            ])]),
        )
        .unwrap();
        changed = true;
        Json::Object(Default::default())
    } else if words.starts_with(&["plugin", "remove"])
        || (claude && words.starts_with(&["plugin", "uninstall"]))
    {
        if claude {
            assert!(words.contains(&"--keep-data"));
            let cache = exe.with_extension("cache");
            if cache.exists() {
                std::fs::remove_dir_all(cache).unwrap();
            }
        }
        doc.set("installed", Json::array(vec![])).unwrap();
        changed = true;
        Json::Null
    } else {
        std::process::exit(2);
    };
    if changed {
        std::fs::write(state, json::canonical_text(&doc)).unwrap();
    }
    println!("{}", json::canonical_text(&result).trim());
}

fn copy_owned_fixture(source: &std::path::Path, target: &std::path::Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        assert!(!entry.file_type().unwrap().is_symlink());
        let destination = target.join(entry.file_name());
        if entry.path().is_dir() {
            copy_owned_fixture(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

#[cfg(test)]
pub mod update {
    pub use crate::fixture_sha as sha256;
}
