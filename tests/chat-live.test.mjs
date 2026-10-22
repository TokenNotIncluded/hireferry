import test from 'node:test';
import assert from 'node:assert/strict';
import { applyChatEvent, chatActionDraft } from '../static/service-ui.js';

const initial = { id: 4, conversation_id: 'platform_a', body: 'Hello', body_html: '<p>Hello</p>', format: 'markdown', status: 'streaming', sequence: 0 };
const event = (sequence, delta) => ({ id: 20 + sequence, type: 'message.delta', conversation_id: 'platform_a', message_id: 4, sequence, delta });

test('stream deltas preserve Unicode and invalidate stale server HTML without mutation', () => {
  const result = applyChatEvent([initial], event(1, ' 世界 🌏'));
  assert.equal(result.message.body, 'Hello 世界 🌏');
  assert.equal(result.message.body_html, '');
  assert.equal(result.message.sequence, 1);
  assert.equal(initial.body, 'Hello');
});

test('replayed deltas are ignored and sequence gaps request a snapshot', () => {
  const latest = applyChatEvent([initial], event(1, ' there')).message;
  assert.deepEqual(applyChatEvent([latest], event(1, ' there')), {});
  assert.deepEqual(applyChatEvent([latest], event(3, '!')), { resync: true });
  assert.deepEqual(applyChatEvent([], event(1, ' there')), { resync: true });
});

test('terminal snapshots recover missed deltas and cannot regress to an older state', () => {
  const done = { ...initial, body: 'Hello there!', sequence: 3, status: 'complete' };
  const result = applyChatEvent([initial], { id: 24, type: 'message.completed', conversation_id: 'platform_a', message_id: 4, sequence: 3, message: done });
  assert.equal(result.message, done);
  assert.deepEqual(applyChatEvent([done], { type: 'message.started', conversation_id: 'platform_a', message_id: 4, message: initial }), {});
  assert.deepEqual(applyChatEvent([done], event(4, ' unexpected')), { resync: true });
});

test('invalid message identities and oversized accumulated input never reach a renderer', () => {
  assert.deepEqual(applyChatEvent([initial], { type: 'message.created', conversation_id: 'platform_a', message_id: 4, message: { ...initial, id: 5 } }), { resync: true });
  assert.deepEqual(applyChatEvent([initial], { type: 'message.created', conversation_id: 'platform_b', message_id: 4, message: initial }), { resync: true });
  assert.deepEqual(applyChatEvent([{ ...initial, body: 'x'.repeat(10000) }], event(1, 'x')), { resync: true });
  assert.deepEqual(applyChatEvent([initial], null), { resync: true });
});

test('OpenUI form actions preserve entered scalar values in a reviewable draft', () => {
  assert.equal(chatActionDraft({ type: 'draft', text: '确认安排', formState: { 姓名: '李明', 人数: 2, 可参加: true, hidden: { instruction: 'ignored' }, invalid: NaN, constructor: 'ignored' } }), '确认安排\n\n姓名: 李明\n人数: 2\n可参加: true');
  assert.equal(chatActionDraft({ type: 'draft', text: '  选择周三  ' }), '选择周三');
  assert.equal(chatActionDraft({ type: 'open_url', text: 'https://example.test' }), null);
  assert.equal(chatActionDraft({ type: 'draft', text: 'x'.repeat(4097) }), null);
  assert.equal(chatActionDraft({ type: 'draft', text: '发送', formState: { 内容: 'x'.repeat(5000) } }).length, 2 + 2 + 4 + 1024);
});
