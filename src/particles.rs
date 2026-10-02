//! Deterministic evaluator for Sonolus particle resource timelines.

use crate::{formats, runtime::ParticleEffectInstance};
use anyhow::{bail, Result};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleSpriteDraw {
    pub sprite_id: usize,
    pub corners: [[f64; 2]; 4],
    pub color: [u8; 3],
    pub alpha: f64,
    pub order: (i64, usize, u32, usize),
}

pub fn render_instances(
    assets: &formats::ParticleAssets,
    effect_bindings: &BTreeMap<i64, String>,
    instances: &[ParticleEffectInstance],
    time: f64,
) -> Result<Vec<ParticleSpriteDraw>> {
    if !time.is_finite() {
        bail!("particle render time must be finite");
    }
    let mut draws = Vec::new();
    for instance in instances {
        let Some(effect_name) = effect_bindings.get(&instance.effect_id) else {
            continue;
        };
        let Some(effect) = assets.effects.get(effect_name) else {
            continue;
        };
        if !instance.duration.is_finite() || instance.duration <= 0.0 {
            continue;
        }
        let age = (time - instance.spawned_at).max(0.0);
        if !instance.is_looped && age >= instance.duration {
            continue;
        }
        let effect_time = (age / instance.duration).fract();
        let mut transform_rng = Rng::new(instance.instance_id as u64 ^ 0x9e3779b97f4a7c15);
        let effect_random = std::array::from_fn(|_| transform_rng.next());
        let effect_corners = transform_corners(effect, instance.corners, &effect_random);
        for (group_index, group) in effect.groups.iter().enumerate() {
            for copy in 0..group.count {
                let seed = (instance.instance_id as u64).wrapping_mul(0xd6e8feb86659fd93)
                    ^ ((group_index as u64) << 32)
                    ^ u64::from(copy);
                let mut rng = Rng::new(seed);
                let randoms = std::array::from_fn(|_| rng.next());
                for (particle_index, particle) in group.particles.iter().enumerate() {
                    let particle_end = particle.start + particle.duration;
                    if particle.duration <= 0.0
                        || effect_time < particle.start
                        || effect_time >= particle_end
                    {
                        continue;
                    }
                    let progress = (effect_time - particle.start) / particle.duration;
                    let [x, y, width, height, rotation, alpha] = [
                        property(&particle.x, progress, &randoms),
                        property(&particle.y, progress, &randoms),
                        property(&particle.w, progress, &randoms),
                        property(&particle.h, progress, &randoms),
                        property(&particle.r, progress, &randoms),
                        property(&particle.a, progress, &randoms),
                    ];
                    if ![x, y, width, height, rotation, alpha]
                        .iter()
                        .all(|value| value.is_finite())
                    {
                        continue;
                    }
                    let [red, green, blue, _] =
                        formats::parse_background_color(&particle.color, false)?;
                    let corners = particle_corners(effect_corners, x, y, width, height, rotation);
                    draws.push(ParticleSpriteDraw {
                        sprite_id: particle.sprite,
                        corners,
                        color: [red, green, blue],
                        alpha: alpha.clamp(0.0, 1.0),
                        order: (instance.instance_id, group_index, copy, particle_index),
                    });
                }
            }
        }
    }
    draws.sort_by_key(|draw| draw.order);
    Ok(draws)
}

fn transform_corners(
    effect: &formats::ParticleEffectData,
    input: [[f64; 2]; 4],
    randoms: &[f64; 8],
) -> [[f64; 2]; 4] {
    let mut variables = BTreeMap::<String, f64>::new();
    variables.insert("c".to_owned(), 1.0);
    for corner in 0..4 {
        variables.insert(format!("x{}", corner + 1), input[corner][0]);
        variables.insert(format!("y{}", corner + 1), input[corner][1]);
    }
    add_random_variables(&mut variables, randoms);
    std::array::from_fn(|corner| {
        let x_name = format!("x{}", corner + 1);
        let y_name = format!("y{}", corner + 1);
        [
            evaluate_expression(effect.transform.get(&x_name), &variables),
            evaluate_expression(effect.transform.get(&y_name), &variables),
        ]
    })
}

fn property(property: &formats::ParticleProperty, time: f64, randoms: &[f64; 8]) -> f64 {
    let mut variables = BTreeMap::new();
    variables.insert("c".to_owned(), 1.0);
    add_random_variables(&mut variables, randoms);
    let from = evaluate_expression(Some(&property.from), &variables);
    let to = evaluate_expression(Some(&property.to), &variables);
    let eased = match property.ease.as_deref().unwrap_or("linear") {
        "linear" | "none" => time,
        name => {
            let (direction, family) = [
                ("inOut", "InOut"),
                ("outIn", "OutIn"),
                ("in", "In"),
                ("out", "Out"),
            ]
            .into_iter()
            .find_map(|(prefix, direction)| {
                let family = name.strip_prefix(prefix)?;
                (!family.is_empty()).then_some((direction, family))
            })
            .unwrap_or(("In", "Linear"));
            crate::runtime::ease_curve(family, direction, time)
        }
    };
    from + (to - from) * eased
}

