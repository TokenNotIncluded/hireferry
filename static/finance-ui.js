// The report is calculated by the server over the whole date range, never the current page.
const DAY = 86400000;
const METRICS = ['revenue_cents', 'gross_cents', 'refund_cents', 'orders'];
const dateText = value => new Date(value).toISOString().slice(0, 10);
const timestamp = value => Date.parse(value + (value.length === 7 ? '-01' : '') + 'T00:00:00Z');

export function financeRange(preset, now = new Date()) {
  if (preset === 'all') return { from: '', to: '' };
  const end = timestamp(dateText(now));
  return { from: preset === 'month' ? dateText(end).slice(0, 7) + '-01' : dateText(end - (preset === '7' ? 6 : 29) * DAY), to: dateText(end) };
}

export function financePath(tab, values, changes = {}) {
  const params = new URLSearchParams({ tab });
  for (const key of ['from', 'to', 'currency', 'mode', 'status', 'page']) {
    const value = Object.hasOwn(changes, key) ? changes[key] : values[key];
    if (value) params.set(key, value);
  }
  return '/admin?' + params;
}

export function financeSeries(report, filters = {}, now = new Date()) {
  const rows = [...(report.series || [])].sort((a, b) => a.date.localeCompare(b.date));
  const end = timestamp(filters.to || dateText(now));
  const start = timestamp(filters.from || (rows[0]?.date.length === 7 ? rows[0].date + '-01' : rows[0]?.date) || dateText(end - 6 * DAY));
  if (!Number.isFinite(start) || !Number.isFinite(end) || start > end) throw new RangeError('Invalid reporting dates');
  const first = new Date(start), last = new Date(end);
  const months = (last.getUTCFullYear() - first.getUTCFullYear()) * 12 + last.getUTCMonth() - first.getUTCMonth() + 1;
  const unit = months > 600 ? 'year' : report.bucket === 'month' || end - start > 366 * DAY ? 'month' : 'day';
  const step = unit === 'year' ? Math.ceil((last.getUTCFullYear() - first.getUTCFullYear() + 1) / 600) : 1;
  const startYear = first.getUTCFullYear();
  const keyOf = value => unit === 'year'
    ? String(startYear + Math.floor((Number(value.slice(0, 4)) - startYear) / step) * step).padStart(4, '0')
    : value.slice(0, unit === 'month' ? 7 : 10);
  const grouped = new Map();
  for (const row of rows) {
    const key = keyOf(row.date), item = grouped.get(key) || Object.fromEntries(METRICS.map(metric => [metric, 0]));
    for (const metric of METRICS) {
      const value = Number(row[metric] ?? 0);
      if (!Number.isSafeInteger(value) || !Number.isSafeInteger(item[metric] + value)) throw new RangeError('Reporting amount is outside the safe integer range');
      item[metric] += value;
    }
    grouped.set(key, item);
  }
  const cursor = new Date(start), items = [];
  if (unit !== 'day') cursor.setUTCDate(1);
  if (unit === 'year') cursor.setUTCMonth(0);
  while (cursor.getTime() <= end && items.length <= 600) {
    const key = keyOf(dateText(cursor));
    items.push({ date: key, ...Object.fromEntries(METRICS.map(metric => [metric, 0])), ...grouped.get(key) });
    if (unit === 'year') cursor.setUTCFullYear(cursor.getUTCFullYear() + step);
    else if (unit === 'month') cursor.setUTCMonth(cursor.getUTCMonth() + 1);
    else cursor.setUTCDate(cursor.getUTCDate() + 1);
  }
  return { items, unit, step, from: dateText(start), to: dateText(end) };
}

