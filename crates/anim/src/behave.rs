//! Effects played from the behaviours the file writes: PowerPoint stores each preset as `p:anim`
//! (with formulas over `#ppt_x`, `#ppt_y`, `#ppt_w`, `#ppt_h`), `p:animScale`, `p:animRot`,
//! `p:animClr`, `p:animMotion`, `p:animEffect` and `p:set`, and playing those reproduces it.

use deckcraft_model::anim::{Behavior, BehaviorKind, ColorShift};
use deckcraft_model::{Rgba, Xfrm};

use crate::AnimState;
use crate::easing::ease;
use crate::path::MotionPath;

/// Most nested parentheses and characters a formula may have.
const MAX_DEPTH: usize = 32;
const MAX_LEN: usize = 1024;

/// The values a formula may read: the shape's centre and size as slide fractions as the effect
/// found them (`#ppt_x`…) and as the earlier behaviours have left them so far (`ppt_x`…), and `$`.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Vars {
    pub base: [f64; 4],
    pub cur: [f64; 4],
    pub dollar: f64,
}

/// Evaluate a PowerPoint animation formula (`#ppt_x-sin(pi*$)/3`, `(0.5)`, `ppt_h`…). `None` for
/// text that is not a number or formula (`visible`, `bold`) or does not parse.
pub(crate) fn formula(src: &str, v: &Vars) -> Option<f64> {
    if src.len() > MAX_LEN {
        return None;
    }
    let mut p = Parser { s: src.as_bytes(), i: 0, v, depth: 0 };
    let r = p.sum()?;
    p.ws();
    (p.i == p.s.len() && r.is_finite()).then_some(r)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    v: &'a Vars,
    depth: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn sum(&mut self) -> Option<f64> {
        let mut a = self.product()?;
        loop {
            if self.eat(b'+') {
                a += self.product()?;
            } else if self.eat(b'-') {
                a -= self.product()?;
            } else {
                return Some(a);
            }
        }
    }
    fn product(&mut self) -> Option<f64> {
        let mut a = self.unary()?;
        loop {
            if self.eat(b'*') {
                a *= self.unary()?;
            } else if self.eat(b'/') {
                let b = self.unary()?;
                a = if b == 0.0 { 0.0 } else { a / b };
            } else if self.eat(b'%') {
                let b = self.unary()?;
                a = if b == 0.0 { 0.0 } else { a % b };
            } else {
                return Some(a);
            }
        }
    }
    fn unary(&mut self) -> Option<f64> {
        if self.eat(b'-') {
            return Some(-self.unary()?);
        }
        if self.eat(b'+') {
            return self.unary();
        }
        let b = self.atom()?;
        if self.eat(b'^') {
            let e = self.unary()?;
            return Some(b.powf(e));
        }
        Some(b)
    }
    fn atom(&mut self) -> Option<f64> {
        self.ws();
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return None;
        }
        let r = self.atom_inner();
        self.depth -= 1;
        r
    }
    fn atom_inner(&mut self) -> Option<f64> {
        let c = *self.s.get(self.i)?;
        if c == b'(' {
            self.i += 1;
            let v = self.sum()?;
            return self.eat(b')').then_some(v);
        }
        if c.is_ascii_digit() || c == b'.' {
            let start = self.i;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                self.i += 1;
            }
            // An exponent (`1e-3`), only when digits follow.
            if matches!(self.s.get(self.i), Some(b'e' | b'E')) {
                let mut j = self.i + 1;
                if matches!(self.s.get(j), Some(b'+' | b'-')) {
                    j += 1;
                }
                if self.s.get(j).is_some_and(u8::is_ascii_digit) {
                    self.i = j;
                    while self.s.get(self.i).is_some_and(u8::is_ascii_digit) {
                        self.i += 1;
                    }
                }
            }
            return std::str::from_utf8(&self.s[start..self.i]).ok()?.parse().ok();
        }
        if c == b'$' {
            self.i += 1;
            return Some(self.v.dollar);
        }
        if c == b'#' || c.is_ascii_alphabetic() || c == b'_' {
            let start = self.i;
            self.i += 1;
            while self.s.get(self.i).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_') {
                self.i += 1;
            }
            let raw = std::str::from_utf8(&self.s[start..self.i]).ok()?;
            let vals = if raw.starts_with('#') { self.v.base } else { self.v.cur };
            let name = raw.trim_start_matches('#').to_ascii_lowercase();
            if self.eat(b'(') {
                let mut args = vec![self.sum()?];
                while self.eat(b',') {
                    args.push(self.sum()?);
                    if args.len() > 8 {
                        return None;
                    }
                }
                if !self.eat(b')') {
                    return None;
                }
                return call(&name, &args);
            }
            return match name.as_str() {
                "ppt_x" => Some(vals[0]),
                "ppt_y" => Some(vals[1]),
                "ppt_w" => Some(vals[2]),
                "ppt_h" => Some(vals[3]),
                "ppt_r" | "ppt_c" => Some(0.0),
                "pi" => Some(std::f64::consts::PI),
                "e" => Some(std::f64::consts::E),
                _ => None,
            };
        }
        None
    }
}

