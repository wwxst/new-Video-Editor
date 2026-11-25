//! Built-in effect manifests (docs/03 §4, docs/09 §1). Single source of truth for validation,
//! the inspector, and MCP discovery. Each id here has a matching WGSL pass in `shaders/`.

use arc_core::effects::{EffectManifest, ParamSpec, ParamType, StaticCatalog};
use std::sync::{Arc, OnceLock};

fn p(key: &str, range: [f64; 2], default: f64, unit: Option<&str>) -> ParamSpec {
    ParamSpec {
        key: key.into(),
        ty: ParamType::Float,
        range,
        default,
        unit: unit.map(Into::into),
    }
}

fn m(id: &str, display: &str, category: &str, params: Vec<ParamSpec>) -> EffectManifest {
    EffectManifest {
        id: id.into(),
        version: 1,
        display: display.into(),
        category: vec![category.into()],
        params,
    }
}

pub fn manifests() -> Vec<EffectManifest> {
    vec![
        m(
            "core.color.correct",
            "Color Correction",
            "color",
            vec![
                p("exposure", [-5.0, 5.0], 0.0, Some("stops")),
                p("contrast", [0.2, 2.0], 1.0, None),
                p("saturation", [0.0, 2.0], 1.0, None),
                p("temperature", [-100.0, 100.0], 0.0, None),
            ],
        ),
        m(
            "core.color.lut",
            "LUT",
            "color",
            vec![p("mix", [0.0, 1.0], 1.0, None)],
        ),
        m(
            "core.blur.gaussian",
            "Gaussian Blur",
            "blur",
            vec![p("radius", [0.0, 200.0], 8.0, Some("px"))],
        ),
        m(
            "core.key.chroma",
            "Chroma Key",
            "key",
            vec![
                p("key_r", [0.0, 1.0], 0.0, None),
                p("key_g", [0.0, 1.0], 1.0, None),
                p("key_b", [0.0, 1.0], 0.0, None),
                p("tolerance", [0.0, 1.0], 0.3, None),
                p("softness", [0.0, 1.0], 0.1, None),
                p("spill", [0.0, 1.0], 0.5, None),
            ],
        ),
        m(
            "core.stylize.sharpen",
            "Sharpen",
            "stylize",
            vec![
                p("amount", [0.0, 5.0], 0.5, None),
                p("radius", [0.5, 10.0], 1.0, Some("px")),
            ],
        ),
        m(
            "core.stylize.vignette",
            "Vignette",
            "stylize",
            vec![
                p("amount", [0.0, 1.0], 0.5, None),
                p("midpoint", [0.0, 1.0], 0.5, None),
                p("roundness", [0.0, 1.0], 1.0, None),
            ],
        ),
    ]
}

/// The shared built-in catalog.
pub fn catalog() -> Arc<StaticCatalog> {
    static CAT: OnceLock<Arc<StaticCatalog>> = OnceLock::new();
    CAT.get_or_init(|| Arc::new(StaticCatalog::new(manifests())))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arc_core::EffectCatalog;

    #[test]
    fn manifests_are_consistent() {
        let c = catalog();
        assert_eq!(c.all().len(), 6);
        for m in c.all() {
            for p in &m.params {
                assert!(
                    p.range[0] <= p.default && p.default <= p.range[1],
                    "{}.{}",
                    m.id,
                    p.key
                );
            }
        }
    }
}
