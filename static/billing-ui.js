// Payment status is confirmed by the server. Local expiry only removes stale actions.
function publicationPromotion(order) {
  if (order.kind !== 'publish') return null;
  try { const promotion = JSON.parse(order.snapshot_json || '{}').promo; return promotion?.code_id ? promotion : null; }
  catch { return null; }
}

function promotionPaymentDeadline(order) {
  const reservedUntil = Number(publicationPromotion(order)?.reserved_until);
  if (!Number.isFinite(reservedUntil) || reservedUntil <= 0) return 0;
  const checkoutUntil = Number(order.checkout_expires_at);
  return Number.isFinite(checkoutUntil) && checkoutUntil > 0 ? Math.min(reservedUntil, checkoutUntil) : reservedUntil;
}

export function effectiveOrderStatus(order, now = Date.now()) {
  const deadline = ['onboarding', 'donation'].includes(order.kind) ? Number(order.checkout_expires_at) : promotionPaymentDeadline(order);
  return order.status === 'pending' && deadline > 0 && deadline * 1000 <= now ? 'expired' : order.status;
}

export function activationRecovery(user, orders = [], now = Date.now()) {
  if (user?.role !== 'employer' || user.banned) return { kind: 'unavailable' };
  if (user.employer_paid) return { kind: 'active' };
  const activation = orders.filter(order => order.kind === 'onboarding').sort((a, b) => b.created_at - a.created_at);
  const review = activation.find(order => ['paid', 'paid_needs_review', 'partially_refunded'].includes(order.status) && (order.provider === 'free' || Number(order.amount_cents) > Number(order.refunded_cents || 0)));
  if (review) return { kind: 'review', order: review };
  const pending = activation.find(order => effectiveOrderStatus(order, now) === 'pending');
  if (pending) return { kind: 'pending', order: pending };
  return { kind: activation.length ? 'retry' : 'start', order: activation[0] };
}

function cancellablePromoOrder(order) { return order.status === 'pending' && !!publicationPromotion(order); }

