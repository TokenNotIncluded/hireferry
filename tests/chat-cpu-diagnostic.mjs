import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {readFile, writeFile} from 'node:fs/promises';
import {cpus} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {createNavigationHarness, fixtureUser} from './helpers/navigation-harness.mjs';

// Optional CPU diagnostic, deliberately separate from correctness tests.
// Prepare a baseline with `git archive REF static`, then run:
// node tests/chat-cpu-diagnostic.mjs BASELINE/static [output.json] [samples>=5]
const repository = fileURLToPath(new URL('../', import.meta.url));
const baseline = resolve(process.argv[2] || join(repository, '../qa/chat-cpu-baseline/static'));
const output = resolve(process.argv[3] || join(repository, 'docs/validation/chat-cpu.json'));
const samples = Math.max(5, Math.min(30, Number(process.argv[4]) || 5));
const baselineCommit = (await readFile(resolve(baseline, '../source-commit.txt'), 'utf8').catch(() => 'unspecified; identify baseline by source hashes')).trim();
const files = ['app.js', 'service-ui.js', 'money.js', 'navigation.js', 'humangrant-ui.js', 'requests.js', 'cli-auth-return.js'];
const variants = [['baseline', baseline], ['revised', join(repository, 'static')]];
const sha256 = source => createHash('sha256').update(source).digest('hex');
const describe = values => {
  const sorted = [...values].sort((a, b) => a - b);
  return {median: +sorted[Math.floor(sorted.length / 2)].toFixed(3), min: +sorted[0].toFixed(3), max: +sorted.at(-1).toFixed(3)};
};

function observeDateFormatting() {
  const Original = Intl.DateTimeFormat;
  const originalLocale = Date.prototype.toLocaleString, originalDay = Date.prototype.toLocaleDateString;
  const counts = {nativeLocaleStringCalls: 0, nativeLocaleDateStringCalls: 0, explicitIntlConstructions: 0, cachedIntlFormatCalls: 0, formatMs: 0, explicitConstructionMs: 0};
  const time = (key, callback) => { const began = performance.now(); try { return callback(); } finally { counts[key] += performance.now() - began; } };
  Date.prototype.toLocaleString = function(...args) { counts.nativeLocaleStringCalls++; return time('formatMs', () => originalLocale.apply(this, args)); };
  Date.prototype.toLocaleDateString = function(...args) { counts.nativeLocaleDateStringCalls++; return time('formatMs', () => originalDay.apply(this, args)); };
  Intl.DateTimeFormat = new Proxy(Original, {
    construct(target, args) {
      counts.explicitIntlConstructions++;
      const instance = time('explicitConstructionMs', () => Reflect.construct(target, args));
      const format = value => { counts.cachedIntlFormatCalls++; return time('formatMs', () => instance.format(value)); };
      return new Proxy(instance, {get(object, property) {
        if (property === 'format') return format;
        const value = Reflect.get(object, property, object);
        return typeof value === 'function' ? value.bind(object) : value;
      }});
    },
  });
  return {counts, restore() { Intl.DateTimeFormat = Original; Date.prototype.toLocaleString = originalLocale; Date.prototype.toLocaleDateString = originalDay; }};
}

