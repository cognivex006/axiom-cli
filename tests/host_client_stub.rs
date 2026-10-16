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
    let mut changed = false;
    let result = if words.starts_with(&["plugin", "marketplace", "list"]) {
        Json::from_pairs(vec![(
            "marketplaces",
            doc.get("marketplaces").unwrap().clone(),
        )])
    } else if words.starts_with(&["plugin", "list"]) {
        Json::from_pairs(vec![("installed", doc.get("installed").unwrap().clone())])
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
    } else if words.starts_with(&["plugin", "add"]) {
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
        let manifest = std::path::Path::new(root).join("plugins/axiom/.codex-plugin/plugin.json");
        let plugin = json::parse(&std::fs::read_to_string(manifest).unwrap()).unwrap();
        doc.set(
            "installed",
            Json::array(vec![Json::from_pairs(vec![
                ("name", Json::text("axiom")),
                ("marketplaceName", Json::text("axiom-skills")),
                ("installed", Json::bool(true)),
                ("enabled", Json::bool(true)),
                ("version", plugin.get("version").unwrap().clone()),
            ])]),
        )
        .unwrap();
        changed = true;
        Json::Object(Default::default())
    } else if words.starts_with(&["plugin", "remove"]) {
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

#[cfg(test)]
pub mod update {
    pub use crate::fixture_sha as sha256;
}
