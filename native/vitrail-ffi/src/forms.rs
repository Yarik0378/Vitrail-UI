//! `<select>`: the closed box is plain CSS (the chosen option is `:checked`, see Blitz's default.css); open, the library
//! lays a `<vitrail-popup>` of `<vitrail-option>`s over the page, which pages style like any element. Mouse, keys and the
//! gamepad (`nav.rs`) drive it; choosing fires `input` and `change` on the select, as in a browser.
use crate::{Vitrail, js_event};
use blitz_dom::{Attribute, BaseDocument, LocalName, NodeId, QualName, ns};

/// The open list: its select, its element, and per row the row's element and its option (rows of optgroup labels
/// are not kept); `current` is the highlighted row.
pub(crate) struct Popup {
    pub select: NodeId,
    pub root: NodeId,
    pub rows: Vec<(NodeId, NodeId)>,
    pub current: usize,
}

fn tag(doc: &BaseDocument, id: NodeId) -> String {
    doc.get_node(id).and_then(|n| n.element_data()).map(|e| e.name.local.to_string()).unwrap_or_default()
}

fn has(doc: &BaseDocument, id: NodeId, attr: &str) -> bool {
    doc.get_node(id).and_then(|n| n.attr(LocalName::from(attr))).is_some()
}

/// A select's options in order (directly inside it or inside its optgroups).
pub(crate) fn options(doc: &BaseDocument, select: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let Some(node) = doc.get_node(select) else { return out };
    for &c in &node.children {
        match tag(doc, c).as_str() {
            "option" => out.push(c),
            "optgroup" => for &o in &doc.get_node(c).map(|n| n.children.clone()).unwrap_or_default() { if tag(doc, o) == "option" { out.push(o); } },
            _ => {}
        }
    }
    out
}

/// An option can't be chosen when it or its optgroup is disabled.
fn disabled(doc: &BaseDocument, option: NodeId) -> bool {
    has(doc, option, "disabled") || doc.get_node(option).and_then(|n| n.parent).is_some_and(|p| tag(doc, p) == "optgroup" && has(doc, p, "disabled"))
}

/// The select (or option) holding `id`, if it is inside one and the select is enabled.
pub(crate) fn select_of(doc: &BaseDocument, id: NodeId) -> Option<NodeId> {
    let mut c = Some(id);
    while let Some(n) = c {
        if tag(doc, n) == "select" { return (!has(doc, n, "disabled")).then_some(n); }
        c = doc.get_node(n)?.parent;
    }
    None
}

/// Every select has exactly one chosen option: kept when there is one, else the last with `selected`, else the first
/// enabled one (the HTML rules). Runs after the document changed (`ui.check_selects`).
pub(crate) fn sync(ui: &mut Vitrail) {
    if !ui.check_selects { return; }
    ui.check_selects = false;
    let selects = ui.doc.query_selector_all("select").unwrap_or_default();
    for s in selects {
        let opts = options(&ui.doc, s);
        let checked: Vec<NodeId> = opts.iter().copied().filter(|&o| ui.doc.is_checked(o)).collect();
        if checked.len() == 1 { continue; }
        let pick = checked.first().copied()
            .or_else(|| opts.iter().rev().copied().find(|&o| has(&ui.doc, o, "selected")))
            .or_else(|| opts.iter().copied().find(|&o| !disabled(&ui.doc, o)));
        for &o in &opts { ui.doc.set_option_checked(o, Some(o) == pick); }
        ui.dirty = true;
    }
}

/// The chosen option of a select.
pub(crate) fn chosen(doc: &BaseDocument, select: NodeId) -> Option<NodeId> {
    options(doc, select).into_iter().find(|&o| doc.is_checked(o))
}

/// An option's value: its `value`, else its text.
pub(crate) fn value_of(doc: &BaseDocument, option: NodeId) -> String {
    doc.get_node(option).map(|n| n.attr(LocalName::from("value")).map(|v| v.to_string()).unwrap_or_else(|| label(doc, option))).unwrap_or_default()
}