fn call(name: &str, a: &[f64]) -> Option<f64> {
    let x = *a.first()?;
    Some(match (name, a.len()) {
        ("abs", 1) => x.abs(),
        ("acos", 1) => x.clamp(-1.0, 1.0).acos(),
        ("asin", 1) => x.clamp(-1.0, 1.0).asin(),
        ("atan", 1) => x.atan(),
        ("ceil", 1) => x.ceil(),
        ("cos", 1) => x.cos(),
        ("cosh", 1) => x.cosh(),
        ("deg", 1) => x.to_degrees(),
        ("exp", 1) => x.exp(),
        ("floor", 1) => x.floor(),
        ("ln", 1) => {
            if x > 0.0 {
                x.ln()
            } else {
                0.0
            }
        }
        ("rad", 1) => x.to_radians(),
        ("sin", 1) => x.sin(),
        ("sinh", 1) => x.sinh(),
        ("sqrt", 1) => x.max(0.0).sqrt(),
        ("tan", 1) => x.tan(),
        ("tanh", 1) => x.tanh(),
        // PowerPoint's `rand(n)`: a fixed draw keeps the show repeatable.
        ("rand", 1) => x * 0.5,
        ("max", n) if n >= 2 => a.iter().copied().fold(f64::MIN, f64::max),
        ("min", n) if n >= 2 => a.iter().copied().fold(f64::MAX, f64::min),
        _ => return None,
    })
}

/// A behaviour's progress `t` seconds after the effect started: `None` before it begins.
fn progress(b: &Behavior, t: f64) -> Option<f64> {
    let local = t - b.begin_ms as f64 / 1000.0;
    if local.is_nan() || local < 0.0 {
        return None;
    }
    let dur = b.dur_ms as f64 / 1000.0;
    if dur <= 0.0 {
        return Some(1.0);
    }
    let reps = if b.repeat.is_finite() && b.repeat > 0.0 { b.repeat.min(1.0e4) } else { 1.0 };
    let period = if b.auto_reverse { 2.0 * dur } else { dur };
    let span = period * reps;
    // A behaviour that ends keeps its last value while the effect lasts (PowerPoint's presets
    // chain behaviours that each start from where the one before stopped); the effect's own end
    // decides what stays.
    let c = if local >= span {
        // Where the last iteration stopped (a fractional repeat stops part way).
        let frac = reps.fract();
        if frac > 0.0 { frac * period } else { period }
    } else {
        local % period
    };
    let raw = if c > dur { 2.0 - c / dur } else { c / dur };
    let p = filter_time(&b.tm_filter, raw.clamp(0.0, 1.0));
    Some(ease(p, b.accel, b.decel, 0.0))
}

