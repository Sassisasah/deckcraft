//! Slide transitions (`p:transition`) and animations (`p:timing`).

use deckcraft_model::anim::{
    AnimClass, AnimStart, Animation, Behavior, BehaviorKind, BuildRole, Keyframe, TextBuild, Transition, build_filled, paragraph_build_roles,
};
use deckcraft_model::{Shape, TextBody, find_shape};

use super::shapes::IdCtx;
use super::{Imp, Part, dml};
use crate::maps;
use crate::xml::El;

fn ms_attr(e: &El, name: &str) -> Option<u32> {
    let v = e.attr(name)?.trim();
    if v == "indefinite" {
        return None;
    }
    v.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 86_400_000.0) as u32)
}

pub fn transition(imp: &mut Imp, part: &Part, t: &El) -> Option<Transition> {
    let effect = t.elements().find(|e| !matches!(e.local(), "sndAc" | "extLst"));
    let mut tr = Transition { advance_on_click: t.bool("advClick").unwrap_or(true), advance_after_ms: ms_attr(t, "advTm"), ..Default::default() };
    match effect.and_then(maps::transition_from_xml) {
        Some((k, o)) => {
            tr.kind = k;
            tr.option = o;
        }
        None => match effect {
            Some(e) => {
                tr.kind = e.local().to_string();
                tr.raw = Some(part.keep(t));
            }
            None => tr.kind = "none".into(),
        },
    }
    let default_dur = deckcraft_model::anim::TRANSITIONS.iter().find(|x| x.0 == tr.kind).map(|x| x.3).unwrap_or(1000);
    // `spd` defaults to "fast" in the schema; PowerPoint omits both attributes for 0.5 s.
    tr.duration_ms = ms_attr(t, "dur").unwrap_or(match t.attr("spd") {
        Some("med") => 750,
        Some("slow") => default_dur.max(1000),
        _ => 500,
    });
    if let Some(snd) = t.path(&["sndAc", "stSnd", "snd"]) {
        tr.sound = dml::media_ref(imp, part, snd);
        tr.sound_loop = t.path(&["sndAc", "stSnd"]).and_then(|s| s.attr("loop")).is_some_and(|v| v == "1" || v == "true");
    }
    tr.stop_sound = t.path(&["sndAc", "endSnd"]).is_some();
    if tr.kind == "none" && tr.advance_on_click && tr.advance_after_ms.is_none() && tr.sound.is_none() && !tr.stop_sound {
        return None;
    }
    Some(tr)
}

struct Found<'a> {
    ctn: &'a El,
    trigger: Option<u32>,
}

fn collect<'a>(e: &'a El, trigger: Option<u32>, out: &mut Vec<Found<'a>>, depth: usize) {
    if depth > 64 || out.len() > 10_000 {
        return;
    }
    for c in e.elements() {
        if c.is("cTn") && c.has_attr("presetClass") {
            out.push(Found { ctn: c, trigger });
            continue;
        }
        let mut trig = trigger;
        if c.is("seq")
            && let Some(ctn) = c.child("cTn")
            && ctn.attr("nodeType") == Some("interactiveSeq")
        {
            trig = ctn.path(&["stCondLst", "cond", "tgtEl", "spTgt"]).and_then(|s| s.u32("spid"));
        }
        collect(c, trig, out, depth + 1);
    }
}

fn max_dur(e: &El, depth: usize) -> Option<u32> {
    if depth > 64 {
        return None;
    }
    let mut best: Option<u32> = None;
    for c in e.elements() {
        if c.is("cTn")
            && let Some(d) = ms_attr(c, "dur")
            && d > 1
        {
            best = Some(best.map_or(d, |b: u32| b.max(d)));
        }
        if let Some(d) = max_dur(c, depth + 1) {
            best = Some(best.map_or(d, |b| b.max(d)));
        }
    }
    best
}

/// Most behaviours kept per effect.
const MAX_BEHAVIORS: usize = 256;

/// The behaviours under an effect node, timed from the effect's start. Empty when any of them
/// can't be played as written (open-ended timing, media commands, unknown elements).
fn behaviors(effect: &El) -> Vec<Behavior> {
    let mut out = vec![];
    let ok = effect.child("childTnLst").is_some_and(|l| collect_behaviors(l, 0, &mut out, 0));
    if ok && out.len() <= MAX_BEHAVIORS { out } else { vec![] }
}

