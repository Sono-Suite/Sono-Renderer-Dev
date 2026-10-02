use anyhow::{anyhow, bail, Context, Result};
use flate2::read::{GzDecoder, ZlibDecoder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeSet,
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineMetadata {
    #[serde(default)]
    pub version: Option<u64>,
    #[serde(default)]
    pub title: Option<Value>,
    #[serde(default)]
    pub skin_name: Option<String>,
    #[serde(default)]
    pub background_name: Option<String>,
    #[serde(default)]
    pub effect_name: Option<String>,
    #[serde(default)]
    pub particle_name: Option<String>,
    #[serde(default)]
    pub skin: Option<Value>,
    #[serde(default)]
    pub background: Option<Value>,
    #[serde(default)]
    pub effect: Option<Value>,
    #[serde(default)]
    pub particle: Option<Value>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}
impl EngineMetadata {
    pub fn resource_defaults(&self) -> Vec<(String, String)> {
        let pairs = [
            ("skins", self.skin_name.as_ref(), self.skin.as_ref()),
            (
                "backgrounds",
                self.background_name.as_ref(),
                self.background.as_ref(),
            ),
            ("effects", self.effect_name.as_ref(), self.effect.as_ref()),
            (
                "particles",
                self.particle_name.as_ref(),
                self.particle.as_ref(),
            ),
        ];
        pairs
            .into_iter()
            .filter_map(|(category, legacy, item)| {
                legacy
                    .cloned()
                    .or_else(|| {
                        item.and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned))
                            .or_else(|| item.and_then(Value::as_str).map(str::to_owned))
                    })
                    .map(|name| (category.to_owned(), name))
            })
            .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelData {
    #[serde(default, rename = "bgmOffset", alias = "bgm_offset")]
    pub bgm_offset: Option<f64>,
    #[serde(default)]
    pub entities: Vec<LevelEntity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelEntity {
    #[serde(default)]
    pub archetype: Value,
    #[serde(default)]
    pub data: Value,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
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

#[derive(Debug, Clone)]
pub struct SkinSpriteAsset {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    /// Output vertex x/y expressions, with inputs ordered x1,y1,...,x4,y4.
    pub transform: [[f64; 8]; 8],
}

#[derive(Debug, Clone)]
pub struct SkinAssets {
    pub width: u32,
    pub height: u32,
    pub interpolation: bool,
    /// Decoded top-to-bottom RGBA8 atlas pixels.
    pub rgba: Vec<u8>,
    pub sprites: std::collections::BTreeMap<String, SkinSpriteAsset>,
}

#[derive(Debug, Clone)]
pub struct EffectAssets {
    /// Decoded clip files keyed by the `name` in EffectData. `None` means the
    /// EffectData entry was valid but its optional audio member was unavailable.
    pub clips: std::collections::BTreeMap<String, Option<Vec<u8>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleAssets {
    pub width: u32,
    pub height: u32,
    pub interpolation: bool,
    pub rgba: Vec<u8>,
    pub sprites: Vec<ParticleSpriteAsset>,
    pub effects: std::collections::BTreeMap<String, ParticleEffectData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleSpriteAsset {
    pub x: u32,
    pub y: u32,
    #[serde(rename = "w")]
    pub width: u32,
    #[serde(rename = "h")]
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleEffectData {
    pub name: String,
    pub transform: std::collections::BTreeMap<String, std::collections::BTreeMap<String, f64>>,
    pub groups: Vec<ParticleGroupData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleGroupData {
    pub count: u32,
    pub particles: Vec<ParticleDataEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticleDataEntry {
    pub sprite: usize,
    pub color: String,
    pub start: f64,
    pub duration: f64,
    pub x: ParticleProperty,
    pub y: ParticleProperty,
    pub w: ParticleProperty,
    pub h: ParticleProperty,
    pub r: ParticleProperty,
    pub a: ParticleProperty,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ParticleProperty {
    #[serde(default)]
    pub from: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    pub to: std::collections::BTreeMap<String, f64>,
    #[serde(default)]
    pub ease: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundData {
    #[serde(default, rename = "aspectRatio")]
    pub aspect_ratio: Option<f64>,
    pub fit: String,
    pub color: String,
    #[serde(default, rename = "scaleX")]
    pub scale_x: Option<f64>,
    #[serde(default, rename = "scaleY")]
    pub scale_y: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundConfiguration {
    #[serde(default)]
    pub blur: f64,
    #[serde(default = "default_background_mask")]
    pub mask: String,
}

fn default_background_mask() -> String {
    "#0000".to_owned()
}

#[derive(Debug, Clone)]
pub struct BackgroundAssets {
    pub width: u32,
    pub height: u32,
    /// Top-to-bottom RGBA8 background image.
    pub rgba: Vec<u8>,
    pub data: BackgroundData,
    pub configuration: BackgroundConfiguration,
}

impl BackgroundData {
    /// Resolve the initial Runtime Background quad in Sonolus screen coordinates.
    pub fn runtime_quad(
        &self,
        screen_aspect_ratio: f64,
        natural_image_aspect_ratio: f64,
    ) -> Result<[[f64; 2]; 4]> {
        if !screen_aspect_ratio.is_finite() || screen_aspect_ratio <= 0.0 {
            bail!("screen aspect ratio must be finite and positive");
        }
        let image_aspect_ratio = self.aspect_ratio.unwrap_or(natural_image_aspect_ratio);
        if !image_aspect_ratio.is_finite() || image_aspect_ratio <= 0.0 {
            bail!("background aspect ratio must be finite and positive");
        }
        let (mut half_width, mut half_height) = match self.fit.as_str() {
            "width" => (
                screen_aspect_ratio,
                screen_aspect_ratio / image_aspect_ratio,
            ),
            "height" => (image_aspect_ratio, 1.0),
            "contain" if image_aspect_ratio >= screen_aspect_ratio => (
                screen_aspect_ratio,
                screen_aspect_ratio / image_aspect_ratio,
            ),
            "contain" => (image_aspect_ratio, 1.0),
            "cover" if image_aspect_ratio >= screen_aspect_ratio => (image_aspect_ratio, 1.0),
            "cover" => (
                screen_aspect_ratio,
                screen_aspect_ratio / image_aspect_ratio,
            ),
            fit => bail!("unsupported background fit mode {fit:?}"),
        };
        let scale_x = self.scale_x.unwrap_or(1.0);
        let scale_y = self.scale_y.unwrap_or(1.0);
        if !scale_x.is_finite() || !scale_y.is_finite() {
            bail!("background scales must be finite");
        }
        half_width *= scale_x;
        half_height *= scale_y;
        Ok([
            [-half_width, -half_height],
            [-half_width, half_height],
            [half_width, half_height],
            [half_width, -half_height],
        ])
    }
}

fn read_payload(path: &Path) -> Result<Vec<u8>> {
    let data = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    decode_payload(data)
}
fn decode_payload(data: Vec<u8>) -> Result<Vec<u8>> {
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut decoder = GzDecoder::new(data.as_slice());
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded)?;
        Ok(decoded)
    } else {
        Ok(data)
    }
}
fn zip_member(path: &Path, candidates: &[&str]) -> Result<Vec<u8>> {
    zip_member_optional(path, candidates)?
        .ok_or_else(|| anyhow!("none of {:?} found in {}", candidates, path.display()))
}
fn zip_member_optional(path: &Path, candidates: &[&str]) -> Result<Option<Vec<u8>>> {
    let file =
        fs::File::open(path).with_context(|| format!("opening package {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("package is not a valid ZIP archive")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        if candidates
            .iter()
            .any(|candidate| name == *candidate || name.ends_with(&format!("/{candidate}")))
        {
            let mut data = Vec::new();
            entry.read_to_end(&mut data)?;
            return decode_payload(data).map(Some);
        }
    }
    Ok(None)
}
fn json_file(path: &Path) -> Result<Value> {
    serde_json::from_slice(&read_payload(path)?)
        .with_context(|| format!("decoding JSON {}", path.display()))
}
fn pick(root: &Path, candidates: &[&str]) -> Result<PathBuf> {
    for name in candidates {
        let p = root.join(name);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(anyhow!(
        "none of {:?} found under {}",
        candidates,
        root.display()
    ))
}

pub fn load_engine(path: &Path) -> Result<EnginePackage> {
    if path.is_file() && is_zip_archive(path)? {
        let metadata: EngineMetadata =
            serde_json::from_slice(&zip_member(path, &["engine.json", "item.json"])?)?;
        let configuration = serde_json::from_slice(&zip_member(
            path,
            &["EngineConfiguration", "configuration"],
        )?)?;
        // Engine Rom is optional in EngineItem. An absent ROM is an empty ROM
        // block; Watch reads from its unpopulated slots resolve to zero.
        let rom = zip_member_optional(path, &["EngineRom", "rom"])?.unwrap_or_default();
        let watch = serde_json::from_slice(&zip_member(
            path,
            &["EngineWatchData", "watchData", "watch_data"],
        )?)?;
        return Ok(EnginePackage {
            root: path.to_path_buf(),
            metadata,
            configuration,
            rom,
            watch: serde_json::from_value(watch)?,
        });
    }
    let root = if path.is_file() {
        path.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        path.to_path_buf()
    };
    let meta_path = pick(&root, &["engine.json", "item.json"])?;
    let metadata: EngineMetadata = serde_json::from_value(json_file(&meta_path)?)?;
    let config = pick(&root, &["EngineConfiguration", "configuration"])?;
    let rom = ["EngineRom", "rom"]
        .iter()
        .map(|name| root.join(name))
        .find(|path| path.is_file());
    let watch = pick(&root, &["EngineWatchData", "watchData", "watch_data"])?;
    let watch_value = json_file(&watch)?;
    Ok(EnginePackage {
        root,
        metadata,
        configuration: json_file(&config)?,
        rom: rom
            .map(|path| read_payload(&path))
            .transpose()?
            .unwrap_or_default(),
        watch: serde_json::from_value(watch_value)?,
    })
}

fn is_zip_archive(path: &Path) -> Result<bool> {
    let file =
        fs::File::open(path).with_context(|| format!("opening engine {}", path.display()))?;
    match ZipArchive::new(file) {
        Ok(_) => Ok(true),
        Err(error)
            if path.extension().is_some_and(|extension| {
                extension.to_string_lossy().eq_ignore_ascii_case("zip")
            }) =>
        {
            Err(error).context("engine archive is not a valid ZIP file")
        }
        Err(_) => Ok(false),
    }
}

pub fn load_level(path: &Path) -> Result<LevelData> {
    if path.is_file()
        && path
            .extension()
            .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("zip"))
    {
        let mut level: LevelData =
            serde_json::from_slice(&zip_member(path, &["level.data", "data", "LevelData"])?)?;
        level.metadata = match zip_member(path, &["item.json"]) {
            Ok(bytes) => Some(serde_json::from_slice(&bytes)?),
            Err(_) => None,
        };
        return Ok(level);
    }
    if path.is_dir() {
        let data = pick(path, &["level.data", "data", "LevelData"])?;
        let mut level = load_level_data(&data)?;
        let metadata = path.join("item.json");
        level.metadata = metadata
            .is_file()
            .then(|| json_file(&metadata))
            .transpose()?;
        return Ok(level);
    }
    // Standalone LevelData (including level.json.gz) has no metadata
    // dependency. An adjacent item.json belongs to the legacy package adapter.
    load_level_data(path)
}

/// Load Sonolus LevelData directly from JSON or gzip-compressed JSON.
/// No level item metadata, archive, cover, or music file is required here.
pub fn load_level_data(path: &Path) -> Result<LevelData> {
    let level: LevelData = serde_json::from_slice(&read_payload(path)?)
        .with_context(|| format!("decoding level data {}", path.display()))?;
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
        if parts.len() != 3
            || parts[0] != "sonolus"
            || parts[2].is_empty()
            || parts[2] == "list"
            || ![
                "skins",
                "backgrounds",
                "effects",
                "particles",
                "engines",
                "levels",
            ]
            .contains(&parts[1])
        {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let item = v.get("item").unwrap_or(&v);
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(parts[2])
            .to_string();
        let mut files = Vec::new();
        if let Some(obj) = item.as_object() {
            for (role, val) in obj {
                if let Some(hash) = val.get("hash").and_then(Value::as_str) {
                    files.push(format!("{role}:{hash}"));
                }
            }
        }
        resources.push(ScpResource {
            category: parts[1].to_string(),
            name,
            version: item.get("version").and_then(Value::as_u64),
            files,
        });
    }
    Ok(ScpIndex { resources })
}

/// Load the named skin's declared sprite names from an SCP content-addressed
/// repository. The referenced data payload's SHA-1 is verified before decode.
pub fn load_skin_sprite_names(path: &Path, name: &str) -> Result<BTreeSet<String>> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/skins/{name}");
    let mut manifest_bytes = Vec::new();
    zip.by_name(&manifest_path)
        .with_context(|| format!("skin {name:?} is not present in {}", path.display()))?
        .read_to_end(&mut manifest_bytes)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding skin manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let hash = item
        .get("data")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .context("skin manifest has no data hash")?;
    let repository_path = format!("sonolus/repository/{hash}");
    let mut payload = Vec::new();
    zip.by_name(&repository_path)
        .with_context(|| format!("skin data payload {repository_path} is missing"))?
        .read_to_end(&mut payload)?;
    let actual = hex::encode(Sha1::digest(&payload));
    if actual != hash {
        bail!("skin data payload digest {actual} does not match {hash}");
    }
    let data: Value =
        serde_json::from_slice(&decode_payload(payload)?).context("decoding skin data JSON")?;
    let sprites = data
        .get("sprites")
        .and_then(Value::as_array)
        .context("skin data has no sprites array")?;
    let mut names = BTreeSet::new();
    for (index, sprite) in sprites.iter().enumerate() {
        let name = sprite
            .get("name")
            .and_then(Value::as_str)
            .with_context(|| format!("skin sprite {index} has no string name"))?;
        if !names.insert(name.to_owned()) {
            bail!("skin {name:?} contains a duplicate sprite name");
        }
    }
    Ok(names)
}

/// Load the available clip names from a named Sonolus effect resource.
/// The content-addressed data payload is verified before JSON decoding.
pub fn load_effect_clip_names(path: &Path, name: &str) -> Result<BTreeSet<String>> {
    load_named_resource_names(path, "effects", name, "clips")
}

/// Load names for an optional effect resource. A missing named resource yields
/// `None`; malformed present resources and I/O failures remain errors.
pub fn load_effect_clip_names_optional(
    path: &Path,
    name: &str,
) -> Result<Option<BTreeSet<String>>> {
    load_named_resource_names_optional(path, "effects", name, "clips")
}

/// Load the available effect names from a named Sonolus particle resource.
/// The content-addressed data payload is verified before JSON decoding.
pub fn load_particle_effect_names(path: &Path, name: &str) -> Result<BTreeSet<String>> {
    load_named_resource_names(path, "particles", name, "effects")
}

/// Load the selected effect's named audio clips from its content-addressed ZIP.
pub fn load_effect_assets(path: &Path, name: &str) -> Result<EffectAssets> {
    load_effect_assets_inner(path, name)?.context("selected effect resource is missing")
}

/// Load effect assets when present. A missing named effect resource is an
/// unavailable optional SFX resource; archive, I/O, and parsing failures remain errors.
pub fn load_effect_assets_optional(path: &Path, name: &str) -> Result<Option<EffectAssets>> {
    load_effect_assets_inner(path, name)
}

fn load_effect_assets_inner(path: &Path, name: &str) -> Result<Option<EffectAssets>> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/effects/{name}");
    let mut manifest_bytes = Vec::new();
    match zip.by_name(&manifest_path) {
        Ok(mut entry) => {
            entry.read_to_end(&mut manifest_bytes)?;
        }
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {manifest_path}")),
    }
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding effect manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let data = read_resource_blob(&mut zip, item, "data", "effect")?;
    let audio = read_optional_resource_blob(&mut zip, item, "audio", "effect")?;
    let data: Value =
        serde_json::from_slice(&decode_payload(data)?).context("decoding EffectData JSON")?;
    let clips = data
        .get("clips")
        .and_then(Value::as_array)
        .context("EffectData has no clips array")?;
    let mut archive = audio
        .map(|bytes| ZipArchive::new(Cursor::new(bytes)).context("effect audio is not a ZIP"))
        .transpose()?;
    let mut result = std::collections::BTreeMap::new();
    for (index, clip) in clips.iter().enumerate() {
        let clip_name = clip
            .get("name")
            .and_then(Value::as_str)
            .with_context(|| format!("EffectData clip {index} has no name"))?;
        let filename = clip
            .get("filename")
            .and_then(Value::as_str)
            .with_context(|| format!("EffectData clip {clip_name:?} has no filename"))?;
        let mut bytes = Vec::new();
        let payload = match archive.as_mut() {
            Some(archive) => match archive.by_name(filename) {
                Ok(mut entry) => {
                    entry.read_to_end(&mut bytes)?;
                    Some(bytes)
                }
                Err(zip::result::ZipError::FileNotFound) => None,
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("reading effect audio archive member {filename:?}")
                    })
                }
            },
            None => None,
        };
        if result.insert(clip_name.to_owned(), payload).is_some() {
            bail!("EffectData contains duplicate clip name {clip_name:?}");
        }
    }
    Ok(Some(EffectAssets { clips: result }))
}

/// Load a particle atlas and the effect timelines that reference its sprites.
pub fn load_particle_assets(path: &Path, name: &str) -> Result<ParticleAssets> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/particles/{name}");
    let mut manifest_bytes = Vec::new();
    zip.by_name(&manifest_path)
        .with_context(|| format!("particle resource {name:?} is missing"))?
        .read_to_end(&mut manifest_bytes)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding particle manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let data = read_resource_blob(&mut zip, item, "data", "particle")?;
    let texture = read_resource_blob(&mut zip, item, "texture", "particle")?;
    let data: Value =
        serde_json::from_slice(&decode_payload(data)?).context("decoding ParticleData JSON")?;
    let width = data
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .context("ParticleData has invalid width")?;
    let height = data
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .context("ParticleData has invalid height")?;
    let interpolation = data
        .get("interpolation")
        .and_then(Value::as_bool)
        .context("ParticleData has invalid interpolation flag")?;
    let sprites: Vec<ParticleSpriteAsset> = serde_json::from_value(
        data.get("sprites")
            .cloned()
            .context("ParticleData has no sprites array")?,
    )
    .context("decoding ParticleData sprites")?;
    for (index, sprite) in sprites.iter().enumerate() {
        if sprite.width == 0
            || sprite.height == 0
            || sprite.x.checked_add(sprite.width).is_none_or(|x| x > width)
            || sprite
                .y
                .checked_add(sprite.height)
                .is_none_or(|y| y > height)
        {
            bail!("ParticleData sprite {index} is outside its declared atlas");
        }
    }
    let raw_effects = data
        .get("effects")
        .and_then(Value::as_array)
        .context("ParticleData has no effects array")?;
    let mut effects = std::collections::BTreeMap::new();
    for (index, value) in raw_effects.iter().enumerate() {
        let effect: ParticleEffectData = serde_json::from_value(value.clone())
            .with_context(|| format!("decoding ParticleData effect {index}"))?;
        validate_particle_effect(&effect, sprites.len())?;
        if effects.insert(effect.name.clone(), effect).is_some() {
            bail!("ParticleData contains duplicate effect names");
        }
    }
    let (texture_width, texture_height, rgba) = decode_png_rgba(&texture)?;
    if texture_width != width || texture_height != height {
        bail!("particle texture dimensions do not match ParticleData");
    }
    Ok(ParticleAssets {
        width,
        height,
        interpolation,
        rgba,
        sprites,
        effects,
    })
}

fn validate_particle_effect(effect: &ParticleEffectData, sprite_count: usize) -> Result<()> {
    if effect.name.is_empty() {
        bail!("ParticleData effect name cannot be empty");
    }
    for output in ["x1", "x2", "x3", "x4", "y1", "y2", "y3", "y4"] {
        if !effect.transform.contains_key(output) {
            bail!(
                "particle effect {:?} has no transform expression for {output}",
                effect.name
            );
        }
    }
    for (output, expression) in &effect.transform {
        if !matches!(
            output.as_str(),
            "x1" | "x2" | "x3" | "x4" | "y1" | "y2" | "y3" | "y4"
        ) {
            bail!(
                "particle effect {:?} has unsupported transform output {output:?}",
                effect.name
            );
        }
        if expression
            .values()
            .any(|coefficient| !coefficient.is_finite())
        {
            bail!(
                "particle effect {:?} has a non-finite transform coefficient",
                effect.name
            );
        }
        validate_particle_expression(effect, expression.keys())?;
    }
    for group in &effect.groups {
        for particle in &group.particles {
            if particle.sprite >= sprite_count {
                bail!(
                    "particle effect {:?} references missing sprite {}",
                    effect.name,
                    particle.sprite
                );
            }
            if !particle.start.is_finite()
                || !particle.duration.is_finite()
                || !(0.0..=1.0).contains(&particle.start)
                || particle.duration < 0.0
            {
                bail!(
                    "particle effect {:?} has an invalid particle timeline",
                    effect.name
                );
            }
            parse_background_color(&particle.color, false)
                .with_context(|| format!("particle effect {:?} has invalid color", effect.name))?;
            for property in [
                &particle.x,
                &particle.y,
                &particle.w,
                &particle.h,
                &particle.r,
                &particle.a,
            ] {
                if property
                    .from
                    .values()
                    .chain(property.to.values())
                    .any(|v| !v.is_finite())
                {
                    bail!(
                        "particle effect {:?} has a non-finite particle expression",
                        effect.name
                    );
                }
                validate_particle_expression(
                    effect,
                    property.from.keys().chain(property.to.keys()),
                )?;
                if let Some(ease) = &property.ease {
                    if !matches!(
                        ease.as_str(),
                        "linear"
                            | "none"
                            | "inSine"
                            | "outSine"
                            | "inOutSine"
                            | "outInSine"
                            | "inQuad"
                            | "outQuad"
                            | "inOutQuad"
                            | "outInQuad"
                            | "inCubic"
                            | "outCubic"
                            | "inOutCubic"
                            | "outInCubic"
                            | "inQuart"
                            | "outQuart"
                            | "inOutQuart"
                            | "outInQuart"
                            | "inQuint"
                            | "outQuint"
                            | "inOutQuint"
                            | "outInQuint"
                            | "inExpo"
                            | "outExpo"
                            | "inOutExpo"
                            | "outInExpo"
                            | "inCirc"
                            | "outCirc"
                            | "inOutCirc"
                            | "outInCirc"
                            | "inBack"
                            | "outBack"
                            | "inOutBack"
                            | "outInBack"
                            | "inElastic"
                            | "outElastic"
                            | "inOutElastic"
                            | "outInElastic"
                    ) {
                        bail!(
                            "particle effect {:?} uses unsupported easing {ease:?}",
                            effect.name
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn validate_particle_expression<'a>(
    effect: &ParticleEffectData,
    mut variables: impl Iterator<Item = &'a String>,
) -> Result<()> {
    let supported = |name: &str| {
        matches!(
            name,
            "c" | "x1" | "x2" | "x3" | "x4" | "y1" | "y2" | "y3" | "y4"
        ) || (1..=8).any(|index| {
            name == format!("r{index}")
                || name == format!("sinr{index}")
                || name == format!("cosr{index}")
        })
    };
    if let Some(name) = variables.find(|name| !supported(name)) {
        bail!(
            "particle effect {:?} uses unsupported expression variable {name:?}",
            effect.name
        );
    }
    Ok(())
}

fn load_named_resource_names(
    path: &Path,
    category: &str,
    name: &str,
    collection: &str,
) -> Result<BTreeSet<String>> {
    load_named_resource_names_optional(path, category, name, collection)?
        .with_context(|| format!("{category} resource {name:?} is missing"))
}

fn load_named_resource_names_optional(
    path: &Path,
    category: &str,
    name: &str,
    collection: &str,
) -> Result<Option<BTreeSet<String>>> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/{category}/{name}");
    let mut manifest_bytes = Vec::new();
    match zip.by_name(&manifest_path) {
        Ok(mut entry) => {
            entry.read_to_end(&mut manifest_bytes)?;
        }
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {manifest_path}")),
    }
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding resource manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let hash = item
        .get("data")
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .context("resource manifest has no data hash")?;
    let repository_path = format!("sonolus/repository/{hash}");
    let mut payload = Vec::new();
    zip.by_name(&repository_path)
        .with_context(|| format!("resource data payload {repository_path} is missing"))?
        .read_to_end(&mut payload)?;
    let actual = hex::encode(Sha1::digest(&payload));
    if actual != hash {
        bail!("resource data payload digest {actual} does not match {hash}");
    }
    let data: Value =
        serde_json::from_slice(&decode_payload(payload)?).context("decoding resource data JSON")?;
    let entries = data
        .get(collection)
        .and_then(Value::as_array)
        .with_context(|| format!("resource data has no {collection} array"))?;
    let mut names = BTreeSet::new();
    for (index, entry) in entries.iter().enumerate() {
        if let Some(entry_name) = entry.get("name").and_then(Value::as_str) {
            if !names.insert(entry_name.to_owned()) {
                bail!("resource data contains duplicate {collection} name {entry_name:?}");
            }
        } else {
            bail!("{category} resource entry {index} has no string name");
        }
    }
    Ok(Some(names))
}

/// Load and verify the selected skin's data and texture from an SCP archive.
pub fn load_skin_assets(path: &Path, name: &str) -> Result<SkinAssets> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/skins/{name}");
    let mut manifest_bytes = Vec::new();
    zip.by_name(&manifest_path)
        .with_context(|| format!("skin {name:?} is not present in {}", path.display()))?
        .read_to_end(&mut manifest_bytes)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding skin manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let data = read_skin_blob(&mut zip, item, "data")?;
    let texture = read_skin_blob(&mut zip, item, "texture")?;
    let data: Value =
        serde_json::from_slice(&decode_payload(data)?).context("decoding skin data JSON")?;
    let width = data
        .get("width")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .context("skin data has invalid width")?;
    let height = data
        .get("height")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .context("skin data has invalid height")?;
    let interpolation = data
        .get("interpolation")
        .and_then(Value::as_bool)
        .context("skin data has invalid interpolation flag")?;
    let sprite_values = data
        .get("sprites")
        .and_then(Value::as_array)
        .context("skin data has no sprites array")?;
    let mut sprites = std::collections::BTreeMap::new();
    for (index, value) in sprite_values.iter().enumerate() {
        let sprite_name = value
            .get("name")
            .and_then(Value::as_str)
            .with_context(|| format!("skin sprite {index} has no string name"))?;
        let coordinate = |key: &str| -> Result<u32> {
            value
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
                .with_context(|| format!("skin sprite {sprite_name:?} has invalid {key}"))
        };
        let x = coordinate("x")?;
        let y = coordinate("y")?;
        let sprite_width = coordinate("w")?;
        let sprite_height = coordinate("h")?;
        let transform = parse_skin_transform(value.get("transform"), sprite_name)?;
        let end_x = x
            .checked_add(sprite_width)
            .context("sprite x range overflow")?;
        let end_y = y
            .checked_add(sprite_height)
            .context("sprite y range overflow")?;
        if sprite_width == 0 || sprite_height == 0 || end_x > width || end_y > height {
            bail!("skin sprite {sprite_name:?} rectangle is outside the declared atlas");
        }
        let sprite = SkinSpriteAsset {
            x,
            y,
            width: sprite_width,
            height: sprite_height,
            transform,
        };
        if sprites.insert(sprite_name.to_owned(), sprite).is_some() {
            bail!("skin contains duplicate sprite name {sprite_name:?}");
        }
    }
    let (texture_width, texture_height, rgba) = decode_png_rgba(&texture)?;
    if texture_width != width || texture_height != height {
        bail!(
            "skin texture is {texture_width}x{texture_height}, SkinData declares {width}x{height}"
        );
    }
    Ok(SkinAssets {
        width,
        height,
        interpolation,
        rgba,
        sprites,
    })
}

/// Load a named Sonolus background image and its data/configuration from an SCP.
/// Every content-addressed payload is SHA-1 verified before decoding.
pub fn load_background_assets(path: &Path, name: &str) -> Result<BackgroundAssets> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/backgrounds/{name}");
    let mut manifest_bytes = Vec::new();
    zip.by_name(&manifest_path)
        .with_context(|| format!("background {name:?} is not present in {}", path.display()))?
        .read_to_end(&mut manifest_bytes)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding background manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let data = read_background_blob(&mut zip, item, "data")?;
    let image = read_background_blob(&mut zip, item, "image")?;
    let configuration = read_background_blob(&mut zip, item, "configuration")?;
    let data: BackgroundData =
        serde_json::from_slice(&decode_payload(data)?).context("decoding BackgroundData JSON")?;
    let configuration: BackgroundConfiguration =
        serde_json::from_slice(&decode_payload(configuration)?)
            .context("decoding BackgroundConfiguration JSON")?;
    if !configuration.blur.is_finite() || !(0.0..=1.0).contains(&configuration.blur) {
        bail!("background blur must be finite and in 0..=1");
    }
    let (width, height, rgba) = decode_png_rgba(&image)?;
    let natural_aspect = f64::from(width) / f64::from(height);
    let _ = data.runtime_quad(1.0, natural_aspect)?;
    parse_background_color(&data.color, false)?;
    parse_background_color(&configuration.mask, true)?;
    Ok(BackgroundAssets {
        width,
        height,
        rgba,
        data,
        configuration,
    })
}

/// Resolve only the Runtime Background quad inputs. This verifies and decodes
/// the selected background metadata, but reads only the PNG dimensions instead
/// of decoding/uploading its pixels. Event-only export passes use this to keep
/// Watch initialization identical without preparing a framebuffer.
pub fn load_background_runtime_quad(
    path: &Path,
    name: &str,
    screen_aspect_ratio: f64,
) -> Result<[[f64; 2]; 4]> {
    let file = fs::File::open(path).with_context(|| format!("opening SCP {}", path.display()))?;
    let mut zip = ZipArchive::new(file).context("SCP is not a valid ZIP archive")?;
    let manifest_path = format!("sonolus/backgrounds/{name}");
    let mut manifest_bytes = Vec::new();
    zip.by_name(&manifest_path)
        .with_context(|| format!("background {name:?} is not present in {}", path.display()))?
        .read_to_end(&mut manifest_bytes)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .with_context(|| format!("decoding background manifest {manifest_path}"))?;
    let item = manifest.get("item").unwrap_or(&manifest);
    let data = read_background_blob(&mut zip, item, "data")?;
    let image = read_background_blob(&mut zip, item, "image")?;
    let configuration = read_background_blob(&mut zip, item, "configuration")?;
    let data: BackgroundData =
        serde_json::from_slice(&decode_payload(data)?).context("decoding BackgroundData JSON")?;
    let configuration: BackgroundConfiguration =
        serde_json::from_slice(&decode_payload(configuration)?)
            .context("decoding BackgroundConfiguration JSON")?;
    if !configuration.blur.is_finite() || !(0.0..=1.0).contains(&configuration.blur) {
        bail!("background blur must be finite and in 0..=1");
    }
    let (width, height) = png_dimensions(&image)?;
    parse_background_color(&data.color, false)?;
    parse_background_color(&configuration.mask, true)?;
    data.runtime_quad(screen_aspect_ratio, f64::from(width) / f64::from(height))
}

fn png_dimensions(data: &[u8]) -> Result<(u32, u32)> {
    const HEADER: &[u8; 16] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    if data.len() < 24 || !data.starts_with(HEADER) {
        bail!("background image has no valid PNG IHDR header");
    }
    let width = u32::from_be_bytes(data[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(data[20..24].try_into().unwrap());
    if width == 0 || height == 0 {
        bail!("background image dimensions must be nonzero");
    }
    Ok((width, height))
}

fn read_background_blob(
    zip: &mut ZipArchive<fs::File>,
    item: &Value,
    role: &str,
) -> Result<Vec<u8>> {
    let hash = item
        .get(role)
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .with_context(|| format!("background manifest has no {role} hash"))?;
    let repository_path = format!("sonolus/repository/{hash}");
    let mut payload = Vec::new();
    zip.by_name(&repository_path)
        .with_context(|| format!("background {role} payload {repository_path} is missing"))?
        .read_to_end(&mut payload)?;
    let actual = hex::encode(Sha1::digest(&payload));
    if actual != hash {
        bail!("background {role} payload digest {actual} does not match {hash}");
    }
    Ok(payload)
}

pub(crate) fn parse_background_color(color: &str, allow_alpha: bool) -> Result<[u8; 4]> {
    let digits = color
        .strip_prefix('#')
        .context("background color must start with #")?;
    let (r, g, b, a) = match digits.len() {
        3 => {
            let values = digits
                .chars()
                .map(|c| c.to_digit(16).map(|n| (n * 17) as u8))
                .collect::<Option<Vec<_>>>()
                .context("invalid short background color")?;
            (values[0], values[1], values[2], 255)
        }
        4 if allow_alpha => {
            let values = digits
                .chars()
                .map(|c| c.to_digit(16).map(|n| (n * 17) as u8))
                .collect::<Option<Vec<_>>>()
                .context("invalid short background mask color")?;
            (values[0], values[1], values[2], values[3])
        }
        6 => {
            let values = (0..3)
                .map(|i| u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).ok())
                .collect::<Option<Vec<_>>>()
                .context("invalid background color")?;
            (values[0], values[1], values[2], 255)
        }
        8 if allow_alpha => {
            let values = (0..4)
                .map(|i| u8::from_str_radix(&digits[i * 2..i * 2 + 2], 16).ok())
                .collect::<Option<Vec<_>>>()
                .context("invalid background mask color")?;
            (values[0], values[1], values[2], values[3])
        }
        _ => bail!("unsupported background color format {color:?}"),
    };
    Ok([r, g, b, a])
}

fn read_resource_blob(
    zip: &mut ZipArchive<fs::File>,
    item: &Value,
    role: &str,
    resource_kind: &str,
) -> Result<Vec<u8>> {
    let hash = item
        .get(role)
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .with_context(|| format!("{resource_kind} manifest has no {role} hash"))?;
    let repository_path = format!("sonolus/repository/{hash}");
    let mut payload = Vec::new();
    zip.by_name(&repository_path)
        .with_context(|| format!("{resource_kind} {role} payload {repository_path} is missing"))?
        .read_to_end(&mut payload)?;
    let actual = hex::encode(Sha1::digest(&payload));
    if actual != hash {
        bail!("{resource_kind} {role} payload digest {actual} does not match {hash}");
    }
    Ok(payload)
}

fn read_optional_resource_blob(
    zip: &mut ZipArchive<fs::File>,
    item: &Value,
    role: &str,
    resource_kind: &str,
) -> Result<Option<Vec<u8>>> {
    let Some(resource) = item.get(role) else {
        return Ok(None);
    };
    let hash = resource
        .get("hash")
        .and_then(Value::as_str)
        .with_context(|| format!("{resource_kind} manifest has invalid {role} reference"))?;
    let repository_path = format!("sonolus/repository/{hash}");
    let mut payload = Vec::new();
    match zip.by_name(&repository_path) {
        Ok(mut entry) => {
            entry.read_to_end(&mut payload)?;
        }
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("reading {resource_kind} {role} payload {repository_path}")
            })
        }
    }
    let actual = hex::encode(Sha1::digest(&payload));
    if actual != hash {
        bail!("resource {role} payload digest {actual} does not match {hash}");
    }
    Ok(Some(payload))
}

fn read_skin_blob(zip: &mut ZipArchive<fs::File>, item: &Value, role: &str) -> Result<Vec<u8>> {
    let hash = item
        .get(role)
        .and_then(|value| value.get("hash"))
        .and_then(Value::as_str)
        .with_context(|| format!("skin manifest has no {role} hash"))?;
    let repository_path = format!("sonolus/repository/{hash}");
    let mut payload = Vec::new();
    zip.by_name(&repository_path)
        .with_context(|| format!("skin {role} payload {repository_path} is missing"))?
        .read_to_end(&mut payload)?;
    let actual = hex::encode(Sha1::digest(&payload));
    if actual != hash {
        bail!("skin {role} payload digest {actual} does not match {hash}");
    }
    Ok(payload)
}

fn parse_skin_transform(value: Option<&Value>, name: &str) -> Result<[[f64; 8]; 8]> {
    let value = value.with_context(|| format!("skin sprite {name:?} has no transform"))?;
    let keys = ["x1", "y1", "x2", "y2", "x3", "y3", "x4", "y4"];
    let mut matrix = [[0.0; 8]; 8];
    for (output, key) in keys.iter().enumerate() {
        let expression = value
            .get(key)
            .and_then(Value::as_object)
            .with_context(|| format!("skin sprite {name:?} transform has no {key} expression"))?;
        for (input, input_key) in keys.iter().enumerate() {
            if let Some(coefficient) = expression.get(*input_key) {
                let coefficient = coefficient.as_f64().with_context(|| {
                    format!("skin sprite {name:?} transform coefficient {key}.{input_key} is not numeric")
                })?;
                if !coefficient.is_finite() {
                    bail!("skin sprite {name:?} transform has a non-finite coefficient");
                }
                matrix[output][input] = coefficient;
            }
        }
    }
    Ok(matrix)
}

fn decode_png_rgba(data: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(SIGNATURE) {
        bail!("skin texture is not a PNG image");
    }
    let mut cursor = 8usize;
    let (mut width, mut height, mut color_type) = (0u32, 0u32, 255u8);
    let mut bit_depth = 0u8;
    let mut interlace = 0u8;
    let (mut palette, mut transparency, mut compressed) = (Vec::new(), Vec::new(), Vec::new());
    let mut ended = false;
    while cursor < data.len() {
        let header_end = cursor.checked_add(8).context("PNG chunk header overflow")?;
        let header = data
            .get(cursor..header_end)
            .context("truncated PNG chunk header")?;
        let length = u32::from_be_bytes(header[0..4].try_into().unwrap()) as usize;
        let kind = &header[4..8];
        let chunk_end = header_end
            .checked_add(length)
            .and_then(|n| n.checked_add(4))
            .context("PNG chunk size overflow")?;
        let chunk = data
            .get(header_end..chunk_end)
            .context("truncated PNG chunk")?;
        let payload = &chunk[..length];
        match kind {
            b"IHDR" => {
                if length != 13 || width != 0 {
                    bail!("invalid PNG IHDR chunk");
                }
                width = u32::from_be_bytes(payload[0..4].try_into().unwrap());
                height = u32::from_be_bytes(payload[4..8].try_into().unwrap());
                bit_depth = payload[8];
                color_type = payload[9];
                if payload[10] != 0 || payload[11] != 0 {
                    bail!("unsupported PNG compression or filter method");
                }
                interlace = payload[12];
            }
            b"PLTE" => palette.extend_from_slice(payload),
            b"tRNS" => transparency.extend_from_slice(payload),
            b"IDAT" => compressed.extend_from_slice(payload),
            b"IEND" => {
                ended = true;
                break;
            }
            _ => {}
        }
        cursor = chunk_end;
    }
    if !ended || width == 0 || height == 0 {
        bail!("PNG is missing a valid image header or IEND chunk");
    }
    if bit_depth != 8 || interlace != 0 {
        bail!("skin PNG must use 8-bit non-interlaced samples");
    }
    if width > 16384 || height > 16384 {
        bail!("skin PNG dimensions exceed 16384x16384");
    }
    let channels = match color_type {
        0 | 3 => 1usize,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => bail!("unsupported PNG color type {color_type}"),
    };
    if color_type == 3 && (palette.is_empty() || palette.len() % 3 != 0) {
        bail!("indexed PNG has an invalid palette");
    }
    let row_bytes = (width as usize)
        .checked_mul(channels)
        .context("PNG row size overflow")?;
    let filtered_size = row_bytes
        .checked_add(1)
        .and_then(|stride| stride.checked_mul(height as usize))
        .context("PNG image size overflow")?;
    let rgba_size = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4))
        .context("PNG RGBA image size overflow")?;
    if rgba_size > 512 * 1024 * 1024 {
        bail!("decoded skin texture exceeds 512 MiB");
    }
    let decoder = ZlibDecoder::new(Cursor::new(compressed));
    let mut filtered = Vec::with_capacity(filtered_size);
    decoder
        .take(filtered_size as u64 + 1)
        .read_to_end(&mut filtered)
        .context("decompressing PNG image data")?;
    if filtered.len() != filtered_size {
        bail!("PNG image data has an unexpected decompressed length");
    }
    let mut raw = vec![0u8; row_bytes * height as usize];
    for y in 0..height as usize {
        let source_start = y * (row_bytes + 1);
        let filter = filtered[source_start];
        let source = &filtered[source_start + 1..source_start + 1 + row_bytes];
        let row_start = y * row_bytes;
        for x in 0..row_bytes {
            let left = if x >= channels {
                raw[row_start + x - channels]
            } else {
                0
            };
            let above = if y > 0 {
                raw[row_start - row_bytes + x]
            } else {
                0
            };
            let upper_left = if y > 0 && x >= channels {
                raw[row_start - row_bytes + x - channels]
            } else {
                0
            };
            let predictor = match filter {
                0 => 0,
                1 => left,
                2 => above,
                3 => ((u16::from(left) + u16::from(above)) / 2) as u8,
                4 => paeth(left, above, upper_left),
                _ => bail!("PNG row uses invalid filter {filter}"),
            };
            raw[row_start + x] = source[x].wrapping_add(predictor);
        }
    }
    let mut rgba = vec![0u8; rgba_size];
    for (pixel, source) in raw.chunks_exact(channels).enumerate() {
        let target = pixel * 4;
        match color_type {
            0 => {
                rgba[target..target + 3].fill(source[0]);
                rgba[target + 3] = if transparency.len() >= 2
                    && u16::from_be_bytes([transparency[0], transparency[1]])
                        == u16::from(source[0])
                {
                    0
                } else {
                    255
                };
            }
            2 => {
                rgba[target..target + 3].copy_from_slice(source);
                rgba[target + 3] = if transparency.len() >= 6
                    && (0..3).all(|c| {
                        u16::from_be_bytes([transparency[c * 2], transparency[c * 2 + 1]])
                            == u16::from(source[c])
                    }) {
                    0
                } else {
                    255
                };
            }
            3 => {
                let palette_index = usize::from(source[0]);
                let color = palette
                    .get(palette_index * 3..palette_index * 3 + 3)
                    .context("PNG palette index is out of range")?;
                rgba[target..target + 3].copy_from_slice(color);
                rgba[target + 3] = transparency.get(palette_index).copied().unwrap_or(255);
            }
            4 => {
                rgba[target..target + 3].fill(source[0]);
                rgba[target + 3] = source[1];
            }
            6 => rgba[target..target + 4].copy_from_slice(source),
            _ => unreachable!(),
        }
    }
    Ok((width, height, rgba))
}

fn paeth(left: u8, above: u8, upper_left: u8) -> u8 {
    let left = i32::from(left);
    let above = i32::from(above);
    let upper_left = i32::from(upper_left);
    let estimate = left + above - upper_left;
    let left_distance = (estimate - left).abs();
    let above_distance = (estimate - above).abs();
    let upper_left_distance = (estimate - upper_left).abs();
    if left_distance <= above_distance && left_distance <= upper_left_distance {
        left as u8
    } else if above_distance <= upper_left_distance {
        above as u8
    } else {
        upper_left as u8
    }
}

pub fn verify_scp_references(path: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(path)?;
    let mut zip = ZipArchive::new(file)?;
    let mut issues = Vec::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_string();
        let parts: Vec<_> = name.split('/').collect();
        if parts.len() != 3
            || parts[0] != "sonolus"
            || parts[2].is_empty()
            || parts[2] == "list"
            || ![
                "skins",
                "backgrounds",
                "effects",
                "particles",
                "engines",
                "levels",
            ]
            .contains(&parts[1])
        {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        drop(entry);
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        let item = v.get("item").unwrap_or(&v);
        if let Some(obj) = item.as_object() {
            for (role, val) in obj {
                if let Some(hash) = val.get("hash").and_then(Value::as_str) {
                    let repo = format!("sonolus/repository/{hash}");
                    if zip.by_name(&repo).is_err() {
                        issues.push(format!("{name} {role} references missing {repo}"));
                        continue;
                    }
                    let mut r = zip.by_name(&repo)?;
                    let mut data = Vec::new();
                    r.read_to_end(&mut data)?;
                    let actual = hex::encode(Sha1::digest(data));
                    if actual != hash {
                        issues.push(format!("{repo} digest is {actual}, expected {hash}"));
                    }
                }
            }
        }
    }
    Ok(issues)
}
