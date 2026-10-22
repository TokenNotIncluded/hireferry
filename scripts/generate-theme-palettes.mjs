// Paper and ink use hand-tuned neutral roles. Optional accents use Material
// Color Utilities with the same surfaces; no runtime dependency is shipped.
import {writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
const library = process.argv[2] ? pathToFileURL(resolve(process.argv[2])).href : '@material/material-color-utilities';
const {Hct,SchemeVibrant,argbFromHex,hexFromArgb} = await import(library);
const seeds = {neutral:'#252420',blue:'#1a73e8',violet:'#7c3aed',rose:'#c2185b',amber:'#d97706',teal:'#0d9488'};
const surfaces = {
  light: {surface:'#f8f7f3',surfaceContainerLowest:'#fdfcf9',surfaceContainerLow:'#f1f0eb',surfaceContainer:'#ebeae4',surfaceContainerHigh:'#e4e3dc',onSurface:'#252420',onSurfaceVariant:'#65635c',outline:'#817e74',outlineVariant:'#d7d4cb'},
  dark: {surface:'#171715',surfaceContainerLowest:'#111210',surfaceContainerLow:'#1f201d',surfaceContainer:'#262722',surfaceContainerHigh:'#30312b',onSurface:'#eeeae2',onSurfaceVariant:'#b8b5aa',outline:'#88877b',outlineVariant:'#42433b'},
};
const paperInk = {
  light: {primary:'#252420',onPrimary:'#f8f7f3',primaryContainer:'#e7e5dd',onPrimaryContainer:'#36352e',secondary:'#5c5c50',secondaryContainer:'#e8e7de',onSecondaryContainer:'#3f4035',tertiary:'#68614c',tertiaryContainer:'#eae4d5',onTertiaryContainer:'#494333'},
  dark: {primary:'#eeeae2',onPrimary:'#252420',primaryContainer:'#373831',onPrimaryContainer:'#eeeae2',secondary:'#cfcdc0',secondaryContainer:'#3b3d33',onSecondaryContainer:'#e9e7da',tertiary:'#d8ceaf',tertiaryContainer:'#433e2e',onTertiaryContainer:'#ede2c3'},
};
const roles = ['primary','onPrimary','primaryContainer','onPrimaryContainer','secondary','secondaryContainer','onSecondaryContainer','tertiary','tertiaryContainer','onTertiaryContainer','surface','surfaceContainerLowest','surfaceContainerLow','surfaceContainer','surfaceContainerHigh','onSurface','onSurfaceVariant','outline','outlineVariant','error','errorContainer','onErrorContainer'];
const palettes = {};
let css = '/* Paper & ink defaults with Material accent palettes, contrast 0.\n * Accent generator: @material/material-color-utilities 0.3.0, SchemeVibrant.\n * https://github.com/material-foundation/material-color-utilities\n * Regenerate: node scripts/generate-theme-palettes.mjs [path-to-library-index.js] */\n';
for (const [name,seed] of Object.entries(seeds)) {
  palettes[name] = {seed};
  for (const dark of [false,true]) {
    const scheme = new SchemeVibrant(Hct.fromInt(argbFromHex(seed)),dark,0);
    const mode = dark ? 'dark' : 'light';
    palettes[name][mode] = {...Object.fromEntries(roles.map(role=>[role,hexFromArgb(scheme[role])])),...surfaces[mode],...(name==='neutral'?paperInk[mode]:{})};
    const selector = `${name==='neutral'?(dark?':root[data-theme="dark"], ':':root, '):''}:root[data-palette="${name}"][data-theme="${mode}"]`;
    css += `${selector} {\n` + roles.map(role=>`  --md-${role.replace(/[A-Z]/g,c=>'-'+c.toLowerCase())}: ${palettes[name][mode][role]};`).join('\n') + '\n}\n';
  }
}
for (const mode of ['light','dark']) {
  css += `${mode==='light'?':root':':root[data-theme="dark"]'} {\n`+Object.entries(palettes).map(([name,p])=>`  --palette-${name}: ${p[mode].primary};\n  --palette-${name}-ink: ${p[mode].onPrimary};`).join('\n')+'\n}\n';
}
writeFileSync('static/theme-palettes.css',css);
writeFileSync('static/theme-palettes.json',JSON.stringify({generator:'@material/material-color-utilities@0.3.0',variant:'SchemeVibrant with paper and ink neutrals',contrast:0,palettes},null,2)+'\n');
console.log(Object.fromEntries(Object.entries(palettes).map(([name,p])=>[name,{light:p.light.primary,dark:p.dark.primary,surface:p.light.surface}])));