export function createFinanceUI(ctx) {
  const { api, state, t, esc, money, currency, currencies, qs, shell, icon, orderTable, pagination, go, statusText } = ctx;
  let sequence = 0, pending = null, mountedCleanup = null;
  const identity = () => JSON.stringify([state.csrf, state.user?.id, state.admin, state.owner, state.staff?.id, state.staffRole]);
  const labels = () => ({ revenue_cents: t('净收款', 'Net receipts'), gross_cents: t('总收款', 'Payments'), refund_cents: t('成功退款', 'Refunds'), orders: t('收款笔数', 'Paid orders') });
  const count = value => new Intl.NumberFormat(state.lang === 'zh' ? 'zh-CN' : 'en-US').format(value || 0);
  const metricTotal = (report, metric) => metric === 'orders' ? count(report.stats.paid_orders) : money(report.stats[metric], report);
  const displayDate = value => value.replaceAll('-', '.');
  const rangeLabel = filters => filters.from || filters.to ? `${filters.from ? displayDate(filters.from) : t('最早记录', 'First record')} — ${filters.to ? displayDate(filters.to) : t('今天', 'Today')}` : t('历史全部', 'All time');

  function selectedFilters() {
    const current = qs(), filters = {};
    for (const key of ['from', 'to', 'status', 'page']) if (current.get(key)) filters[key] = current.get(key);
    filters.currency = current.get('currency') || currency().currency;
    filters.mode = current.get('mode') || (state.config?.development ? 'test' : 'live');
    return filters;
  }

  function apiPath(path, filters, paginated = false) {
    const params = new URLSearchParams();
    for (const key of ['from', 'to', 'status', 'currency', 'mode', ...(paginated ? ['page'] : [])]) if (filters[key]) params.set(key, filters[key]);
    return path + '?' + params;
  }

  function filtersMarkup(tab, report, values) {
    const codes = [...new Set([...currencies().map(item => item.currency), ...(report.currency_totals || []).map(item => item.currency), report.currency])];
    const choices = [['7', t('近 7 天', '7 days')], ['30', t('近 30 天', '30 days')], ['month', t('本月', 'This month')], ['all', t('全部', 'All time')]];
    return `<div class="finance-toolbar"><nav class="finance-presets" aria-label="${t('统计时间范围', 'Reporting period')}">${choices.map(([preset, label]) => {
      const range = financeRange(preset), active = (values.from || '') === range.from && (values.to || '') === range.to;
      return `<a href="${esc(financePath(tab, values, { ...range, page: '' }))}" data-nav ${active ? 'aria-current="true"' : ''}>${label}</a>`;
    }).join('')}</nav><form class="finance-filters" data-finance-filter action="/admin" method="get"><input type="hidden" name="tab" value="${tab}"><div class="finance-dates"><label for="finance-from">${t('开始日期 · UTC', 'From · UTC')}<input class="input" id="finance-from" type="date" name="from" value="${esc(values.from || '')}" aria-describedby="finance-timezone"></label><span class="finance-date-separator" aria-hidden="true">—</span><label for="finance-to">${t('结束日期', 'Through')}<input class="input" id="finance-to" type="date" name="to" value="${esc(values.to || '')}" aria-describedby="finance-timezone"></label></div><div class="finance-selects"><label for="finance-currency">${t('币种', 'Currency')}<select class="input" id="finance-currency" name="currency">${codes.map(code => `<option value="${esc(code)}" ${code === values.currency ? 'selected' : ''}>${esc(code)}</option>`).join('')}</select></label><label for="finance-mode">${t('流水', 'Payments')}<select class="input" id="finance-mode" name="mode"><option value="live" ${values.mode === 'live' ? 'selected' : ''}>${t('实收', 'Live')}</option><option value="test" ${values.mode === 'test' ? 'selected' : ''}>${t('测试', 'Test')}</option></select></label>${tab === 'orders' ? `<label for="finance-status">${t('订单状态', 'Order status')}<select class="input" id="finance-status" name="status"><option value="">${t('全部状态', 'All statuses')}</option>${['pending', 'paid', 'paid_needs_review', 'partially_refunded', 'refunded', 'expired', 'failed', 'cancelled'].map(status => `<option value="${status}" ${values.status === status ? 'selected' : ''}>${statusText(status)}</option>`).join('')}</select></label>` : ''}</div><button type="submit" hidden>${t('应用筛选', 'Apply filters')}</button></form></div>`;
  }

  function chartMarkup(report, values, metric = 'revenue_cents', width = 780) {
    const data = financeSeries(report, values), items = data.items, names = labels();
    const w = Math.max(280, Math.min(1200, Math.round(width))), h = w < 500 ? 220 : 256;
    const decimals = currency(report).currency_decimals, scale = metric === 'orders' ? 1 : 10 ** decimals;
    const minimum = Math.min(0, ...items.map(item => item[metric])), maximum = Math.max(0, ...items.map(item => item[metric]));
    const rawStep = Math.max(scale * 4, maximum - minimum) / 4, magnitude = 10 ** Math.floor(Math.log10(rawStep));
    const fraction = rawStep / magnitude, step = Math.max(1, Math.ceil((fraction <= 1 ? 1 : fraction <= 2 ? 2 : fraction <= 5 ? 5 : 10) * magnitude));
    const ymin = Math.floor(minimum / step) * step, ymax = Math.ceil(maximum / step) * step || (minimum === 0 ? 4 * step : 0);
    const ticks = Array.from({ length: Math.round((ymax - ymin) / step) + 1 }, (_, i) => ymin + i * step);
    const axis = value => new Intl.NumberFormat(state.lang === 'zh' ? 'zh-CN' : 'en-US', { notation: Math.abs(value / scale) >= 100000 ? 'compact' : 'standard', maximumFractionDigits: metric === 'orders' ? 0 : Math.min(decimals, 2) }).format(value / scale);
    const left = Math.max(38, Math.min(110, Math.max(...ticks.map(value => axis(value).length * 7 + 12)))), right = 16, top = 22, bottom = 31;
    const x = index => left + (items.length > 1 ? index / (items.length - 1) : .5) * (w - left - right);
    const y = value => top + (h - top - bottom) * (1 - (value - ymin) / (ymax - ymin));
    const points = items.map((item, index) => [x(index), y(item[metric])]);
    const path = points.map(([px, py], i) => `${i ? 'L' : 'M'}${px.toFixed(2)},${py.toFixed(2)}`).join(' ');
    const detail = item => metric === 'orders' ? count(item.orders) : money(item[metric], report);
    const dateLabel = item => data.step > 1 ? `${item.date}–${Math.min(Number(data.to.slice(0, 4)), Number(item.date) + data.step - 1)}` : item.date;
    const labelCount = w < 500 ? 3 : 6;
    const labelIndices = new Set(Array.from({ length: labelCount }, (_, i) => Math.round(i * (items.length - 1) / (labelCount - 1))));
    const unitLabel = data.unit === 'day' ? t('每日', 'Daily') : data.unit === 'month' ? t('每月', 'Monthly') : data.step > 1 ? t(`每 ${data.step} 年`, `Every ${data.step} years`) : t('每年', 'Yearly');
    const empty = !(report.series || []).length;
    return `<div class="finance-chart-meta"><span>${unitLabel} · ${metric === 'orders' ? t('笔', 'orders') : esc(report.currency)}</span><span>${esc(displayDate(data.from))} — ${esc(displayDate(data.to))}</span></div><svg class="finance-chart-svg" viewBox="0 0 ${w} ${h}" role="img" aria-label="${esc(`${names[metric]} · ${data.from} — ${data.to} · ${metricTotal(report, metric)}`)}"><title>${esc(names[metric])} · ${esc(metricTotal(report, metric))}</title>${ticks.map(value => `<line class="finance-grid${value === 0 ? ' is-zero' : ''}" x1="${left}" x2="${w - right}" y1="${y(value)}" y2="${y(value)}"/><text class="finance-axis" x="${left - 10}" y="${y(value) + 4}" text-anchor="end">${esc(axis(value))}</text>`).join('')}<path class="finance-area" d="${path} L${points.at(-1)[0]},${y(0)} L${points[0][0]},${y(0)} Z"/><path class="finance-line${empty ? ' is-empty' : ''}" d="${path}"/>${points.map(([px, py], index) => `<circle class="finance-point" cx="${px}" cy="${py}" r="${items.length < 45 ? 3 : 1}"><title>${esc(dateLabel(items[index]))} · ${esc(detail(items[index]))}</title></circle>`).join('')}${[...labelIndices].map(index => `<text class="finance-axis" x="${x(index)}" y="${h - 7}" text-anchor="${index === 0 ? 'start' : index === items.length - 1 ? 'end' : 'middle'}">${esc(data.unit === 'day' ? items[index].date.slice(5).replace('-', '/') : items[index].date)}</text>`).join('')}</svg>${empty ? `<p class="finance-empty">${t('这段时间暂无已确认流水', 'No confirmed transactions in this period')}</p>` : ''}<details class="finance-data"><summary>${t('查看统计明细', 'View chart data')}</summary><div class="table-wrap"><table class="data-table"><thead><tr><th>${t('日期（UTC）', 'Date (UTC)')}</th><th>${names[metric]}</th></tr></thead><tbody>${items.map(item => `<tr><th scope="row">${esc(dateLabel(item))}</th><td>${esc(detail(item))}</td></tr>`).join('')}</tbody></table></div></details>`;
  }

  function currencySummary(report) {
    const rows = report.currency_totals || [];
    if (rows.length < 2) return '';
    return `<details class="finance-currencies"><summary>${t('各币种汇总', 'Totals by currency')}<span>${rows.length} ${t('个币种 · 分别计价', 'currencies · separate totals')}</span></summary><p class="finance-currency-note">${t('各币种分别统计，不相加。', 'Each currency is reported separately.')}</p><div class="table-wrap"><table class="data-table"><thead><tr><th>${t('币种', 'Currency')}</th><th>${t('收款', 'Payments')}</th><th>${t('成功退款', 'Refunds')}</th><th>${t('净收款', 'Net receipts')}</th></tr></thead><tbody>${rows.map(row => `<tr><th scope="row">${esc(row.currency)}</th><td>${money(row.gross_cents, row)}</td><td>${money(row.refund_cents, row)}</td><td>${money(row.revenue_cents, row)}</td></tr>`).join('')}</tbody></table></div></details>`;
  }

  async function page(tab) {
    const request = ++sequence, version = state.render, owner = identity(), path = globalThis.location?.href;
    pending = null;
    const values = selectedFilters();
    const [report, orders, paymentSettings] = await Promise.all([api(apiPath('/api/admin/overview', values)), tab === 'orders' ? api(apiPath('/api/admin/orders', values, true)) : Promise.resolve(null), state.owner ? api("/api/admin/payments/waffo") : Promise.resolve(null)]);
    if (request !== sequence || version !== state.render || owner !== identity() || path !== globalThis.location?.href) return '';
    if (!report.stats || !Array.isArray(report.series) || !['gross_cents', 'refund_cents', 'revenue_cents', 'paid_orders'].every(key => Number.isSafeInteger(report.stats[key])) || !/^[A-Z]{3}$/.test(report.currency || '') || !['live', 'test'].includes(report.payment_mode)) throw new Error(t('统计数据未完整返回，请重试。', 'The report is incomplete. Please retry.'));
    values.currency = report.currency;
    values.mode = report.payment_mode;
    const key = String(request), names = labels(), stats = report.stats;
    const paymentSwitch = paymentSettings ? `<section class="panel"><h2>${t('Waffo 支付环境', 'Waffo payment environment')}</h2><p data-payment-mode-label>${paymentSettings.mode === 'prod' ? t('生产模式 · 新订单会真实扣款', 'Production · New orders use real payments') : t('测试模式 · 不真实扣款', 'Test mode · No real charges')}</p><p>${t('仅影响新订单，已有订单继续使用原环境。', 'Applies to new orders; existing orders keep their environment.')}</p><button class="button secondary" type="button" data-waffo-switch data-mode="${paymentSettings.mode}" ${!(paymentSettings.mode === 'prod' ? paymentSettings.test_ready : paymentSettings.production_ready) ? 'disabled' : ''}>${paymentSettings.mode === 'prod' ? t('切回测试模式', 'Switch to test') : t('切换到生产模式', 'Switch to production')}</button><p role="status" data-payment-mode-status>${!paymentSettings.production_ready ? t('生产配置尚未就绪', 'Production configuration is not ready') : ''}</p></section>` : '';
    const body = `<div class="finance-page" data-finance-report="${key}"><header class="finance-page-heading"><h1>${tab === 'orders' ? t('订单与资金', 'Orders & payments') : t('经营总览', 'Overview')}</h1><span class="finance-mode ${report.payment_mode === 'test' ? 'is-test' : ''}">${report.payment_mode === 'test' ? t('测试流水', 'Test payments') : t('实收流水', 'Live payments')}</span></header>${paymentSwitch}<section class="panel finance-report">${filtersMarkup(tab, report, values)}<div class="finance-totals"><div class="finance-primary"><span class="finance-primary-label" data-finance-main-label>${t('区间', 'Period')} · ${names.revenue_cents}</span><strong data-finance-main-total>${esc(metricTotal(report, 'revenue_cents'))}</strong><span class="finance-period">${esc(rangeLabel(values))}</span></div><dl class="finance-secondary-totals"><div><dt>${names.gross_cents}</dt><dd>${esc(money(stats.gross_cents, report))}</dd></div><div><dt>${names.refund_cents}</dt><dd>${esc(money(stats.refund_cents, report))}</dd></div><div><dt>${names.orders}</dt><dd>${count(stats.paid_orders)}<small>${t('笔', 'orders')}</small></dd></div></dl></div><div class="finance-chart-heading"><h2>${t('资金趋势', 'Payment trend')}</h2><div class="finance-metrics" role="group" aria-label="${t('图表指标', 'Chart metric')}">${METRICS.map(metric => `<button type="button" data-finance-metric="${metric}" aria-pressed="${metric === 'revenue_cents'}">${names[metric]}</button>`).join('')}</div></div><div class="finance-chart" data-finance-chart>${chartMarkup(report, values)}</div><footer class="finance-note"><span id="finance-timezone">${t('UTC · 包含结束日期当天', 'UTC · End date is inclusive')}</span><span>${t('按收款 / 成功退款确认日统计', 'Payments / successful refunds by confirmation date')}</span></footer>${currencySummary(report)}</section>${tab === 'overview' ? `<section class="finance-business" aria-label="${t('平台当前状态（不限日期）', 'Current platform state, all time')}"><a href="/admin?tab=matches" data-nav>${icon('person')}<span>${t('全部用户', 'People')}<small>${t('当前平台', 'Current total')}</small></span><strong>${count(stats.users)}</strong></a><a href="/admin?tab=matches" data-nav>${icon('briefcase')}<span>${t('招聘中岗位', 'Open roles')}<small>${t('当前平台', 'Current total')}</small></span><strong>${count(stats.active_jobs)}</strong></a><a href="/admin?tab=orders&status=paid_needs_review" data-nav>${icon('receipt')}<span>${t('待核对收款', 'Needs review')}<small>${t('全部币种与流水', 'All currencies and modes')}</small></span><strong>${count(stats.review_orders)}</strong></a></section>` : ''}<section class="panel finance-orders"><div class="finance-table-heading"><div><h2>${tab === 'orders' ? t('订单明细', 'Order details') : t('近期订单', 'Recent orders')}${orders ? `<span>${count(orders.total)} ${t('笔', 'orders')}</span>` : ''}</h2><p>${t('按订单创建日期筛选 · 与上方共用日期、币种和流水类型', 'By order creation date · Same dates, currency and payment mode as above')}</p></div>${tab === 'overview' ? `<a class="text-button" href="${esc(financePath('orders', values, { page: '' }))}" data-nav>${t('全部订单', 'All orders')}${icon('arrow')}</a>` : ''}</div>${orderTable(orders ? orders.items || [] : report.recent_orders || [])}${orders ? pagination(orders.total, orders.page_size ?? orders.limit) : ''}</section>${tab === 'orders' ? `<p class="finance-footnote">${t('只计入已确认收款与成功退款；不同币种、实收与测试流水分别统计。退款在支付服务商后台处理。', 'Only confirmed payments and successful refunds count. Currencies and live/test payments are reported separately. Process refunds with the payment provider.')}</p>` : ''}</div>`;
    const html = shell(body, tab, true);
    // One-use handoff to bind(), released on mounting, navigation or identity change.
    pending = { key, report, values, version, owner, path };
    return html;
  }

  function bind(root = document) {
    mountedCleanup?.();
    const saved = pending;
    pending = null;
    const panel = root.querySelector('[data-finance-report]');
    if (!panel || !saved || panel.dataset.financeReport !== saved.key || saved.version !== state.render || saved.owner !== identity() || saved.path !== globalThis.location?.href) return () => {};
    let { report, values } = saved;
    saved.report = saved.values = null;
    const form = panel.querySelector('[data-finance-filter]'), chart = panel.querySelector('[data-finance-chart]');
    let metric = 'revenue_cents', width = 0;
    const redraw = () => {
      if (!report) return;
      const nextWidth = Math.round(chart.getBoundingClientRect().width);
      if (nextWidth > 0 && nextWidth !== width) { width = nextWidth; chart.innerHTML = chartMarkup(report, values, metric, width); }
    };
    const apply = event => {
      if (!report) return;
      event.preventDefault();
      const data = new FormData(form), from = form.elements.namedItem('from'), to = form.elements.namedItem('to');
      to.setCustomValidity(from.value && to.value && from.value > to.value ? t('结束日期不能早于开始日期。', 'End date must not be before the start date.') : '');
      if (!form.reportValidity()) return;
      go(financePath(String(data.get('tab')), Object.fromEntries(data), { page: '' }));
    };
    const changeMetric = event => {
      if (!report) return;
      const button = event.target.closest('[data-finance-metric]');
      if (!button || !METRICS.includes(button.dataset.financeMetric)) return;
      metric = button.dataset.financeMetric;
      for (const item of panel.querySelectorAll('[data-finance-metric]')) item.setAttribute('aria-pressed', String(item === button));
      panel.querySelector('[data-finance-main-label]').textContent = `${t('区间', 'Period')} · ${labels()[metric]}`;
      panel.querySelector('[data-finance-main-total]').textContent = metricTotal(report, metric);
      chart.innerHTML = chartMarkup(report, values, metric, width || 780);
    };
    const switchButton = panel.querySelector('[data-waffo-switch]');
    const switchMode = async () => {
      const previous = switchButton.dataset.mode, mode = previous === 'prod' ? 'test' : 'prod';
      switchButton.disabled = true;
      const status = panel.querySelector('[data-payment-mode-status]');
      try {
        const result = await api('/api/admin/payments/waffo', { method: 'POST', body: { mode, expected_mode: previous } });
        if (!panel.isConnected) return;
        switchButton.dataset.mode = result.mode;
        switchButton.textContent = result.mode === 'prod' ? t('切回测试模式', 'Switch to test') : t('切换到生产模式', 'Switch to production');
        panel.querySelector('[data-payment-mode-label]').textContent = result.mode === 'prod' ? t('生产模式 · 新订单会真实扣款', 'Production · New orders use real payments') : t('测试模式 · 不真实扣款', 'Test mode · No real charges');
        if (state.config?.payment_modes) state.config.payment_modes.waffo = result.mode;
        status.textContent = t('已保存', 'Saved');
      } catch (error) { if (panel.isConnected) status.textContent = error.message; }
      finally { switchButton.disabled = false; }
    };
    switchButton?.addEventListener('click', switchMode);
    form.addEventListener('submit', apply);
    form.addEventListener('change', apply);
    panel.addEventListener('click', changeMetric);
    const observer = typeof ResizeObserver === 'function' ? new ResizeObserver(redraw) : null;
    observer?.observe(chart);
    redraw();
    const cleanup = () => { switchButton?.removeEventListener("click", switchMode); observer?.disconnect(); form.removeEventListener('submit', apply); form.removeEventListener('change', apply); panel.removeEventListener('click', changeMetric); report = values = null; if (mountedCleanup === cleanup) mountedCleanup = null; };
    mountedCleanup = cleanup;
    return cleanup;
  }

  function destroy() { sequence += 1; pending = null; mountedCleanup?.(); mountedCleanup = null; }
  return { overviewPage: () => page('overview'), ordersPage: () => page('orders'), bind, destroy };
}
