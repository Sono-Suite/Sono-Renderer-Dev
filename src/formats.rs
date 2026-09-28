use anyhow::{anyhow, Context, Result};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{fs, io::Read, path::{Path, PathBuf}};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineMetadata {
    #[serde(default)] pub version: Option<u64>,
    #[serde(default)] pub title: Option<Value>,
    #[serde(default)] pub skin_name: Option<String>,
    #[serde(default)] pub background_name: Option<String>,
    #[serde(default)] pub effect_name: Option<String>,
    #[serde(default)] pub particle_name: Option<String>,
    #[serde(default)] pub skin: Option<Value>,
    #[serde(default)] pub background: Option<Value>,
    #[serde(default)] pub effect: Option<Value>,
    #[serde(default)] pub particle: Option<Value>,
    #[serde(flatten)] pub extra: serde_json::Map<String, Value>,
}
impl EngineMetadata {
    pub fn resource_defaults(&self) -> Vec<(String, String)> {
        let pairs = [
            ("skins", self.skin_name.as_ref(), self.skin.as_ref()),
            ("backgrounds", self.background_name.as_ref(), self.background.as_ref()),
            ("effects", self.effect_name.as_ref(), self.effect.as_ref()),
            ("particles", self.particle_name.as_ref(), self.particle.as_ref()),
        ];
        pairs.into_iter().filter_map(|(category, legacy, item)| {
            legacy.cloned().or_else(|| item.and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned)).or_else(|| item.and_then(Value::as_str).map(str::to_owned)))
                .map(|name| (category.to_owned(), name))
        }).collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelData {
    #[serde(default)] pub bgm_offset: Option<f64>,
    #[serde(default)] pub entities: Vec<LevelEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub metadata: Option<Value>,
    #[serde(flatten)] pub extra: serde_json::Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelEntity {
    #[serde(default)] pub archetype: Value,
    #[serde(default)] pub data: Value,
    #[serde(flatten)] pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnginePackage {
    pub root: PathBuf,
    pub metadata: EngineMetadata,
    pub configuration: Value,
    pub rom: Vec<u8>,
    pub watch: crate::watch::WatchData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpIndex {
    pub resources: Vec<ScpResource>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScpResource {
    pub category: String,
    pub name: String,
    pub version: Option<u64>,
    pub files: Vec<String>,
}

fn read_payload(path: &Path) -> Result<Vec<u8>> {
    let data = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    decode_payload(data)
}
fn decode_payload(data: Vec<u8>) -> Result<Vec<u8>> {
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(data.as_slice());
        let mut decoded = Vec::new(); decoder.read_to_end(&mut decoded)?; Ok(decoded)
    } else { Ok(data) }
}
fn zip_member(path: &Path, candidates: &[&str]) -> Result<Vec<u8>> {
    let file = fs::File::open(path).with_context(|| format!("opening package {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("package is not a valid ZIP archive")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if candidates.iter().any(|candidate| name == *candidate || name.ends_with(&format!("/{candidate}"))) {
            let mut data = Vec::new(); entry.read_to_end(&mut data)?; return decode_payload(data);
        }
    }
    Err(anyhow!("none of {:?} found in {}", candidates, path.display()))
}
fn json_file(path: &Path) -> Result<Value> {
    serde_json::from_slice(&read_payload(path)?).with_context(|| format!("decoding JSON {}", path.display()))
}
fn pick(root: &Path, candidates: &[&str]) -> Result<PathBuf> {
    for name in candidates { let p = root.join(name); if p.is_file() { return Ok(p); } }
    Err(anyhow!("none of {:?} found under {}", candidates, root.display()))
}

pub fn load_engine(path: &Path) -> Result<EnginePackage> {
    if path.is_file() && path.extension().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("zip")) {
        let metadata: EngineMetadata = serde_json::from_slice(&zip_member(path, &["engine.json", "item.json"])?)?;
        let configuration = serde_json::from_slice(&zip_member(path, &["EngineConfiguration", "configuration"] )?)?;
        let rom = zip_member(path, &["EngineRom", "rom"])?;
        let watch = serde_json::from_slice(&zip_member(path, &["EngineWatchData", "watchData", "watch_data"] )?)?;
        return Ok(EnginePackage { root:path.to_path_buf(), metadata, configuration, rom, watch:serde_json::from_value(watch)? });
    }
    let root = if path.is_file() { path.parent().unwrap_or(Path::new(".")).to_path_buf() } else { path.to_path_buf() };
    let meta_path = pick(&root, &["engine.json", "item.json"])?;
    let metadata: EngineMetadata = serde_json::from_value(json_file(&meta_path)?)?;
    let config = pick(&root, &["EngineConfiguration", "configuration"])?;
    let rom = pick(&root, &["EngineRom", "rom"])?;
    let watch = pick(&root, &["EngineWatchData", "watchData", "watch_data"])?;
    let watch_value = json_file(&watch)?;
    Ok(EnginePackage { root, metadata, configuration: json_file(&config)?, rom: read_payload(&rom)?, watch: serde_json::from_value(watch_value)? })
}

