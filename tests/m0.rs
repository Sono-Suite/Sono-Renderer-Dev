use flate2::{write::GzEncoder, Compression};
use std::io::Write;

#[test]
fn watch_inventory_keeps_arbitrary_operation_names() {
    let raw = br#"{"archetypes":[{"name":"Unfamiliar","imports":[],"updateParallel":{"index":4}}],"nodes":[{"func":"AlienOperation","args":[0,1]},{"func":"NovelFn","args":[]}] }"#;
    let parsed: serde_json::Value = serde_json::from_slice(raw).unwrap();
    let watch: renderer::watch::WatchData = serde_json::from_value(parsed).unwrap();
    let inv = renderer::watch::inventory(&watch);
    assert_eq!(inv.archetypes, vec!["Unfamiliar"]);
    assert!(inv.functions.contains(&"AlienOperation".to_string()));
    assert!(inv.functions.contains(&"NovelFn".to_string()));
}

#[test]
fn watch_inventory_resolves_memory_block_ids_from_get_nodes() {
    let raw = br#"{"nodes":[{"value":1000},{"value":0},{"func":"Get","args":[0,1]}]}"#;
    let watch: renderer::watch::WatchData = serde_json::from_slice(raw).unwrap();
    assert_eq!(renderer::watch::inventory(&watch).memory_blocks, vec!["1000"]);
}

#[test]
fn engine_resource_defaults_accept_item_and_legacy_metadata() {
    let metadata: renderer::formats::EngineMetadata = serde_json::from_value(serde_json::json!({
        "version": 13,
        "skin": {"name":"skin-a"},
        "background_name":"background-b",
        "effect": {"name":"effect-c"},
        "particle": {"name":"particle-d"}
    })).unwrap();
    let defaults=metadata.resource_defaults();
    assert_eq!(defaults.len(),4);
    assert!(defaults.contains(&("skins".into(),"skin-a".into())));
    assert!(defaults.contains(&("backgrounds".into(),"background-b".into())));
}

#[test]
fn adjacent_reference_fixtures_load_when_present() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let engine = repo.join("Next Sekai Engine.zip");
    let level = repo.join("Exorcist.zip");
    let scp = repo.join("ProSeka Faithful 0.8.3.scp");
    if !(engine.is_file() && level.is_file() && scp.is_file()) { return; }
    let pkg = renderer::formats::load_engine(&engine).unwrap();
    assert!(!pkg.watch.nodes.is_empty());
    assert!(!pkg.rom.is_empty());
    assert!(!renderer::formats::load_level(&level).unwrap().entities.is_empty());
    assert!(!renderer::formats::inspect_scp(&scp).unwrap().resources.is_empty());
    let report = renderer::compatibility::analyze(&engine, &scp, &level).unwrap();
    assert!(report.level_entity_count > 0);
    assert!(!report.level_archetypes.is_empty());
    assert!(report.engine.support.vm_runtime.contains("not implemented"));
    assert!(report.resource_integrity_issues.is_empty());
}

#[test]
fn gzip_magic_decodes_json_payload() {
    let mut encoder=GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(br#"{"bgmOffset":0,"entities":[]}"#).unwrap();
    let compressed=encoder.finish().unwrap();
    let dir=tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("level.data"),compressed).unwrap();
    let level=renderer::formats::load_level(dir.path()).unwrap();
    assert_eq!(level.bgm_offset,Some(0.0));
}
