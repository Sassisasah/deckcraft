//! PowerPoint writes "Auto-reverse" on the behaviour nodes (`p:cBhvr/p:cTn autoRev="1"`), not on
//! the effect node: an effect whose timed behaviours all reverse plays forward and back.

use std::io::{Cursor, Read, Write};
use std::sync::Arc;

use deckcraft_geom::Xfrm;
use deckcraft_model::anim::{AnimClass, Animation};
use deckcraft_model::{Shape, ShapeId, defaults};

fn pulse_pptx(effect: &str) -> Vec<u8> {
    let mut p = defaults::new_presentation(None);
    let id = ShapeId(p.alloc_id().expect("id"));
    let slide = Arc::make_mut(&mut p.slides[0]);
    slide.shapes = vec![Shape { id, name: "Box".into(), xfrm: Some(Xfrm::new(40.0, 40.0, 200.0, 100.0)), ..Default::default() }];
    slide.animations = vec![Animation { shape: id, class: AnimClass::Emphasis, effect: effect.into(), duration_ms: 500, ..Default::default() }];
    deckcraft_pptx::export(&p).expect("export")
}

/// Put `autoRev="1"` on every timed behaviour node of the slide.
fn reverse_behaviours(src: &[u8]) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(Cursor::new(src)).expect("zip");
    let mut out = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut changed = 0;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).expect("entry");
        let name = f.name().to_string();
        let mut data = vec![];
        f.read_to_end(&mut data).expect("read");
        if name == "ppt/slides/slide1.xml" {
            let text = String::from_utf8(data).expect("utf8");
            let mut parts = text.split("<p:cBhvr").map(str::to_string).collect::<Vec<_>>();
            for part in parts.iter_mut().skip(1) {
                if let Some(at) = part.find("<p:cTn ").map(|i| i + "<p:cTn ".len())
                    && part[at..].split('>').next().is_some_and(|attrs| attrs.contains("dur=\"") && !attrs.contains("dur=\"1\""))
                {
                    part.insert_str(at, "autoRev=\"1\" ");
                    changed += 1;
                }
            }
            data = parts.join("<p:cBhvr").into_bytes();
        }
        out.start_file(&name, opts).expect("start");
        out.write_all(&data).expect("write");
    }
    assert!(changed > 0, "no timed behaviour written");
    out.finish().expect("finish").into_inner()
}

#[test]
fn auto_reverse_on_every_timed_behaviour_reverses_the_effect() {
    let src = pulse_pptx("pulse");
    let plain = deckcraft_pptx::import(&src).expect("import");
    assert!(!plain.slides[0].animations[0].auto_reverse);
    let p = deckcraft_pptx::import(&reverse_behaviours(&src)).expect("import");
    assert!(p.slides[0].animations[0].auto_reverse, "the pulse grows and shrinks back");
}