export function createBillingUI({ state, t, esc, icon, money, date, kindText, providerText, badge, orderRefundDetails, empty, contactMailto }) {
  const time = value => new Date(value * 1000).toLocaleString(state.lang === 'en' ? 'en-GB' : 'zh-CN', { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' });
  const expiry = order => {
    const deadline = promotionPaymentDeadline(order);
    if (order.status === 'pending' && deadline) return `<p class="help order-deadline">${t('优惠付款截止','Promotion payment deadline')} <time datetime="${new Date(deadline * 1000).toISOString()}">${esc(time(deadline))}</time>${effectiveOrderStatus(order) === 'expired' ? t('（已到期）',' (expired)') : ''}</p>`;
    return effectiveOrderStatus(order) === 'pending' && order.checkout_expires_at ? `<p class="help order-deadline">${order.checkout_url?t('支付链接有效至','Payment link valid until'):t('订单有效至','Order valid until')} <time datetime="${new Date(order.checkout_expires_at * 1000).toISOString()}">${esc(time(order.checkout_expires_at))}</time></p>` : '';
  };

  function summary(recovery) {
    if (recovery.kind === 'unavailable') return '';
    if (recovery.kind === 'active') return `<section class="activation-overview"><div><h2>${t('招聘方账户已开通','Your employer account is active')}</h2><p>${t('开通费只需支付一次，可以继续发布岗位。','Your activation is complete. You can post roles now.')}</p></div><a href="/app?tab=jobs" data-nav class="button secondary">${t('管理招聘','Manage roles')}${icon('arrow')}</a></section>`;
    if (recovery.kind === 'review') return `<section class="activation-overview"><div><h2>${t('已有开通款项，正在核对','An activation payment needs review')}</h2><p>${t('账户尚未激活，请提供订单号联系平台核对。无需再次付款。','Your account is not active yet. Contact support with the order number to check this payment.')}</p></div><a class="button secondary" href="${esc(contactMailto(t('招聘方开通付款核对','Employer activation payment review')))}">${t('核对付款','Review payment')}${icon('arrow')}</a></section>`;
    const pending = recovery.kind === 'pending';
    const retry = recovery.kind === 'retry';
    const expired = retry && effectiveOrderStatus(recovery.order) === 'expired';
    const title = expired ? t('订单已过期，账户尚未开通','Order expired. Your account is not active yet.') : t('招聘方账户尚未开通','Your employer account is not active yet');
    const description = pending ? t('完成这笔开通付款后，即可发布岗位。','Complete your activation payment to start posting roles.') : expired ? t('可以重新下单，确认币种与金额后再前往支付。旧订单会保留在下方。','Create a new order, review the currency and amount, then continue to payment. Your previous order stays below.') : t('确认一次性开通费用后即可前往支付。','Review the one-time activation fee, then continue to payment.');
    const action = pending ? `data-action="pay-order" data-id="${esc(recovery.order.id)}"` : 'data-action="role-employer"';
    const label = pending ? t('继续支付','Continue payment') : retry ? t('重新发起开通','Restart activation') : t('开通招聘方账户','Activate employer account');
    return `<section class="activation-overview"><div><h2>${title}</h2><p>${description}</p>${pending ? expiry(recovery.order) : ''}</div><button type="button" class="button" ${action}>${label}${icon('arrow')}</button></section>`;
  }

  function row(order, recovery) {
    const status = effectiveOrderStatus(order);
    const primary = recovery.order?.id === order.id && ['pending', 'review', 'retry'].includes(recovery.kind);
    const canPay = status === 'pending' && !state.user?.banned && !(order.kind === 'onboarding' && ['active', 'review'].includes(recovery.kind));
    const action = canPay && !primary ? `<button type="button" class="text-button" data-action="pay-order" data-id="${esc(order.id)}">${t('继续支付','Continue payment')}${icon('arrow')}</button>` : '';
    const cancel = !state.user?.banned && cancellablePromoOrder(order) ? `<button type="button" class="text-button" data-action="promo-cancel-order" data-id="${esc(order.id)}">${t('取消并释放优惠次数','Cancel and release allowance')}</button>` : '';
    const actions = cancel ? `<div class="order-pending">${action}${cancel}</div>` : action;
    const statusBadge = order.provider === 'free' ? `<span class="status paid">${t('免费完成','Completed free')}</span>` : badge(status);
    const review = status === 'paid_needs_review' ? `<p class="help">${t('款项已收到，请联系平台核对。无需再次付款。','Payment received. Contact support for a review; another payment is not needed.')}</p>` : '';
    return `<article class="order-row" data-dom-key="order-${esc(order.id)}"><div class="order-main"><h3>${kindText(order.kind)}</h3><div class="list-meta">${statusBadge}<span>${date(order.created_at)}</span><span>${esc(providerText(order.provider))}</span>${order.live_mode === false && order.provider !== 'free' ? `<span class="test-badge">${t('测试订单','Test order')}</span>` : ''}</div>${!primary ? expiry(order) : ''}${review}${actions}<details class="section-disclosure order-details" data-disclosure="order-${esc(order.id)}"><summary>${t('订单详情','Order details')}</summary><div class="disclosure-content"><p class="help">${t('订单号','Order number')}</p><p class="order-number">${esc(order.id)}</p><a class="text-button" href="/billing/return?order_id=${encodeURIComponent(order.id)}" data-nav>${t('查看付款状态','View payment status')}${icon('arrow')}</a></div></details></div><div class="order-amount"><strong>${money(order.amount_cents, order)}</strong>${orderRefundDetails(order)}</div></article>`;
  }

  function ordersContent(orders) {
    const recovery = activationRecovery(state.user, orders);
    const current = orders.filter(order => ['pending', 'paid_needs_review'].includes(effectiveOrderStatus(order)) || cancellablePromoOrder(order) || (recovery.kind === 'retry' && recovery.order?.id === order.id));
    const history = orders.filter(order => !current.includes(order));
    const rows = items => `<div class="order-list">${items.map(order => row(order, recovery)).join('')}</div>`;
    return `${summary(recovery)}${current.length ? rows(current) : ''}${history.length ? `<details class="section-disclosure orders-history" data-disclosure="orders-history" ${current.length || ['retry', 'start'].includes(recovery.kind) ? '' : 'open'}><summary>${t('历史订单','Order history')} <span class="muted">${history.length}</span></summary><div class="disclosure-content">${rows(history)}</div></details>` : ''}${!orders.length ? empty(t('还没有订单','No orders yet'), state.user.role === 'seeker' ? t('求职者使用免费。','HireFerry is free for job seekers.') : t('开通、发布和修改的订单会保留在这里。','Your activation, publication, and update orders will appear here.'), '', 'receipt') : ''}`;
  }
  return { ordersContent };
}
