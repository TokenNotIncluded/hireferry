import test from 'node:test';
import assert from 'node:assert/strict';
import { storyPosition } from '../static/home-story.js';
import { createTimeline } from '../static/runtime/timeline.js';

test('scrolling visits all chapters once and holds the last before entering the page flow', () => {
  assert.equal(storyPosition(-400, 2400, 3), 0);
  assert.equal(storyPosition(0, 2400, 3), 0);
  assert.equal(storyPosition(800, 2400, 3), 1);
  assert.equal(storyPosition(1800, 2400, 3), 2);
  assert.equal(storyPosition(2400, 2400, 3), 2);
  assert.equal(storyPosition(4000, 2400, 3), 2);
});

test('slow scrolling and reversing keep progress continuous and never wrap to the first chapter', () => {
  let previous = 0;
  for (let offset = 0; offset <= 2700; offset++) {
    const position = storyPosition(offset, 2400, 3);
    assert.ok(position >= previous && position <= 2);
    assert.ok(position - previous < .003);
    previous = position;
  }
  const forward = [0, 350, 800, 1350, 1800, 2400].map(y => storyPosition(y, 2400, 3));
  const backward = [2400, 1800, 1350, 800, 350, 0].map(y => storyPosition(y, 2400, 3));
  assert.deepEqual(backward, forward.toReversed());
});

test('particle transitions follow fractional chapter progress and keep ambient time on reversal', () => {
  const scenes = ['ferry', 'bridge', 'compass'].map(id => ({ id }));
  const timeline = createTimeline(scenes);
  const forward = timeline.at(17, .5);
  assert.equal(forward.from.id, 'ferry');
  assert.equal(forward.to.id, 'bridge');
  assert.equal(forward.mix, .5);
  assert.equal(forward.age, 17);
  assert.equal(forward.nextAge, 17);
  assert.deepEqual(timeline.at(17, .5), forward);
  assert.equal(timeline.at(17, 2).mix, 0);
  assert.equal(timeline.at(17, 100).from.id, 'compass');
  assert.equal(timeline.at(17, -4).from.id, 'ferry');
});
