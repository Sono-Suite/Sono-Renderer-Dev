//! User-facing input model for a render project.
//!
//! The level is standalone LevelData, music is a separate required input, and
//! cover/MV files are optional presentation inputs. This portable model does
//! not invoke FFmpeg or depend on a GUI.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInputs {
    pub engine_package: PathBuf,
    pub resource_package: PathBuf,
    pub level_data: PathBuf,
    pub music: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_mv: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ValidatedProject {
    pub engine: crate::formats::EnginePackage,
    pub resources: crate::formats::ScpIndex,
    pub level: crate::formats::LevelData,
    pub music: PathBuf,
    pub cover: Option<PathBuf>,
    pub custom_mv: Option<PathBuf>,
}

impl ProjectInputs {
    /// Validate required inputs and parse the engine, resource index, and
    /// standalone LevelData. An absent cover or MV is a normal valid state.
    pub fn validate(&self) -> Result<ValidatedProject> {
        require_file(&self.engine_package, "engine ZIP")?;
        if !self
            .engine_package
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("zip"))
        {
            bail!(
                "engine input must be an engine ZIP: {}",
                self.engine_package.display()
            );
        }
        require_file(&self.resource_package, "SCP/resource package")?;
        require_file(&self.level_data, "level data")?;
        require_file(&self.music, "music/audio")?;
        if let Some(cover) = &self.cover {
            require_file(cover, "cover image")?;
        }
        if let Some(mv) = &self.custom_mv {
            require_file(mv, "custom MV/video")?;
        }

        let engine = crate::formats::load_engine(&self.engine_package)
            .with_context(|| format!("loading engine {}", self.engine_package.display()))?;
        let resources = crate::formats::inspect_scp(&self.resource_package)
            .with_context(|| format!("loading resources {}", self.resource_package.display()))?;
        let level = crate::formats::load_level_data(&self.level_data)
            .with_context(|| format!("loading level data {}", self.level_data.display()))?;

        Ok(ValidatedProject {
            engine,
            resources,
            level,
            music: self.music.clone(),
            cover: self.cover.clone(),
            custom_mv: self.custom_mv.clone(),
        })
    }
}

fn require_file(path: &std::path::Path, label: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("accessing {label} at {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("{label} must be a non-empty file: {}", path.display());
    }
    Ok(())
}