async function measure(directory, conversationCount) {
  const harness = await createNavigationHarness(directory, {latencyMs: 0});
  let observation;
  try {
    await harness.prime();
    const rooms = Array.from({length: conversationCount}, (_, index) => ({
      id: index ? `match_fixture_${index}` : 'platform_fixture-user', kind: index ? 'matched' : 'platform',
      title: index ? `Introduction ${index}` : 'HireFerry team', user_id: fixtureUser.id, writable: true,
      participants: [fixtureUser, {id: `counterpart-${index}`, name: `Counterpart ${index}`}],
      unread_count: index % 4, last_message_at: 1790000000 + index,
      last_message_preview: `Synthetic conversation preview ${index}`,
    }));
    const messages = Array.from({length: 100}, (_, index) => ({
      id: index + 1, sender_kind: index % 2 ? 'user' : 'platform', sender_id: index % 2 ? fixtureUser.id : null,
      sender_name: index % 2 ? fixtureUser.name : 'HireFerry',
      body: `CPU_FIXTURE_MESSAGE_${index}`, body_html: `<p>CPU_FIXTURE_MESSAGE_${index}</p>`, created_at: 1790000000 + index,
    }));
    harness.overrides.set('/api/chat/conversations', {items: rooms});
    harness.overrides.set('/api/chat/conversations/platform_fixture-user/messages?limit=100', {items: messages, next_cursor: 100, first_cursor: 1, has_older: false, has_more: false});
    observation = observeDateFormatting();
    const route = await harness.route('/app?tab=messages', '<p>CPU_FIXTURE_MESSAGE_99</p>');
    observation.restore();
    const final = harness.mounts.findLast(mount => !mount.busy);
    const requiredReads = route.requests.filter(request => request.method === 'GET' && request.path.startsWith('/api/chat/'));
    assert.equal(requiredReads.length, 2, 'both variants must use the same authorized index + history reads');
    assert.equal((final.html.match(/class="chat-message /g) || []).length, 100);
    assert.equal((final.html.match(/class="conversation-item /g) || []).length, conversationCount);
    return {
      routeMs: route.contentMs,
      afterLastChatReadToHtmlMs: route.contentMs - Math.max(...requiredReads.map(request => request.finishedMs)),
      formatting: observation.counts,
      htmlBytes: Buffer.byteLength(final.html),
      openingTags: (final.html.match(/<[a-z][\w:-]*(?=[\s>])/gi) || []).length,
      htmlPublications: harness.mounts.length,
      submittedBytes: harness.mounts.reduce((total, mount) => total + Buffer.byteLength(mount.html), 0),
      requiredChatRequests: requiredReads.length,
    };
  } finally { observation?.restore(); harness.dispose(); }
}

{
  const report = {
    generatedAt: new Date().toISOString(), baselineCommit, revisedRef: process.env.HIREFERRY_CPU_REVISED_REF || 'working tree; identify exact source by hashes',
    environment: {node: process.version, v8: process.versions.v8, icu: process.versions.icu, platform: process.platform, arch: process.arch, cpu: cpus()[0]?.model, timeZone: new Intl.DateTimeFormat().resolvedOptions().timeZone},
    samples, warmupsPerVariantAndSize: 1, conversationCounts: [20, 200], messagesPerConversationView: 100,
    scope: 'Actual app/service JavaScript with synthetic fixtures and a minimal HTML publication adapter; requested fetch delay is 0 ms.',
    limitations: [
      'No browser, HTML parser, CSS, DOM reconciliation, layout, paint, input latency, transfer, or production backend timing is measured.',
      'Date instrumentation adds overhead to both variants; values are diagnostics for this runtime and fixture, not device or production guarantees.',
      'The after-last-chat-read metric includes response cloning, promise scheduling and page HTML generation/publication. Zero-delay timers still contribute to total route time.',
      'Explicit Intl constructor calls are counted separately from format calls. Native Date locale methods may construct internal formatters, but those internal constructions are not observable or counted here.',
      'The baseline is the first workspace redesign at f0c8127; revised app HTML also contains later upstream palette/checkout changes. This is a chat-route generation comparison, not a framework benchmark.',
      'Both samples are fresh workspace page renders with primed identity. Same-room fast navigation and keyed polling mutations are validated by correctness tests and are outside this CPU timing.',
    ], sourceHashes: {}, variants: {}, summary: {},
    harnessSha256: sha256(await readFile(new URL('./helpers/navigation-harness.mjs', import.meta.url))),
    diagnosticSha256: sha256(await readFile(fileURLToPath(import.meta.url))),
  };
  for (const [name, directory] of variants) report.sourceHashes[name] = Object.fromEntries(await Promise.all(files.map(async file => [file, sha256(await readFile(join(directory, file)))])));
  for (const count of report.conversationCounts) {
    for (const [, directory] of variants) await measure(directory, count);
    for (let sample = 0; sample < samples; sample++) {
      for (const [name, directory] of sample % 2 ? [...variants].reverse() : variants) {
        report.variants[name] ||= {}; (report.variants[name][count] ||= []).push(await measure(directory, count));
      }
    }
  }
  for (const [name, sizes] of Object.entries(report.variants)) {
    report.summary[name] = {};
    for (const [count, results] of Object.entries(sizes)) {
      const metrics = {routeMs: result => result.routeMs, afterLastChatReadToHtmlMs: result => result.afterLastChatReadToHtmlMs, dateFormatMs: result => result.formatting.formatMs, explicitDateConstructionMs: result => result.formatting.explicitConstructionMs};
      report.summary[name][count] = {
        ...Object.fromEntries(Object.entries(metrics).map(([key, value]) => [key, describe(results.map(value))])),
        formattingCounts: Object.fromEntries(['nativeLocaleStringCalls', 'nativeLocaleDateStringCalls', 'explicitIntlConstructions', 'cachedIntlFormatCalls'].map(key => [key, [...new Set(results.map(result => result.formatting[key]))]])),
        output: Object.fromEntries(['htmlBytes', 'openingTags', 'htmlPublications', 'submittedBytes', 'requiredChatRequests'].map(key => [key, [...new Set(results.map(result => result[key]))]])),
      };
    }
  }
  await writeFile(output, JSON.stringify(report, null, 2) + '\n');
  console.log(JSON.stringify({output, baselineCommit, revisedRef: report.revisedRef, samples, summary: report.summary}, null, 2));
}
