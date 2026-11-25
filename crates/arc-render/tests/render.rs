//! GPU compositor tests (docs/03 §9 determinism, §4 effects, docs/08 §6 goldens).

use arc_core::model::{Transform, Vec2};
use arc_render::plan::EffectPlan;
use arc_render::renderer::color_correct_reference;
use arc_render::{DrawItem, DrawLayer, Image, Renderer};
use std::collections::BTreeMap;

fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    (0..w * h)
        .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
        .collect()
}

fn gradient(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::new();
    for y in 0..h {
        for x in 0..w {
            v.extend_from_slice(&[
                (x * 255 / (w - 1)) as u8,
                (y * 255 / (h - 1)) as u8,
                ((x + y) % 256) as u8,
                255,
            ]);
        }
    }
    v
}

fn px(buf: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
}

fn item<'a>(
    img: Image<'a>,
    t: Transform,
    opacity: f64,
    additive: bool,
    fx: &'a [EffectPlan],
) -> DrawItem<'a> {
    DrawItem {
        image: img,
        transform: t,
        opacity,
        additive,
        effects: fx,
    }
}

fn renderer() -> Renderer {
    Renderer::new_headless().expect("a GPU or software adapter is required for render tests")
}

#[test]
fn passthrough_is_faithful_and_deterministic() {
    let mut r = renderer();
    eprintln!("adapter: {}", r.adapter_info());
    let (w, h) = (256, 144);
    let src = gradient(w, h);
    let layers = [DrawLayer {
        opacity: 1.0,
        items: vec![item(
            Image {
                width: w,
                height: h,
                rgba: &src,
            },
            Transform::default(),
            1.0,
            false,
            &[],
        )],
    }];
    let (a, rep) = r
        .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
        .expect("render");
    assert!(rep.unsupported_effects.is_empty());
    let (b, _) = r
        .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
        .expect("render 2");
    assert_eq!(a, b, "same machine, same inputs -> bit-exact (docs/03 §9)");
    let max_diff = a
        .iter()
        .zip(&src)
        .map(|(x, y)| (*x as i32 - *y as i32).abs())
        .max()
        .unwrap_or(0);
    assert!(
        max_diff <= 1,
        "sRGB -> linear RGBA16F -> sRGB round trip within 1 LSB, got {max_diff}"
    );
}

#[test]
fn color_correct_matches_cpu_reference() {
    let mut r = renderer();
    let (w, h) = (64, 64);
    let colors = [
        [200u8, 120, 40],
        [30, 90, 200],
        [128, 128, 128],
        [250, 250, 250],
        [5, 10, 15],
    ];
    let params: BTreeMap<String, f64> = [
        ("exposure", 0.5),
        ("contrast", 1.2),
        ("saturation", 0.7),
        ("temperature", 30.0),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect();
    let fx = [EffectPlan {
        effect: "core.color.correct".into(),
        params: params.clone(),
    }];
    for c in colors {
        let src = solid(w, h, c);
        let layers = [DrawLayer {
            opacity: 1.0,
            items: vec![item(
                Image {
                    width: w,
                    height: h,
                    rgba: &src,
                },
                Transform::default(),
                1.0,
                false,
                &fx,
            )],
        }];
        let (out, _) = r
            .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
            .expect("render");
        let got = px(&out, w, 32, 32);
        let want = color_correct_reference(c, &params);
        for i in 0..3 {
            assert!(
                (got[i] as i32 - want[i] as i32).abs() <= 2,
                "{c:?}: gpu {got:?} vs cpu {want:?}"
            );
        }
    }
}

#[test]
fn transform_scale_and_position() {
    let mut r = renderer();
    let (w, h) = (200, 100);
    let src = solid(64, 32, [255, 0, 0]);
    let t = Transform {
        scale: Vec2 { x: 0.5, y: 0.5 },
        position: Vec2 { x: 0.25, y: 0.0 },
        ..Transform::default()
    };
    let layers = [DrawLayer {
        opacity: 1.0,
        items: vec![item(
            Image {
                width: 64,
                height: 32,
                rgba: &src,
            },
            t,
            1.0,
            false,
            &[],
        )],
    }];
    let (out, _) = r
        .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
        .expect("render");
    // fit = 100/32 -> 200x100, scaled 0.5 -> 100x50, centered at (150, 50)
    assert_eq!(px(&out, w, 150, 50)[..3], [255, 0, 0]);
    assert_eq!(px(&out, w, 105, 30)[..3], [255, 0, 0]);
    assert_eq!(
        px(&out, w, 95, 50)[..3],
        [0, 0, 0],
        "left of the scaled image is background"
    );
    assert_eq!(
        px(&out, w, 150, 10)[..3],
        [0, 0, 0],
        "above the scaled image is background"
    );
}

#[test]
fn layer_opacity_and_cross_dissolve_mix_in_linear_light() {
    let mut r = renderer();
    let (w, h) = (32, 32);
    let white = solid(w, h, [255, 255, 255]);
    let black = solid(w, h, [0, 0, 0]);
    // white layer at 50% over black: linear 0.5 -> sRGB 188
    let layers = [DrawLayer {
        opacity: 0.5,
        items: vec![item(
            Image {
                width: w,
                height: h,
                rgba: &white,
            },
            Transform::default(),
            1.0,
            false,
            &[],
        )],
    }];
    let (out, _) = r
        .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
        .expect("render");
    let v = px(&out, w, 16, 16)[0];
    assert!(
        (187..=189).contains(&v),
        "50% white over black in linear light ~ 188, got {v}"
    );
    // cross dissolve at w = 0.25: 0.75 black + 0.25 white (additive) -> linear 0.25 -> sRGB 137
    let layers = [DrawLayer {
        opacity: 1.0,
        items: vec![
            item(
                Image {
                    width: w,
                    height: h,
                    rgba: &black,
                },
                Transform::default(),
                0.75,
                true,
                &[],
            ),
            item(
                Image {
                    width: w,
                    height: h,
                    rgba: &white,
                },
                Transform::default(),
                0.25,
                true,
                &[],
            ),
        ],
    }];
    let (out, _) = r
        .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
        .expect("render");
    let v = px(&out, w, 16, 16)[0];
    assert!(
        (136..=138).contains(&v),
        "dissolve weights sum to 1 in linear light, got {v}"
    );
}

#[test]
fn every_builtin_effect_runs_or_is_reported() {
    let mut r = renderer();
    let (w, h) = (64, 36);
    let src = gradient(w, h);
    for m in arc_render::builtin::manifests() {
        let fx = [EffectPlan {
            effect: m.id.clone(),
            params: m
                .params
                .iter()
                .map(|p| (p.key.clone(), p.default))
                .collect(),
        }];
        let layers = [DrawLayer {
            opacity: 1.0,
            items: vec![item(
                Image {
                    width: w,
                    height: h,
                    rgba: &src,
                },
                Transform::default(),
                1.0,
                false,
                &fx,
            )],
        }];
        let (out, rep) = r
            .render(w, h, [0.0, 0.0, 0.0, 1.0], &layers)
            .expect("render");
        assert_eq!(out.len(), (w * h * 4) as usize);
        let supported = Renderer::supported_effects().contains(&m.id.as_str());
        assert_eq!(rep.unsupported_effects.is_empty(), supported, "{}", m.id);
    }
}
