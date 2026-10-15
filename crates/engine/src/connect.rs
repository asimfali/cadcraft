//! Connectors: named points of a block where wires attach (a contactor's terminals, a tee's
//! three ends), and the wires attached to them.
//!
//! - A connector is a POINT in a block definition carrying xdata [`CONNECTOR_APP`] with its
//!   name (`1000`). Other programs see a point and keep the data.
//! - A wire (polyline or line) whose end lies on a block reference's connector is attached to
//!   it: xdata [`LINK_APP`] on the wire holds, for each attached end, `1000 start|end`,
//!   `1005` the block reference and `1000` the connector's name.
//! - [`after_command`] keeps the links after every command: an end drawn or dragged onto a
//!   connector attaches, an end taken off one lets go, and when a block reference moves,
//!   turns, scales or mirrors (or its definition changes), the attached ends follow its
//!   connectors. The segment at a following end keeps its direction when it was horizontal or
//!   vertical: the vertex next to the end moves with it.

use std::collections::HashMap;
use std::sync::Arc;

use cadcraft_doc::{Drawing, Entity, EntityKind, EntityStore, Handle, Insert, RawTag, Space};
use cadcraft_geom::Vec2;

/// The xdata application of a connector point in a block definition.
pub const CONNECTOR_APP: &str = "CADCRAFT_CONNECTOR";
/// The xdata application of a wire's links to connectors.
pub const LINK_APP: &str = "CADCRAFT_LINK";
/// Longest connector name kept.
pub const MAX_NAME: usize = 100;
/// Most connectors read from one block definition (hostile files).
const MAX_CONNECTORS: usize = 1000;
/// Most connector positions gathered from one space.
const MAX_INDEX: usize = 200_000;

/// A connector of a block reference, in world coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Connector {
    pub insert: Handle,
    pub name: String,
    pub point: Vec2,
}

/// What a wire end is attached to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub insert: Handle,
    pub name: String,
}

/// The connector name of a point carrying [`CONNECTOR_APP`] xdata.
pub fn connector_name(e: &Entity) -> Option<String> {
    if !matches!(e.kind, EntityKind::Point(_)) {
        return None;
    }
    let body = app_body(&e.common.xdata, CONNECTOR_APP)?;
    Some(body.iter().find(|t| t.code == 1000).map(|t| t.value.clone()).unwrap_or_default())
}

/// The xdata making a point a connector named `name`, replacing any it had.
pub fn set_connector_name(xdata: &mut Vec<RawTag>, name: &str) {
    let name: String = name.trim().chars().take(MAX_NAME).collect();
    replace_app(xdata, CONNECTOR_APP, vec![RawTag { code: 1000, value: name }]);
}

/// Make a connector point a plain point again.
pub fn clear_connector(xdata: &mut Vec<RawTag>) {
    replace_app(xdata, CONNECTOR_APP, Vec::new());
}

/// The tags of application `app` after its `1001` group (up to the next application).
fn app_body<'a>(xdata: &'a [RawTag], app: &str) -> Option<&'a [RawTag]> {
    let start = xdata.iter().position(|t| t.code == 1001 && t.value.eq_ignore_ascii_case(app))?;
    let rest = xdata.get(start + 1..)?;
    let end = rest.iter().position(|t| t.code == 1001).unwrap_or(rest.len());
    rest.get(..end)
}

/// Replace application `app`'s xdata with `body` (none: remove it).
fn replace_app(xdata: &mut Vec<RawTag>, app: &str, body: Vec<RawTag>) {
    if let Some(start) = xdata.iter().position(|t| t.code == 1001 && t.value.eq_ignore_ascii_case(app)) {
        let end = xdata.iter().skip(start + 1).position(|t| t.code == 1001).map_or(xdata.len(), |n| start + 1 + n);
        xdata.drain(start..end);
    }
    if !body.is_empty() {
        xdata.push(RawTag { code: 1001, value: app.to_string() });
        xdata.extend(body);
    }
}

/// The connectors of block `name`, in the block's own coordinates (relative to nothing: the
/// reference's transform places them).
pub fn block_connectors(d: &Drawing, name: &str) -> Vec<(String, Vec2)> {
    let Some(blk) = d.block(name) else { return Vec::new() };
    blk.entities.iter().filter_map(|e| Some((connector_name(e)?, point_of(e)?))).take(MAX_CONNECTORS).collect()
}

