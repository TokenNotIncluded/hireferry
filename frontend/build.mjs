import { build } from 'esbuild';
import { readFile, writeFile, mkdir, readdir } from 'node:fs/promises';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const require = createRequire(import.meta.url);
const check = process.argv.includes('--check');
const outputs = new Map();
const common = { absWorkingDir: root, bundle: true, write: false, legalComments: 'external', define: { 'process.env.NODE_ENV': '"production"' }, logLevel: 'warning' };
const bundled = await build({ ...common, entryPoints: ['frontend/renderer.jsx'], outfile: 'static/chat-renderer.js', platform: 'browser', format: 'esm', target: ['es2022'], minify: true, metafile: true });
for (const file of bundled.outputFiles) outputs.set(path.relative(root, file.path), file.contents);

// Preserve the full copyright/license text of every dependency bundled into JS.
const packageRoots = new Set();
for (const input of Object.keys(bundled.metafile.inputs)) {
  const parts = input.split('/');
  const index = parts.lastIndexOf('node_modules');
  if (index < 0) continue;
  packageRoots.add(parts.slice(0, index + (parts[index + 1].startsWith('@') ? 3 : 2)).join('/'));
}
const notices = ['HireFerry chat renderer: bundled third-party software notices.\n'];
notices.push('\n=== Open Intelligent UI: compatible component signatures ===\n', await readFile(path.join(root, 'frontend/licenses/open-intelligent-ui-LICENSE'), 'utf8'));
for (const relative of [...packageRoots].sort()) {
  const directory = path.join(root, relative);
  const pkg = JSON.parse(await readFile(path.join(directory, 'package.json'), 'utf8'));
  const names = (await readdir(directory)).filter((name) => /^(?:licen[cs]e|copying)(?:\.[\w-]+)?$/iu.test(name));
  notices.push(`\n=== ${pkg.name} ${pkg.version} (${pkg.license || 'see license below'}) ===\n`);
  if (!names.length) {
    // These npm packages omit their monorepo's root license in their tarballs.
    if (!['remark-math', 'rehype-katex'].includes(pkg.name)) throw new Error(`Missing bundled license: ${relative}`);
    notices.push(await readFile(path.join(root, 'frontend/licenses/remark-math-LICENSE'), 'utf8'));
  } else for (const name of names.sort()) notices.push(await readFile(path.join(directory, name), 'utf8'));
}
outputs.set('static/chat-renderer-NOTICES.txt', Buffer.from(notices.join('\n')));

// A private Node bundle generates the exact same library contract as the browser.
// It is evaluated locally by the build only; never served or executed from messages.
const specBundle = await build({ ...common, entryPoints: ['frontend/library.jsx'], platform: 'node', format: 'cjs', packages: 'external', legalComments: 'none' });
const module = { exports: {} };
new Function('module', 'exports', 'require', specBundle.outputFiles[0].text)(module, module.exports, require);
const { library, example, promptOptions } = module.exports;
const metadata = {
  version: 1,
  library: 'hireferry@1',
  language: 'OpenUI Lang 1.0',
  upstream: 'https://github.com/thesysdev/open-intelligent-ui',
  compatibility: 'Documented subset; use only components and signatures in this contract.',
  components: Object.keys(library.components),
  formats: ['plain', 'markdown', 'openui'],
  actions: { ToAssistant: 'Prepares a user-reviewed draft; never auto-sends.', Set: 'Local state only.', Reset: 'Local state only.' },
  restrictions: ['No arbitrary HTML/JavaScript/iframes', 'No automatic Query or Mutation network calls', 'Remote images never load automatically', 'Chart supports unstacked bar, line and area'],
  limits: { messageCharacters: 10000, rendererCodeUnits: 65536, tableRows: 200, chartRows: 100, chartSeries: 6 },
  example,
  spec: library.toSpec(),
  schema: library.toJSONSchema(),
  prompt: library.prompt(promptOptions),
};
outputs.set('static/chat-openui.json', Buffer.from(`${JSON.stringify(metadata, null, 2)}\n`));

// Keep KaTeX fonts local. WOFF2 covers supported browsers; remove unused fallbacks.
let katexCss = await readFile(path.join(root, 'node_modules/katex/dist/katex.min.css'), 'utf8');
katexCss = katexCss.replace(/src:url\(fonts\/([^)]*\.woff2)\) format\("woff2"\)[^;}]*/gu, 'src:url(chat-renderer-fonts/$1) format("woff2")');
const css = `${katexCss}\n${await readFile(path.join(root, 'frontend/chat-renderer.css'), 'utf8')}`;
outputs.set('static/chat-renderer.css', Buffer.from(css));
for (const name of (await readdir(path.join(root, 'node_modules/katex/dist/fonts'))).filter((name) => name.endsWith('.woff2'))) {
  outputs.set(`static/chat-renderer-fonts/${name}`, await readFile(path.join(root, 'node_modules/katex/dist/fonts', name)));
}

let mismatched = false;
for (const [relative, content] of outputs) {
  const target = path.join(root, relative);
  if (check) {
    const existing = await readFile(target).catch(() => null);
    if (!existing || !Buffer.from(content).equals(existing)) { console.error(`Outdated generated asset: ${relative}`); mismatched = true; }
  } else {
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, content);
  }
}
if (mismatched) process.exitCode = 1;
else {
  const bytes = [...outputs.values()].reduce((total, value) => total + value.length, 0);
  console.log(`${check ? 'Verified' : 'Built'} ${outputs.size} chat assets (${Math.round(bytes / 1024)} KiB, self-hosted).`);
}