pub fn load_level(path: &Path) -> Result<LevelData> {
    if path.is_file() && path.extension().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("zip")) {
        let mut level: LevelData = serde_json::from_slice(&zip_member(path, &["level.data", "data", "LevelData"])?)?;
        level.metadata = match zip_member(path, &["item.json"]) { Ok(bytes) => Some(serde_json::from_slice(&bytes)?), Err(_) => None };
        return Ok(level);
    }
    let (data, metadata) = if path.is_dir() { (pick(path, &["level.data", "data", "LevelData"])?, path.join("item.json")) } else { (path.to_path_buf(), path.parent().unwrap_or(Path::new(".")).join("item.json")) };
    let mut level: LevelData = serde_json::from_slice(&read_payload(&data)?).with_context(|| format!("decoding level data {}", data.display()))?;
    level.metadata = metadata.is_file().then(|| json_file(&metadata)).transpose()?;
    Ok(level)
}

pub fn inspect_scp(path: &Path) -> Result<ScpIndex> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let mut resources = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let n = entry.name().to_string();
        let parts: Vec<_> = n.split('/').collect();
        if parts.len() != 3 || parts[0] != "sonolus" || parts[2].is_empty() || parts[2] == "list" || !["skins", "backgrounds", "effects", "particles", "engines", "levels"].contains(&parts[1]) { continue; }
        let mut bytes = Vec::new(); entry.read_to_end(&mut bytes)?;
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else { continue };
        let item = v.get("item").unwrap_or(&v);
        let name = item.get("name").and_then(Value::as_str).unwrap_or(parts[2]).to_string();
        let mut files = Vec::new();
        if let Some(obj) = item.as_object() {
            for (role, val) in obj { if let Some(hash) = val.get("hash").and_then(Value::as_str) { files.push(format!("{role}:{hash}")); } }
        }
        resources.push(ScpResource { category: parts[1].to_string(), name, version: item.get("version").and_then(Value::as_u64), files });
    }
    Ok(ScpIndex { resources })
}

pub fn verify_scp_references(path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(path)?; let mut zip = ZipArchive::new(file)?; let mut issues = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?; let name = entry.name().to_string(); let parts: Vec<_> = name.split('/').collect();
        if parts.len()!=3 || parts[0]!="sonolus" || parts[2].is_empty() || parts[2]=="list" || !["skins","backgrounds","effects","particles","engines","levels"].contains(&parts[1]) { continue; }
        let mut bytes=Vec::new(); entry.read_to_end(&mut bytes)?; drop(entry); let Ok(v)=serde_json::from_slice::<Value>(&bytes) else {continue}; let item=v.get("item").unwrap_or(&v);
        if let Some(obj)=item.as_object() { for (role,val) in obj { if let Some(hash)=val.get("hash").and_then(Value::as_str) {
            let repo=format!("sonolus/repository/{hash}"); if zip.by_name(&repo).is_err(){issues.push(format!("{name} {role} references missing {repo}"));continue;}
            let mut r=zip.by_name(&repo)?;let mut data=Vec::new();r.read_to_end(&mut data)?;let actual=hex::encode(Sha1::digest(data)); if actual!=hash {issues.push(format!("{repo} digest is {actual}, expected {hash}"));}
        }}}
    } Ok(issues)
}

