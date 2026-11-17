//! Moving the focus with a gamepad (or any direction keys the game maps): spatial navigation between the focusable
//! elements, as on consoles and in Coherent. The focus ring shows (`:focus-visible`), the focused element is scrolled
//! into view, and "activate" presses it as the mouse would (a click, a select opening, a checkbox ticking).
use crate::{Vitrail, forms};
use blitz_dom::{LocalName, NodeId};

#[derive(Clone, Copy)]
struct R { x0: f64, y0: f64, x1: f64, y1: f64 }

fn rect(ui: &Vitrail, id: NodeId) -> Option<R> {
    let r = ui.doc.get_client_bounding_rect(id)?;
    (r.width > 0.0 && r.height > 0.0).then(|| R { x0: r.x, y0: r.y, x1: r.x + r.width, y1: r.y + r.height })
}

/// The elements a gamepad can reach: focusable ones (not `tabindex="-1"`) that are laid out and visible.
fn candidates(ui: &Vitrail) -> Vec<(NodeId, R)> {
    let found = ui.doc.query_selector_all("button, a[href], input, select, textarea, [tabindex], [contenteditable]").unwrap_or_default();
    found.into_iter().filter(|&id| {
        let Some(n) = ui.doc.get_node(id) else { return false };
        crate::focusable(&ui.doc, id) && n.attr(LocalName::from("tabindex")) != Some("-1") && !forms::in_popup(ui, id)
            && n.attr(LocalName::from("type")) != Some("hidden")
    }).filter_map(|id| rect(ui, id).map(|r| (id, r))).collect()
}

/// The best candidate from `from` in direction `dir` (0 up, 1 right, 2 down, 3 left): it must lie that way, and the
/// nearest wins, the distance across the direction counting double (so the one in line beats a closer one aside).
fn best(from: R, dir: u32, all: &[(NodeId, R)], skip: NodeId) -> Option<NodeId> {
    let (cx, cy) = ((from.x0 + from.x1) / 2.0, (from.y0 + from.y1) / 2.0);
    let mut out: Option<(f64, NodeId)> = None;
    for &(id, r) in all {
        if id == skip { continue; }
        let (rx, ry) = ((r.x0 + r.x1) / 2.0, (r.y0 + r.y1) / 2.0);
        let gap = |a0: f64, a1: f64, b0: f64, b1: f64| if b1 < a0 { a0 - b1 } else if b0 > a1 { b0 - a1 } else { 0.0 };
        let (ahead, along, across, off) = match dir {
            0 => (ry < cy - 1.0 && r.y0 < from.y0, (from.y0 - r.y1).max(0.0), gap(from.x0, from.x1, r.x0, r.x1), (rx - cx).abs()),
            2 => (ry > cy + 1.0 && r.y1 > from.y1, (r.y0 - from.y1).max(0.0), gap(from.x0, from.x1, r.x0, r.x1), (rx - cx).abs()),
            3 => (rx < cx - 1.0 && r.x0 < from.x0, (from.x0 - r.x1).max(0.0), gap(from.y0, from.y1, r.y0, r.y1), (ry - cy).abs()),
            _ => (rx > cx + 1.0 && r.x1 > from.x1, (r.x0 - from.x1).max(0.0), gap(from.y0, from.y1, r.y0, r.y1), (ry - cy).abs()),
        };
        if !ahead { continue; }
        let score = along + across * 2.0 + off * 0.01;
        if out.is_none_or(|(s, _)| score < s) { out = Some((score, id)); }
    }
    out.map(|o| o.1)
}

/// Scrolls the containers holding `id` (and the page) so it shows whole, with a little room.
pub(crate) fn reveal(ui: &mut Vitrail, id: NodeId) {
    ui.doc.as_mut().resolve(ui.time);
    let mut c = ui.doc.get_node(id).and_then(|n| n.parent);
    while let Some(p) = c {
        let Some(node) = ui.doc.get_node(p) else { break };
        let next = node.parent;
        let scrolls = node.scroll_height() > node.client_height() + 0.5 || node.scroll_width() > node.client_width() + 0.5;
        if scrolls {
            if let (Some(b), Some(r)) = (ui.doc.get_client_bounding_rect(p), ui.doc.get_client_bounding_rect(id)) {
                let l = node.final_layout();
                let (top, left) = (b.y + l.border.top as f64, b.x + l.border.left as f64);
                let (h, w) = (node.client_height() as f64, node.client_width() as f64);
                let room = 8.0;
                let dy = if r.y < top { r.y - top - room } else if r.y + r.height > top + h { (r.y + r.height - (top + h) + room).min(r.y - top) } else { 0.0 };
                let dx = if r.x < left { r.x - left - room } else if r.x + r.width > left + w { (r.x + r.width - (left + w) + room).min(r.x - left) } else { 0.0 };
                if dx != 0.0 || dy != 0.0 { ui.doc.scroll_by(p, dx, dy, blitz_dom::ScrollBehavior::Instant); ui.dirty = true; ui.doc.as_mut().resolve(ui.time); }
            }
        }
        c = next;
    }
}

/// Focuses `id` from the gamepad or keys: the ring shows, and it scrolls into view.
fn focus(ui: &mut Vitrail, id: NodeId) {
    crate::set_focus(ui, Some(id));
    ui.doc.set_focus_ring(id, true);
    ui.dirty = true;
    reveal(ui, id);
}

/// One step in a direction. With the select list open, up and down move its highlight (left and right do nothing).
/// With nothing focused the first reachable element (top, then left) takes the focus. Returns whether it moved.
pub(crate) fn navigate(ui: &mut Vitrail, dir: u32) -> bool {
    if ui.popup.is_some() {
        match dir { 0 => forms::step(ui, -1), 2 => forms::step(ui, 1), _ => {} }
        return true;
    }
    forms::sync(ui);
    ui.doc.as_mut().resolve(ui.time);
    let all = candidates(ui);
    let root = ui.doc.root_element().id;
    let focused = ui.doc.get_focussed_node_id().filter(|&f| f != root);
    let from = focused.and_then(|f| rect(ui, f).map(|r| (f, r)));
    let to = match from {
        Some((f, r)) => best(r, dir, &all, f),
        None => all.iter().min_by(|a, b| (a.1.y0, a.1.x0).partial_cmp(&(b.1.y0, b.1.x0)).unwrap()).map(|a| a.0),
    };
    match to { Some(t) => { focus(ui, t); true } None => false }
}

/// The element the next activation presses: the focused one.
pub(crate) fn focused(ui: &Vitrail) -> Option<NodeId> {
    let root = ui.doc.root_element().id;
    ui.doc.get_focussed_node_id().filter(|&f| f != root)
}