/// `tmFilter`: piecewise-linear time → progress.
fn filter_time(pts: &[(f64, f64)], p: f64) -> f64 {
    if pts.len() < 2 {
        return p;
    }
    for w in pts.windows(2) {
        let ((t0, v0), (t1, v1)) = (w[0], w[1]);
        if p <= t1 || std::ptr::eq(&w[1], pts.last().unwrap_or(&w[1])) {
            let span = t1 - t0;
            return if span > 1e-12 { v0 + (v1 - v0) * ((p - t0) / span).clamp(0.0, 1.0) } else { v1 };
        }
    }
    p
}

/// The value of a `p:anim` at progress `p`, given the attribute's current value.
fn anim_value(b: &Behavior, p: f64, cur: f64, v: &Vars) -> Option<f64> {
    let num = |s: &str, d: f64| formula(s, &Vars { dollar: d, ..*v });
    if b.keys.is_empty() {
        let from = b.from.as_deref().and_then(|s| num(s, 0.0)).unwrap_or(cur);
        let to = match (&b.to, &b.by) {
            (Some(t), _) => num(t, 0.0)?,
            (None, Some(by)) => from + num(by, 0.0)?,
            _ => return None,
        };
        return Some(if b.discrete { if p < 1.0 { from } else { to } } else { from + (to - from) * p });
    }
    // The keyframe segment `p` falls in; past the last key, its value.
    let k = b.keys.iter().rposition(|k| k.time <= p).unwrap_or(0);
    let a = &b.keys[k];
    let val = |key: &deckcraft_model::anim::Keyframe| num(&key.value, 0.0);
    let x = match b.keys.get(k + 1) {
        Some(n) if !b.discrete => {
            let (va, vb) = (val(a)?, val(n)?);
            let span = n.time - a.time;
            let f = if span > 1e-12 { ((p - a.time) / span).clamp(0.0, 1.0) } else { 1.0 };
            va + (vb - va) * f
        }
        _ => val(a)?,
    };
    // The key's formula reads the interpolated value as `$`.
    match &a.fmla {
        Some(f) if !f.trim().is_empty() => num(f, x),
        _ => Some(x),
    }
}

fn str_value(b: &Behavior, p: f64) -> Option<String> {
    if b.keys.is_empty() {
        return b.to.clone();
    }
    let k = b.keys.iter().rposition(|k| k.time <= p).unwrap_or(0);
    b.keys.get(k).map(|k| k.value.clone())
}