fn collect_behaviors(list: &El, begin: u32, out: &mut Vec<Behavior>, depth: usize) -> bool {
    if depth > 8 || out.len() > MAX_BEHAVIORS {
        return false;
    }
    for e in list.elements() {
        let kind = match e.local() {
            "anim" => BehaviorKind::Anim,
            "set" => BehaviorKind::Set,
            "animScale" => BehaviorKind::Scale,
            "animRot" => BehaviorKind::Rotate,
            "animClr" => BehaviorKind::Color,
            "animMotion" => BehaviorKind::Motion,
            "animEffect" => BehaviorKind::Effect,
            "par" | "seq" => {
                // A nested group: its children start after its own delay.
                let Some(ctn) = e.child("cTn") else { return false };
                let Some(d) = delay_of(ctn) else { return false };
                match ctn.child("childTnLst") {
                    Some(l) if collect_behaviors(l, begin.saturating_add(d), out, depth + 1) => continue,
                    Some(_) => return false,
                    None => continue,
                }
            }
            _ => return false,
        };
        let Some(b) = behavior(e, kind, begin) else { return false };
        out.push(b);
    }
    true
}

/// A node's start delay in ms (`None` when it waits for an event).
fn delay_of(ctn: &El) -> Option<u32> {
    match ctn.path(&["stCondLst", "cond"]) {
        Some(c) if c.attr("delay").is_some() => ms_attr(c, "delay"),
        Some(c) if c.attr("evt").is_some() => None,
        _ => Some(0),
    }
}

/// `p:val`/`p:to`/`p:from`/`p:by` as text: `strVal`, `fltVal`, `intVal` or `boolVal`.
fn val_text(v: Option<&El>) -> Option<String> {
    let v = v?;
    v.elements().find_map(|c| match c.local() {
        "strVal" | "fltVal" | "intVal" | "boolVal" => c.attr("val").map(str::to_string),
        _ => None,
    })
}

fn pct_pair(e: Option<&El>) -> Option<(f64, f64)> {
    let e = e?;
    let f = |k: &str| e.f64(k).filter(|v| v.is_finite()).map(|v| (v / 100_000.0).clamp(-1000.0, 1000.0));
    Some((f("x").unwrap_or(1.0), f("y").unwrap_or(1.0)))
}

fn behavior(e: &El, kind: BehaviorKind, begin: u32) -> Option<Behavior> {
    let bh = e.child("cBhvr")?;
    let ctn = bh.child("cTn")?;
    let dur = match ctn.attr("dur") {
        None => 0,
        Some(_) => ms_attr(ctn, "dur")?,
    };
    let repeat = match ctn.attr("repeatCount") {
        None => 1.0,
        Some("indefinite") => return None,
        Some(v) => v.parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0).map(|v| v / 1000.0).unwrap_or(1.0),
    };
    let frac = |k: &str| ctn.f64(k).filter(|v| v.is_finite()).map(|v| (v / 100_000.0).clamp(0.0, 1.0)).unwrap_or(0.0);
    let tm_filter = ctn
        .attr("tmFilter")
        .map(|f| {
            f.split(';')
                .filter_map(|p| {
                    let (a, b) = p.split_once(',')?;
                    Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
                })
                .filter(|(a, b)| a.is_finite() && b.is_finite())
                .take(64)
                .collect()
        })
        .unwrap_or_default();
    let mut b = Behavior {
        kind,
        attr: bh.path(&["attrNameLst", "attrName"]).map(|a| a.text().trim().to_string()).unwrap_or_default(),
        begin_ms: begin.saturating_add(delay_of(ctn)?),
        dur_ms: dur,
        repeat,
        auto_reverse: ctn.bool("autoRev").unwrap_or(false),
        accel: frac("accel"),
        decel: frac("decel"),
        hold: ctn.attr("fill") == Some("hold"),
        tm_filter,
        ..Default::default()
    };
    match kind {
        BehaviorKind::Anim => {
            b.discrete = e.attr("calcmode") == Some("discrete") || e.attr("valueType") == Some("str");
            b.from = e.attr("from").map(str::to_string);
            b.to = e.attr("to").map(str::to_string);
            b.by = e.attr("by").map(str::to_string);
            if let Some(l) = e.child("tavLst") {
                for t in l.children_named("tav").take(256) {
                    let time = match t.attr("tm") {
                        None => 0.0,
                        Some(v) => v.parse::<f64>().ok().filter(|v| v.is_finite())? / 100_000.0,
                    };
                    b.keys.push(Keyframe {
                        time: time.clamp(0.0, 1.0),
                        value: val_text(t.child("val")).unwrap_or_default(),
                        fmla: t.attr("fmla").map(str::to_string),
                    });
                }
            }
        }
        BehaviorKind::Set => {
            b.to = val_text(e.child("to"));
            // A colour set (`p:to/p:clrVal`): the colour at once.
            b.color_to = e.path(&["to", "clrVal"]).and_then(dml::color);
        }
        BehaviorKind::Scale => {
            b.scale_from = pct_pair(e.child("from"));
            b.scale_to = pct_pair(e.child("to"));
            b.scale_by = pct_pair(e.child("by"));
        }
        BehaviorKind::Rotate => {
            let deg = |k: &str| e.f64(k).filter(|v| v.is_finite()).map(|v| v / 60_000.0);
            b.degrees = deg("by").or_else(|| Some(deg("to")? - deg("from").unwrap_or(0.0)));
        }
        BehaviorKind::Color => {
            b.hsl_space = e.attr("clrSpc") == Some("hsl");
            b.ccw = e.attr("dir") == Some("ccw");
            b.color_to = e.child("to").and_then(dml::color);
            b.hsl_by = e.path(&["by", "hsl"]).map(|h| {
                let f = |k: &str, d: f64| h.f64(k).filter(|v| v.is_finite()).map_or(0.0, |v| v / d);
                (f("h", 60_000.0), f("s", 100_000.0), f("l", 100_000.0))
            });
        }
        BehaviorKind::Motion => b.path = e.attr("path").map(str::to_string),
        BehaviorKind::Effect => {
            b.filter = e.attr("filter").map(str::to_string);
            b.out = e.attr("transition") == Some("out");
        }
    }
    Some(b)
}

