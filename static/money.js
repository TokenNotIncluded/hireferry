/* Amounts in the API are integer minor units, including fields named *_cents. */
const validDecimals = value => Number.isInteger(value) && value >= 0 && value <= 6;

export function currencyMeta(config = {}, record = null) {
  const pricing = config?.pricing || config || {};
  const currency = String(typeof record === 'string' ? record : record?.currency || record?.salary_currency || pricing.currency || 'CNY').toUpperCase();
  const option = pricing.currency_options?.find(item => item.currency === currency);
  let decimals = typeof record === 'object' && record !== null ? record.currency_decimals : undefined;
  if (!validDecimals(decimals)) decimals = option?.currency_decimals;
  if (!validDecimals(decimals) && currency === pricing.currency) decimals = pricing.currency_decimals;
  if (!validDecimals(decimals)) decimals = new Intl.NumberFormat('en', { style: 'currency', currency }).resolvedOptions().maximumFractionDigits;
  return { currency, currency_decimals: decimals };
}

export function currencyOptions(config = {}) {
  const pricing = config?.pricing || config || {};
  const allowed = pricing.allowed_currencies?.length ? pricing.allowed_currencies : [currencyMeta(config).currency];
  return [...new Set(allowed)].map(currency => ({ ...pricing.currency_options?.find(item => item.currency === currency), ...currencyMeta(config, currency) }));
}

function exponent(decimals) {
  if (!validDecimals(decimals)) throw new RangeError('Invalid currency precision');
  return 10n ** BigInt(decimals);
}

function integerAmount(value) {
  if (typeof value === 'number' && !Number.isSafeInteger(value)) throw new RangeError('Amount is outside the safe integer range');
  if (!/^-?\d+$/.test(String(value))) throw new TypeError('Amount must be integer minor units');
  return BigInt(value);
}

export function minorToDecimal(value, decimals) {
  const units = integerAmount(value), scale = exponent(decimals), absolute = units < 0n ? -units : units;
  return `${units < 0n ? '-' : ''}${absolute / scale}${decimals ? '.' + String(absolute % scale).padStart(decimals, '0') : ''}`;
}

export function parseMinor(value, decimals, { min = 0, max = Number.MAX_SAFE_INTEGER } = {}) {
  const scale = exponent(decimals), text = String(value ?? '').trim();
  if (!/^\d+(?:\.\d+)?$/.test(text)) throw new TypeError('Enter a non-negative decimal amount');
  const [whole, fraction = ''] = text.split('.');
  if (fraction.length > decimals) throw new RangeError('Too many decimal places for this currency');
  const units = BigInt(whole) * scale + BigInt(fraction.padEnd(decimals, '0') || '0');
  if (units < integerAmount(min) || units > integerAmount(max) || units > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError('Amount is outside the allowed range');
  return Number(units);
}

export function minorStep(decimals) {
  exponent(decimals);
  return decimals ? `0.${'0'.repeat(decimals - 1)}1` : '1';
}

export function formatMinor(value, meta, locale = 'en-US') {
  const amount = minorToDecimal(value ?? 0, meta.currency_decimals);
  return new Intl.NumberFormat(locale, { style: 'currency', currency: meta.currency, currencyDisplay: 'code', minimumFractionDigits: meta.currency_decimals, maximumFractionDigits: meta.currency_decimals }).format(amount);
}

export function calculateFee(salaryMinor, { mode, bps = 0, fixed_minor = 0, discount_percent = 0 }) {
  const salary = integerAmount(salaryMinor);
  if (salary < 0n) throw new RangeError('Salary must not be negative');
  let result;
  if (mode === 'free') result = 0n;
  else if (mode === 'fixed') result = integerAmount(fixed_minor);
  else if (mode === 'salary_percent') result = (salary * integerAmount(bps) + 5000n) / 10000n;
  else throw new TypeError('Unknown fee mode');
  const discount = integerAmount(discount_percent);
  if (discount < 0n || discount > 100n) throw new RangeError('Invalid discount percent');
  if (discount === 100n) result = 0n;
  else if (discount > 0n) result = (result * (100n - discount) + 50n) / 100n;
  if (mode !== 'free' && discount !== 100n && result === 0n) throw new RangeError('Fee below one minor unit');
  if (result < 0n || result > BigInt(Number.MAX_SAFE_INTEGER)) throw new RangeError('Fee is outside the allowed range');
  return Number(result);
}