/// What an `p:animEffect` filter shows at reveal `e` (0 hidden … 1 shown): opacity, clip and a
/// slide-in offset (fractions of the shape's size).
fn filter_state(filter: &str, e: f64, out: bool) -> (f64, Option<[f64; 4]>, (f64, f64)) {
    let e = e.clamp(0.0, 1.0);
    let q = 1.0 - e;
    let (name, arg) = match filter.split_once('(') {
        Some((n, a)) => (n.trim(), a.trim_end_matches(')').trim()),
        None => (filter.trim(), ""),
    };
    let center = |h: bool, v: bool| {
        let f = e / 2.0;
        let (l, r) = if h { (0.5 - f, 0.5 + f) } else { (0.0, 1.0) };
        let (t, b) = if v { (0.5 - f, 0.5 + f) } else { (0.0, 1.0) };
        Some([l, t, r, b])
    };
    match name {
        "fade" | "image" => (e, None, (0.0, 0.0)),
        // PowerPoint names the wipe by the edge it moves away from: Wipe From Bottom is
        // `wipe(down)` and shows the bottom first; Peek From Bottom rises with `wipe(up)`.
        // Leaving, the filter names the way the edge moves: `wipe(down)` clears the shape from
        // the bottom up, the top going last.
        "wipe" if out => (
            1.0,
            Some(match arg {
                "up" => [0.0, q, 1.0, 1.0],
                "left" => [q, 0.0, 1.0, 1.0],
                "right" => [0.0, 0.0, e, 1.0],
                _ => [0.0, 0.0, 1.0, e],
            }),
            (0.0, 0.0),
        ),
        "wipe" => (
            1.0,
            Some(match arg {
                "up" => [0.0, 0.0, 1.0, e],
                "left" => [0.0, 0.0, e, 1.0],
                "right" => [q, 0.0, 1.0, 1.0],
                _ => [0.0, q, 1.0, 1.0],
            }),
            (0.0, 0.0),
        ),
        "slide" => match arg {
            "fromTop" => (1.0, Some([0.0, q, 1.0, 1.0]), (0.0, -q)),
            "fromLeft" => (1.0, Some([q, 0.0, 1.0, 1.0]), (-q, 0.0)),
            "fromRight" => (1.0, Some([0.0, 0.0, e, 1.0]), (q, 0.0)),
            _ => (1.0, Some([0.0, 0.0, 1.0, e]), (0.0, q)),
        },
        "barn" if arg.starts_with("out") => (1.0, if arg.ends_with("Vertical") { center(true, false) } else { center(false, true) }, (0.0, 0.0)),
        "box" | "circle" | "diamond" | "plus" if arg == "out" => {
            let f = if matches!(name, "circle" | "diamond") { (e * std::f64::consts::SQRT_2).min(1.0) } else { e };
            let g = f / 2.0;
            (1.0, Some([0.5 - g, 0.5 - g, 0.5 + g, 0.5 + g]), (0.0, 0.0))
        }
        "strips" => (
            1.0,
            Some(match arg {
                "upRight" => [0.0, q, e, 1.0],
                "downLeft" => [q, 0.0, 1.0, e],
                "downRight" => [0.0, 0.0, e, e],
                _ => [q, q, 1.0, 1.0],
            }),
            (0.0, 0.0),
        ),
        // Patterns a rectangle clip can't draw (bars, blinds, wedges, closing shapes): a fade.
        _ => (e, None, (0.0, 0.0)),
    }
}