/// Flatten the timing tree into the Animation Pane list.
pub fn animations(imp: &mut Imp, part: &Part, ctx: &IdCtx, timing: &El, shapes: &[Shape]) -> Vec<Animation> {
    let _ = (imp, part);
    let Some(tn) = timing.child("tnLst") else { return vec![] };
    let mut found = vec![];
    collect(tn, None, &mut found, 0);
    // Paragraph builds (`bldP build="p"`) by (spid, grpId).
    let by_para: Vec<(u32, String)> = timing
        .child("bldLst")
        .map(|b| {
            b.children_named("bldP")
                .filter(|p| p.attr("build") == Some("p"))
                .filter_map(|p| Some((p.u32("spid")?, p.attr("grpId").unwrap_or("0").to_string())))
                .collect()
        })
        .unwrap_or_default();
    let mut out: Vec<(Animation, u32, String, bool)> = vec![];
    for f in found {
        let c = f.ctn;
        let class = AnimClass::from_xml(c.attr("presetClass").unwrap_or("entr"));
        let preset = c.u32("presetID").unwrap_or(0);
        let sub = c.u32("presetSubtype").unwrap_or(0);
        let Some(tgt) = c.find("spTgt") else { continue };
        let Some(file_id) = tgt.u32("spid") else { continue };
        let Some(shape) = ctx.lookup(file_id) else { continue };
        let paragraph = tgt.path(&["txEl", "pRg"]).and_then(|p| p.u32("st"));
        let effect = maps::effect_for_preset(class, preset).unwrap_or_else(|| maps::fallback_effect(class));
        // PowerPoint writes Float Down as its own preset (47), not as Float with a direction.
        let (effect, option) = match (class, preset) {
            (AnimClass::Entrance, 47) => ("float", "d".to_string()),
            (AnimClass::Exit, 47) => ("floatOut", "d".to_string()),
            _ => (effect, maps::option_from_subtype(effect, sub)),
        };
        let start = match c.attr("nodeType") {
            Some("clickEffect") => AnimStart::OnClick,
            Some("afterEffect") => AnimStart::AfterPrevious,
            _ => AnimStart::WithPrevious,
        };
        let mut a = Animation {
            shape,
            class,
            effect: effect.to_string(),
            option,
            start,
            duration_ms: max_dur(c, 0).unwrap_or(0),
            delay_ms: c.path(&["stCondLst", "cond"]).and_then(|d| ms_attr(d, "delay")).unwrap_or(0),
            paragraph,
            trigger: f.trigger.and_then(|t| ctx.lookup(t)),
            preset_id: Some(preset),
            preset_subtype: Some(sub),
            auto_reverse: c.bool("autoRev").unwrap_or(false),
            // PowerPoint's "Rewind when done playing" is `fill="remove"` on the effect node.
            rewind: c.attr("fill") == Some("remove"),
            smooth_start: c.f64("accel").map(|v| v / 100_000.0).unwrap_or(0.0),
            smooth_end: c.f64("decel").map(|v| v / 100_000.0).unwrap_or(0.0),
            bounce_end: c.f64("bounceEnd").map(|v| (v / 100_000.0).clamp(0.0, 1.0)).unwrap_or(0.0),
            ..Default::default()
        };
        a.repeat = match c.attr("repeatCount") {
            Some("indefinite") => u32::MAX,
            Some(v) => v.parse::<f64>().ok().filter(|v| v.is_finite()).map(|v| (v / 1000.0).round().clamp(1.0, 10_000.0) as u32).unwrap_or(1),
            None => 1,
        };
        if let Some(m) = c.find("animMotion") {
            a.path = m.attr("path").map(String::from);
        }
        if let Some(r) = c.find("animRot") {
            a.amount = r.f64("by").map(|v| v / 60_000.0);
            if a.effect == "spin" && a.amount.is_some_and(|v| v < 0.0) {
                a.option = "counterClockwise".into();
            }
        } else if let Some(s) = c.find("animScale").and_then(|s| s.child("by")) {
            a.amount = s.f64("x").map(|v| v / 100_000.0);
        }
        if let Some(clr) = c.find("animClr").and_then(|x| x.child("to")) {
            a.color = dml::color(clr);
        }
        a.behaviors = behaviors(c);
        let grp = c.attr("grpId").unwrap_or("0").to_string();
        let build = paragraph.is_some() && by_para.iter().any(|(s, g)| *s == file_id && *g == grp);
        out.push((a, file_id, grp, build));
    }
    // A paragraph build's effects, one per paragraph, become one by-paragraph animation only when
    // the timeline plays that back exactly: PowerPoint lets each paragraph keep its own start and
    // effect, and other shapes' effects can sit between them (#844).
    let same_build =
        |x: &(Animation, u32, String, bool), y: &(Animation, u32, String, bool)| x.3 && y.3 && x.1 == y.1 && x.2 == y.2 && x.0.trigger == y.0.trigger;
    let mut anims = Vec::with_capacity(out.len());
    for run in out.chunk_by(same_build) {
        let effects: Vec<&Animation> = run.iter().map(|(a, ..)| a).collect();
        let build = run.first().is_some_and(|r| r.3);
        let text = effects.first().and_then(|a| find_shape(shapes, a.shape)).and_then(|s| s.text.as_ref());
        match (effects.first(), text) {
            (Some(first), Some(text)) if build && plays_as_build(&effects, text) => {
                anims.push(Animation { text_build: TextBuild::ByParagraph, paragraph: None, ..(*first).clone() });
            }
            _ => anims.extend(effects.into_iter().cloned()),
        }
    }
    anims
}

