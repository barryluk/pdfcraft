//! Blend Like Ink and stamp rotation: wrapper forms around a comment's appearance that change how
//! it is drawn.

use super::create::{bounds, finite, normalize, num_array};
use super::lookup::{annot_dict, annot_ref};
use super::modify::{touch, unlocked};
use super::{AnnotError, Dict, Document, Meta, ObjRef, Object, Stream};
/// The normal appearance stream (`/AP /N`) of an annotation dictionary, when it is one stream.
pub(crate) fn normal_appearance(doc: &Document, d: &Dict) -> Option<ObjRef> {
    let ap = d.get(b"AP").map(|a| doc.resolve(a))?;
    let n = ap.as_dict()?.reference(b"N")?;
    matches!(&*doc.get(n), Object::Stream(_)).then_some(n)
}

/// The appearance a Blend Like Ink wrapper draws, when `ap` is one of ours.
pub(crate) fn ink_original(doc: &Document, ap: ObjRef) -> Option<ObjRef> {
    let obj = doc.get(ap);
    let d = obj.as_dict()?;
    if !matches!(d.get(b"PCInk"), Some(Object::Bool(true))) {
        return None;
    }
    let res = doc.resolve(d.get(b"Resources")?);
    let xo = doc.resolve(res.as_dict()?.get(b"XObject")?);
    xo.as_dict()?.reference(b"PCOrig")
}

/// Whether the comment `d` is drawn with Blend Like Ink.
pub fn ink_blended(doc: &Document, d: &Dict) -> bool {
    normal_appearance(doc, d).is_some_and(|ap| ink_original(doc, ap).is_some())
}

/// Blend Like Ink: draw a comment's appearance (a stamp's, typically) in the Multiply blend mode
/// (ISO 32000-2 §11.3.5), so its white parts drop out and its colour darkens what is under it,
/// like a seal pressed over printed text. It stays a comment. On wraps the appearance in a form
/// that draws it with `/BM /Multiply`; off puts the original appearance back.
pub fn set_ink_blend(doc: &mut Document, page: usize, index: usize, on: bool) -> Result<(), AnnotError> {
    let (_, r) = annot_ref(doc, page, index)?;
    unlocked(doc, r)?;
    let d = annot_dict(doc, r);
    let current = normal_appearance(doc, &d).ok_or_else(|| AnnotError::Invalid("this comment has no drawing to blend".into()))?;
    let original = ink_original(doc, current);
    let new_n = match (on, original) {
        (true, Some(_)) => return Err(AnnotError::Invalid("the comment is already blended like ink".into())),
        (false, None) => return Err(AnnotError::Invalid("the comment is not blended like ink".into())),
        (false, Some(orig)) => orig,
        (true, None) => ink_wrap(doc, current)?,
    };
    set_normal_appearance(doc, r, new_n)
}

/// Point the annotation `r`'s normal appearance (`/AP /N`) at `n`, keeping the rest of `/AP`.
fn set_normal_appearance(doc: &mut Document, r: ObjRef, n: ObjRef) -> Result<(), AnnotError> {
    let d = annot_dict(doc, r);
    let mut ap = d.get(b"AP").map(|a| doc.resolve(a)).and_then(|a| a.as_dict().cloned()).unwrap_or_default();
    ap.set(b"N".to_vec(), Object::Ref(n));
    match d.reference(b"AP") {
        Some(apr) => doc.set(apr, Object::Dict(ap)),
        None => doc.update_dict(r, |d| d.set(b"AP".to_vec(), Object::Dict(ap)))?,
    }
    Ok(())
}

/// The box a form XObject draws in: its `/BBox` under its `/Matrix` (the box `Do` fills).
fn form_bounds(doc: &Document, form: ObjRef) -> Option<[f64; 4]> {
    let obj = doc.get(form);
    let d = obj.as_dict()?;
    let nums = |k: &[u8]| -> Option<Vec<f64>> {
        let o = doc.resolve(d.get(k)?);
        o.as_array()?.iter().map(|x| doc.resolve(x).as_f64()).collect()
    };
    let b = nums(b"BBox").filter(|b| b.len() == 4 && b.iter().all(|v| v.is_finite()))?;
    let m = nums(b"Matrix").filter(|m| m.len() == 6 && m.iter().all(|v| v.is_finite())).unwrap_or_else(|| vec![1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    bounds(
        [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])].into_iter().map(|(x, y)| [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]),
    )
}