fn evaluate_expression(
    expression: Option<&BTreeMap<String, f64>>,
    variables: &BTreeMap<String, f64>,
) -> f64 {
    expression
        .into_iter()
        .flat_map(|values| values.iter())
        .map(|(name, coefficient)| coefficient * variables.get(name).copied().unwrap_or(0.0))
        .sum()
}

fn add_random_variables(variables: &mut BTreeMap<String, f64>, randoms: &[f64; 8]) {
    for (index, random) in randoms.iter().copied().enumerate() {
        let suffix = index + 1;
        variables.insert(format!("r{suffix}"), random);
        variables.insert(
            format!("sinr{suffix}"),
            (std::f64::consts::TAU * random).sin(),
        );
        variables.insert(
            format!("cosr{suffix}"),
            (std::f64::consts::TAU * random).cos(),
        );
    }
}

fn particle_corners(
    quad: [[f64; 2]; 4],
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rotation: f64,
) -> [[f64; 2]; 4] {
    let local = [
        [x - width * 0.5, y - height * 0.5],
        [x - width * 0.5, y + height * 0.5],
        [x + width * 0.5, y + height * 0.5],
        [x + width * 0.5, y - height * 0.5],
    ];
    let (sin, cos) = rotation.sin_cos();
    local.map(|[px, py]| {
        let dx = px - x;
        let dy = py - y;
        let u = (x + dx * cos - dy * sin + 1.0) * 0.5;
        let v = (y + dx * sin + dy * cos + 1.0) * 0.5;
        bilinear(quad, u, v)
    })
}

fn bilinear(quad: [[f64; 2]; 4], u: f64, v: f64) -> [f64; 2] {
    let weights = [(1.0 - u) * (1.0 - v), (1.0 - u) * v, u * v, u * (1.0 - v)];
    std::array::from_fn(|axis| {
        (0..4)
            .map(|corner| quad[corner][axis] * weights[corner])
            .sum()
    })
}

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0xa0761d6478bd642f)
    }

    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
        value ^= value >> 31;
        (value >> 11) as f64 / ((1_u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn property(from: f64, to: f64) -> formats::ParticleProperty {
        formats::ParticleProperty {
            from: [("c".to_owned(), from)].into(),
            to: [("c".to_owned(), to)].into(),
            ease: Some("linear".to_owned()),
        }
    }

    #[test]
    fn active_particle_uses_lifecycle_progress_and_composites_to_frame() {
        let effect = formats::ParticleEffectData {
            name: "one".into(),
            transform: (1..=4)
                .flat_map(|i| {
                    [
                        (format!("x{i}"), [(format!("x{i}"), 1.0)].into()),
                        (format!("y{i}"), [(format!("y{i}"), 1.0)].into()),
                    ]
                })
                .collect(),
            groups: vec![formats::ParticleGroupData {
                count: 1,
                particles: vec![formats::ParticleDataEntry {
                    sprite: 0,
                    color: "#ffffff".into(),
                    start: 0.0,
                    duration: 1.0,
                    x: property(0.0, 0.0),
                    y: property(0.0, 0.0),
                    w: property(1.0, 1.0),
                    h: property(1.0, 1.0),
                    r: property(0.0, 0.0),
                    a: property(1.0, 1.0),
                }],
            }],
        };
        let assets = formats::ParticleAssets {
            width: 1,
            height: 1,
            interpolation: false,
            rgba: vec![255, 0, 0, 255],
            sprites: vec![formats::ParticleSpriteAsset {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            }],
            effects: [("one".into(), effect)].into(),
        };
        let instance = ParticleEffectInstance {
            instance_id: 3,
            effect_id: 7,
            corners: [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
            duration: 2.0,
            is_looped: false,
            spawned_at: 0.0,
        };
        let instances = vec![instance];
        let draws =
            render_instances(&assets, &[(7, "one".into())].into(), &instances, 1.0).unwrap();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].alpha, 1.0);
        let mut rgb = vec![0, 0, 0];
        crate::runtime::DisplayList::composite_particle_sprites(
            &mut rgb,
            1,
            1,
            1.0,
            &assets,
            &draws,
            &[
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        )
        .unwrap();
        assert_eq!(rgb, [255, 0, 0]);
        assert!(
            render_instances(&assets, &[(7, "one".into())].into(), &instances, 3.0)
                .unwrap()
                .is_empty()
        );
    }
}
