//! Skin mode selection and homogeneous quad UV interpolation.
//! Lightweight's projective convention is reference-supported, not a client oracle result.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinRenderMode {
    #[default]
    Standard,
    Lightweight,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinRenderModePreference {
    #[default]
    Default,
    Standard,
    Lightweight,
}

impl SkinRenderModePreference {
    pub fn resolve(self, user_mode: SkinRenderMode) -> SkinRenderMode {
        match self {
            Self::Default => user_mode,
            Self::Standard => SkinRenderMode::Standard,
            Self::Lightweight => SkinRenderMode::Lightweight,
        }
    }
}

/// BL/TL/TR/BR homogeneous weights, derived from the diagonals' intersection.
/// A common scaling of all weights does not affect UV. Invalid/concave diagonal
/// intersections use affine triangles; this fallback is an explicit assumption.
pub(crate) fn projective_weights(q: &[[f64; 2]; 4]) -> [f64; 4] {
    let sub = |a: [f64; 2], b: [f64; 2]| [a[0] - b[0], a[1] - b[1]];
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let a = sub(q[2], q[0]);
    let b = sub(q[3], q[1]);
    let offset = sub(q[1], q[0]);
    let denominator = cross(a, b);
    if !denominator.is_finite() || denominator.abs() < 1e-12 {
        return [1.0; 4];
    }
    let t = cross(offset, b) / denominator;
    let s = cross(offset, a) / denominator;
    if !(0.0..1.0).contains(&t) || !(0.0..1.0).contains(&s) || t == 0.0 || s == 0.0 {
        return [1.0; 4];
    }
    let weights = [1.0 / (1.0 - t), 1.0 / (1.0 - s), 1.0 / t, 1.0 / s];
    let scale = weights.iter().copied().fold(0.0, f64::max);
    weights.map(|w| w / scale)
}

/// Interpolate (U*q,V*q,q) in BL/TL/TR then BL/TR/BR; divide by q.
pub(crate) fn projective_uv(
    q: &[[f64; 2]; 4],
    weights: &[f64; 4],
    p: [f64; 2],
) -> Option<(f64, f64)> {
    // Match GPU vertex/fragment precision for thin-quad coverage as well as UV.
    let q = q.map(|point| point.map(|value| value as f32));
    let weights = weights.map(|value| value as f32);
    let p = p.map(|value| value as f32);
    for indices in [[0, 1, 2], [0, 2, 3]] {
        let [a, b, c] = indices.map(|i| q[i]);
        let ab = [b[0] - a[0], b[1] - a[1]];
        let ac = [c[0] - a[0], c[1] - a[1]];
        let ap = [p[0] - a[0], p[1] - a[1]];
        let det = ab[0] * ac[1] - ab[1] * ac[0];
        if det.abs() < 1e-12 {
            continue;
        }
        let beta = (ap[0] * ac[1] - ap[1] * ac[0]) / det;
        let gamma = (ab[0] * ap[1] - ab[1] * ap[0]) / det;
        let barycentric = [1.0 - beta - gamma, beta, gamma];
        if barycentric.iter().any(|v| *v < -1e-7) {
            continue;
        }
        let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        let weighted = std::array::from_fn::<_, 3, _>(|i| barycentric[i] * weights[indices[i]]);
        let denominator: f32 = weighted.iter().sum();
        if !denominator.is_finite() || denominator.abs() < 1e-12 {
            continue;
        }
        return Some((
            f64::from((0..3).map(|i| weighted[i] * uv[indices[i]][0]).sum::<f32>() / denominator),
            f64::from((0..3).map(|i| weighted[i] * uv[indices[i]][1]).sum::<f32>() / denominator),
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skin_mode_parsing_defaults_and_engine_precedence() {
        for (json, expected) in [
            (serde_json::json!({}), SkinRenderModePreference::Default),
            (
                serde_json::json!({"skin":{"sprites":[]}}),
                SkinRenderModePreference::Default,
            ),
            (
                serde_json::json!({"skin":{"renderMode":"default"}}),
                SkinRenderModePreference::Default,
            ),
            (
                serde_json::json!({"skin":{"renderMode":"standard"}}),
                SkinRenderModePreference::Standard,
            ),
            (
                serde_json::json!({"skin":{"renderMode":"lightweight"}}),
                SkinRenderModePreference::Lightweight,
            ),
        ] {
            let watch: crate::watch::WatchData = serde_json::from_value(json).unwrap();
            assert_eq!(watch.skin_render_mode().unwrap(), expected);
            for user in [SkinRenderMode::Standard, SkinRenderMode::Lightweight] {
                let wanted = match expected {
                    SkinRenderModePreference::Default => user,
                    SkinRenderModePreference::Standard => SkinRenderMode::Standard,
                    SkinRenderModePreference::Lightweight => SkinRenderMode::Lightweight,
                };
                assert_eq!(expected.resolve(user), wanted);
            }
        }
        for malformed in [
            serde_json::json!("future"),
            serde_json::json!("Lightweight"),
            serde_json::json!(null),
            serde_json::json!(1),
            serde_json::json!(true),
        ] {
            assert!(
                serde_json::from_value::<crate::watch::WatchData>(serde_json::json!({
                    "skin":{"renderMode":malformed}
                }))
                .is_err()
            );
        }
    }

    #[test]
    fn next_rush_explicitly_overrides_standard() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine =
            crate::formats::load_engine(&root.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        assert_eq!(
            engine.watch.skin_render_mode().unwrap(),
            SkinRenderModePreference::Lightweight
        );
        assert_eq!(
            engine
                .watch
                .skin_render_mode()
                .unwrap()
                .resolve(SkinRenderMode::Standard),
            SkinRenderMode::Lightweight
        );
    }

    #[test]
    fn projective_quad_corners_parallelogram_clipping_and_degenerate_fallback() {
        let quad = [[-2.0, -1.0], [-1.0, 2.0], [3.0, 3.0], [4.0, -2.0]];
        let weights = projective_weights(&quad);
        for (i, wanted) in [
            (0, (0.0, 0.0)),
            (1, (0.0, 1.0)),
            (2, (1.0, 1.0)),
            (3, (1.0, 0.0)),
        ] {
            let actual = projective_uv(&quad, &weights, quad[i]).unwrap();
            assert!((actual.0 - wanted.0).abs() < 1e-6 && (actual.1 - wanted.1).abs() < 1e-6);
        }
        assert!(projective_uv(&quad, &weights, [20.0, 20.0]).is_none());
        let square = [[-1.0, -1.0], [-1.0, 1.0], [1.0, 1.0], [1.0, -1.0]];
        let uv = projective_uv(&square, &projective_weights(&square), [0.2, -0.4]).unwrap();
        assert!((uv.0 - 0.6).abs() < 1e-6 && (uv.1 - 0.3).abs() < 1e-6);
        let line = [[0.0, 0.0]; 4];
        assert_eq!(projective_weights(&line), [1.0; 4]);
        assert!(projective_uv(&line, &[1.0; 4], [0.0, 0.0]).is_none());
    }
}
