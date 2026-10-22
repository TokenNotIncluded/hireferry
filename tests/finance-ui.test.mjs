import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../static/finance-ui.js', import.meta.url), 'utf8');
const { financeRange, financePath, financeSeries, createFinanceUI } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const now = new Date('2026-10-22T15:30:00Z');
const escape = value => String(value ?? '').replace(/[&<>"']/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]);

test('quick ranges are inclusive UTC dates and preserve the other filters', () => {
  assert.deepEqual(financeRange('7', now), { from: '2026-10-16', to: '2026-10-22' });
  assert.deepEqual(financeRange('30', now), { from: '2026-09-23', to: '2026-10-22' });
  assert.deepEqual(financeRange('month', now), { from: '2026-10-01', to: '2026-10-22' });
  const current = { currency: 'JPY', mode: 'test', status: 'refunded', page: '4' };
  const next = new URL(financePath('orders', current, { ...financeRange('7', now), page: '' }), 'https://example.test');
  assert.equal(next.searchParams.get('currency'), 'JPY');
  assert.equal(next.searchParams.get('mode'), 'test');
  assert.equal(next.searchParams.get('status'), 'refunded');
  assert.equal(next.searchParams.has('page'), false);
});

test('chart covers the entire selected range, including zero days and negative refund days', () => {
  const series = [{ date: '2026-10-20', revenue_cents: -300, refund_cents: 300, orders: 0 }, { date: '2026-10-18', revenue_cents: 1000, gross_cents: 1000, orders: 1 }];
  const result = financeSeries({ series, bucket: 'day' }, { from: '2026-10-16', to: '2026-10-22' }, now);
  assert.equal(result.items.length, 7);
  assert.equal(result.items[0].date, '2026-10-16');
  assert.equal(result.items.at(-1).date, '2026-10-22');
  assert.deepEqual(result.items.map(item => item.revenue_cents), [0, 0, 1000, 0, -300, 0, 0]);
  assert.equal(series[0].date, '2026-10-20', 'input data is not mutated');
});

test('empty, one-day, monthly and very long reports stay truthful and bounded', () => {
  const empty = financeSeries({ series: [], bucket: 'day' }, {}, now);
  assert.equal(empty.items.length, 7);
  assert.ok(empty.items.every(item => item.revenue_cents === 0));
  const one = financeSeries({ series: [] }, { from: '2026-10-22', to: '2026-10-22' }, now);
  assert.equal(one.items.length, 1);
  const monthly = financeSeries({ series: [{ date: '2025-02', revenue_cents: 7 }], bucket: 'month' }, { from: '2025-01-10', to: '2025-03-22' }, now);
  assert.deepEqual(monthly.items.map(item => [item.date, item.revenue_cents]), [['2025-01', 0], ['2025-02', 7], ['2025-03', 0]]);
  const long = financeSeries({ series: [{ date: '2026-10', revenue_cents: 100 }], bucket: 'month' }, { from: '0001-01-01', to: '9999-12-31' }, now);
  assert.ok(long.items.length <= 601);
  assert.equal(long.items.reduce((sum, item) => sum + item.revenue_cents, 0), 100);
  assert.throws(() => financeSeries({ series: [] }, { from: '2026-10-23', to: '2026-10-22' }), /dates/);
  assert.throws(() => financeSeries({ series: [{ date: '2026-10-22', revenue_cents: Number.MAX_SAFE_INTEGER + 1 }] }, {}, now), /safe integer/);
});

function fixture(api, query = '', state = { config: { development: false }, lang: 'en' }) {
  return createFinanceUI({
    api, state, t: (_, en) => en, esc: escape,
    money: (minor, row) => { const decimals = row?.currency_decimals ?? (row?.currency === 'JPY' ? 0 : 2); return `${row?.currency || 'CNY'} ${(Number(minor || 0) / 10 ** decimals).toFixed(decimals)}`; },
    currency: row => ({ currency: row?.currency || 'CNY', currency_decimals: row?.currency_decimals ?? (row?.currency === 'JPY' ? 0 : 2) }), currencies: () => [{ currency: 'CNY' }, { currency: 'USD' }],
    qs: () => new URLSearchParams(query), shell: html => html, icon: name => `<i>${name}</i>`,
    orderTable: items => `<div class="orders">${items.map(item => escape(item.id)).join(',')}</div>`,
    pagination: (total, size) => `<span data-total="${total}" data-size="${size}"></span>`, statusText: value => value, go: () => {},
  });
}
const report = changes => ({
  currency: 'CNY', currency_decimals: 2, payment_mode: 'live', bucket: 'day',
  stats: { gross_cents: 10000, refund_cents: 2000, revenue_cents: 8000, paid_orders: 12, users: 5, active_jobs: 2 },
  series: [{ date: '2026-10-22', revenue_cents: 8000, gross_cents: 10000, refund_cents: 2000, orders: 12 }],
  recent_orders: [], currency_totals: [], ...changes,
});

test('orders use one scope for report and listing and display aggregate totals independent of pagination', async () => {
  const calls = [];
  const ui = fixture(async path => {
    calls.push(new URL(path, 'https://example.test'));
    return path.startsWith('/api/admin/overview') ? report() : { items: [{ id: '<page-one>' }], total: 12, page_size: 1 };
  }, 'from=2026-10-01&to=2026-10-22&currency=CNY&mode=live&status=paid&page=2');
  const html = await ui.ordersPage();
  assert.equal(calls.length, 2);
  for (const call of calls) for (const [key, value] of Object.entries({ from: '2026-10-01', to: '2026-10-22', currency: 'CNY', mode: 'live', status: 'paid' })) assert.equal(call.searchParams.get(key), value);
  assert.equal(calls[0].searchParams.has('page'), false);
  assert.equal(calls[1].searchParams.get('page'), '2');
  assert.match(html, /data-finance-main-total>CNY 80\.00/);
  assert.match(html, /CNY 100\.00/);
  assert.match(html, /CNY 20\.00/);
  assert.match(html, /data-total="12"/);
  assert.match(html, /&lt;page-one&gt;/);
  assert.match(html, /confirmation date/);
  assert.match(html, /order creation date/);
  assert.doesNotMatch(html, /A clear record|Every connection/);
});

test('empty reports render a real zero line, while failed or incomplete reports reject', async () => {
  const ui = fixture(async () => report({ series: [], stats: { gross_cents: 0, refund_cents: 0, revenue_cents: 0, paid_orders: 0 } }), 'from=2026-10-21&to=2026-10-22');
  const html = await ui.overviewPage();
  assert.match(html, /finance-line is-empty/);
  assert.match(html, /No confirmed transactions/);
  assert.match(html, /CNY 0\.00/);
  await assert.rejects(fixture(async () => { throw new Error('network unavailable'); }).overviewPage(), /network unavailable/);
  await assert.rejects(fixture(async () => ({})).overviewPage(), /incomplete/);
  await assert.rejects(fixture(async () => report({ stats: {} })).overviewPage(), /incomplete/);
});

test('a single refund-only day keeps its negative total and a zero baseline above the point', async () => {
  const ui = fixture(async () => report({ series: [{ date: '2026-10-22', revenue_cents: -500, refund_cents: 500, orders: 0 }], stats: { gross_cents: 0, refund_cents: 500, revenue_cents: -500, paid_orders: 0 } }), 'from=2026-10-22&to=2026-10-22');
  const html = await ui.overviewPage();
  assert.match(html, /data-finance-main-total>CNY -5\.00/);
  assert.match(html, /class="finance-grid is-zero"[^>]+y1="22"/);
  assert.match(html, /class="finance-point"[^>]+cy="\d+(\.\d+)?"/);
  assert.doesNotMatch(html, /NaN|Infinity|is-empty/);
});

test('reports retain separate JPY and USD totals and each currency precision', async () => {
  const data = report({ currency: 'JPY', currency_decimals: 0, stats: { gross_cents: 3000, refund_cents: 500, revenue_cents: 2500, paid_orders: 2 }, series: [{ date: '2026-10-22', revenue_cents: 2500, gross_cents: 3000, refund_cents: 500, orders: 2 }], currency_totals: [{ currency: 'JPY', currency_decimals: 0, gross_cents: 3000, refund_cents: 500, revenue_cents: 2500 }, { currency: 'USD', currency_decimals: 2, gross_cents: 2000, refund_cents: 500, revenue_cents: 1500 }] });
  const html = await fixture(async () => data, 'from=2026-10-22&to=2026-10-22&currency=JPY').overviewPage();
  assert.match(html, /data-finance-main-total>JPY 2500</);
  assert.match(html, /<td>USD 15\.00<\/td>/);
  assert.match(html, /<td>JPY 2500<\/td>/);
  assert.doesNotMatch(html, /JPY 25\.00|JPY 4000|USD 4000/);
  assert.match(html, /Each currency is reported separately/);
});

test('late reports cannot restore data after navigation, logout or explicit destruction', async () => {
  for (const change of [state => { state.render += 1; }, state => { state.csrf = 'different-session'; }, state => { state.admin = false; }, (_state, ui) => { ui.destroy(); }]) {
    let resolve;
    const deferred = new Promise(done => { resolve = done; });
    const state = { render: 1, admin: true, csrf: 'original-session', config: { development: false }, lang: 'en' };
    const ui = fixture(() => deferred, 'from=2026-10-22&to=2026-10-22', state);
    const loading = ui.overviewPage();
    change(state, ui);
    resolve(report());
    assert.equal(await loading, '');
    assert.doesNotThrow(() => ui.bind({ querySelector: () => ({ dataset: { financeReport: '1' }, querySelector: () => { throw new Error('stale report was bound'); } }) })());
  }
});

test('a superseded request and a destroyed unmounted report are discarded', async () => {
  let resolve, calls = 0;
  const first = new Promise(done => { resolve = done; });
  const ui = fixture(() => ++calls === 1 ? first : Promise.resolve(report()), 'from=2026-10-22&to=2026-10-22');
  const earlier = ui.overviewPage();
  const current = await ui.overviewPage();
  assert.match(current, /data-finance-report="2"/);
  resolve(report());
  assert.equal(await earlier, '');
  ui.destroy();
  assert.doesNotThrow(() => ui.bind({ querySelector: () => ({ dataset: { financeReport: '2' }, querySelector: () => { throw new Error('destroyed report was retained'); } }) })());
});