/// The state of a shape `t` seconds into an effect described by `behaviors`. `colors` holds each
/// behaviour's resolved target colour.
pub(crate) fn eval(behaviors: &[Behavior], colors: &[Option<Rgba>], t: f64, xf: Xfrm, sw: f64, sh: f64) -> AnimState {
    let b0 = [(xf.x + xf.w / 2.0) / sw, (xf.y + xf.h / 2.0) / sh, xf.w / sw, xf.h / sh];
    let [mut x, mut y, mut w, mut h] = b0;
    let (mut sx, mut sy, mut rot) = (1.0, 1.0, 0.0);
    let (mut mx, mut my) = (0.0, 0.0);
    let mut st = AnimState::default();
    let mut slide_off = (0.0, 0.0);
    for (i, b) in behaviors.iter().enumerate() {
        let Some(p) = progress(b, t) else { continue };
        let attr = b.attr.to_ascii_lowercase();
        let base = Vars { base: b0, cur: [x, y, w, h], dollar: 0.0 };
        match b.kind {
            BehaviorKind::Set if b.color_to.is_some() => {
                let shift = ColorShift { to: colors.get(i).copied().flatten(), t: 1.0, ..Default::default() };
                apply_color(&mut st, &attr, shift);
            }
            BehaviorKind::Set | BehaviorKind::Anim => {
                let numeric = |cur: f64| {
                    if b.kind == BehaviorKind::Set { b.to.as_deref().and_then(|s| formula(s, &base)) } else { anim_value(b, p, cur, &base) }
                };
                match attr.as_str() {
                    "ppt_x" => x = numeric(x).unwrap_or(x),
                    "ppt_y" => y = numeric(y).unwrap_or(y),
                    "ppt_w" => w = numeric(w).unwrap_or(w),
                    "ppt_h" => h = numeric(h).unwrap_or(h),
                    "r" | "style.rotation" => rot = numeric(rot).unwrap_or(rot),
                    "style.opacity" => st.opacity *= numeric(1.0).unwrap_or(1.0).clamp(0.0, 1.0),
                    "style.visibility" => {
                        if let Some(s) = str_value(b, p) {
                            st.visible = s != "hidden";
                        }
                    }
                    "style.fontweight" => {
                        if let Some(s) = str_value(b, p) {
                            st.bold = Some(s == "bold");
                        }
                    }
                    _ => {}
                }
            }
            BehaviorKind::Scale => {
                let from = b.scale_from.unwrap_or((sx, sy));
                let to = match (b.scale_to, b.scale_by) {
                    (Some(t), _) => t,
                    (None, Some(by)) => (from.0 * by.0, from.1 * by.1),
                    _ => from,
                };
                sx = from.0 + (to.0 - from.0) * p;
                sy = from.1 + (to.1 - from.1) * p;
            }
            // Turning in 3D (`3d.view.rotation.y`…) leaves a flat shape as it is.
            BehaviorKind::Rotate if matches!(attr.as_str(), "" | "r" | "style.rotation") => rot += b.degrees.unwrap_or(0.0) * p,
            BehaviorKind::Rotate => {}
            BehaviorKind::Motion => {
                if let Some(path) = &b.path {
                    let (fx, fy) = MotionPath::parse(path).at(p);
                    mx += fx;
                    my += fy;
                }
            }
            BehaviorKind::Color => {
                let shift = ColorShift { to: colors.get(i).copied().flatten(), by: b.hsl_by.unwrap_or_default(), hsl: b.hsl_space, ccw: b.ccw, t: p };
                apply_color(&mut st, &attr, shift);
            }
            BehaviorKind::Effect => {
                let e = if b.out { 1.0 - p } else { p };
                let (o, clip, off) = filter_state(b.filter.as_deref().unwrap_or("fade"), e, b.out);
                st.opacity *= o;
                if let Some(c) = clip {
                    st.clip = Some(match st.clip {
                        Some(a) => [a[0].max(c[0]), a[1].max(c[1]), a[2].min(c[2]), a[3].min(c[3])],
                        None => c,
                    });
                }
                slide_off = (slide_off.0 + off.0, slide_off.1 + off.1);
            }
        }
    }
    let ratio = |v: f64, b: f64| if b.abs() > 1e-12 { v / b } else { 1.0 };
    st.offset_x = (x - b0[0] + mx) * sw + slide_off.0 * xf.w;
    st.offset_y = (y - b0[1] + my) * sh + slide_off.1 * xf.h;
    st.scale_x = ratio(w, b0[2]) * sx;
    st.scale_y = ratio(h, b0[3]) * sy;
    st.rotate = rot;
    st.sanitized()
}

fn apply_color(st: &mut AnimState, attr: &str, shift: ColorShift) {
    match attr {
        "style.color" => st.text_shift = Some(shift),
        "stroke.color" => st.line_shift = Some(shift),
        "fillcolor" | "fill.color" => st.fill_shift = Some(shift),
        _ => {}
    }
}