fn point_of(e: &Entity) -> Option<Vec2> {
    match &e.kind {
        EntityKind::Point(p) => Some(p.p.xy()).filter(|p| p.is_finite()),
        _ => None,
    }
}

/// Where the connectors `local` (from [`block_connectors`]) of reference `ins` are. A
/// multiple-insert array has none: its cells are not separate objects to attach to.
pub fn place(d: &Drawing, ins: &Insert, local: &[(String, Vec2)]) -> Vec<(String, Vec2)> {
    if local.is_empty() || ins.cols > 1 || ins.rows > 1 {
        return Vec::new();
    }
    let Some(blk) = d.block(&ins.block) else { return Vec::new() };
    let m = ins.transform(blk.base.xy());
    local.iter().map(|(n, p)| (n.clone(), m.apply(*p))).filter(|(_, p)| p.is_finite()).collect()
}

/// The connectors of block reference `e` (none for other objects).
pub fn connectors_of(d: &Drawing, e: &Entity) -> Vec<(String, Vec2)> {
    match &e.kind {
        EntityKind::Insert(ins) => place(d, ins, &block_connectors(d, &ins.block)),
        _ => Vec::new(),
    }
}

/// Every block reference connector in `space`.
pub fn connectors_in(d: &Drawing, space: &Space) -> Vec<Connector> {
    let Some(store) = d.space(space) else { return Vec::new() };
    let mut blocks = BlockCache::default();
    index(d, store, &mut blocks)
}

/// The connector at `p` in `space` (within `tol`), nearest first.
pub fn connector_at(d: &Drawing, space: &Space, p: Vec2, tol: f64) -> Option<Connector> {
    nearest(&connectors_in(d, space), p, tol).cloned()
}

fn nearest(index: &[Connector], p: Vec2, tol: f64) -> Option<&Connector> {
    index.iter().filter(|c| c.point.dist(p) <= tol).min_by(|a, b| a.point.dist(p).total_cmp(&b.point.dist(p)))
}

/// The connectors of each block definition, read once per pass.
#[derive(Default)]
struct BlockCache(HashMap<String, Arc<Vec<(String, Vec2)>>>);

impl BlockCache {
    fn get(&mut self, d: &Drawing, name: &str) -> Arc<Vec<(String, Vec2)>> {
        self.0.entry(name.to_string()).or_insert_with(|| Arc::new(block_connectors(d, name))).clone()
    }
}

fn index(d: &Drawing, store: &EntityStore, blocks: &mut BlockCache) -> Vec<Connector> {
    let mut out = Vec::new();
    for e in store.iter() {
        let EntityKind::Insert(ins) = &e.kind else { continue };
        let local = blocks.get(d, &ins.block);
        for (name, point) in place(d, ins, &local) {
            if out.len() >= MAX_INDEX {
                return out;
            }
            out.push(Connector { insert: e.handle, name, point });
        }
    }
    out
}

/// The links of a wire's start and end.
pub fn links(e: &Entity) -> [Option<Link>; 2] {
    let mut out = [None, None];
    let Some(body) = app_body(&e.common.xdata, LINK_APP) else { return out };
    let mut i = 0;
    while let (Some(which), Some(h), Some(name)) = (body.get(i), body.get(i + 1), body.get(i + 2)) {
        i += 3;
        let slot = match which.value.as_str() {
            "start" => 0,
            "end" => 1,
            _ => continue,
        };
        if which.code != 1000 || h.code != 1005 || name.code != 1000 {
            continue;
        }
        if let (Some(insert), Some(s)) = (Handle::parse_hex(&h.value), out.get_mut(slot)) {
            *s = Some(Link { insert, name: name.value.clone() });
        }
    }
    out
}

/// Write a wire's links (both none: the xdata goes).
pub fn set_links(xdata: &mut Vec<RawTag>, links: &[Option<Link>; 2]) {
    let mut body = Vec::new();
    for (which, l) in ["start", "end"].iter().zip(links) {
        if let Some(l) = l {
            body.push(RawTag { code: 1000, value: which.to_string() });
            body.push(RawTag { code: 1005, value: l.insert.hex() });
            body.push(RawTag { code: 1000, value: l.name.clone() });
        }
    }
    replace_app(xdata, LINK_APP, body);
}

