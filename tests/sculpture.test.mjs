import test from 'node:test';
import assert from 'node:assert/strict';
import { makeScenes } from '../static/scenes.js';
import { createTimeline } from '../static/runtime/timeline.js';
import { posterPalette } from '../static/runtime/renderer.js';

const scenes = makeScenes();
test('particle text retains its theme foreground and transparent antialiased edges', () => {
  for (const [background, ink, primary] of [
    [[249, 248, 244], [37, 36, 32], [37, 36, 32]],
    [[24, 23, 21], [242, 238, 229], [242, 238, 229]],
  ]) {
    const palette = posterPalette(background, { ink, primary }, true);
    const pixel = palette.ink([32, 32, 32], 'ink');
    assert.equal(palette.background, 0);
    assert.deepEqual([pixel & 255, (pixel >>> 8) & 255, (pixel >>> 16) & 255], ink);
    assert.equal(pixel >>> 24, 255);
    const edge = palette.coverage(pixel, .5);
    assert.equal(edge >>> 24, 128);
    assert.equal(edge & 0xffffff, pixel & 0xffffff);
    assert.equal(palette.coverage(pixel, 0) >>> 24, 0);
  }
});

test('each scene is a finite, three-dimensional sculpture within the phone budget', () => {
  assert.deepEqual(scenes.map(s => s.id), ['ferry', 'bridge', 'compass']);
  for (const scene of scenes) {
    assert.ok(scene.points.length > 1000, scene.id);
    assert.ok(scene.points.length <= 22000, `${scene.id}: ${scene.points.length} samples`);
    const depths = scene.points.map(p => p.z);
    assert.ok(Math.max(...depths) - Math.min(...depths) > .1, `${scene.id} must have depth`);
    for (const p of scene.points) {
      assert.ok([p.x, p.y, p.z, ...p.color].every(Number.isFinite));
      assert.ok(p.color.every(c => c >= 0 && c <= 255));
    }
  }
});
test('scene animation keeps sampled geometry immutable and all poses in view', () => {
  for (const scene of scenes) {
    const before = JSON.stringify(scene.points);
    scene.points.forEach(p => { Object.freeze(p.color); Object.freeze(p); });
    Object.freeze(scene.points);
    let motion = false;
    for (const time of [0, .4, 2.1, 7.7, 17.2]) {
      const deform = scene.animate(time);
      for (const p of scene.points) {
        const v = [p.x, p.y, p.z];
        deform(p, v);
        assert.ok(v.every(Number.isFinite), `${scene.id} at ${time}`);
        assert.ok(v.every(n => Math.abs(n) <= 1.6), `${scene.id} stays in the projection bounds`);
        if (v.some((n,i) => Math.abs(n - [p.x,p.y,p.z][i]) > .01)) motion = true;
      }
    }
    assert.equal(JSON.stringify(scene.points), before);
    assert.ok(motion, `${scene.id} has meaningful movement`);
  }
});
test('carousel has bounded transitions and can select a still scene', () => {
  const timeline = createTimeline(scenes, {hold:8, morph:2});
  assert.equal(timeline.at(0).from.id, 'ferry');
  assert.equal(timeline.at(10).from.id, 'bridge');
  assert.equal(timeline.at(20).from.id, 'compass');
  const transition = timeline.at(9);
  assert.equal(transition.from.id, 'ferry');
  assert.equal(transition.to.id, 'bridge');
  assert.ok(transition.mix > 0 && transition.mix < 1);
  assert.equal(timeline.at(100, 2).from.id, 'compass');
  assert.equal(timeline.at(100, 2).mix, 0);
});
