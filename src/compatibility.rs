use crate::{formats, watch};
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Serialize)]
pub struct EngineReport {
    pub sonolus_version: Option<u64>,
    pub archetypes: Vec<String>,
    pub functions_operators: Vec<String>,
    pub memory_blocks: Vec<String>,
    pub skin_bindings: Value,
    pub effect_bindings: Value,
    pub particle_bindings: Value,
    pub required_resources: Vec<String>,
    pub currently_unsupported_runtime_features: Vec<String>,
    pub support: LayerSupport,
}
#[derive(Debug, Serialize)]
pub struct LayerSupport {
    pub parse: String,
    pub vm_runtime: String,
    pub rendering: String,
    pub particles: String,
    pub audio: String,
}
#[derive(Debug, Serialize)]
pub struct CompatibilityReport {
    pub engine: EngineReport,
    pub level_entity_count: usize,
    pub level_archetypes: Vec<LevelArchetypeStatus>,
    pub resources: formats::ScpIndex,
    pub required_resource_status: Vec<ResourceStatus>,
    pub resource_integrity_issues: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct ResourceStatus {
    pub category: String,
    pub name: String,
    pub resolved: bool,
}
#[derive(Debug, Serialize)]
pub struct LevelArchetypeStatus {
    pub archetype: Value,
    pub engine_provides: bool,
}

fn report(path: &Path) -> Result<(formats::EnginePackage, EngineReport)> {
    let pkg = formats::load_engine(path)?;
    let inv = watch::inventory(&pkg.watch);
    let mut required = BTreeSet::new();
    let defaults = pkg.metadata.resource_defaults();
    for (_, name) in &defaults {
        required.insert(name.clone());
    }
    let report = EngineReport {
        sonolus_version: pkg.metadata.version,
        archetypes: inv.archetypes,
        functions_operators: inv.functions,
        memory_blocks: inv.memory_blocks,
        skin_bindings: inv.skin_bindings,
        effect_bindings: inv.effect_bindings,
        particle_bindings: inv.particle_bindings,
        required_resources: required.into_iter().collect(),
        currently_unsupported_runtime_features: vec![
            "Watch operations and host semantics outside the implemented subset".into(),
            "Complete Sonolus engine lifecycle and scheduling semantics".into(),
            "Rendering".into(),
            "Particle simulation".into(),
            "Audio scheduling and mixing".into(),
        ],
        support: LayerSupport {
            parse: "supported for recognized package envelopes; unknown fields retained".into(),
            vm_runtime: "partial Watch VM and entity lifecycle execution; operation and host semantics remain incomplete".into(),
            rendering: "diagnostic PPM rasterizer only; Sonolus skin/resource rendering is not implemented".into(),
            particles: "not implemented (M0)".into(),
            audio: "not implemented (M0)".into(),
        },
    };
    Ok((pkg, report))
}
pub fn inspect_engine(path: &Path) -> Result<EngineReport> {
    Ok(report(path)?.1)
}
pub fn analyze(
    engine: &Path,
    resource_path: &Path,
    level_path: &Path,
) -> Result<CompatibilityReport> {
    let (pkg, engine_report) = report(engine)?;
    let level = formats::load_level(level_path)?;
    let known: BTreeSet<_> = engine_report.archetypes.iter().cloned().collect();
    let mut archetypes = BTreeSet::new();
    for entity in &level.entities {
        archetypes.insert(entity.archetype.to_string());
    }
    let level_archetypes = archetypes
        .into_iter()
        .map(|s| {
            let a = serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s));
            let name = a.as_str().unwrap_or("").to_string();
            LevelArchetypeStatus {
                engine_provides: known.contains(&name),
                archetype: a,
            }
        })
        .collect();
    let resources = formats::inspect_scp(resource_path)?;
    let required_resource_status = pkg
        .metadata
        .resource_defaults()
        .into_iter()
        .map(|(category, name)| ResourceStatus {
            resolved: resources.resources.iter().any(|r| {
                r.category.as_str() == category.as_str() && r.name.as_str() == name.as_str()
            }),
            category,
            name,
        })
        .collect();
    Ok(CompatibilityReport {
        engine: engine_report,
        level_entity_count: level.entities.len(),
        level_archetypes,
        resources,
        required_resource_status,
        resource_integrity_issues: formats::verify_scp_references(resource_path)?,
    })
}
