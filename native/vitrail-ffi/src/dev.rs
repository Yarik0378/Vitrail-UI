//! Developer tools: what an inspector needs to show a page's elements, their boxes and computed styles, to pick one
//! under the pointer and to edit its style live. Node ids cross the C API as the node's id + 1 (0 is "none").
use crate::{Vitrail, text, write_c};
use blitz_dom::{BaseDocument, LocalName, NodeId};
use std::ffi::c_char;
use std::fmt::Write;

fn nid(n: u64) -> Option<NodeId> { n.checked_sub(1).map(NodeId::from_u64) }

/// Writes `s` into `buf` (NUL-terminated): 1, or minus the bytes needed.
unsafe fn out(s: &str, buf: *mut c_char, cap: usize) -> i32 {
    if s.len() >= cap || buf.is_null() { return -((s.len() + 1) as i32); }
    unsafe { write_c(buf, cap, s) };
    1
}

/// An element's label as an inspector shows it: tag#id.class.class.
fn label(doc: &BaseDocument, id: NodeId) -> String {
    let Some(node) = doc.get_node(id) else { return String::new() };
    let Some(el) = node.element_data() else { return String::new() };
    let mut s = el.name.local.to_string();
    if let Some(i) = node.attr(LocalName::from("id")) { let _ = write!(s, "#{i}"); }
    if let Some(c) = node.attr(LocalName::from("class")) { for c in c.split_whitespace() { let _ = write!(s, ".{c}"); } }
    s
}

/// The document as lines "id\tdepth\tkind\tlabel" in tree order: kind e for an element (label tag#id.class), t for a
/// text node (its first 60 characters, whitespace-only ones left out).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_dev_tree(ui: *mut Vitrail, buf: *mut c_char, cap: usize) -> i32 {
    crate::guard("vitrail_dev_tree", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let doc: &BaseDocument = &ui.doc;
    let mut s = String::new();
    let mut stack = vec![(doc.root_element().id, 0u32)];
    while let Some((id, depth)) = stack.pop() {
        let Some(node) = doc.get_node(id) else { continue };
        if node.is_element() {
            let _ = writeln!(s, "{}\t{depth}\te\t{}", id.as_u64() + 1, label(doc, id));
            for &c in node.children.iter().rev() { stack.push((c, depth + 1)); }
        } else if let Some(t) = node.text_data() {
            let t: String = t.content.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() { let _ = writeln!(s, "{}\t{depth}\tt\t{}", id.as_u64() + 1, t.chars().take(60).collect::<String>()); }
        }
    }
    unsafe { out(&s, buf, cap) }
    })
}

/// One element: lines "rect\tx y w h" (its border box, device px from the page's top-left), "margin", "border",
/// "padding" (top right bottom left, CSS px), "attr\tname\tvalue" for each attribute, then "style\tname\tvalue" for every
/// computed CSS property.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_dev_node(ui: *mut Vitrail, node: u64, buf: *mut c_char, cap: usize) -> i32 {
    crate::guard("vitrail_dev_node", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let Some(id) = nid(node) else { return 0 };
    ui.doc.as_mut().resolve(ui.time);
    let doc: &BaseDocument = &ui.doc;
    let Some(n) = doc.get_node(id) else { return 0 };
    let s_ = ui.scale as f64;
    let mut s = String::new();
    if let Some(r) = doc.get_client_bounding_rect(id) { let _ = writeln!(s, "rect\t{} {} {} {}", r.x * s_, r.y * s_, r.width * s_, r.height * s_); }
    let l = n.final_layout();
    let _ = writeln!(s, "margin\t{} {} {} {}", l.margin.top, l.margin.right, l.margin.bottom, l.margin.left);
    let _ = writeln!(s, "border\t{} {} {} {}", l.border.top, l.border.right, l.border.bottom, l.border.left);
    let _ = writeln!(s, "padding\t{} {} {} {}", l.padding.top, l.padding.right, l.padding.bottom, l.padding.left);
    if let Some(el) = n.element_data() {
        for a in el.attrs().iter() { let _ = writeln!(s, "attr\t{}\t{}", a.name.local, a.value.replace(['\t', '\n'], " ")); }
        for p in blitz_dom::resolved_style_property_names() {
            let v = doc.resolved_style_value(id, p);
            if !v.is_empty() { let _ = writeln!(s, "style\t{p}\t{}", v.replace(['\t', '\n'], " ")); }
        }
    }
    unsafe { out(&s, buf, cap) }
    })
}

/// The element under (x, y) device px (for picking): its id, or 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_dev_pick(ui: *mut Vitrail, x: f32, y: f32) -> u64 {
    crate::guard("vitrail_dev_pick", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let s = ui.scale.max(0.01);
    ui.doc.as_mut().resolve(ui.time);
    crate::doc_of(ui).element_from_point(x / s, y / s).map(|n| n.as_u64() + 1).unwrap_or(0)
    })
}

/// An element's border box: x, y, width, height in device px from the page's top-left. Returns 1 when it has one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_dev_rect(ui: *mut Vitrail, node: u64, rect: *mut f32) -> i32 {
    crate::guard("vitrail_dev_rect", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let Some(id) = nid(node) else { return 0 };
    ui.doc.as_mut().resolve(ui.time);
    let Some(r) = ui.doc.get_client_bounding_rect(id) else { return 0 };
    let s = ui.scale;
    if !rect.is_null() { unsafe { std::ptr::copy_nonoverlapping([r.x as f32 * s, r.y as f32 * s, r.width as f32 * s, r.height as f32 * s].as_ptr(), rect, 4) }; }
    1
    })
}

/// Sets (or with an empty value removes) a CSS property in an element's style attribute, as an inspector edits it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_dev_set_style(ui: *mut Vitrail, node: u64, name: *const c_char, value: *const c_char) -> i32 {
    crate::guard("vitrail_dev_set_style", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let Some(id) = nid(node) else { return 0 };
    let (name, value) = unsafe { (text(name), text(value)) };
    if ui.doc.get_node(id).is_none() { return 0; }
    let mut m = ui.doc.mutate();
    if value.is_empty() { m.remove_style_property(id, &name); } else { m.set_style_property(id, &name, &value); }
    drop(m);
    ui.dirty = true;
    1
    })
}