/// A form that draws the appearance `current` in the Multiply blend mode (Blend Like Ink).
fn ink_wrap(doc: &mut Document, current: ObjRef) -> Result<ObjRef, AnnotError> {
    // `Do` applies the original's /Matrix, so the wrapper's box is the original's box under it.
    let [x0, y0, x1, y1] =
        form_bounds(doc, current).ok_or_else(|| AnnotError::Invalid("this comment's drawing has no usable bounding box".into()))?;
    let mut gs = Dict::new();
    gs.set(b"Type".to_vec(), Object::name("ExtGState"));
    gs.set(b"BM".to_vec(), Object::name("Multiply"));
    let mut gss = Dict::new();
    gss.set(b"PCInk".to_vec(), Object::Dict(gs));
    let mut xo = Dict::new();
    xo.set(b"PCOrig".to_vec(), Object::Ref(current));
    let mut res = Dict::new();
    res.set(b"ExtGState".to_vec(), Object::Dict(gss));
    res.set(b"XObject".to_vec(), Object::Dict(xo));
    let mut w = Dict::new();
    w.set(b"Type".to_vec(), Object::name("XObject"));
    w.set(b"Subtype".to_vec(), Object::name("Form"));
    w.set(b"BBox".to_vec(), num_array(&[x0, y0, x1, y1]));
    w.set(b"Resources".to_vec(), Object::Dict(res));
    w.set(b"PCInk".to_vec(), Object::Bool(true));
    Ok(doc.add(Object::Stream(Stream::flate(w, b"q /PCInk gs /PCOrig Do Q"))))
}

/// The unturned appearance a rotation wrapper draws, when `ap` is one of ours (`/PCRotOf`).
pub(crate) fn rotation_original(doc: &Document, ap: ObjRef) -> Option<ObjRef> {
    doc.get(ap).as_dict()?.reference(b"PCRotOf")
}

/// The appearance stream a comment's own drawing is in: its normal appearance, seen through
/// a Blend Like Ink wrapper and a rotation wrapper.
pub(crate) fn drawing_appearance(doc: &Document, d: &Dict) -> Option<ObjRef> {
    let n = normal_appearance(doc, d)?;
    let n = ink_original(doc, n).unwrap_or(n);
    Some(rotation_original(doc, n).unwrap_or(n))
}

/// A stamp's rotation (`/PCAngle`, degrees counterclockwise in user space) and the size it has
/// unturned (`/PCRotSize`, width and height): `None` for an unturned stamp.
pub fn stamp_rotation(doc: &Document, d: &Dict) -> Option<(f64, [f64; 2])> {
    let angle = d.get(b"PCAngle").and_then(|a| doc.resolve(a).as_f64()).filter(|a| a.is_finite() && *a != 0.0)?;
    let size = d.get(b"PCRotSize").map(|s| doc.resolve(s)).and_then(|s| s.as_array().cloned())?;
    let size: Vec<f64> = size.iter().filter_map(|v| doc.resolve(v).as_f64()).collect();
    match size[..] {
        [w, h] if finite(&[w, h]) && w > 0.0 && h > 0.0 => Some((angle, [w, h])),
        _ => None,
    }
}

pub(crate) fn annot_rect(doc: &Document, d: &Dict) -> Option<[f64; 4]> {
    let r = d.get(b"Rect").map(|o| doc.resolve(o)).and_then(|o| o.as_array().cloned())?;
    let r: Vec<f64> = r.iter().filter_map(|v| doc.resolve(v).as_f64()).collect();
    match r[..] {
        [a, b, c, e] if finite(&[a, b, c, e]) => Some(normalize([a, b, c, e])),
        _ => None,
    }
}

/// `degrees` as an angle in (-180, 180], to a hundredth of a degree.
pub(crate) fn normal_angle(degrees: f64) -> f64 {
    let a = (degrees.rem_euclid(360.0) * 100.0).round() / 100.0;
    let a = if a > 180.0 { a - 360.0 } else { a };
    if a.abs() < 0.005 { 0.0 } else { a }
}

/// Turn a stamp to `degrees` (counterclockwise in user space, about its centre), as the
/// rotation handle does; 0 puts it upright again. Its unturned size stays; `/Rect` becomes the
/// turned stamp's bounding box. The drawing is kept and drawn turned by a wrapper form, so any
/// stamp (another program's artwork too) can turn; Blend Like Ink stays on.
pub fn rotate_stamp(doc: &mut Document, page: usize, index: usize, degrees: f64, meta: &Meta) -> Result<(), AnnotError> {
    if !degrees.is_finite() {
        return Err(AnnotError::Invalid("invalid angle".into()));
    }
    let (_, r) = annot_ref(doc, page, index)?;
    unlocked(doc, r)?;
    if annot_dict(doc, r).name(b"Subtype") != Some(b"Stamp") {
        return Err(AnnotError::Invalid("only stamps can be rotated".into()));
    }
    rotate_ref(doc, r, normal_angle(degrees))?;
    doc.update_dict(r, |d| touch(d, meta))?;
    Ok(())
}

