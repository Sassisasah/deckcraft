//! An effect's behaviours (`p:anim`, `p:set`, `p:animScale`, `p:animEffect`…) are read with their
//! timing, so the show can play what the file says instead of an approximation of the preset.

use std::sync::Arc;

use deckcraft_geom::Xfrm;
use deckcraft_model::anim::{AnimClass, Animation, BehaviorKind};
use deckcraft_model::{Shape, ShapeId, defaults};

#[test]
fn an_effects_behaviours_are_read_with_their_timing() {
    let mut p = defaults::new_presentation(None);
    let id = ShapeId(p.alloc_id().expect("id"));
    let slide = Arc::make_mut(&mut p.slides[0]);
    slide.shapes = vec![Shape { id, name: "Box".into(), xfrm: Some(Xfrm::new(40.0, 40.0, 200.0, 100.0)), ..Default::default() }];
    slide.animations = vec![
        Animation {
            shape: id,
            class: AnimClass::Entrance,
            effect: "fly".into(),
            option: "fromBottom".into(),
            duration_ms: 500,
            ..Default::default()
        },
        Animation { shape: id, class: AnimClass::Emphasis, effect: "pulse".into(), duration_ms: 500, ..Default::default() },
    ];
    let back = deckcraft_pptx::import(&deckcraft_pptx::export(&p).expect("export")).expect("import");
    let anims = &back.slides[0].animations;
    assert_eq!(anims.len(), 2);
    let fly = &anims[0].behaviors;
    assert!(fly.iter().any(|b| b.kind == BehaviorKind::Set && b.attr == "style.visibility"), "{fly:?}");
    assert!(fly.iter().any(|b| b.kind == BehaviorKind::Anim && b.attr.starts_with("ppt_") && !b.keys.is_empty()), "{fly:?}");
    assert!(fly.iter().all(|b| b.begin_ms + b.dur_ms <= 500), "timed within the effect: {fly:?}");
    assert!(!anims[1].behaviors.is_empty(), "the pulse's behaviours are read too");
}
