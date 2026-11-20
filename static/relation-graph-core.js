export const ROW_HEIGHT = 72;
export const PAGE_LIMIT = 100;
export const EDGE_BUDGET = 80;
export const STAGES = ['introduced', 'interview', 'hired', 'declined'];

export function graphWindow(length, scrollTop, viewportHeight, overscan = 3) {
  const start = Math.max(0, Math.floor(Math.max(0, scrollTop) / ROW_HEIGHT) - overscan);
  return { start: Math.min(length, start), end: Math.min(length, Math.ceil((Math.max(0, scrollTop) + Math.max(1, viewportHeight)) / ROW_HEIGHT) + overscan), height: length * ROW_HEIGHT };
}
export function graphEndpoint(index, scrollTop, height) {
  const y = index * ROW_HEIGHT + 32 - scrollTop;
  return { y: Math.max(8, Math.min(height - 8, y)), zone: y < 0 ? 'above' : y > height ? 'below' : 'visible' };
}

// Each loaded edge has one disposition. Each side's portal count is separate:
// an edge whose two endpoints are offscreen must not be counted twice in totals.
export function graphProjection(edges, leftIds, rightIds, viewport, hidden = {}) {
  const indices = { left: new Map(leftIds.map((id, i) => [id, i])), right: new Map(rightIds.map((id, i) => [id, i])) };
  const hide = { left: new Set(hidden.left || []), right: new Set(hidden.right || []), edges: new Set(hidden.edges || []) };
  const counts = { visible: 0, offscreen: 0, hidden: 0, missing: 0, left: { above: 0, below: 0 }, right: { above: 0, below: 0 } }, paths = new Map();
  for (const edge of edges) {
    if (hide.left.has(edge.seeker_id) || hide.right.has(edge.job_id) || hide.edges.has(edge.id)) { counts.hidden++; continue; }
    if (!indices.left.has(edge.seeker_id) || !indices.right.has(edge.job_id)) { counts.missing++; continue; }
    const a = graphEndpoint(indices.left.get(edge.seeker_id), viewport.left.scrollTop, viewport.left.height), b = graphEndpoint(indices.right.get(edge.job_id), viewport.right.scrollTop, viewport.right.height);
    if (a.zone === 'visible' && b.zone === 'visible') counts.visible++; else counts.offscreen++;
    if (a.zone !== 'visible') counts.left[a.zone]++;
    if (b.zone !== 'visible') counts.right[b.zone]++;
    if (a.zone !== 'visible' && b.zone !== 'visible') continue;
    const key = JSON.stringify([a.zone === 'visible' ? edge.seeker_id : a.zone, b.zone === 'visible' ? edge.job_id : b.zone, edge.status]);
    const old = paths.get(key);
    if (old) old.count++; else paths.set(key, { a, b, status: edge.status, count: 1 });
  }
  return { counts, paths: [...paths.values()] };
}

export function emptyGraphView() {
  return { mode: 'overview', filters: {}, grouping: { left: 'role', right: 'status' }, focus: null, ordering: 'stable', edge_display: 'auto', hidden: { left: [], right: [], edges: [] }, presentation: 'standard', fullscreen_requested: false };
}
export function graphStateKey(value) {
  const normalize = v => Array.isArray(v) ? v.map(normalize).sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b))) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().filter(k => v[k] !== null && v[k] !== undefined).map(k => [k, normalize(v[k])])) : v;
  return JSON.stringify(normalize(value));
}
export function applyGraphCommands(current, commands) {
  const next = structuredClone(current);
  for (const c of commands) {
    if (c.op === 'set_filters') next.filters = structuredClone(c.filters || {});
    else if (c.op === 'set_grouping') next.grouping = structuredClone(c.grouping);
    else if (c.op === 'set_mode') next.mode = c.mode;
    else if (c.op === 'set_focus') next.focus = c.focus ? { ...c.focus } : null;
    else if (c.op === 'set_ordering') next.ordering = c.ordering;
    else if (c.op === 'set_edge_display') next.edge_display = c.edge_display;
    else if (c.op === 'set_presentation') { next.presentation = c.presentation;if (c.presentation === 'standard') next.fullscreen_requested = false; }
    else if (c.op === 'request_fullscreen') next.fullscreen_requested = true;
    else if (c.op === 'set_visibility' && ['left', 'right', 'edges'].includes(c.side)) {
      const ids = new Set(next.hidden[c.side]);
      for (const id of c.ids) { if (c.visible) ids.delete(id); else ids.add(id); }
      next.hidden[c.side] = [...ids];
    }
  }
  return next;
}

export function graphGroupOrder(groups, bundles, previous = {}) {
  const preserve = (ids, old = []) => [...old.filter(id => ids.includes(id)), ...ids.filter(id => !old.includes(id))];
  let left = preserve((groups.left || []).map(g => g.id), previous.left), right = preserve((groups.right || []).map(g => g.id), previous.right);
  const crossing = (l, r) => {
    const li = new Map(l.map((id, i) => [id, i])), ri = new Map(r.map((id, i) => [id, i])); let sum = 0;
    for (let a = 0; a < bundles.length; a++) for (let b = a + 1; b < bundles.length; b++) {
      const x = bundles[a], y = bundles[b];
      if ((li.get(x.left_group_id) - li.get(y.left_group_id)) * (ri.get(x.right_group_id) - ri.get(y.right_group_id)) < 0) sum += x.count * y.count;
    }
    return sum;
  };
  let score = crossing(left, right);
  for (let sweep = 0; sweep < 4; sweep++) for (const side of ['right', 'left']) {
    const own = side === 'left' ? left : right, otherSide = side === 'left' ? 'right' : 'left', other = side === 'left' ? right : left, rank = new Map(other.map((id, i) => [id, i]));
    const proposed = own.map((id, old) => {
      let weight = 0, sum = 0;
      for (const b of bundles) if (b[side + '_group_id'] === id && rank.has(b[otherSide + '_group_id'])) { weight += b.count; sum += b.count * rank.get(b[otherSide + '_group_id']); }
      return { id, old, rank: weight ? sum / weight : old };
    }).sort((a, b) => a.rank - b.rank || a.old - b.old).map(x => x.id);
    const value = crossing(side === 'left' ? proposed : left, side === 'right' ? proposed : right);
    if (value < score) { if (side === 'left') left = proposed; else right = proposed; score = value; }
  }
  return { left, right };
}