/// Are `effects` (paragraph effects of one shape, in file order) what a by-paragraph build of the
/// first one plays: one effect per paragraph the build animates, in paragraph order, later top
/// paragraphs on click (after previous when the build doesn't start on click), deeper ones with
/// their parent, all with the same settings?
fn plays_as_build(effects: &[&Animation], text: &TextBody) -> bool {
    let roles = paragraph_build_roles(text);
    let expected = roles.iter().enumerate().filter(|(i, r)| match r {
        BuildRole::Top => true,
        BuildRole::Member(_) => text.paragraphs.get(*i).is_some_and(build_filled),
        BuildRole::Unowned => false,
    });
    let Some(first) = effects.first() else { return false };
    let settings = |a: &Animation| Animation { start: AnimStart::OnClick, paragraph: None, ..a.clone() };
    let base = settings(first);
    let mut matched = 0;
    for (k, (i, role)) in expected.enumerate() {
        let Some(a) = effects.get(k) else { return false };
        let start_ok = match role {
            BuildRole::Top if k == 0 => true,
            BuildRole::Top if first.start == AnimStart::OnClick => a.start == AnimStart::OnClick,
            BuildRole::Top => a.start == AnimStart::AfterPrevious,
            _ => a.start == AnimStart::WithPrevious,
        };
        if a.paragraph != u32::try_from(i).ok() || !start_ok || settings(a) != base {
            return false;
        }
        matched = k + 1;
    }
    matched == effects.len()
}
