import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createNavigationHarness } from './helpers/navigation-harness.mjs';

const directory = fileURLToPath(new URL('../static/', import.meta.url));
const keyword = 'R&D "Rust"';
const query = new URLSearchParams({ q: keyword }).toString();
const role = { id: 'role-fixture', title: 'Rust platform engineer', employer: { name: 'Fixture team' }, salary_cents: 2400000, currency: 'CNY', headcount: 2, created_at: 1800000000, updated_at: 1800000000, jd_html: '<p>Build reliable developer tools.</p>' };
const person = { handle: 'fixture-person', name: 'Fixture candidate', resume_html: '<p>Rust and distributed systems.</p>', ai_profile_links: {}, created_at: 1800000000 };
const links = html => [...html.matchAll(/href="([^"]*)"/g)].map(match => match[1].replaceAll('&amp;', '&'));
async function settle(h) {
  for (let i = 0; i < 100; i++) {
    await new Promise(resolve => setTimeout(resolve, 1));
    if (h.network.every(request => request.finished !== null)) return;
  }
  assert.fail('Public page requests did not settle');
}

test('switching directory categories keeps the keyword and starts at the first page', async () => {
  const h = await createNavigationHarness(directory);
  h.setUser(null);
  h.overrides.set(`/api/public/jobs?${query}&page=3`, { items: [role], total: 120, limit: 50 });
  h.overrides.set(`/api/public/people?${query}`, { items: [person], total: 1, limit: 50 });
  try {
    await h.app.go(`/discover?tab=jobs&${query}&page=3`);
    const target = links(h.root.innerHTML).find(href => href.startsWith('/discover?tab=people'));
    assert.ok(target, 'The public profile category is reachable');
    const params = new URL(target, 'https://hireferry.test').searchParams;
    assert.equal(params.get('q'), keyword);
    assert.equal(params.has('page'), false);
    await h.app.go(target);
    assert.ok(h.network.some(request => request.path === `/api/public/people?${query}`));
    assert.match(h.root.innerHTML, /Fixture candidate/);
    assert.match(h.root.innerHTML, /value="R&amp;D &quot;Rust&quot;"/);
  } finally { await settle(h); h.dispose(); }
});

for (const [tab, item, id] of [['jobs', role, role.id], ['people', person, person.handle]]) {
  test(`${tab} detail returns to the same keyword and results page`, async () => {
    const h = await createNavigationHarness(directory);
    h.setUser(null);
    const endpoint = `/api/public/${tab}?${query}&page=2`;
    h.overrides.set(endpoint, { items: [item], total: 80, limit: 50 });
    h.overrides.set(`/api/public/${tab}/${id}`, item);
    try {
      await h.app.go(`/discover?tab=${tab}&${query}&page=2`);
      const detail = links(h.root.innerHTML).find(href => href.startsWith(`/discover/${tab}/${id}?`));
      assert.ok(detail, 'The result carries its browsing context into the detail page');
      await h.app.go(detail);
      const back = links(h.root.innerHTML).find(href => href.startsWith(`/discover?tab=${tab}`));
      assert.ok(back, 'The detail offers a directory return link');
      const params = new URL(back, 'https://hireferry.test').searchParams;
      assert.equal(params.get('q'), keyword);
      assert.equal(params.get('page'), '2');
      await h.app.go(back);
      assert.equal(h.network.filter(request => request.path === endpoint).length, 2);
      assert.doesNotMatch(h.root.innerHTML, /route-error/);
    } finally { await settle(h); h.dispose(); }
  });

  test(`${tab} unavailable detail returns to the same keyword and results page`, async () => {
    const h = await createNavigationHarness(directory);
    h.setUser(null);
    const endpoint = `/api/public/${tab}?${query}&page=2`;
    h.overrides.set(endpoint, { items: [item], total: 80, limit: 50 });
    h.overrides.set(`/api/public/${tab}/${id}`, { httpStatus: 404, body: { error: 'Not found' } });
    try {
      await h.app.go(`/discover?tab=${tab}&${query}&page=2`);
      const detail = links(h.root.innerHTML).find(href => href.startsWith(`/discover/${tab}/${id}?`));
      assert.ok(detail, 'The result carries its browsing context into the detail page');
      await h.app.go(detail);
      assert.match(h.root.innerHTML, /public-route-error/);
      const back = links(h.root.innerHTML).find(href => href.startsWith(`/discover?tab=${tab}`));
      assert.ok(back, 'Unavailable public content offers a directory return link');
      const params = new URL(back, 'https://hireferry.test').searchParams;
      assert.equal(params.get('q'), keyword);
      assert.equal(params.get('page'), '2');
      assert.ok(h.root.innerHTML.includes(`href="${back.replaceAll('&', '&amp;')}"`), 'Return query separators are HTML-escaped');
      await h.app.go(back);
      assert.equal(h.network.filter(request => request.path === endpoint).length, 2);
      assert.doesNotMatch(h.root.innerHTML, /route-error/);
    } finally { await settle(h); h.dispose(); }
  });
}