/// Whether `k` can be attached as a wire.
pub fn is_wire(k: &EntityKind) -> bool {
    match k {
        EntityKind::LwPolyline(p) => !p.closed && p.vertices.len() >= 2,
        EntityKind::Line(_) => true,
        _ => false,
    }
}

/// A wire's start and end points.
pub fn ends(k: &EntityKind) -> Option<[Vec2; 2]> {
    match k {
        EntityKind::LwPolyline(p) if !p.closed => Some([p.vertices.first()?.p, p.vertices.last()?.p]),
        EntityKind::Line(l) => Some([l.a.xy(), l.b.xy()]),
        _ => None,
    }
}

/// Tolerance for "on the connector" at `p`.
fn tol(p: Vec2) -> f64 {
    1e-6 * (1.0 + p.x.abs().max(p.y.abs()))
}

/// Move end `which` (0 start, 1 end) of wire `k` to `to`. A polyline's neighbouring vertex
/// follows along the segment's direction when that segment was horizontal or vertical, so the
/// wire keeps its right angles.
fn move_end(k: &mut EntityKind, which: usize, to: Vec2) {
    match k {
        EntityKind::Line(l) => {
            let p = if which == 0 { &mut l.a } else { &mut l.b };
            p.x = to.x;
            p.y = to.y;
        }
        EntityKind::LwPolyline(pl) => {
            let n = pl.vertices.len();
            if n < 2 {
                return;
            }
            let (end, next) = if which == 0 { (0, 1) } else { (n - 1, n - 2) };
            let Some(old) = pl.vertices.get(end).map(|v| v.p) else { return };
            if n >= 3
                && let Some(nb) = pl.vertices.get_mut(next)
            {
                let t = tol(old);
                if (nb.p.y - old.y).abs() <= t && (nb.p.x - old.x).abs() > t {
                    nb.p.y = to.y;
                } else if (nb.p.x - old.x).abs() <= t && (nb.p.y - old.y).abs() > t {
                    nb.p.x = to.x;
                }
            }
            if let Some(v) = pl.vertices.get_mut(end) {
                v.p = to;
            }
        }
        _ => {}
    }
}

/// The post-command pass (see the module docs). `before` is the drawing the command started
/// from.
pub fn after_command(s: &mut crate::Session, before: Option<&Arc<Drawing>>, uid: Option<u64>) {
    let Some(before) = before else { return };
    if let Ok(st) = s.state_mut()
        && uid.is_none_or(|u| u == st.uid)
        && !Arc::ptr_eq(before, &st.doc)
    {
        update(&mut st.doc, before);
    }
}

/// Bring every wire's links up to date after a change from `before`. Returns the number of
/// wires changed.
pub fn update(after: &mut Arc<Drawing>, before: &Drawing) -> usize {
    let spaces: Vec<Space> = std::iter::once(Space::Model).chain(after.layouts.iter().map(|l| Space::Paper(l.name.clone()))).collect();
    let blocks_changed =
        after.blocks.len() != before.blocks.len() || after.blocks.iter().any(|(n, b)| before.blocks.get(n).is_none_or(|o| !Arc::ptr_eq(o, b)));
    let mut changes: Vec<(Handle, Entity)> = Vec::new();
    for space in &spaces {
        let (Some(now), was) = (after.space(space), before.space(space)) else { continue };
        if was.is_some_and(|w| w.same_as(now)) && !blocks_changed {
            continue;
        }
        changes.extend(plan(after, before, now, was, blocks_changed));
    }
    let n = changes.len();
    if n > 0 {
        let d = Arc::make_mut(after);
        for (h, e) in changes {
            let _ = d.modify_entity(h, |x| *x = e);
        }
    }
    n
}

