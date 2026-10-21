import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = await readFile(new URL('../static/money.js', import.meta.url), 'utf8');
const { currencyMeta, currencyOptions, minorToDecimal, parseMinor, minorStep, formatMinor, calculateFee } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const config = { pricing: { currency: 'USD', currency_decimals: 2, allowed_currencies: ['USD', 'JPY'], currency_options: [{ currency: 'USD', currency_decimals: 2, publish_fixed_minor: 199 }, { currency: 'JPY', currency_decimals: 0, publish_fixed_minor: 350 }] } };

test('record currency and precision survive a different site default', () => {
  assert.deepEqual(currencyMeta(config), { currency: 'USD', currency_decimals: 2 });
  assert.deepEqual(currencyMeta(config, { salary_currency: 'JPY', currency_decimals: 0 }), { currency: 'JPY', currency_decimals: 0 });
  assert.deepEqual(currencyMeta(config, { currency: 'EUR', currency_decimals: 2 }), { currency: 'EUR', currency_decimals: 2 });
  assert.equal(currencyOptions(config)[1].publish_fixed_minor, 350);
  assert.match(formatMinor(350, currencyMeta(config, 'JPY')), /JPY\s*350$/);
  assert.match(formatMinor(-199, currencyMeta(config, 'USD')), /-USD\s*1\.99$/);
});

test('decimal input converts exactly to minor units without floating rounding', () => {
  assert.equal(parseMinor('0.29', 2), 29);
  assert.equal(parseMinor('00012.01', 2), 1201);
  assert.equal(parseMinor('90071992547409.91', 2), Number.MAX_SAFE_INTEGER);
  assert.equal(minorToDecimal(Number.MAX_SAFE_INTEGER, 2), '90071992547409.91');
  assert.equal(minorToDecimal(-1, 2), '-0.01');
  assert.equal(minorStep(0), '1');
  assert.equal(minorStep(2), '0.01');
});

test('zero-decimal currencies and amount limits reject ambiguous inputs', () => {
  assert.equal(parseMinor('3000', 0, { min: 1, max: 100000000 }), 3000);
  for (const value of ['1.0', '0.01', '1e3', '-1', '1,000', '', '.5', '  ']) assert.throws(() => parseMinor(value, 0));
  assert.throws(() => parseMinor('0', 2, { min: 1 }));
  assert.throws(() => parseMinor('1.001', 2));
  assert.throws(() => parseMinor('90071992547409.92', 2));
  assert.throws(() => parseMinor('100000001', 0, { max: 100000000 }));
  assert.equal(parseMinor('0', 2), 0);
});

test('free, fixed, and percentage fee quotes use integer rounding', () => {
  assert.equal(calculateFee(600000, { mode: 'free', bps: 1000, fixed_minor: 199 }), 0);
  assert.equal(calculateFee(600000, { mode: 'fixed', fixed_minor: 199 }), 199);
  assert.equal(calculateFee(29, { mode: 'salary_percent', bps: 5000 }), 15);
  assert.equal(calculateFee(600000, { mode: 'salary_percent', bps: 1000 }), 60000);
  assert.throws(() => calculateFee(1, { mode: 'unknown' }));
  assert.throws(() => calculateFee(1, { mode: 'salary_percent', bps: 1 }), /below one minor unit/);
});
