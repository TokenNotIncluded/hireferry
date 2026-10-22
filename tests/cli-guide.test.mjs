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
  assert.match(commands.globalSkill,/ -g$/);
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
    assert.doesNotMatch(html,/data-form="email-start"|client_secret/);
  }
});