pub(crate) fn rotate_ref(doc: &mut Document, r: ObjRef, angle: f64) -> Result<(), AnnotError> {
    let d = annot_dict(doc, r);
    let rect = annot_rect(doc, &d).ok_or_else(|| AnnotError::Invalid("the stamp has no usable rectangle".into()))?;
    let n = normal_appearance(doc, &d).ok_or_else(|| AnnotError::Invalid("this stamp has no drawing to rotate".into()))?;
    let blended = ink_original(doc, n).is_some();
    let drawn = ink_original(doc, n).unwrap_or(n);
    let wrapped = rotation_original(doc, drawn);
    let base = wrapped.unwrap_or(drawn);
    // The unturned size: kept while turned (resizing scales it), else the rectangle's.
    let [w, h] = match (wrapped, stamp_rotation(doc, &d)) {
        (Some(_), Some((_, size))) => size,
        _ => [rect[2] - rect[0], rect[3] - rect[1]],
    };
    if !(w > 0.0 && h > 0.0) {
        return Err(AnnotError::Invalid("the stamp has no size".into()));
    }
    let (cx, cy) = ((rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0);
    let inner = [cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0];
    let (new_rect, new_drawn) = if angle == 0.0 {
        (inner, base)
    } else {
        let bb = form_bounds(doc, base).filter(|b| b[2] > b[0] && b[3] > b[1]);
        let bb = bb.ok_or_else(|| AnnotError::Invalid("this stamp's drawing has no usable bounding box".into()))?;
        let (sin, cos) = angle.to_radians().sin_cos();
        let outer = bounds(
            [[inner[0], inner[1]], [inner[2], inner[1]], [inner[0], inner[3]], [inner[2], inner[3]]]
                .into_iter()
                .map(|[x, y]| [cx + (x - cx) * cos - (y - cy) * sin, cy + (x - cx) * sin + (y - cy) * cos]),
        )
        .ok_or_else(|| AnnotError::Invalid("invalid angle".into()))?;
        // The drawing's box onto the unturned rectangle, then turned about the centre (§8.4.4:
        // the last `cm` applies first).
        let (sx, sy) = ((inner[2] - inner[0]) / (bb[2] - bb[0]), (inner[3] - inner[1]) / (bb[3] - bb[1]));
        let fit = [sx, 0.0, 0.0, sy, inner[0] - bb[0] * sx, inner[1] - bb[1] * sy];
        let turn = [cos, sin, -sin, cos, cx - cx * cos + cy * sin, cy - cx * sin - cy * cos];
        // Six places: a turn's sine and cosine need more than `n`'s three.
        let cm =
            |m: [f64; 6]| m.iter().map(|v| format!("{v:.6}").trim_end_matches('0').trim_end_matches('.').to_string()).collect::<Vec<_>>().join(" ");
        let content = format!("q {} cm {} cm /PCRotOf Do Q", cm(turn), cm(fit));
        let mut xo = Dict::new();
        xo.set(b"PCRotOf".to_vec(), Object::Ref(base));
        let mut res = Dict::new();
        res.set(b"XObject".to_vec(), Object::Dict(xo));
        let mut f = Dict::new();
        f.set(b"Type".to_vec(), Object::name("XObject"));
        f.set(b"Subtype".to_vec(), Object::name("Form"));
        f.set(b"BBox".to_vec(), num_array(&outer));
        f.set(b"Resources".to_vec(), Object::Dict(res));
        f.set(b"PCRotOf".to_vec(), Object::Ref(base));
        (outer, doc.add(Object::Stream(Stream::flate(f, content.as_bytes()))))
    };
    let new_n = if blended { ink_wrap(doc, new_drawn)? } else { new_drawn };
    doc.update_dict(r, |d| {
        d.set(b"Rect".to_vec(), num_array(&new_rect));
        if angle == 0.0 {
            d.remove(b"PCAngle");
            d.remove(b"PCRotSize");
        } else {
            d.set(b"PCAngle".to_vec(), Object::Real(angle));
            d.set(b"PCRotSize".to_vec(), num_array(&[w, h]));
        }
    })?;
    set_normal_appearance(doc, r, new_n)
}
