//! Renderer-owned, deliberately generic HUD overlays.
//!
//! This module does not reproduce Sonolus Runtime UI behavior. Values are
//! supplied by explicit providers and drawn with a small built-in bitmap font.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum UiValueProvider {
    Fixed {
        value: f64,
    },
    External {
        value: f64,
    },
    Memory {
        block_id: i64,
        index: usize,
    },
    Progress,
    /// Reserved until an authoritative engine metric source is available.
    EngineMetric {
        key: String,
    },
    /// Reserved until generic replay judgment aggregation is implemented.
    Accuracy,
    /// Reserved for a caller-defined judgment-to-value mapping.
    JudgmentDerived {
        mapping: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiMetric {
    pub enabled: bool,
    pub label: String,
    pub provider: UiValueProvider,
    pub maximum: Option<f64>,
}

impl Default for UiMetric {
    fn default() -> Self {
        Self {
            enabled: true,
            label: "METRIC".into(),
            provider: UiValueProvider::Fixed { value: 0.0 },
            maximum: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiValueComponent {
    pub enabled: bool,
    pub label: String,
    pub provider: UiValueProvider,
}

impl Default for UiValueComponent {
    fn default() -> Self {
        Self {
            enabled: false,
            label: String::new(),
            provider: UiValueProvider::Fixed { value: 0.0 },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RendererUiConfig {
    pub enabled: bool,
    pub primary_metric: UiMetric,
    pub secondary_metric: UiMetric,
    pub combo: UiValueComponent,
    pub judgment: UiValueComponent,
    pub progress: bool,
    pub theme: UiTheme,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct UiTheme {
    pub panel: [u8; 3],
    pub panel_alpha: u8,
    pub text: [u8; 3],
    pub label: [u8; 3],
    pub accent: [u8; 3],
    pub progress_track: [u8; 3],
}

impl Default for UiTheme {
    fn default() -> Self {
        Self {
            panel: [12, 16, 26],
            panel_alpha: 205,
            text: [255, 255, 255],
            label: [210, 220, 235],
            accent: [70, 210, 220],
            progress_track: [25, 29, 40],
        }
    }
}

impl Default for RendererUiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            primary_metric: UiMetric::default(),
            secondary_metric: UiMetric {
                enabled: false,
                label: "SECONDARY".into(),
                ..UiMetric::default()
            },
            combo: UiValueComponent {
                label: "COMBO".into(),
                ..UiValueComponent::default()
            },
            judgment: UiValueComponent {
                label: "JUDGMENT".into(),
                ..UiValueComponent::default()
            },
            progress: true,
            theme: UiTheme::default(),
        }
    }
}

impl RendererUiConfig {
    pub fn validate(&self) -> Result<()> {
        for (name, metric) in [
            ("primary metric", &self.primary_metric),
            ("secondary metric", &self.secondary_metric),
        ] {
            if let Some(maximum) = metric.maximum {
                if !maximum.is_finite() || maximum <= 0.0 {
                    bail!("{name} maximum must be finite and positive");
                }
            }
            validate_provider(name, &metric.provider)?;
        }
        validate_provider("combo", &self.combo.provider)?;
        validate_provider("judgment", &self.judgment.provider)?;
        Ok(())
    }
}

fn validate_provider(name: &str, provider: &UiValueProvider) -> Result<()> {
    match provider {
        UiValueProvider::Fixed { value } | UiValueProvider::External { value }
            if !value.is_finite() =>
        {
            bail!("{name} provider value must be finite")
        }
        UiValueProvider::EngineMetric { key } if key.trim().is_empty() => {
            bail!("{name} engine metric key cannot be empty")
        }
        UiValueProvider::JudgmentDerived { mapping } if mapping.trim().is_empty() => {
            bail!("{name} judgment mapping cannot be empty")
        }
        _ => {}
    }
    Ok(())
}

pub fn render_overlay(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    time: f64,
    segment_start: f64,
    segment_duration: f64,
    config: &RendererUiConfig,
    mut value: impl FnMut(&UiValueProvider) -> Result<f64>,
) -> Result<()> {
    config.validate()?;
    if !config.enabled {
        return Ok(());
    }
    if rgb.len() != width as usize * height as usize * 3 {
        bail!("UI overlay frame buffer dimensions do not match RGB data");
    }
    let w = width as i32;
    let h = height as i32;
    let margin = (w.min(h) / 40).clamp(6, 24);
    let panel_w = (w / 3).max(150).min(w - margin * 2);
    let row_h = 34;
    if config.primary_metric.enabled {
        draw_value_panel(
            rgb,
            w,
            h,
            margin,
            margin,
            panel_w,
            row_h,
            &config.primary_metric.label,
            value(&config.primary_metric.provider)?,
            config.primary_metric.maximum,
            &config.theme,
        );
    }
    if config.secondary_metric.enabled {
        draw_value_panel(
            rgb,
            w,
            h,
            margin,
            margin + row_h + 5,
            panel_w,
            row_h,
            &config.secondary_metric.label,
            value(&config.secondary_metric.provider)?,
            config.secondary_metric.maximum,
            &config.theme,
        );
    }
    if config.combo.enabled {
        let panel_x = w - panel_w - margin;
        draw_value_panel(
            rgb,
            w,
            h,
            panel_x,
            margin,
            panel_w,
            row_h,
            &config.combo.label,
            value(&config.combo.provider)?,
            None,
            &config.theme,
        );
    }
    if config.judgment.enabled {
        let panel_x = w - panel_w - margin;
        draw_value_panel(
            rgb,
            w,
            h,
            panel_x,
            margin + row_h + 5,
            panel_w,
            row_h,
            &config.judgment.label,
            value(&config.judgment.provider)?,
            None,
            &config.theme,
        );
    }
    if config.progress {
        let x = margin;
        let y = h - margin - 10;
        let track_w = w - margin * 2;
        blend_rect(
            rgb,
            w,
            h,
            x,
            y,
            track_w,
            8,
            config.theme.progress_track,
            220,
        );
        let progress = if segment_duration > 0.0 {
            ((time - segment_start) / segment_duration).clamp(0.0, 1.0)
        } else {
            0.0
        };
        blend_rect(
            rgb,
            w,
            h,
            x,
            y,
            (f64::from(track_w) * progress).round() as i32,
            8,
            config.theme.accent,
            255,
        );
    }
    Ok(())
}

fn draw_value_panel(
    rgb: &mut [u8],
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    panel_w: i32,
    panel_h: i32,
    label: &str,
    value: f64,
    maximum: Option<f64>,
    theme: &UiTheme,
) {
    blend_rect(
        rgb,
        width,
        height,
        x,
        y,
        panel_w,
        panel_h,
        theme.panel,
        theme.panel_alpha,
    );
    draw_text(
        rgb,
        width,
        height,
        x + 7,
        y + 5,
        &label.to_uppercase(),
        theme.label,
    );
    draw_text(
        rgb,
        width,
        height,
        x + panel_w / 2,
        y + 5,
        &format_value(value),
        theme.text,
    );
    if let Some(maximum) = maximum {
        let fraction = (value / maximum).clamp(0.0, 1.0);
        let bar_w = panel_w - 14;
        blend_rect(
            rgb,
            width,
            height,
            x + 7,
            y + panel_h - 5,
            bar_w,
            2,
            theme.progress_track,
            255,
        );
        blend_rect(
            rgb,
            width,
            height,
            x + 7,
            y + panel_h - 5,
            (f64::from(bar_w) * fraction).round() as i32,
            2,
            theme.accent,
            255,
        );
    }
}

fn format_value(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1.0e15 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn blend_rect(
    rgb: &mut [u8],
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    rect_w: i32,
    rect_h: i32,
    color: [u8; 3],
    alpha: u8,
) {
    for py in y.max(0)..(y + rect_h).min(height) {
        for px in x.max(0)..(x + rect_w).min(width) {
            let offset = (py as usize * width as usize + px as usize) * 3;
            for channel in 0..3 {
                rgb[offset + channel] = ((u16::from(rgb[offset + channel])
                    * u16::from(255 - alpha)
                    + u16::from(color[channel]) * u16::from(alpha))
                    / 255) as u8;
            }
        }
    }
}

fn draw_text(rgb: &mut [u8], width: i32, height: i32, x: i32, y: i32, text: &str, color: [u8; 3]) {
    let mut cursor = x;
    for ch in text.chars().take(32) {
        let rows = glyph(ch);
        for (row, bits) in rows.into_iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    blend_rect(
                        rgb,
                        width,
                        height,
                        cursor + col,
                        y + row as i32,
                        1,
                        1,
                        color,
                        255,
                    );
                }
            }
        }
        cursor += 6;
    }
}

fn glyph(ch: char) -> [u8; 7] {
    match ch {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 6, 6],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '%' => [17, 2, 4, 8, 17, 0, 0],
        ' ' => [0; 7],
        _ => [31, 17, 5, 4, 4, 0, 4],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_is_off_by_default_and_each_component_can_be_independent() {
        let mut frame = vec![0; 320 * 180 * 3];
        let config = RendererUiConfig::default();
        render_overlay(&mut frame, 320, 180, 0.5, 0.0, 1.0, &config, |_| Ok(7.0)).unwrap();
        assert!(frame.iter().all(|byte| *byte == 0));

        for component in 0..5 {
            let mut frame = vec![0; 320 * 180 * 3];
            let mut config = RendererUiConfig::default();
            config.enabled = true;
            config.primary_metric.enabled = component == 0;
            config.secondary_metric.enabled = component == 1;
            config.combo.enabled = component == 2;
            config.judgment.enabled = component == 3;
            config.progress = component == 4;
            render_overlay(&mut frame, 320, 180, 0.5, 0.0, 1.0, &config, |_| Ok(7.0)).unwrap();
            assert!(frame.iter().any(|byte| *byte != 0), "component {component}");
        }
    }

    #[test]
    fn memory_and_fixed_metric_providers_round_trip_as_configuration() {
        let json = serde_json::json!({"kind":"memory","block_id":2000,"index":7});
        let provider: UiValueProvider = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(serde_json::to_value(provider).unwrap(), json);
        RendererUiConfig::default().validate().unwrap();
        assert!(RendererUiConfig {
            primary_metric: UiMetric {
                maximum: Some(0.0),
                ..UiMetric::default()
            },
            ..RendererUiConfig::default()
        }
        .validate()
        .is_err());
    }
}