/// The wires of one space to change.
fn plan(d: &Drawing, before: &Drawing, now: &EntityStore, was: Option<&EntityStore>, blocks_changed: bool) -> Vec<(Handle, Entity)> {
    // The chunks the command touched (all of them in a space that is new).
    let old_keys: std::collections::HashSet<usize> = was.map(|w| w.chunk_slices().map(|(k, _)| k).collect()).unwrap_or_default();
    let new_keys: std::collections::HashSet<usize> = now.chunk_slices().map(|(k, _)| k).collect();
    let touched: Vec<&Arc<Entity>> = now.chunk_slices().filter(|(k, _)| !old_keys.contains(k)).flat_map(|(_, s)| s.iter()).collect();
    let dropped = was.into_iter().flat_map(|w| w.chunk_slices()).filter(|(k, _)| !new_keys.contains(k)).flat_map(|(_, s)| s.iter());
    let inserts_changed = blocks_changed
        || touched.iter().any(|e| matches!(e.kind, EntityKind::Insert(_)) && before.entity(e.handle).is_none_or(|o| !Arc::ptr_eq(o, e)))
        || dropped.filter(|e| matches!(e.kind, EntityKind::Insert(_))).any(|e| now.get(e.handle).is_none_or(|n| !Arc::ptr_eq(n, e)));
    let mut blocks = BlockCache::default();
    let mut index_cache: Option<Vec<Connector>> = None;
    let mut out = Vec::new();
    // Wires the command changed or made: their ends attach to what they lie on now.
    let changed: Vec<&Arc<Entity>> =
        touched.iter().copied().filter(|e| is_wire(&e.kind) && before.entity(e.handle).is_none_or(|o| !Arc::ptr_eq(o, e))).collect();
    let mut done = std::collections::HashSet::new();
    for e in &changed {
        let Some(pts) = ends(&e.kind) else { continue };
        let old = before.entity(e.handle);
        let old_pts = old.and_then(|o| ends(&o.kind));
        let mut ls = links(e);
        let mut kind = e.kind.clone();
        for (i, p) in pts.iter().enumerate() {
            let Some(slot) = ls.get_mut(i) else { continue };
            let stayed = old_pts.and_then(|o| o.get(i).copied()).is_some_and(|o| o.near(*p, tol(*p)));
            if stayed && slot.is_some() {
                // The end was not moved: it follows its connector (when that moved).
                follow(d, &mut blocks, &mut kind, i, slot);
                continue;
            }
            let idx = index_cache.get_or_insert_with(|| index(d, now, &mut blocks));
            match nearest(idx, *p, tol(*p)) {
                Some(c) => *slot = Some(Link { insert: c.insert, name: c.name.clone() }),
                // A wire that is new here (a copy, or the drawing back from the Block Editor)
                // only lets go of a reference this drawing had before the command: one it
                // doesn't know of is not its to judge.
                None if old.is_none() && slot.as_ref().is_some_and(|l| before.entity(l.insert).is_none()) => {}
                None => *slot = None,
            }
        }
        done.insert(e.handle);
        push_change(&mut out, e, kind, &ls);
    }
    // Wires the command left alone: their attached ends follow the block references that moved.
    if inserts_changed {
        for e in now.iter() {
            if done.contains(&e.handle) || !is_wire(&e.kind) {
                continue;
            }
            let mut ls = links(e);
            if ls.iter().all(Option::is_none) {
                continue;
            }
            let mut kind = e.kind.clone();
            for (i, slot) in ls.iter_mut().enumerate() {
                if slot.is_some() {
                    follow(d, &mut blocks, &mut kind, i, slot);
                }
            }
            push_change(&mut out, e, kind, &ls);
        }
    }
    out
}

/// Where the connector of link `l` is now.
fn locate(d: &Drawing, blocks: &mut BlockCache, l: &Link) -> Option<Vec2> {
    let e = d.entity(l.insert)?;
    let EntityKind::Insert(ins) = &e.kind else { return None };
    let local = blocks.get(d, &ins.block);
    place(d, ins, &local).into_iter().find(|(n, _)| *n == l.name).map(|(_, p)| p)
}

/// End `i` of wire `kind` goes to where its connector is now; a connector (or reference) that is
/// gone lets it go.
fn follow(d: &Drawing, blocks: &mut BlockCache, kind: &mut EntityKind, i: usize, slot: &mut Option<Link>) {
    let Some(l) = slot.as_ref() else { return };
    match locate(d, blocks, l) {
        Some(to) => {
            if ends(kind).and_then(|p| p.get(i).copied()).is_some_and(|p| !p.near(to, tol(to))) {
                move_end(kind, i, to);
            }
        }
        None => *slot = None,
    }
}

fn push_change(out: &mut Vec<(Handle, Entity)>, e: &Arc<Entity>, kind: EntityKind, ls: &[Option<Link>; 2]) {
    let mut ne = Entity::clone(e);
    ne.kind = kind;
    set_links(&mut ne.common.xdata, ls);
    if ne != **e {
        out.push((e.handle, ne));
    }
}