/// Can these behaviours be played? Every one names something [`eval`] understands, and at least
/// one moves, scales, turns, recolours or reveals the shape.
pub(crate) fn playable(behaviors: &[Behavior]) -> bool {
    !behaviors.is_empty() && behaviors.iter().all(|b| b.kind != BehaviorKind::Motion || b.path.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v() -> Vars {
        Vars { base: [0.5, 0.25, 0.2, 0.1], cur: [0.6, 0.25, 0.2, 0.1], dollar: 0.5 }
    }

    #[test]
    fn formulas_read_the_shape_and_progress() {
        assert_eq!(formula("#ppt_x", &v()), Some(0.5));
        assert_eq!(formula("ppt_x", &v()), Some(0.6), "without # the value so far");
        assert_eq!(formula("ppt_y+0.026", &v()), Some(0.276));
        assert_eq!(formula("(#ppt_y+0.4)", &v()), Some(0.65));
        assert_eq!(formula("0-#ppt_w/2", &v()), Some(-0.1));
        let b = formula("#ppt_y-sin(pi*$)/3", &v()).unwrap();
        assert!((b - (0.25 - 1.0 / 3.0)).abs() < 1e-9);
        assert_eq!(formula("2^3^2", &v()), Some(512.0));
        assert_eq!(formula("max(1,2,3)", &v()), Some(3.0));
        assert_eq!(formula("visible", &v()), None);
        assert_eq!(formula("1e-3", &v()), Some(0.001));
    }

    #[test]
    fn zip_moves_up_from_below_and_settles() {
        // PowerPoint's Zip: from the slide's middle, 0.4 below, zoomed, to the shape's place.
        let set = |attr: &str, to: &str, dur: u32| Behavior {
            kind: BehaviorKind::Set,
            attr: attr.into(),
            dur_ms: dur,
            hold: true,
            to: Some(to.into()),
            ..Default::default()
        };
        let anim = |attr: &str, from: &str, to: &str| Behavior {
            kind: BehaviorKind::Anim,
            attr: attr.into(),
            begin_ms: 770,
            dur_ms: 1230,
            accel: 1.0,
            hold: true,
            from: Some(from.into()),
            to: Some(to.into()),
            ..Default::default()
        };
        let bs = vec![set("ppt_y", "(#ppt_y+0.4)", 770), anim("ppt_y", "(#ppt_y+0.4)", "(#ppt_y)")];
        let xf = Xfrm::new(100.0, 100.0, 200.0, 100.0);
        let early = eval(&bs, &[None, None], 0.1, xf, 1000.0, 500.0);
        assert!((early.offset_y - 200.0).abs() < 1e-6, "{early:?}");
        let done = eval(&bs, &[None, None], 2.5, xf, 1000.0, 500.0);
        assert!(done.offset_y.abs() < 1e-6, "{done:?}");
    }

    #[test]
    fn chained_behaviours_start_where_the_last_one_stopped() {
        // Bounce: a fall, then a hop measured from where the fall ended.
        let key = |t: f64, v: &str| deckcraft_model::anim::Keyframe { time: t, value: v.into(), fmla: None };
        let fall = Behavior {
            kind: BehaviorKind::Anim,
            attr: "ppt_y".into(),
            dur_ms: 500,
            keys: vec![key(0.0, "ppt_y"), key(1.0, "ppt_y+0.3")],
            ..Default::default()
        };
        let hop = Behavior {
            kind: BehaviorKind::Anim,
            attr: "ppt_y".into(),
            begin_ms: 500,
            dur_ms: 500,
            keys: vec![key(0.0, "ppt_y"), key(0.5, "ppt_y-0.1"), key(1.0, "ppt_y")],
            ..Default::default()
        };
        let xf = Xfrm::new(0.0, 0.0, 100.0, 100.0);
        let mid = eval(&[fall.clone(), hop.clone()], &[None, None], 0.75, xf, 1000.0, 1000.0);
        assert!((mid.offset_y - 200.0).abs() < 1e-6, "{mid:?}");
        let end = eval(&[fall, hop], &[None, None], 1.0, xf, 1000.0, 1000.0);
        assert!((end.offset_y - 300.0).abs() < 1e-6, "{end:?}");
    }

    #[test]
    fn time_filters_map_time_to_progress() {
        let f = [(0.0, 0.0), (0.33333, 1.0), (1.0, 1.0)];
        assert!((filter_time(&f, 0.166665) - 0.5).abs() < 1e-6);
        assert_eq!(filter_time(&f, 0.8), 1.0);
    }
}