fn label(doc: &BaseDocument, option: NodeId) -> String {
    doc.get_node(option).map(|n| n.text_content().split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// Makes `option` the select's chosen one; with `user`, fires `input` and `change` when that changed something.
pub(crate) fn choose(ui: &mut Vitrail, select: NodeId, option: NodeId, user: bool) {
    let before = chosen(&ui.doc, select);
    for o in options(&ui.doc, select) { ui.doc.set_option_checked(o, o == option); }
    ui.dirty = true;
    if user && before != Some(option) {
        js_event(ui, "input", select, 0.0, 0.0, -1);
        js_event(ui, "change", select, 0.0, 0.0, -1);
    }
}

fn el(name: &str, attrs: &[(&str, String)]) -> (QualName, Vec<Attribute>) {
    (QualName::new(None, ns!(html), LocalName::from(name)),
        attrs.iter().map(|(k, v)| Attribute { name: QualName::new(None, ns!(), LocalName::from(*k)), value: v.clone() }).collect())
}

/// Opens the list under the select (above it when there is more room there), the chosen row highlighted.
pub(crate) fn open(ui: &mut Vitrail, select: NodeId) {
    close(ui);
    sync(ui);
    ui.doc.as_mut().resolve(ui.time);
    let Some(r) = ui.doc.get_client_bounding_rect(select) else { return };
    let view_h = ui.height as f64 / ui.scale.max(0.01) as f64;
    let (below, above) = (view_h - (r.y + r.height) - 4.0, r.y - 4.0);
    let place = if below >= 160.0 || below >= above {
        format!("left:{}px;top:{}px;min-width:{}px;max-height:{}px", r.x, r.y + r.height + 1.0, r.width, below.max(40.0))
    } else {
        format!("left:{}px;bottom:{}px;min-width:{}px;max-height:{}px", r.x, view_h - r.y + 1.0, r.width, above.max(40.0))
    };
    let chosen_now = chosen(&ui.doc, select);
    // The rows, from the select's children: optgroup labels, then options.
    let mut plan: Vec<(Option<NodeId>, String, bool)> = Vec::new();
    let kids = ui.doc.get_node(select).map(|n| n.children.clone()).unwrap_or_default();
    for c in kids {
        match tag(&ui.doc, c).as_str() {
            "option" => plan.push((Some(c), label(&ui.doc, c), disabled(&ui.doc, c))),
            "optgroup" => {
                let l = ui.doc.get_node(c).and_then(|n| n.attr(LocalName::from("label"))).unwrap_or("").to_string();
                plan.push((None, l, false));
                for o in ui.doc.get_node(c).map(|n| n.children.clone()).unwrap_or_default() {
                    if tag(&ui.doc, o) == "option" { plan.push((Some(o), label(&ui.doc, o), disabled(&ui.doc, o))); }
                }
            }
            _ => {}
        }
    }
    let mut m = ui.doc.mutate();
    let (name, attrs) = el("vitrail-popup", &[("style", place)]);
    let root = m.create_element(name, attrs);
    let mut rows = Vec::new();
    let mut current = 0;
    for (opt, text, off) in plan {
        let mut a: Vec<(&str, String)> = Vec::new();
        let tagname = if let Some(o) = opt {
            if Some(o) == chosen_now { a.push(("checked", String::new())); a.push(("current", String::new())); }
            if off { a.push(("disabled", String::new())); }
            "vitrail-option"
        } else { "vitrail-optgroup" };
        let (name, attrs) = el(tagname, &a);
        let row = m.create_element(name, attrs);
        let t = m.create_text_node(&text);
        m.append_children(row, &[t]);
        m.append_children(root, &[row]);
        if let Some(o) = opt {
            if Some(o) == chosen_now { current = rows.len(); }
            rows.push((row, o));
        }
    }
    drop(m);
    let host = ui.doc.query_selector("body").ok().flatten().unwrap_or_else(|| ui.doc.root_element().id);
    ui.doc.mutate().append_children(host, &[root]);
    ui.popup = Some(Popup { select, root, rows, current });
    ui.dirty = true;
    reveal(ui);
}

/// Closes the list (if open).
pub(crate) fn close(ui: &mut Vitrail) {
    let Some(p) = ui.popup.take() else { return };
    let mut m = ui.doc.mutate();
    m.remove_and_drop_node(p.root);
    drop(m);
    ui.dirty = true;
}

/// Highlights row `i` (the mouse over it, or the keys).
pub(crate) fn highlight(ui: &mut Vitrail, i: usize) {
    let Some(p) = ui.popup.as_mut() else { return };
    if i >= p.rows.len() || i == p.current { return; }
    let (old, new) = (p.rows[p.current].0, p.rows[i].0);
    p.current = i;
    let mut m = ui.doc.mutate();
    m.clear_attribute(old, QualName::new(None, ns!(), LocalName::from("current")));
    m.set_attribute(new, QualName::new(None, ns!(), LocalName::from("current")), "");
    drop(m);
    ui.dirty = true;
    reveal(ui);
}

/// Scrolls the list so its highlighted row shows.
fn reveal(ui: &mut Vitrail) {
    let Some(p) = ui.popup.as_ref() else { return };
    let (root, row) = (p.root, p.rows.get(p.current).map(|r| r.0));
    let Some(row) = row else { return };
    ui.doc.as_mut().resolve(ui.time);
    let (Some(b), Some(r)) = (ui.doc.get_client_bounding_rect(root), ui.doc.get_client_bounding_rect(row)) else { return };
    let dy = if r.y < b.y { r.y - b.y - 2.0 } else if r.y + r.height > b.y + b.height { r.y + r.height - (b.y + b.height) + 2.0 } else { return };
    ui.doc.scroll_by(root, 0.0, dy, blitz_dom::ScrollBehavior::Instant);
    ui.dirty = true;
}

/// Moves the highlight by `step` rows (skipping disabled ones; ±1000 goes to the ends).
pub(crate) fn step(ui: &mut Vitrail, step: i32) {
    let Some(p) = ui.popup.as_ref() else { return };
    let n = p.rows.len() as i32;
    if n == 0 { return; }
    let rows: Vec<NodeId> = p.rows.iter().map(|r| r.1).collect();
    let mut i = (p.current as i32 + step).clamp(0, n - 1);
    let dir = if step > 0 { 1 } else { -1 };
    // Off a disabled row: on in the same direction, else back.
    while (0..n).contains(&i) && disabled(&ui.doc, rows[i as usize]) { i += dir; }
    if !(0..n).contains(&i) {
        i = (p.current as i32 + step).clamp(0, n - 1);
        while (0..n).contains(&i) && disabled(&ui.doc, rows[i as usize]) { i -= dir; }
    }
    if (0..n).contains(&i) { highlight(ui, i as usize); }
}

/// Chooses the highlighted row and closes the list.
pub(crate) fn pick(ui: &mut Vitrail) {
    let Some(p) = ui.popup.as_ref() else { return };
    let (select, option) = (p.select, p.rows.get(p.current).map(|r| r.1));
    close(ui);
    if let Some(o) = option.filter(|&o| !disabled(&ui.doc, o)) { choose(ui, select, o, true); }
}

/// The popup row holding `id`, if any.
pub(crate) fn row_at(ui: &Vitrail, id: NodeId) -> Option<usize> {
    let p = ui.popup.as_ref()?;
    let mut c = Some(id);
    while let Some(n) = c {
        if let Some(i) = p.rows.iter().position(|r| r.0 == n) { return Some(i); }
        if n == p.root { return None; }
        c = ui.doc.get_node(n)?.parent;
    }
    None
}

/// Whether `id` is inside the open list.
pub(crate) fn in_popup(ui: &Vitrail, id: NodeId) -> bool {
    let Some(p) = ui.popup.as_ref() else { return false };
    crate::is_inside(&ui.doc, id, p.root)
}

/// A key for a select: the open list takes the arrows, Home/End, PageUp/PageDown, Enter or Space (choose), Escape and
/// Tab (close); a focused closed select opens on Enter, Space, Alt+arrows or F4 and steps its choice on the arrows.
/// Returns whether the key was taken.
pub(crate) fn key(ui: &mut Vitrail, key: &str, alt: bool) -> bool {
    if ui.popup.is_some() {
        match key {
            "ArrowDown" => step(ui, 1),
            "ArrowUp" => step(ui, -1),
            "PageDown" => step(ui, 10),
            "PageUp" => step(ui, -10),
            "Home" => step(ui, -1000),
            "End" => step(ui, 1000),
            "Enter" | " " => pick(ui),
            "Escape" | "Tab" => close(ui),
            _ => {}
        }
        return true;
    }
    let root = ui.doc.root_element().id;
    let Some(f) = ui.doc.get_focussed_node_id().filter(|&f| f != root) else { return false };
    if tag(&ui.doc, f) != "select" || has(&ui.doc, f, "disabled") { return false; }
    match key {
        "Enter" | " " | "F4" => { open(ui, f); true }
        "ArrowDown" | "ArrowUp" if alt => { open(ui, f); true }
        "ArrowDown" | "ArrowUp" | "Home" | "End" => {
            let opts: Vec<NodeId> = options(&ui.doc, f).into_iter().filter(|&o| !disabled(&ui.doc, o)).collect();
            if opts.is_empty() { return true; }
            let at = chosen(&ui.doc, f).and_then(|c| opts.iter().position(|&o| o == c)).unwrap_or(0) as i32;
            let to = match key { "ArrowDown" => at + 1, "ArrowUp" => at - 1, "Home" => 0, _ => opts.len() as i32 - 1 }.clamp(0, opts.len() as i32 - 1);
            choose(ui, f, opts[to as usize], true);
            true
        }
        _ => false,
    }
}
