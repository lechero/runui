use super::*;
use crate::{Context, Snapshot, usage};
use serde_json::{Value, json};

fn fixture() -> (tempfile::TempDir, Entry, Value) {
    let temp = tempfile::tempdir().unwrap();
    let root = canonical(&temp.path().to_string_lossy());
    fs::create_dir_all(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("Cargo.toml"), "[package]\nname='example'\nedition='2024'\n[features]\ngated=[]\n[[bin]]\nname='worker'\npath='src/main.rs'\nrequired-features=['gated']\n").unwrap();
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    let context = Context::new(&root, &root);
    let snapshot = Snapshot::build(&context, &runui_project::discover(&root, &root), &[], &[], &usage::Data::default());
    let entry = snapshot.entries().iter().find(|entry| entry.name.starts_with("run")).unwrap().clone();
    let metadata = json!({
        "version":1,"workspace_root":root,"workspace_members":["opaque ID : not a path ☕"],
        "workspace_default_members":["opaque ID : not a path ☕"],"packages":[{
            "id":"opaque ID : not a path ☕","name":"example","manifest_path":format!("{root}/Cargo.toml"),
            "targets":[{"name":"worker","kind":["bin"],"src_path":format!("{root}/src/main.rs"),
                "crate_types":["bin"],"required-features":["gated"],"future_target_field":42}],
            "features":{"gated":[],"future_feature":[]},"default_run":"worker","future_package_field":{}
        }],"future_metadata_field":true
    });
    (temp, entry, metadata)
}

#[test]
fn versioned_metadata_uses_opaque_ids_and_enriches_only_the_selected_definition() {
    let (_temp, entry, value) = fixture();
    let report = metadata(&value.to_string(), &entry).unwrap();
    assert!(report.definition_matches);
    assert!(report.diagnostics.is_empty());
    assert_eq!(report.members.as_slice(), std::slice::from_ref(&entry.directory));
    assert_eq!(report.targets[0], entry.cargo.as_ref().unwrap().target.clone().unwrap());
    assert_eq!(report.available_features, ["future_feature", "gated"]);
    assert_eq!(report.package.as_deref(), Some("example"));
}

#[test]
fn mismatches_unknown_ids_versions_and_external_targets_keep_static_fallback() {
    let (_temp, entry, value) = fixture();
    let mut changed = value.clone();
    changed["packages"][0]["targets"][0]["required-features"] = json!([]);
    let report = metadata(&changed.to_string(), &entry).unwrap();
    assert!(!report.definition_matches);
    assert!(report.diagnostics[0].contains("differs"));
    changed = value.clone();
    changed["workspace_default_members"] = json!(["unknown"]);
    assert!(metadata(&changed.to_string(), &entry).unwrap_err().to_string().contains("unknown package"));
    changed = value.clone();
    changed["version"] = json!(2);
    assert!(metadata(&changed.to_string(), &entry).is_err());
    changed = value.clone();
    changed["packages"][0]["targets"][0]["src_path"] = json!("/outside/checkout/main.rs");
    let report = metadata(&changed.to_string(), &entry).unwrap();
    assert!(!report.definition_matches);
    assert!(report.targets.is_empty());
    assert!(report.diagnostics.iter().any(|message| message.contains("outside")));
    assert_eq!(entry.cargo.as_ref().unwrap().target.as_ref().unwrap().required_features, ["gated"]);
}

#[test]
fn absent_lockfile_is_diagnosed_before_starting_native_tools() {
    let (_temp, entry, _value) = fixture();
    let report = check(&entry, &AtomicBool::new(false));
    assert!(!report.ready);
    assert!(report.checked.is_empty());
    assert!(report.skipped.as_ref().unwrap().contains("existing Cargo.lock"));
    assert!(!Path::new(&entry.directory).join("Cargo.lock").exists());
}
