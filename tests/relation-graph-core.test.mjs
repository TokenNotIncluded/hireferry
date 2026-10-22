import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = await readFile(new URL('../static/relation-graph-core.js', import.meta.url), 'utf8');
const { graphWindow, graphEndpoint, graphProjection, emptyGraphView, applyGraphCommands, graphGroupOrder } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));

test('a thousand-row directory mounts a viewport and overscan, not the whole list', () => {
  const first = graphWindow(1000, 0, 440), middle = graphWindow(1000, 36000, 440), last = graphWindow(1000, 71980, 440);
  assert.equal(first.height, 72000);assert.equal(first.start, 0);assert(first.end < 15);
  assert(middle.end - middle.start <= 14);assert.equal(last.end, 1000);
  assert.deepEqual(graphWindow(0, 100, 440), { start: 0, end: 0, height: 0 });
});
test('logical offscreen endpoints retain a direction when their DOM rows are unmounted', () => {
  assert.equal(graphEndpoint(0, 100, 440).zone, 'above');
  assert.equal(graphEndpoint(10, 0, 440).zone, 'below');
  assert.deepEqual(graphEndpoint(1, 0, 440), { zone: 'visible', y: 104 });
});
test('both-endpoint offscreen links remain counted without ghost curves', () => {
  const ids = Array.from({ length: 30 }, (_, i) => String(i));
  const edges = [{ id: 'a', seeker_id: '0', job_id: '0', status: 'introduced' }, { id: 'b', seeker_id: '20', job_id: '20', status: 'hired' }, { id: 'c', seeker_id: '10', job_id: '0', status: 'interview' }];
  const projection = graphProjection(edges, ids, ids, { left: { scrollTop: 680, height: 440 }, right: { scrollTop: 680, height: 440 } });
  assert.equal(projection.counts.offscreen, 3);assert.equal(projection.paths.length, 1);
  assert.equal(projection.counts.left.above, 1);assert.equal(projection.counts.right.above, 2);
  assert.equal(projection.paths[0].a.zone, 'visible');assert.equal(projection.paths[0].b.zone, 'above');
});
test('hidden, missing, offscreen and visible relationships form a disjoint partition', () => {
  const edges = [{ id: 'visible', seeker_id: 'u', job_id: 'j0', status: 'introduced' }, { id: 'off', seeker_id: 'u', job_id: 'j10', status: 'introduced' }, { id: 'hide', seeker_id: 'u', job_id: 'j1', status: 'declined' }, { id: 'unloaded', seeker_id: 'u', job_id: 'elsewhere', status: 'hired' }];
  const projected = graphProjection(edges, ['u'], Array.from({ length: 20 }, (_, i) => 'j' + i), { left: { scrollTop: 0, height: 440 }, right: { scrollTop: 0, height: 440 } }, { edges: ['hide'] });
  const c = projected.counts;assert.deepEqual([c.visible, c.offscreen, c.hidden, c.missing], [1, 1, 1, 1]);assert.equal(c.right.below, 1);
  assert.equal(projected.paths.length, 2);
});
test('a hub aggregates many invisible neighbors into bounded status portals', () => {
  const ids = Array.from({ length: 1000 }, (_, i) => 'j' + i), stages = ['introduced', 'interview', 'hired', 'declined'];
  const edges = ids.map((job_id, i) => ({ id: String(i), seeker_id: 'hub', job_id, status: stages[i % 4] }));
  const result = graphProjection(edges, ['hub'], ids, { left: { scrollTop: 0, height: 440 }, right: { scrollTop: 0, height: 440 } });
  assert.equal(result.counts.visible + result.counts.offscreen, 1000);assert(result.paths.length <= 11);assert.equal(result.counts.right.below, 994);
});
test('changing view mode never silently drops a filter, and hiding is not deletion', () => {
  const initial = emptyGraphView();initial.filters = { left_group_ids: ['industry:x'], match_status: 'introduced' };
  const next = applyGraphCommands(initial, [{ op: 'set_mode', mode: 'matrix' }, { op: 'set_visibility', side: 'left', ids: ['u'], visible: false }, { op: 'set_edge_display', edge_display: 'hidden' }]);
  assert.deepEqual(next.filters, initial.filters);assert.deepEqual(initial.hidden.left, []);assert.deepEqual(next.hidden.left, ['u']);
  assert.equal(applyGraphCommands(next, [{ op: 'set_visibility', side: 'left', ids: ['u'], visible: true }]).hidden.left.length, 0);
});
test('fullscreen requests await a browser gesture and ending immersive mode clears the request', () => {
  const requested = applyGraphCommands(emptyGraphView(), [{ op: 'request_fullscreen' }]);
  assert.equal(requested.fullscreen_requested, true);assert.equal(requested.presentation, 'standard');
  assert.equal(applyGraphCommands(requested, [{ op: 'set_presentation', presentation: 'standard' }]).fullscreen_requested, false);
});
test('group ordering reduces a crossed pair and leaves equal layouts stable', () => {
  const groups = { left: [{ id: 'a' }, { id: 'b' }], right: [{ id: 'x' }, { id: 'y' }] };
  const bundles = [{ left_group_id: 'a', right_group_id: 'y', count: 10 }, { left_group_id: 'b', right_group_id: 'x', count: 10 }];
  const result = graphGroupOrder(groups, bundles);assert.deepEqual(result.right, ['y', 'x']);assert.deepEqual(graphGroupOrder(groups, bundles, result), result);
  assert.deepEqual(graphGroupOrder(groups, [], { left: ['b', 'a'], right: ['y', 'x'] }), { left: ['b', 'a'], right: ['y', 'x'] });
});
