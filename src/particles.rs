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
    render_instances_traced(assets, effect_bindings, instances, time, None, None)
}

/// Observe the production evaluator without replaying Watch or changing random state.
pub fn render_instances_traced(
    assets: &formats::ParticleAssets,
    effect_bindings: &BTreeMap<i64, String>,
    instances: &[ParticleEffectInstance],
    time: f64,
    trace: Option<&crate::watch_diagnostics::SharedWatchTrace>,
    viewport: Option<(u32, u32, &[f64; 16])>,
) -> Result<Vec<ParticleSpriteDraw>> {
    if !time.is_finite() {
        bail!("particle render time must be finite");
    }
    let mut draws = Vec::new();
    for instance in instances {
        let Some(effect_name) = effect_bindings.get(&instance.effect_id) else {
            trace_instance(
                trace,
                instance,
                time,
                "EffectSuppressed",
                || serde_json::json!({"reason":"unbound effect ID"}),
            )?;
            continue;
        };
        let Some(effect) = assets.effects.get(effect_name) else {
            trace_instance(
                trace,
                instance,
                time,
                "EffectSuppressed",
                || serde_json::json!({"reason":"missing effect resource","resource":effect_name}),
            )?;
            continue;
        };
        if !instance.duration.is_finite() || instance.duration <= 0.0 {
            trace_instance(
                trace,
                instance,
                time,
                "EffectSuppressed",
                || serde_json::json!({"reason":"invalid duration","duration":instance.duration}),
            )?;
            continue;
        }
        let age = (time - instance.spawned_at).max(0.0);
        let last_end = effect
            .groups
            .iter()
            .flat_map(|g| &g.particles)
            .map(|p| (p.start + p.duration) * instance.duration)
            .fold(0.0_f64, f64::max);
        if !instance.is_looped && age >= last_end {
            trace_instance(
                trace,
                instance,
                time,
                "EffectSuppressed",
                || serde_json::json!({"reason":"expired","age":age,"duration":instance.duration}),
            )?;
            continue;
        }
        let effect_time = (age / instance.duration).fract();
        let mut transform_rng = Rng::new(instance.instance_id as u64 ^ 0x9e3779b97f4a7c15);
        let effect_random = std::array::from_fn(|_| transform_rng.next());
        let effect_corners = transform_corners(effect, instance.corners, &effect_random);
        if let Some(trace) = trace {
            trace.lock().map_err(|_|anyhow::anyhow!("particle trace lock poisoned"))?.record_particle(instance,time,"EffectEvaluation",
                serde_json::json!({"resource":effect_name,"definition":effect,"instance":instance,"age":age,"effect_time":effect_time,
                    "effect_seed":instance.instance_id as u64 ^ 0x9e3779b97f4a7c15,"effect_random":effect_random,"resource_corners":effect_corners}));
        }
        for (group_index, group) in effect.groups.iter().enumerate() {
            for copy in 0..group.count {
                let seed = (instance.instance_id as u64).wrapping_mul(0xd6e8feb86659fd93)
                    ^ ((group_index as u64) << 32)
                    ^ u64::from(copy);
                let mut rng = Rng::new(seed);
                let randoms = std::array::from_fn(|_| rng.next());
                for (particle_index, particle) in group.particles.iter().enumerate() {
                    let Some(timing) =
                        particle_timing(instance, time, particle.start, particle.duration)
                    else {
                        trace_instance(
                            trace,
                            instance,
                            time,
                            "ParticleSuppressed",
                            || serde_json::json!({"particle":[group_index,copy,particle_index],"reason":"outside particle lifetime","effect_time":effect_time,"start":particle.start,"duration":particle.duration}),
                        )?;
                        continue;
                    };
                    let progress = timing.progress;
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
                    if let Some(trace) = trace {
                        let local = particle_corners(
                            [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]],
                            x,
                            y,
                            width,
                            height,
                            rotation,
                        );
                        let final_corners = viewport.map(|(_, _, m)| {
                            crate::runtime::gpu_transform_particle_corners(
                                &ParticleSpriteDraw {
                                    sprite_id: particle.sprite,
                                    corners,
                                    color: [red, green, blue],
                                    alpha,
                                    order: (
                                        instance.instance_id,
                                        group_index,
                                        copy,
                                        particle_index,
                                    ),
                                },
                                m,
                            )
                        });
                        let pixels = viewport.zip(final_corners).map(|((w, h, _), q)| {
                            q.map(|[x, y]| {
                                [
                                    (x / (f64::from(w) / f64::from(h)) + 1.0) * 0.5 * f64::from(w),
                                    (1.0 - y) * 0.5 * f64::from(h),
                                ]
                            })
                        });
                        let atlas_uvs = assets.sprites.get(particle.sprite).map(|s| {
                            let left = f64::from(s.x) / f64::from(assets.width);
                            let right = f64::from(s.x + s.width) / f64::from(assets.width);
                            let top = f64::from(s.y) / f64::from(assets.height);
                            let bottom = f64::from(s.y + s.height) / f64::from(assets.height);
                            [[left, bottom], [left, top], [right, top], [right, bottom]]
                        });
                        trace.lock().map_err(|_|anyhow::anyhow!("particle trace lock poisoned"))?.record_particle(instance,time,"ParticleEvaluation",
                            serde_json::json!({"particle":[group_index,copy,particle_index],"group_count":group.count,"entry":particle,
                                "seed":seed,"randoms":randoms,"spawn_time":timing.spawn_time,
                                "lifetime":timing.lifetime,"age":timing.age,
                                "progress":progress,"properties":{"x":x,"y":y,"width":width,"height":height,"rotation":rotation,"alpha":alpha},
                                "local_corners":local,"effect_corners":corners,"runtime_matrix":viewport.map(|(_,_,m)|m),"final_corners":final_corners,"pixels":pixels,
                                "sprite":assets.sprites.get(particle.sprite),"sprite_id":particle.sprite,"atlas_dimensions":[assets.width,assets.height],
                                "uvs":[[0,0],[0,1],[1,1],[1,0]],"atlas_uvs_top_origin":atlas_uvs,"order":[instance.instance_id,group_index as i64,copy as i64,particle_index as i64],
                                "color":[red,green,blue],"interpolation":assets.interpolation,"blend":"straight-alpha source-over", "uv_interpolation":"inverse bilinear"}));
                    }
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

struct ParticleTiming {
    spawn_time: f64,
    lifetime: f64,
    age: f64,
    progress: f64,
}

/// A resource entry has its own birth and lifetime; effect duration sets the
/// loop interval, not a clipping plane for delayed particle tails.
fn particle_timing(
    instance: &ParticleEffectInstance,
    time: f64,
    start: f64,
    duration: f64,
) -> Option<ParticleTiming> {
    let birth = instance.spawned_at + start * instance.duration;
    let lifetime = duration * instance.duration;
    if duration <= 0.0 || time < birth {
        return None;
    }
    let elapsed = time - birth;
    let age = if instance.is_looped {
        elapsed % instance.duration
    } else {
        elapsed
    };
    if (instance.is_looped && age > lifetime) || (!instance.is_looped && age >= lifetime) {
        return None;
    }
    Some(ParticleTiming {
        spawn_time: time - age,
        lifetime,
        age,
        progress: age / lifetime,
    })
}

fn trace_instance(
    trace: Option<&crate::watch_diagnostics::SharedWatchTrace>,
    instance: &ParticleEffectInstance,
    time: f64,
    operation: &str,
    data: impl FnOnce() -> serde_json::Value,
) -> Result<()> {
    if let Some(trace) = trace {
        trace
            .lock()
            .map_err(|_| anyhow::anyhow!("particle trace lock poisoned"))?
            .record_particle(instance, time, operation, data());
    }
    Ok(())
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
        "linear" => time,
        "none" => {
            if time == 1.0 {
                1.0
            } else {
                0.0
            }
        }
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
        [x - width, y - height],
        [x - width, y + height],
        [x + width, y + height],
        [x + width, y - height],
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

    #[test]
    fn particle_size_spans_the_effect_quad_at_unit_size() {
        let quad = [[-0.8, -0.6], [-0.8, 0.6], [0.8, 0.6], [0.8, -0.6]];
        assert_eq!(particle_corners(quad, 0.0, 0.0, 1.0, 1.0, 0.0), quad);
    }

    #[test]
    fn none_easing_holds_from_until_the_exact_endpoint() {
        let mut p = property(2.0, 6.0);
        p.ease = Some("none".into());
        for t in [0.0, 0.25, 0.5, 0.999, 1.001] {
            assert_eq!(super::property(&p, t, &[0.0; 8]), 2.0);
        }
        assert_eq!(super::property(&p, 1.0, &[0.0; 8]), 6.0);
    }

    #[test]
    fn delayed_particle_lifetime_can_cross_the_effect_interval() {
        let mut instance = ParticleEffectInstance {
            instance_id: 0,
            effect_id: 0,
            corners: [[0.0; 2]; 4],
            spawned_at: 2.0,
            duration: 1.0,
            is_looped: false,
        };
        assert!(particle_timing(&instance, 2.7, 0.75, 0.5).is_none());
        let timing = particle_timing(&instance, 3.125, 0.75, 0.5).unwrap();
        assert_eq!(timing.spawn_time, 2.75);
        assert_eq!(timing.age, 0.375);
        assert_eq!(timing.progress, 0.75);
        assert!(particle_timing(&instance, 3.25, 0.75, 0.5).is_none());
        instance.is_looped = true;
        let timing = particle_timing(&instance, 4.125, 0.75, 0.5).unwrap();
        assert_eq!(timing.spawn_time, 3.75);
        assert_eq!(timing.progress, 0.75);
        assert!(particle_timing(&instance, 3.625, 0.75, 0.5).is_none());
    }

    #[test]
    fn looping_particle_includes_its_property_endpoint_but_nonlooping_particle_expires() {
        let mut instance = ParticleEffectInstance {
            instance_id: 0,
            effect_id: 0,
            corners: [[0.0; 2]; 4],
            spawned_at: 0.0,
            duration: 2.0,
            is_looped: true,
        };
        assert_eq!(
            particle_timing(&instance, 0.5, 0.125, 0.125)
                .unwrap()
                .progress,
            1.0
        );
        assert!(particle_timing(&instance, 0.501, 0.125, 0.125).is_none());
        instance.is_looped = false;
        assert!(particle_timing(&instance, 0.5, 0.125, 0.125).is_none());
        assert!(particle_timing(&instance, -1.0, 0.0, 1.0).is_none());
    }

    #[test]
    fn particle_size_rotation_and_pivot_are_applied_before_effect_mapping() {
        let quad = [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        let actual = particle_corners(quad, 0.2, -0.3, 0.4, 0.1, std::f64::consts::FRAC_PI_2);
        let expected = [[0.3, -0.7], [0.1, -0.7], [0.1, 0.1], [0.3, 0.1]];
        for (a, e) in actual.into_iter().zip(expected) {
            for axis in 0..2 {
                assert!((a[axis] - e[axis]).abs() < 1e-12);
            }
        }
    }

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
        let trace = crate::watch_diagnostics::WatchTrace::new(Default::default()).unwrap();
        assert_eq!(
            render_instances_traced(
                &assets,
                &[(7, "one".into())].into(),
                &instances,
                1.0,
                Some(&trace),
                None
            )
            .unwrap(),
            draws
        );
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
        let mut looped = instances[0].clone();
        looped.is_looped = true;
        assert_eq!(
            render_instances(&assets, &[(7, "one".into())].into(), &[looped], 3.0).unwrap(),
            draws
        );
    }
}
