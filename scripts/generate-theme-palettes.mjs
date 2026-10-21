// Google Material Color Utilities supplies the role pairs; no runtime dependency.
import {writeFileSync} from 'node:fs';
import {resolve} from 'node:path';
import {pathToFileURL} from 'node:url';
const library = process.argv[2] ? pathToFileURL(resolve(process.argv[2])).href : '@material/material-color-utilities';
const {Hct,SchemeVibrant,argbFromHex,hexFromArgb} = await import(library);
const seeds = {blue:'#1a73e8',violet:'#7c3aed',rose:'#c2185b'};
const roles = ['primary','onPrimary','primaryContainer','onPrimaryContainer','secondary','secondaryContainer','onSecondaryContainer','tertiary','tertiaryContainer','onTertiaryContainer','surface','surfaceContainerLowest','surfaceContainerLow','surfaceContainer','surfaceContainerHigh','onSurface','onSurfaceVariant','outline','outlineVariant','error','errorContainer','onErrorContainer'];
const palettes = {};
let css = '/* Generated with @material/material-color-utilities 0.3.0, SchemeVibrant, contrast 0.\n * https://github.com/material-foundation/material-color-utilities\n * Regenerate: node scripts/generate-theme-palettes.mjs [path-to-library-index.js] */\n';
for (const [name,seed] of Object.entries(seeds)) {
  palettes[name] = {seed};
  for (const dark of [false,true]) {
    const scheme = new SchemeVibrant(Hct.fromInt(argbFromHex(seed)),dark,0);
    const mode = dark ? 'dark' : 'light';
    palettes[name][mode] = Object.fromEntries(roles.map(role=>[role,hexFromArgb(scheme[role])]));
    const selector = `${name==='blue'&&!dark?':root, ':''}:root[data-palette="${name}"][data-theme="${mode}"]`;
    css += `${selector} {\n` + roles.map(role=>`  --md-${role.replace(/[A-Z]/g,c=>'-'+c.toLowerCase())}: ${palettes[name][mode][role]};`).join('\n') + '\n}\n';
  }
}
writeFileSync('static/theme-palettes.css',css);
writeFileSync('static/theme-palettes.json',JSON.stringify({generator:'@material/material-color-utilities@0.3.0',variant:'SchemeVibrant',contrast:0,palettes},null,2)+'\n');
console.log(Object.fromEntries(Object.entries(palettes).map(([name,p])=>[name,{light:p.light.primary,dark:p.dark.primary,surface:p.light.surface}])));
