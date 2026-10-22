import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
const load = async name => import('data:text/javascript;base64,' + Buffer.from(await readFile(new URL(`../static/${name}`, import.meta.url), 'utf8')).toString('base64'));
const {effectiveOrderStatus, activationRecovery, createBillingUI} = await load('billing-ui.js');
const {formatMinor} = await load('money.js');
const employer = {role:'employer', employer_paid:false};
const order = (changes = {}) => ({id:'activation-one',kind:'onboarding',status:'pending',currency:'JPY',currency_decimals:0,amount_cents:1200,refunded_cents:0,provider:'stripe',live_mode:false,created_at:100,checkout_expires_at:Math.floor(Date.now()/1000)+3600,...changes});
function page(user, orders) {
  const esc = value => String(value??'').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
  return createBillingUI({state:{user,lang:'en'},t:(_zh,en)=>en,esc,icon:()=>'',money:(minor,record)=>formatMinor(minor,record,'en-US'),date:value=>String(value),kindText:kind=>kind,providerText:value=>value,badge:value=>`<span>${esc(value)}</span>`,orderRefundDetails:()=>'',empty:(title,body)=>`<p>${title} ${body}</p>`,contactMailto:()=> 'mailto:support@example.test'}).ordersContent(orders);
}

test('a stale pending activation link becomes a direct, visible restart action', () => {
  const expired = order({checkout_expires_at:1,checkout_url:'https://checkout.stripe.com/stale'});
  const html = page(employer,[expired]);
  assert.equal(effectiveOrderStatus(expired), 'expired');
  assert.equal(activationRecovery(employer,[expired]).kind, 'retry');
  assert.match(html,/Restart activation/);
  assert.doesNotMatch(html,/data-action="pay-order"|https:\/\/checkout/);
  assert.ok(html.indexOf('Restart activation') < html.indexOf('<details'));
});

test('an existing valid order keeps its currency, precision and one continuation action', () => {
  const valid = order();
  const html = page(employer,[valid,order({id:'old',status:'expired',created_at:1})]);
  assert.equal((html.match(/data-action="pay-order"/g)||[]).length,1);
  assert.match(html,/data-id="activation-one"/);
  assert.match(html,/JPY\s*1,200/);
  assert.doesNotMatch(html,/Restart activation|JPY\s*12\.00/);
  assert.match(html,/data-disclosure="orders-history"\s*>/);
});

test('payment already recorded prevents all other activation payment actions', () => {
  for (const status of ['paid','paid_needs_review','partially_refunded']) {
    const html = page(employer,[order(),order({id:'received',status,refunded_cents:500})]);
    assert.match(html,/An activation payment needs review/);
    assert.doesNotMatch(html,/data-action="pay-order"|data-action="role-employer"/);
  }
  assert.equal(activationRecovery(employer,[order({status:'refunded',refunded_cents:1200})]).kind,'retry');
});

test('activated, suspended and seeker accounts are never asked to buy activation again', () => {
  for (const user of [{...employer,employer_paid:true},{...employer,banned:true},{role:'seeker'}]) {
    const html = page(user,[order({status:'expired'})]);
    assert.doesNotMatch(html,/data-action="role-employer"|Restart activation|Activate employer account/);
  }
  assert.doesNotMatch(page({...employer,employer_paid:true},[order()]),/data-action="pay-order"/);
});

test('an empty history still offers activation to an unactivated employer', () => {
  assert.match(page(employer,[]),/data-action="role-employer"/);
  assert.doesNotMatch(page({role:'seeker'},[]),/data-action="role-employer"/);
});

test('only activation expiry is inferred; an absent deadline never invents payment state', () => {
  assert.equal(effectiveOrderStatus(order({checkout_expires_at:null})), 'pending');
  assert.equal(effectiveOrderStatus(order({kind:'publish',checkout_expires_at:1})), 'pending');
  assert.equal(effectiveOrderStatus(order({status:'paid',checkout_expires_at:1})), 'paid');
});

test('older records fold away without hiding review warnings or changing the input order', () => {
  const orders = [order({id:'older',status:'expired',created_at:1}), order({id:'newer',status:'expired',created_at:2})];
  const html = page(employer,orders);
  assert.deepEqual(orders.map(value=>value.id),['older','newer']);
  assert.equal(activationRecovery(employer,orders).order.id,'newer');
  assert.match(html,/data-disclosure="orders-history"\s*>/);
  assert.match(html,/data-disclosure="order-older"/);
  assert.match(html,/data-disclosure="order-newer"/);
});
