import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const source=await readFile(new URL('../static/cli-guide.js',import.meta.url),'utf8');
const {cliCommands,renderCliGuide}=await import('data:text/javascript;base64,'+Buffer.from(source).toString('base64'));
const config={public_origin:'https://hire.example',cli_distribution:{release_repository:'Example/custom-cli',release_tag_prefix:'cli-v'}};
const esc=value=>String(value).replace(/[&<>"']/g,char=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
test('every installation and login command uses deployment configuration',()=>{
  const commands=cliCommands(config,'http://localhost:3117');
  assert.equal(commands.unix,'curl -fsSL https://hire.example/install.sh | sh');
  assert.equal(commands.windows,'irm https://hire.example/install.ps1 | iex');
  assert.equal(commands.skill,'npx skills add Example/custom-cli --skill hireferry');
  assert.equal(commands.windowsSkill,'npx.cmd skills add Example/custom-cli --skill hireferry');
  assert.match(commands.globalSkill,/ -g$/);
  assert.match(commands.globalWindowsSkill,/^npx\.cmd .* -g$/);
  assert.match(commands.oauth,/--server https:\/\/hire.example auth login --oauth$/);
  assert.match(commands.device,/--server https:\/\/hire.example auth login --device$/);
  assert.doesNotMatch(JSON.stringify(commands),/hireferry.lmm.best|TokenNotIncluded/);
});
test('the public guide includes both platform installers, device login, skills and copy controls in both languages',()=>{
  for(const lang of ['zh','en']){
    const html=renderCliGuide({config,origin:config.public_origin,lang,esc,icon:()=>'<svg aria-hidden="true"></svg>'});
    assert.match(html,/Windows/);assert.match(html,/macOS/);assert.match(html,/22\.20/);
    assert.match(html,/data-action="copy-command"/);assert.match(html,/--device/);
    assert.match(html,/href="\/install.ps1"/);assert.match(html,/id="cli-skill"/);
    assert.match(html,/npx skills add Example\/custom-cli --skill hireferry/);
    assert.match(html,/npx\.cmd skills add Example\/custom-cli --skill hireferry/);
    assert.doesNotMatch(html,/data-form="email-start"|client_secret/);
  }
});

test('the terminal demo uses real deployment commands and every copy button retains the complete command',()=>{
  const html=renderCliGuide({config,origin:config.public_origin,lang:'en',esc,icon:()=>'<svg aria-hidden="true"></svg>'});
  assert.match(html,/data-cli-terminal/);
  assert.match(html,/<strong>hfy<\/strong><span data-cli-typed>/);
  assert.match(html,/class="cli-cursor">_<\/span>/);
  for(const command of Object.values(cliCommands(config,config.public_origin)).filter(value=>value.startsWith('hfy '))) {
    assert.ok(html.includes(`data-copy="${esc(command)}"`),`missing full copy text: ${command}`);
    assert.ok(html.includes(esc(command)),`missing accessible command: ${command}`);
  }
  assert.match(html,/data-copy-success="Copied"/);
  assert.match(html,/data-copy-status role="status" aria-live="polite"/);
  assert.match(html,/role="tablist" aria-label="Installation platform"/);
  assert.match(html,/id="cli-tab-windows" role="tab" aria-selected="false" aria-controls="cli-panel-windows" tabindex="-1"/);
  assert.match(html,/id="cli-panel-windows"[^>]+role="tabpanel"[^>]+hidden/);
});


test('command update keys remain unique and stable when the public origin or repository changes',()=>{
  const render=value=>renderCliGuide({config:value,origin:value.public_origin,lang:'en',esc,icon:()=>'<svg aria-hidden="true"></svg>'});
  const keys=html=>[...html.matchAll(/data-refresh-key="([^" ]+)"/g)].map(match=>match[1]);
  const original=keys(render(config));
  const changed=keys(render({...config,public_origin:'https://new.example',cli_distribution:{release_repository:'Example/updated-cli'}}));
  assert.equal(new Set(original).size,9);
  assert.equal(original.length,9);
  assert.deepEqual(changed,original);
  for(const key of ['cli-install-unix','cli-install-windows','cli-auth-browser','cli-auth-device','cli-whoami','cli-capabilities'])assert.ok(original.includes(key));
});
