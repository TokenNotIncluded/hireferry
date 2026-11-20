import { ROW_HEIGHT, PAGE_LIMIT, EDGE_BUDGET, STAGES, graphWindow, graphEndpoint, graphProjection, emptyGraphView, applyGraphCommands, graphGroupOrder, graphStateKey } from './relation-graph-core.js';
/* Relationships belong to a collection, independent of directory pages. */
const fields = ['people_q', 'people_role', 'people_status', 'people_page', 'job_q', 'job_status', 'jobs_page', 'graph_status', 'graph_mode', 'group_left', 'group_right', 'left_group', 'right_group', 'focus_side', 'focus_id'];

export function peopleJobsUrl(source, changes = {}) {
  const input = new URLSearchParams(source);
  const params = new URLSearchParams({ tab: 'matches' });
  for (const key of fields) if (input.get(key)) params.set(key, input.get(key));
  // Existing bookmarks keep their directory filters after the two pages merge.
  if (input.get('tab') === 'users') {
    for (const [from, to] of [['q', 'people_q'], ['role', 'people_role'], ['status', 'people_status'], ['page', 'people_page']]) if (input.get(from)) params.set(to, input.get(from));
  } else if (input.get('tab') === 'jobs') {
    for (const [from, to] of [['q', 'job_q'], ['status', 'job_status'], ['page', 'jobs_page']]) if (input.get(from)) params.set(to, input.get(from));
  }
  for (const [key, value] of Object.entries(changes)) {
    if (!fields.includes(key)) continue;
    if (value === null || value === undefined || value === '' || value === 1 && key.endsWith('_page')) params.delete(key);
    else params.set(key, String(value));
  }
  return '/admin?' + params;
}

export function graphMatchQuery(people, jobs, status = '') {
  const ids = items => [...new Set(items.map(item => String(item.id)))].slice(0, 200).join(',');
  return new URLSearchParams({ seeker_ids: ids(people.filter(person => person.role === 'seeker')), job_ids: ids(jobs), status, limit: '100' });
}

export function graphRelations(people, jobs, matches) {
  const seekers = new Set(people.filter(person => person.role === 'seeker').map(person => person.id));
  const roles = new Set(jobs.map(job => job.id));
  return matches.filter(match => seekers.has(match.seeker_id) && roles.has(match.job_id));
}

export function connectionPath(start, end) {
  const middle = start.x + (end.x - start.x) / 2;
  return `M${start.x},${start.y} C${middle},${start.y} ${middle},${end.y} ${end.x},${end.y}`;
}

// This is a proposed selection, never a saved introduction or a compatibility
// prediction. The existing candidate detail still verifies eligibility/history.
export function graphPreview(people, jobs, relations, personId, jobId) {
  const person = people.find(item => item.id === personId);
  const job = jobs.find(item => item.id === jobId);
  const employer = job && people.find(item => item.id === job.owner_id);
  if (person?.role !== 'seeker' || person.banned || job?.status !== 'published' || !(Number(job.headcount) > 0) || employer?.banned) return null;
  if (relations.some(match => match.seeker_id === person.id && match.job_id === job.id)) return null;
  return { seeker_id: person.id, job_id: job.id };
}

export function graphView(source = '') {
  const p = new URLSearchParams(peopleJobsUrl(source).split('?')[1]), view = emptyGraphView();
  view.mode = 'list';
  for (const [key, param] of [['user_q', 'people_q'], ['job_q', 'job_q'], ['role', 'people_role'], ['account_status', 'people_status'], ['job_status', 'job_status'], ['match_status', 'graph_status']]) if (p.get(param)) view.filters[key] = p.get(param);
  if (p.get('left_group')) view.filters.left_group_ids = [p.get('left_group')];
  if (p.get('right_group')) view.filters.right_group_ids = [p.get('right_group')];
  if (['overview', 'focus', 'matrix', 'list'].includes(p.get('graph_mode'))) view.mode = p.get('graph_mode');
  if (['role', 'industry', 'roles'].includes(p.get('group_left'))) view.grouping.left = p.get('group_left');
  if (p.get('group_right') === 'owner') view.grouping.right = 'owner';
  if (p.get('focus_id') && ['left', 'right'].includes(p.get('focus_side'))) view.focus = { side: p.get('focus_side'), id: p.get('focus_id') };
  return view;
}

export function createPeopleJobsUI({ state, api, t, esc, icon, shell, matchText, go }) {
  let generation = 0, requestVersion = 0, changeVersion = 0, snapshot = null, cleanup = null, requestController = null, host = null;
  let view = emptyGraphView(), viewIdentity = '', groupOrder = {}, note = '', noteError = false, mountActions = null, advancedOpen = false;
  let remote = null, remoteTimer = null, remoteController = null, syncing = false, pendingCommands = [], patchTimer = null;
  let localVersion = 0, localPending = 0, renderedRemote = null, initialPublish = null;
  const clientId = globalThis.crypto?.randomUUID?.() || `browser-${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const cursors = { left: [null], right: [null], neighbors: [null] }, pageIndices = { left: 0, right: 0, neighbors: 0 };
  const owner = () => `${state.admin}:${state.owner}:${state.staff?.id || ''}:${state.user?.id || ''}:${state.csrf || ''}`;
  const txt = (zh, en) => t(zh, en), n = value => Number(value || 0).toLocaleString();
  const option = (value, label, selected) => `<option value="${esc(value)}"${value === selected ? ' selected' : ''}>${esc(label)}</option>`;
  const action = (name, label, extra = '') => `<button type="button" class="text-button" data-pj-action="${name}" ${extra}>${label}</button>`;
  const nodeLabel = node => node?.label || node?.name || node?.title || node?.id || '';
  const viewUrl = (type, extra = {}) => '/admin?' + new URLSearchParams({ tab: 'matches', view: type, ...extra });
  const roleName = role => role === 'seeker' ? txt('求职者', 'Job seeker') : role === 'employer' ? txt('招聘方', 'Employer') : txt('未选择身份', 'No role selected');
  const statusName = status => ({ published: txt('招聘中', 'Hiring'), draft: txt('草稿', 'Draft'), closed: txt('已关闭', 'Closed'), deleted: txt('已删除', 'Deleted') })[status] || status;
  const hiddenCount = () => Object.values(view.hidden).reduce((sum, ids) => sum + ids.length, 0);
  const isCurrent = (version, identity) => version === generation && identity === owner();
  const resetCursors = () => { for (const key of Object.keys(cursors)) { cursors[key] = [null]; pageIndices[key] = 0; } };
  const query = (body, signal) => api('/api/admin/relations/query', { method: 'POST', body, signal, fresh: true });

  function filters(kind) {
    const people = kind === 'people';
    return `<form data-form="people-jobs-filter" data-list="${kind}" class="pj-search"><label class="pj-search-input">${icon('search')}<input class="input" name="${people ? 'people_q' : 'job_q'}" value="${esc(view.filters[people ? 'user_q' : 'job_q'] || '')}" maxlength="100" aria-label="${people ? txt('搜索用户', 'Search people') : txt('搜索岗位', 'Search roles')}" placeholder="${people ? txt('搜索用户', 'Search people') : txt('搜索岗位', 'Search roles')}"><button type="submit" class="icon-button" aria-label="${txt('搜索', 'Search')}">${icon('arrow')}</button></label><details class="pj-filter-extra"><summary>${txt('筛选', 'Filters')}</summary><div class="pj-filter-fields">${people ? `<select class="input" name="people_role" aria-label="${txt('用户身份', 'Person role')}">${[['', txt('全部身份', 'All roles')], ['seeker', txt('求职者', 'Job seekers')], ['employer', txt('招聘方', 'Employers')], ['unassigned', txt('未选择身份', 'No role')]].map(([v, label]) => option(v, label, view.filters.role || '')).join('')}</select><select class="input" name="people_status" aria-label="${txt('账户状态', 'Account access')}">${[['', txt('全部账户', 'All accounts')], ['active', txt('正常', 'Active')], ['suspended', txt('已封禁', 'Suspended')]].map(([v, label]) => option(v, label, view.filters.account_status || '')).join('')}</select>` : `<select class="input" name="job_status" aria-label="${txt('岗位状态', 'Role status')}">${[['', txt('全部岗位', 'All roles')], ['published', txt('招聘中', 'Hiring')], ['draft', txt('草稿', 'Draft')], ['closed', txt('已关闭', 'Closed')], ['deleted', txt('已删除', 'Deleted')]].map(([v, label]) => option(v, label, view.filters.job_status || '')).join('')}</select>`}</div></details></form>`;
  }

  function groupName(side, id) { return snapshot?.overview?.groups?.[side]?.find(g => g.id === id)?.label || id; }
  function toolbar() {
    const groupScoped = (view.filters.left_group_ids?.length || view.filters.right_group_ids?.length || 0) > 0;
    return `<div class="pj-data-bar"><span class="pj-totals">${txt('用户', 'People')} <strong>${n(snapshot.overview.summary.users)}</strong><span>×</span>${txt('岗位', 'Roles')} <strong>${n(snapshot.overview.summary.jobs)}</strong><span>·</span><strong>${n(snapshot.overview.summary.edges)}</strong> ${txt('条关系', 'relations')}</span>${action('retry', txt('刷新', 'Refresh'))}</div><div class="pj-filter-row">${filters('people')}${filters('jobs')}</div><details class="pj-advanced" ${advancedOpen ? 'open' : ''}><summary>${txt('高级视图与筛选', 'Advanced views & filters')}</summary><header class="pj-board-bar"><div class="pj-modes" role="group" aria-label="${txt('查看方式', 'View mode')}">${[['overview', txt('分组总览', 'Overview')], ['focus', txt('局部关系', 'Focus')], ['matrix', txt('关系矩阵', 'Matrix')], ['list', txt('用户与岗位', 'Directory')]].map(([mode, label]) => `<button type="button" data-pj-mode="${mode}" aria-pressed="${view.mode === mode}">${label}</button>`).join('')}</div><div class="pj-board-actions">${action('sort', txt('自动整理', 'Arrange'))}${action('retry', txt('刷新', 'Refresh'))}${action('lines', txt(view.edge_display === 'hidden' ? '显示连线' : '隐藏连线', view.edge_display === 'hidden' ? 'Show lines' : 'Hide lines'), `aria-label="${txt('连线图层', 'Edge layer')}" aria-pressed="${view.edge_display !== 'hidden'}"`)}${action('restore', txt('恢复隐藏', 'Restore hidden') + (hiddenCount() ? ` (${hiddenCount()})` : ''), hiddenCount() ? '' : 'disabled')}${action('fullscreen', host && globalThis.document?.fullscreenElement === host ? txt('退出全屏', 'Exit fullscreen') : txt('全屏', 'Fullscreen'), `aria-pressed="${!!host && globalThis.document?.fullscreenElement === host}"`)}${view.presentation === 'immersive' ? action('exit-immersive', txt('退出展开', 'Exit expanded view')) : ''}</div></header><div class="pj-data-bar"><div class="pj-statuses" role="group" aria-label="${txt('匹配阶段筛选', 'Match stage filter')}">${action('stage', txt('全部', 'All'), `data-stage="" aria-pressed="${!view.filters.match_status}"`)}${STAGES.map(s => action('stage', `<i class="pj-swatch ${s}" aria-hidden="true"></i>${esc(matchText(s))}`, `data-stage="${s}" aria-pressed="${view.filters.match_status === s}"`)).join('')}</div></div><div class="pj-grouping"><label>${txt('用户分组', 'People groups')} <select class="input" data-pj-grouping="left">${[['role', txt('身份', 'Role')], ['industry', txt('目标行业', 'Target industry')], ['roles', txt('意向岗位', 'Target roles')]].map(([v, label]) => option(v, label, view.grouping.left)).join('')}</select></label><label>${txt('岗位分组', 'Role groups')} <select class="input" data-pj-grouping="right">${[['status', txt('状态', 'Status')], ['owner', txt('招聘方', 'Employer')]].map(([v, label]) => option(v, label, view.grouping.right)).join('')}</select></label><label>${view.mode === 'overview' || view.mode === 'matrix' ? txt('分组排序', 'Group order') : txt('本页排序', 'Page order')} <select class="input" data-pj-ordering>${[['stable', txt('保持顺序', 'Stable')], ['name', txt('名称', 'Name')], ['degree', txt('关系数量', 'Relation count')]].map(([v, label]) => option(v, label, view.ordering)).join('')}</select></label>${groupScoped ? `<span class="pj-scope-chip">${esc([...(view.filters.left_group_ids || []).map(id => groupName('left', id)), ...(view.filters.right_group_ids || []).map(id => groupName('right', id))].join(' × '))}</span>${action('clear-groups', txt('清除分组', 'Clear group scope'))}` : ''}</div></details>`;
  }

  function row(node, side, index, group = false) {
    if (group) return `<article class="pj-row pj-group-row" role="listitem" style="top:${index * ROW_HEIGHT}px" data-pj-row="${index}"><button type="button" class="pj-node-select" data-pj-group="${esc(node.id)}" data-side="${side}"><span><strong>${esc(node.label)}</strong><small>${n(node.count)} ${side === 'left' ? txt('位用户', 'people') : txt('个岗位', 'roles')} · ${n(node.edges)} ${txt('条关系', 'relations')}</small></span>${icon('arrow')}</button></article>`;
    const selected = view.focus?.side === side && view.focus?.id === node.id, edge = snapshot?.neighbors?.items?.find(item => item.node.id === node.id && view.focus?.side !== side)?.edge;
    const secondary = side === 'left' ? `${roleName(node.role)}${node.banned ? ' · ' + txt('已封禁', 'Suspended') : ''}` : `${statusName(node.status)} · ${n(node.headcount)} ${txt('人', 'people')}`;
    const management = side === 'left' ? `<button type="button" class="text-button" data-action="admin-edit-user" data-id="${esc(node.id)}">${txt('资料', 'Profile')}</button>${state.config?.email_notifications_enabled && (state.owner || state.staff) ? `<button type="button" class="text-button" data-action="admin-email-user" data-id="${esc(node.id)}">${txt('邮件', 'Email')}</button>` : ''}${state.owner || state.staff ? `<button type="button" class="text-button pj-access" data-action="admin-user-access" data-id="${esc(node.id)}">${node.banned ? txt('恢复', 'Restore') : txt('封禁', 'Suspend')}</button>` : ''}` : `<button type="button" class="text-button" data-action="admin-edit-job" data-id="${esc(node.id)}">${state.owner || state.staff ? txt('管理', 'Manage') : txt('详情', 'Details')}</button>`;
    return `<article class="pj-row ${selected ? 'is-selected' : ''}" role="listitem" style="top:${index * ROW_HEIGHT}px" data-graph-node="${side === 'left' ? 'person' : 'job'}:${esc(node.id)}" data-pj-row="${index}"><button type="button" class="pj-node-select" data-pj-focus-node="${esc(node.id)}" data-side="${side}" aria-pressed="${selected}"><span class="pj-avatar" aria-hidden="true">${side === 'left' ? esc((node.name || node.label || '?').slice(0, 1)) : icon('briefcase')}</span><span class="pj-node-label"><strong>${esc(nodeLabel(node))}</strong><small>${esc(secondary)} · ${n(node.relation_count)} ${txt('条', 'links')}</small></span></button><div class="pj-row-actions">${edge ? `<button type="button" class="pj-stage-button ${esc(edge.status)}" data-action="match-detail" data-id="${esc(edge.id)}" aria-label="${esc(txt('查看匹配：', 'Open match: ') + matchText(edge.status))}"><i class="pj-swatch ${esc(edge.status)}" aria-hidden="true"></i><span>${esc(matchText(edge.status))}</span></button>` : ''}${management}${selected ? action('hide-focus', txt('隐藏', 'Hide'), `aria-label="${txt('从视图隐藏焦点', 'Hide focused node from this view')}"`) : ''}</div></article>`;
  }
  function ordered(items, group = false, side = 'left') {
    let result = items.slice();if (!group) result = result.filter(item => !view.hidden[side].includes(item.id));
    if (view.ordering === 'name') result.sort((a, b) => nodeLabel(a).localeCompare(nodeLabel(b)) || a.id.localeCompare(b.id));
    else if (view.ordering === 'degree') result.sort((a, b) => Number(b.edges ?? b.relation_count) - Number(a.edges ?? a.relation_count) || a.id.localeCompare(b.id));
    else if (group && groupOrder[side]) { const ranks = new Map(groupOrder[side].map((id, i) => [id, i]));result.sort((a, b) => (ranks.get(a.id) ?? 1000) - (ranks.get(b.id) ?? 1000)); }
    return result;
  }
  function sideItems(side) {
    if (view.mode === 'overview') return ordered(snapshot.overview.groups?.[side] || [], true, side);
    let items = snapshot.directories?.[side]?.items || [];
    if (view.mode === 'focus' && view.focus && snapshot.neighbors) {
      if (view.focus.side !== side) items = snapshot.neighbors.items.map(item => item.node);
      else items = [snapshot.neighbors.focus].filter(Boolean);
    }
    return ordered(items, false, side);
  }
  function pagination(side) {
    if (view.mode === 'overview') return `<span>${txt('完整分组；节点按需展开', 'Complete groups; expand nodes on demand')}</span>`;
    if (view.mode === 'focus' && view.focus?.side === side) return `<span>${txt('固定焦点；点击对侧节点切换中心', 'Pinned focus; select a neighbor to change center')}</span>`;
    const key = view.mode === 'focus' && view.focus?.side !== side && view.focus ? 'neighbors' : side, result = key === 'neighbors' ? snapshot.neighbors : snapshot.directories?.[side];
    if (!result) return '';
    const shown = result.items?.length || 0, total = Number(result.total || 0), page = pageIndices[key];
    return `<span>${txt(`本页 ${shown} / 共 ${total}`, `${shown} on page / ${total} total`)}${result.remaining ? ` · ${txt(`后续 ${result.remaining}`, `${result.remaining} after this page`)}` : ''}</span><div>${action('page', icon('back'), `data-page-key="${key}" data-direction="-1" aria-label="${txt('上一页', 'Previous page')}" ${page > 0 ? '' : 'disabled'}`)}${action('page', icon('arrow'), `data-page-key="${key}" data-direction="1" aria-label="${txt('下一页', 'Next page')}" ${result.next_cursor ? '' : 'disabled'}`)}</div>`;
  }
  function graphBody() {
    const group = view.mode === 'overview', focus = view.mode === 'focus' && snapshot.neighbors?.focus;
    return `${focus ? `<div class="pj-focus-summary"><strong>${txt('焦点', 'Focus')}：${esc(nodeLabel(snapshot.neighbors.focus))}</strong><span>${n(snapshot.neighbors.total)} ${txt('条符合筛选的关系', 'relations in this scope')}</span>${action('clear-focus', txt('清除焦点', 'Clear focus'))}</div>` : view.mode === 'focus' ? `<div class="pj-focus-summary">${txt('选择任一用户或岗位，查看完整的一跳关系。招聘方保留管理入口。', 'Select a person or role to inspect its complete one-hop relations. Employer profiles remain available for management.')}</div>` : ''}<div class="pj-stage" data-pj-stage><canvas class="pj-connections" data-pj-canvas aria-hidden="true"></canvas>${['left', 'right'].map(side => {
      const items = sideItems(side);
      return `<section class="pj-column" data-pj-column="${side}"><header class="pj-column-heading"><h2>${icon(side === 'left' ? 'person' : 'briefcase')}${side === 'left' ? txt('用户', 'People') : txt('岗位', 'Roles')}${group ? ` · ${txt('分组', 'Groups')}` : ''}</h2><span>${n(items.length)} ${txt('个已载入项', 'loaded')}</span></header><button type="button" class="pj-portal" data-pj-portal="${side}:above" disabled>↑ ${txt('上方 0 条关系', '0 relations above')}</button><div class="pj-nodes" data-pj-scroll="${side}" role="region" aria-label="${side === 'left' ? txt('用户独立滚动列表', 'People list, scrolls independently') : txt('岗位独立滚动列表', 'Roles list, scrolls independently')}"><div class="pj-node-space" style="height:${items.length * ROW_HEIGHT}px" role="list" data-pj-window="${side}">${items.slice(0, 11).map((item, i) => row(item, side, i, group)).join('')}</div>${items.length ? '' : `<div class="pj-empty">${txt('当前范围没有可显示的记录', 'No visible records in this scope')}</div>`}</div><button type="button" class="pj-portal" data-pj-portal="${side}:below" disabled>↓ ${txt('下方 0 条关系', '0 relations below')}</button><nav class="pj-pagination" aria-label="${side === 'left' ? txt('用户分页', 'People pages') : txt('岗位分页', 'Role pages')}">${pagination(side)}</nav></section>`;
    }).join('')}</div><div class="pj-projection-summary" data-pj-projection role="status"></div>`;
  }
  function matrixBody() {
    const left = ordered(snapshot.overview.groups?.left || [], true, 'left'), right = ordered(snapshot.overview.groups?.right || [], true, 'right'), index = new Map(snapshot.overview.bundles.map(b => [JSON.stringify([b.left_group_id, b.right_group_id]), b]));
    const cell = (l, r) => { const b = index.get(JSON.stringify([l.id, r.id]));return `<button type="button" class="pj-matrix-cell" data-pj-matrix-left="${esc(l.id)}" data-pj-matrix-right="${esc(r.id)}" ${b?.count ? '' : 'disabled'} aria-label="${esc(`${l.label} × ${r.label} · ${n(b?.count)} ${txt('条关系', 'relations')}`)}"><strong>${n(b?.count)}</strong><span class="pj-composition" aria-hidden="true">${STAGES.map(s => `<i class="${s}" style="flex:${b?.state_counts?.[s] || 0}"></i>`).join('')}</span></button>`; };
    return `<div class="pj-matrix-caption">${txt('行：用户分组 · 列：岗位分组 · 点击单元格限定范围；同一关系只计一次。', 'Rows: people groups · Columns: role groups · Select a cell to narrow scope. Every relation is counted once.')}</div><div class="pj-matrix-scroll"><table class="pj-matrix"><thead><tr><th>${txt('用户 / 岗位', 'People / roles')}</th>${right.map(g => `<th>${esc(g.label)}</th>`).join('')}</tr></thead><tbody>${left.map(l => `<tr><th>${esc(l.label)}</th>${right.map(r => `<td>${cell(l, r)}</td>`).join('')}</tr>`).join('')}</tbody></table></div><div class="pj-matrix-mobile">${left.map(l => `<details><summary>${esc(l.label)} · ${n(l.edges)} ${txt('条', 'links')}</summary>${right.map(r => `<div><span>${esc(r.label)}</span>${cell(l, r)}</div>`).join('')}</details>`).join('')}</div>`;
  }
  function board() {
    if (!snapshot?.overview) return `<div class="pj-empty pj-load-error" role="alert">${esc(note || txt('关系数据读取失败', 'Could not load relations'))}${action('retry', txt('重试', 'Retry'))}</div>`;
    return `${toolbar()}${view.mode === 'matrix' ? matrixBody() : graphBody()}<footer class="pj-board-footer"><span>${txt('完整集合计数独立于分页与滚动。隐藏仅改变当前视图。', 'Collection counts are independent of paging and scrolling. Hiding changes only this view.')}</span><a href="${esc(viewUrl('search'))}" class="text-button" data-nav>${txt('建立匹配', 'Make introduction')}${icon('arrow')}</a></footer>`;
  }
  function pageMarkup(version) {
    return shell(`<div class="people-jobs-workspace" data-graph-page="${version}" data-pj-workbench><header class="pj-page-header"><div><h1>${txt('用户与岗位', 'People & roles')}</h1><p>${txt('查看用户和岗位，选择对象跟进匹配。', 'Browse people and roles, then follow up on introductions.')}</p></div><a class="button secondary small" href="${esc(viewUrl('search'))}" data-nav>${icon('plus')}${txt('建立匹配', 'Make introduction')}</a></header><nav class="wb-tabs pj-view-tabs" aria-label="${txt('匹配工作台视图', 'Matching workspace views')}"><a href="${esc(peopleJobsUrl(location.search))}" data-nav aria-current="page">${icon('link')}${txt('用户与岗位', 'People & roles')}</a><a href="${esc(viewUrl('search'))}" data-nav>${icon('search')}${txt('筛选候选人', 'Find candidates')}</a><a href="${esc(viewUrl('followups'))}" data-nav>${icon('board')}${txt('匹配跟进', 'Follow-ups')}</a></nav><section class="pj-board" data-pj-board data-mode="${view.mode}">${board()}</section><div class="pj-scope-note ${noteError ? 'pj-load-error' : ''}" data-pj-note role="status" aria-live="polite">${esc(note)}</div><div class="pj-ai-state" data-pj-ai-status role="status"></div></div>`, 'matches', true);
  }

  async function fetchData({ full = true, queryView = view } = {}) {
    const started = ++requestVersion, version = generation, identity = owner(), changed = changeVersion;
    const requested = structuredClone(queryView), queryBody = (mode, extra = {}) => ({ mode, filters: requested.filters, grouping: requested.grouping, ...extra });
    requestController?.abort();requestController = new AbortController();const signal = requestController.signal;
    if (host) host.setAttribute('aria-busy', 'true');
    try {
      let overview = snapshot?.overview, directories = snapshot?.directories;
      if (full || !overview) {
        overview = await query(queryBody('overview'), signal);
        const [left, right] = await Promise.all(['left', 'right'].map(side => query(queryBody('nodes', { side, limit: PAGE_LIMIT, data_rev: overview.data_rev, cursor: cursors[side][pageIndices[side]] || undefined }), signal)));
        directories = { left, right };
      }
      let neighbors = null;
      if (requested.mode === 'focus' && requested.focus) neighbors = await query(queryBody('neighbors', { focus: requested.focus, limit: PAGE_LIMIT, data_rev: overview.data_rev, cursor: cursors.neighbors[pageIndices.neighbors] || undefined }), signal);
      if (!isCurrent(version, identity) || started !== requestVersion || changed !== changeVersion) return false;
      snapshot = { version, identity, overview, directories, neighbors, neighborFocus: neighbors ? requested.focus : null };if (requested.ordering === 'stable') groupOrder = graphGroupOrder(overview.groups, overview.bundles, groupOrder);noteError = false;note = '';return true;
    } catch (error) {
      if (!isCurrent(version, identity) || started !== requestVersion || changed !== changeVersion || error.name === 'AbortError') return false;
      if ([401, 403].includes(error.status)) throw error;
      noteError = true;note = error.status === 409 ? txt('数据已变化，请刷新关系集合；不会拼接不同版本的分页。', 'Data changed. Refresh the collection; pages from different revisions are not combined.') : txt('关系数据读取失败，请重试。', 'Could not load relation data. Try again.');return false;
    } finally { if (started === requestVersion) host?.removeAttribute('aria-busy'); }
  }
  async function page() {
    cleanup?.();cleanup = null;host = null;
    const version = ++generation, identity = owner();
    if (viewIdentity !== identity) { advancedOpen = false;view = graphView(location.search);viewIdentity = identity;remote = null; }
    else { const fromUrl = graphView(location.search);view = { ...view, mode: fromUrl.mode, filters: fromUrl.filters, grouping: fromUrl.grouping, focus: fromUrl.focus }; }
    resetCursors();note = '';noteError = false;const ok = await fetchData();
    if (!isCurrent(version, identity)) return '';if (!ok) snapshot = null;return pageMarkup(version);
  }
  function syncUrl() {
    if (!globalThis.history?.replaceState) return;
    const changes = { graph_mode: view.mode === 'overview' ? '' : view.mode, group_left: view.grouping.left === 'role' ? '' : view.grouping.left, group_right: view.grouping.right === 'status' ? '' : view.grouping.right, focus_side: view.focus?.side || '', focus_id: view.focus?.id || '', left_group: view.filters.left_group_ids?.length === 1 ? view.filters.left_group_ids[0] : '', right_group: view.filters.right_group_ids?.length === 1 ? view.filters.right_group_ids[0] : '', people_page: '', jobs_page: '' };
    for (const [key, param] of [['user_q', 'people_q'], ['job_q', 'job_q'], ['role', 'people_role'], ['account_status', 'people_status'], ['job_status', 'job_status'], ['match_status', 'graph_status']]) changes[param] = view.filters[key] || '';
    history.replaceState(history.state, '', peopleJobsUrl(location.search, changes));
  }
  function paint(resetScroll = false) {
    if (!host || viewIdentity !== owner()) return;
    const positions = resetScroll ? {} : Object.fromEntries([...host.querySelectorAll('[data-pj-scroll]')].map(element => [element.dataset.pjScroll, element.scrollTop]));
    mountActions?.unobserve();host.querySelector('[data-pj-board]').innerHTML = board();host.querySelector('[data-pj-board]').dataset.mode = view.mode;host.classList.toggle('is-immersive', view.presentation === 'immersive');
    for (const [side, top] of Object.entries(positions)) { const scroll = host.querySelector(`[data-pj-scroll="${side}"]`);if (scroll) scroll.scrollTop = top; }
    host.querySelector('[data-pj-note]').textContent = note || (view.edge_display === 'hidden' ? txt('连线已隐藏；节点、矩阵与统计保持不变。', 'Lines hidden; nodes, matrix and counts are unchanged.') : '');
    host.querySelector('[data-pj-note]').classList.toggle('pj-load-error', noteError);mountActions?.observe();mountActions?.schedule();
  }
  async function change(commands, { publish = true } = {}) {
    const changed = ++changeVersion, previous = view, version = generation, identity = owner();localVersion++;localPending++;renderedRemote = null;
    try {
      view = applyGraphCommands(view, commands);
      const full = commands.some(c => ['set_filters', 'set_grouping'].includes(c.op)), reload = full || commands.some(c => c.op === 'set_focus' || c.op === 'set_mode' && c.mode === 'focus');
      if (full) resetCursors();
      if (commands.some(c => c.op === 'set_focus')) { cursors.neighbors = [null];pageIndices.neighbors = 0; }
      if (reload) { const ok = await fetchData({ full });if (changed !== changeVersion || !isCurrent(version, identity)) return false;if (!ok) { view = previous;paint();return false; } }
      if (commands.some(c => c.op === 'set_ordering' && c.ordering === 'stable') && snapshot?.overview) groupOrder = graphGroupOrder(snapshot.overview.groups, snapshot.overview.bundles, groupOrder);
      syncUrl();paint(full || commands.some(c => c.op === 'set_focus'));if (publish) queueRemote(commands);return true;
    } finally {
      localPending--;if (!localPending && remote && (pendingCommands.length || initialPublish)) { clearTimeout(patchTimer);patchTimer = setTimeout(flushRemote, 180); }
    }
  }
  async function advancePage(key, direction) {
    const result = key === 'neighbors' ? snapshot.neighbors : snapshot.directories[key], previous = pageIndices[key];
    if (direction > 0) { if (!result?.next_cursor) return;cursors[key][previous + 1] = result.next_cursor; }
    if (previous + direction < 0) return;pageIndices[key] += direction;
    if (!(await fetchData({ full: key !== 'neighbors' }))) pageIndices[key] = previous;paint(true);
  }

  function mount(root = document) {
    cleanup?.();cleanup = null;host = root.querySelector?.('.people-jobs-workspace');
    if (!host || Number(host.dataset.graphPage) !== generation || viewIdentity !== owner()) return;
    const mountedHost = host, version = generation, identity = owner();let alive = true, frame = null, observer = null;
    const stillHere = () => alive && isCurrent(version, identity) && host === mountedHost;
    function windowRows(side) {
      const scroll = host.querySelector(`[data-pj-scroll="${side}"]`), target = host.querySelector(`[data-pj-window="${side}"]`);if (!scroll || !target) return;
      const items = sideItems(side), range = graphWindow(items.length, scroll.scrollTop, scroll.clientHeight || 480), stamp = `${range.start}:${range.end}`;
      if (target.dataset.window === stamp) return;target.dataset.window = stamp;target.innerHTML = items.slice(range.start, range.end).map((item, i) => row(item, side, range.start + i, view.mode === 'overview')).join('');
    }
    function portal(side, zone, count) {
      const element = host.querySelector(`[data-pj-portal="${side}:${zone}"]`);if (!element) return;
      element.disabled = !count;element.textContent = `${zone === 'above' ? '↑' : '↓'} ${txt(`${zone === 'above' ? '上方' : '下方'} ${n(count)} 条关系`, `${n(count)} relations ${zone}`)}`;
    }
    function draw() {
      frame = null;if (!stillHere() || !snapshot?.overview) return;windowRows('left');windowRows('right');
      const canvas = host.querySelector('[data-pj-canvas]'), stage = host.querySelector('[data-pj-stage]');if (!canvas || !stage) return;
      const context = canvas.getContext('2d');if (!context) return;
      const box = stage.getBoundingClientRect(), dpr = Math.min(2, globalThis.devicePixelRatio || 1);
      if (canvas.width !== Math.round(box.width * dpr) || canvas.height !== Math.round(box.height * dpr)) { canvas.width = Math.round(box.width * dpr);canvas.height = Math.round(box.height * dpr); }
      context.setTransform(dpr, 0, 0, dpr, 0, 0);context.clearRect(0, 0, box.width, box.height);
      const columns = {}, viewport = {};
      for (const side of ['left', 'right']) { const element = host.querySelector(`[data-pj-scroll="${side}"]`), bounds = element.getBoundingClientRect();columns[side] = { element, bounds, ids: sideItems(side).map(item => item.id) };viewport[side] = { scrollTop: element.scrollTop, height: element.clientHeight }; }
      let paths = [], summary = '';
      if (view.mode === 'overview') {
        const pairs = snapshot.overview.bundles || [], weighted = [];
        for (const b of pairs) for (const status of STAGES) if (b.state_counts?.[status]) weighted.push({ ...b, status, count: b.state_counts[status] });
        weighted.sort((a, b) => b.count - a.count || a.left_group_id.localeCompare(b.left_group_id));
        paths = weighted.map(b => ({ a: graphEndpoint(columns.left.ids.indexOf(b.left_group_id), viewport.left.scrollTop, viewport.left.height), b: graphEndpoint(columns.right.ids.indexOf(b.right_group_id), viewport.right.scrollTop, viewport.right.height), status: b.status, count: b.count })).filter(path => path.a.zone === 'visible' || path.b.zone === 'visible').slice(0, EDGE_BUDGET);
        summary = txt(`绘制 ${paths.length} / ${weighted.length} 条分组状态束；${n(snapshot.overview.summary.edges)} 条关系全部计入矩阵。`, `Drawing ${paths.length} of ${weighted.length} group-stage bundles; all ${n(snapshot.overview.summary.edges)} relations are counted in the matrix.`);
        for (const side of ['left', 'right']) for (const zone of ['above', 'below']) portal(side, zone, pairs.reduce((sum, b) => { const i = columns[side].ids.indexOf(b[side + '_group_id']);return graphEndpoint(i, viewport[side].scrollTop, viewport[side].height).zone === zone ? sum + b.count : sum; }, 0));
      } else if (view.mode === 'focus' && snapshot.neighbors) {
        const projection = graphProjection(snapshot.neighbors.items.map(item => item.edge), columns.left.ids, columns.right.ids, viewport, view.hidden);paths = projection.paths.slice(0, EDGE_BUDGET);
        const c = projection.counts, unloaded = Math.max(0, snapshot.neighbors.total - snapshot.neighbors.items.length);
        summary = txt(`本页关系 ${snapshot.neighbors.items.length}：视口内 ${c.visible} · 离屏 ${c.offscreen} · 隐藏 ${c.hidden} · 端点未载入 ${c.missing}；其他页 ${unloaded} 条。`, `${snapshot.neighbors.items.length} relations on this page: ${c.visible} in view · ${c.offscreen} offscreen · ${c.hidden} hidden · ${c.missing} endpoints not loaded; ${unloaded} on other pages.`);
        for (const side of ['left', 'right']) for (const zone of ['above', 'below']) portal(side, zone, c[side][zone]);
      } else { summary = txt('用户与岗位目录；选择节点查看关系。分页不会定义关系集合。', 'People and roles directory; select a node to inspect relations. Pages do not define the relation collection.');for (const side of ['left', 'right']) for (const zone of ['above', 'below']) portal(side, zone, 0); }
      host.querySelector('[data-pj-projection]').textContent = summary;
      if (view.edge_display === 'hidden' || box.width < 620 || view.mode === 'list') return;
      const style = getComputedStyle(host), colors = Object.fromEntries(STAGES.map(s => [s, style.getPropertyValue('--pj-' + s).trim()])), x1 = columns.left.bounds.right - box.left + 3, x2 = columns.right.bounds.left - box.left - 3;
      for (const line of paths) {
        // Separate stage bundles slightly so a thicker late-drawn stage does
        // not conceal the colors and dash patterns of the same group pair.
        const lane = view.mode === 'overview' ? (STAGES.indexOf(line.status) - (STAGES.length - 1) / 2) * 4 : 0;
        const y1 = columns.left.bounds.top - box.top + line.a.y + lane, y2 = columns.right.bounds.top - box.top + line.b.y + lane;
        context.beginPath();context.moveTo(x1, y1);context.bezierCurveTo(x1 + (x2 - x1) * .48, y1, x2 - (x2 - x1) * .48, y2, x2, y2);
        context.strokeStyle = colors[line.status] || colors.introduced;context.globalAlpha = view.mode === 'overview' ? .6 : .9;context.lineWidth = view.mode === 'overview' ? Math.min(5, 1 + Math.log2(1 + line.count) / 3) : Math.min(4, 1.6 + Math.log2(line.count) / 3);context.setLineDash(line.status === 'declined' ? [2, 5] : line.status === 'interview' ? [7, 4] : []);context.stroke();
      }
      context.globalAlpha = 1;
    }
    function schedule() { if (frame === null && stillHere()) frame = requestAnimationFrame(draw); }
    const observe = () => { if (!stillHere()) return;const stage = host.querySelector('[data-pj-stage]');if (stage && observer) observer.observe(stage);for (const el of host.querySelectorAll('[data-pj-scroll]')) observer?.observe(el); };
    mountActions = { schedule, observe, unobserve: () => observer?.disconnect() };if (globalThis.ResizeObserver) observer = new ResizeObserver(schedule);
    async function onClick(event) {
      if (!stillHere()) return;const button = event.target.closest('button');if (!button || !host.contains(button) || button.disabled) return;const d = button.dataset;
      try {
        if (d.pjMode) await change([{ op: 'set_mode', mode: d.pjMode }]);
        else if (d.pjFocusNode) await change([{ op: 'set_mode', mode: 'focus' }, { op: 'set_focus', focus: { side: d.side, id: d.pjFocusNode } }]);
        else if (d.pjGroup) await change([{ op: 'set_filters', filters: { ...view.filters, [d.side + '_group_ids']: [d.pjGroup] } }, { op: 'set_focus', focus: null }, { op: 'set_mode', mode: 'focus' }]);
        else if (d.pjMatrixLeft) await change([{ op: 'set_filters', filters: { ...view.filters, left_group_ids: [d.pjMatrixLeft], right_group_ids: [d.pjMatrixRight] } }, { op: 'set_focus', focus: null }, { op: 'set_mode', mode: 'focus' }]);
        else if (d.pjPortal) { const [side, zone] = d.pjPortal.split(':'), scroll = host.querySelector(`[data-pj-scroll="${side}"]`);scroll.scrollBy({ top: (zone === 'above' ? -1 : 1) * scroll.clientHeight * .8, behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' }); }
        else if (d.pjAction === 'lines') await change([{ op: 'set_edge_display', edge_display: view.edge_display === 'hidden' ? 'auto' : 'hidden' }]);
        else if (d.pjAction === 'stage') { const filters = { ...view.filters };if (d.stage) filters.match_status = d.stage;else delete filters.match_status;await change([{ op: 'set_filters', filters }]); }
        else if (d.pjAction === 'clear-groups') { const filters = { ...view.filters };delete filters.left_group_ids;delete filters.right_group_ids;await change([{ op: 'set_filters', filters }]); }
        else if (d.pjAction === 'clear-focus') await change([{ op: 'set_focus', focus: null }]);
        else if (d.pjAction === 'exit-immersive') await change([{ op: 'set_presentation', presentation: 'standard' }]);
        else if (d.pjAction === 'hide-focus' && view.focus) await change([{ op: 'set_visibility', side: view.focus.side, ids: [view.focus.id], visible: false }]);
        else if (d.pjAction === 'restore') await change(['left', 'right', 'edges'].flatMap(side => Array.from({ length: Math.ceil(view.hidden[side].length / 100) }, (_, i) => ({ op: 'set_visibility', side, ids: view.hidden[side].slice(i * 100, i * 100 + 100), visible: true }))));
        else if (d.pjAction === 'page') await advancePage(d.pageKey, Number(d.direction));
        else if (d.pjAction === 'sort') { groupOrder = graphGroupOrder(snapshot.overview.groups, snapshot.overview.bundles, groupOrder);await change([{ op: 'set_ordering', ordering: 'stable' }]);note = txt('已按关联整理分组；未改善交叉时保留原顺序。', 'Groups arranged by connections; unchanged crossings preserve the previous order.');paint(); }
        else if (d.pjAction === 'retry') { resetCursors();if (await fetchData()) { paint();await acknowledge(); } else paint(); }
        else if (d.pjAction === 'fullscreen') {
          if (document.fullscreenElement !== host && view.presentation === 'immersive' && !view.fullscreen_requested) { await change([{ op: 'set_presentation', presentation: 'standard' }]);return; }
          const result = document.fullscreenElement === host ? document.exitFullscreen() : host.requestFullscreen?.();
          if (!result) { await change([{ op: 'set_presentation', presentation: view.presentation === 'immersive' ? 'standard' : 'immersive' }]);note = txt('已展开工作台；当前浏览器不支持原生全屏。', 'Workbench expanded; native fullscreen is unavailable.');paint(); }
          else try { await result;await acknowledge(); } catch { await change([{ op: 'set_presentation', presentation: 'immersive' }]);note = txt('浏览器未允许原生全屏，已展开工作台。', 'Native fullscreen was not allowed; the workbench is expanded.');paint(); }
        }
      } catch (error) { if (stillHere() && ![401, 403].includes(error.status)) { noteError = true;note = error.message || txt('操作失败，请重试', 'Action failed; try again');paint(); } }
    }
    function onChange(event) {
      const d = event.target.dataset;
      if (d.pjGrouping) { const filters = { ...view.filters };delete filters.left_group_ids;delete filters.right_group_ids;void change([{ op: 'set_filters', filters }, { op: 'set_grouping', grouping: { ...view.grouping, [d.pjGrouping]: event.target.value } }, { op: 'set_focus', focus: null }]); }
      else if ('pjOrdering' in d) void change([{ op: 'set_ordering', ordering: event.target.value }]);
      else { const form = event.target.closest('form[data-form="people-jobs-filter"]');if (event.target.tagName === 'SELECT' && form) form.requestSubmit(); }
    }
    function onKey(event) {
      const button = event.target.closest('[data-pj-focus-node],[data-pj-group]');if (!button || !['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      const side = button.dataset.side, items = sideItems(side), id = button.dataset.pjFocusNode || button.dataset.pjGroup;let index = items.findIndex(item => item.id === id);
      index = event.key === 'Home' ? 0 : event.key === 'End' ? items.length - 1 : Math.max(0, Math.min(items.length - 1, index + (event.key === 'ArrowDown' ? 1 : -1)));
      event.preventDefault();const scroll = host.querySelector(`[data-pj-scroll="${side}"]`);scroll.scrollTop = Math.max(0, index * ROW_HEIGHT - scroll.clientHeight / 2);windowRows(side);host.querySelector(`[data-pj-window="${side}"] [data-pj-row="${index}"] button`)?.focus();schedule();
    }
    function onFullscreen() {
      if (!stillHere()) return;const active = document.fullscreenElement === host, button = host.querySelector('[data-pj-action="fullscreen"]');
      if (button) { button.textContent = active ? txt('退出全屏', 'Exit fullscreen') : txt('全屏', 'Fullscreen');button.setAttribute('aria-pressed', String(active)); }
      schedule();void acknowledge();
    }
    const onToggle = event => { if (stillHere() && event.target.matches?.('.pj-advanced')) advancedOpen = event.target.open; };
    host.addEventListener('toggle', onToggle, true);host.addEventListener('click', onClick);host.addEventListener('change', onChange);host.addEventListener('keydown', onKey);host.addEventListener('scroll', schedule, true);window.addEventListener('resize', schedule);document.addEventListener('fullscreenchange', onFullscreen);observe();schedule();void connectRemote();
    cleanup = () => {
      alive = false;requestController?.abort();remoteController?.abort();clearTimeout(remoteTimer);clearTimeout(patchTimer);if (frame !== null) cancelAnimationFrame(frame);observer?.disconnect();window.removeEventListener('resize', schedule);document.removeEventListener('fullscreenchange', onFullscreen);
      mountedHost.removeEventListener('toggle', onToggle, true);mountedHost.removeEventListener('click', onClick);mountedHost.removeEventListener('change', onChange);mountedHost.removeEventListener('keydown', onKey);mountedHost.removeEventListener('scroll', schedule, true);if (host === mountedHost) { host = null;mountActions = null; }
    };
  }

  function remoteState(payload) { return payload?.state || payload?.view?.state; }
  function remoteInfo(payload) { return payload?.view_id ? payload : payload?.view; }
  function setRemoteNotice(message) { if (host) host.querySelector('[data-pj-ai-status]').textContent = message; }
  const canReceiveRemote = expected => !!host && expected === localVersion && !localPending && !pendingCommands.length && !initialPublish;
  function stateCommands(desired, saved = emptyGraphView()) {
    const commands = [{ op: 'set_filters', filters: desired.filters }, { op: 'set_grouping', grouping: desired.grouping }, { op: 'set_mode', mode: desired.mode }, { op: 'set_focus', focus: desired.focus }, { op: 'set_ordering', ordering: desired.ordering }, { op: 'set_edge_display', edge_display: desired.edge_display }, { op: 'set_presentation', presentation: desired.presentation }];
    for (const side of ['left', 'right', 'edges']) for (const visible of [true, false]) {
      const from = visible ? saved.hidden?.[side] || [] : desired.hidden[side], into = new Set(visible ? desired.hidden[side] : saved.hidden?.[side] || []), ids = from.filter(id => !into.has(id));
      for (let i = 0; i < ids.length; i += 100) commands.push({ op: 'set_visibility', side, ids: ids.slice(i, i + 100), visible });
    }
    if (desired.fullscreen_requested) commands.push({ op: 'request_fullscreen' });return commands;
  }
  async function acknowledge() {
    if (!remote || !host || !renderedRemote || localPending || pendingCommands.length || initialPublish || snapshot?.identity !== owner()) return;
    const expectedHost = host, revision = remote.view_rev, viewId = remote.view_id, local = localVersion, key = graphStateKey(view);
    if (renderedRemote.view_id !== viewId || renderedRemote.view_rev !== revision || renderedRemote.key !== key) return;
    await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
    if (host !== expectedHost || remote?.view_rev !== revision || remote?.view_id !== viewId || local !== localVersion || graphStateKey(view) !== key || localPending || pendingCommands.length || initialPublish) return;
    const fullscreen = document.fullscreenElement === host ? 'active' : view.fullscreen_requested ? 'requires_user_gesture' : 'windowed';
    try { await api(`/api/admin/relations/views/${encodeURIComponent(viewId)}/ack`, { method: 'POST', body: { view_rev: revision, client_id: clientId, fullscreen }, signal: remoteController?.signal }); }
    catch (error) { if (![401, 403, 409].includes(error.status) && error.name !== 'AbortError') setRemoteNotice(txt('视图已更新，AI回执待重试', 'View updated; AI acknowledgement will retry')); }
  }
  async function receiveRemote(payload, expectedLocalVersion = localVersion) {
    const info = remoteInfo(payload), next = remoteState(payload);if (!info || !next || !canReceiveRemote(expectedLocalVersion)) return false;
    const same = graphStateKey(view) === graphStateKey(next);
    const full = graphStateKey(view.filters) !== graphStateKey(next.filters) || graphStateKey(view.grouping) !== graphStateKey(next.grouping) || info.data_rev !== snapshot?.overview?.data_rev;
    const focusChanged = graphStateKey(view.focus) !== graphStateKey(next.focus), needsNeighbors = next.mode === 'focus' && next.focus && (graphStateKey(snapshot?.neighborFocus) !== graphStateKey(next.focus) || !snapshot?.neighbors);
    const desired = { ...emptyGraphView(), ...next, hidden: { left: [], right: [], edges: [], ...next.hidden } };
    remote = info;renderedRemote = null;
    if (full) resetCursors();else if (focusChanged) { cursors.neighbors = [null];pageIndices.neighbors = 0; }
    if (full || needsNeighbors) { const changed = ++changeVersion, ok = await fetchData({ full, queryView: desired });if (changed !== changeVersion || !canReceiveRemote(expectedLocalVersion)) return false;if (!ok) { paint();return false; } }
    if (!canReceiveRemote(expectedLocalVersion)) return false;view = desired;
    if (view.ordering === 'stable' && snapshot?.overview) groupOrder = graphGroupOrder(snapshot.overview.groups, snapshot.overview.bundles, groupOrder);
    if (!same || full || needsNeighbors) { syncUrl();paint(full || focusChanged); }
    renderedRemote = { view_id: info.view_id, view_rev: info.view_rev, key: graphStateKey(view) };
    setRemoteNotice(view.fullscreen_requested && document.fullscreenElement !== host ? txt('AI请求了全屏；点击“全屏”完成浏览器授权。', 'AI requested fullscreen. Click Fullscreen to grant the browser gesture.') : txt(`AI视图已连接 · 版本 ${info.view_rev}`, `AI view connected · revision ${info.view_rev}`));
    await acknowledge();return true;
  }
  async function connectRemote() {
    const identity = owner(), version = generation, localRevision = localVersion;remoteController?.abort();remoteController = new AbortController();
    try {
      const payload = await api('/api/admin/relations/views', { method: 'POST', body: { mode: 'active', response: 'state' }, signal: remoteController.signal });
      if (!isCurrent(version, identity) || !host) return;
      if (view.mode === 'list' || new URLSearchParams(location.search).size > 1 || localRevision !== localVersion || localPending || pendingCommands.length) {
        remote = remoteInfo(payload);
        const desired = structuredClone(view), saved = remoteState(payload);
        if (saved && localRevision === localVersion && !localPending && !pendingCommands.length) {
          view = { ...emptyGraphView(), ...saved, filters: desired.filters, grouping: desired.grouping, mode: desired.mode, focus: desired.focus };
          paint();
        }
        initialPublish = saved || emptyGraphView();renderedRemote = null;
        await flushRemote();
      } else await receiveRemote(payload, localRevision);
    } catch (error) { if (error.name !== 'AbortError' && ![401, 403].includes(error.status)) setRemoteNotice(txt('AI视图连接暂不可用；本地查看不受影响。', 'AI view connection unavailable; local browsing remains available.')); }
    finally { if (isCurrent(version, identity) && host) remoteTimer = setTimeout(pollRemote, document.hidden ? 15000 : 3000); }
  }
  async function pollRemote() {
    if (!remote || !host || remoteController?.signal.aborted) return;
    try {
      if (!syncing && !pendingCommands.length && !localPending && !initialPublish) {
        const local = localVersion, after = renderedRemote?.view_id === remote.view_id && renderedRemote?.view_rev === remote.view_rev ? `&after_rev=${remote.view_rev}` : '';
        const payload = await api(`/api/admin/relations/views/active?view_id=${encodeURIComponent(remote.view_id)}${after}`, { fresh: true, signal: remoteController.signal });
        if (canReceiveRemote(local)) {
          if (!payload.unchanged) await receiveRemote(payload, local);
          else if (payload.data_rev !== snapshot?.overview?.data_rev) { resetCursors();if (await fetchData()) { paint();await acknowledge(); } else paint(); }
        }
      }
    } catch (error) { if ([401, 403].includes(error.status) || error.name === 'AbortError') return; }
    if (host) remoteTimer = setTimeout(pollRemote, document.hidden ? 15000 : 3000);
  }
  function queueRemote(commands) {
    if (!host || !commands.length) return;pendingCommands.push(...structuredClone(commands));clearTimeout(patchTimer);if (remote) patchTimer = setTimeout(flushRemote, 180);
  }
  async function flushRemote() {
    if (!remote || !host || syncing || localPending) return;
    if (initialPublish) { pendingCommands = stateCommands(view, initialPublish);initialPublish = null; }
    if (!pendingCommands.length) return;
    syncing = true;const commands = pendingCommands.splice(0, 20), id = globalThis.crypto?.randomUUID?.() || `${clientId}-${Date.now()}`, local = localVersion, version = generation, identity = owner();
    try {
      const payload = await api(`/api/admin/relations/views/${encodeURIComponent(remote.view_id)}`, { method: 'PATCH', body: { if_view_rev: remote.view_rev, operation_id: id, commands }, signal: remoteController.signal });
      if (!isCurrent(version, identity) || !host) return;
      const info = remoteInfo(payload);if (info) remote = info;
      if (canReceiveRemote(local)) {
        const canonical = await api(`/api/admin/relations/views/${encodeURIComponent(remote.view_id)}`, { fresh: true, signal: remoteController.signal });
        await receiveRemote(canonical, local);
      }
    } catch (error) {
      if (!isCurrent(version, identity) || !host) return;
      if (error.status === 409) {
        const canonical = await api(`/api/admin/relations/views/${encodeURIComponent(remote.view_id)}`, { fresh: true, signal: remoteController.signal });
        if (canReceiveRemote(local)) { await receiveRemote(canonical, local);setRemoteNotice(txt('AI与本地同时更新了视图，已同步最新版本，请重试刚才的操作。', 'The view changed concurrently. Latest state restored; retry the last action.')); }
        else { remote = remoteInfo(canonical) || remote;initialPublish = remoteState(canonical) || emptyGraphView(); }
      }
      else if (error.name !== 'AbortError' && ![401, 403].includes(error.status)) setRemoteNotice(txt('AI同步失败；本地更改尚未同步。', 'AI sync failed; local changes have not been synchronized.'));
    } finally { syncing = false;if ((pendingCommands.length || initialPublish) && host) patchTimer = setTimeout(flushRemote, 180); }
  }
  function submit(form, data) {
    const mapping = form.dataset.list === 'people' ? [['people_q', 'user_q'], ['people_role', 'role'], ['people_status', 'account_status']] : form.dataset.list === 'jobs' ? [['job_q', 'job_q'], ['job_status', 'job_status']] : [['graph_status', 'match_status']];
    const filters = { ...view.filters };for (const [name, key] of mapping) { const value = String(data.get(name) || '').trim();if (value) filters[key] = value;else delete filters[key]; }
    if (form.dataset.list !== 'relations') { delete filters.left_group_ids;delete filters.right_group_ids; }
    if (host) return change([{ op: 'set_filters', filters }, ...(form.dataset.list === 'relations' ? [] : [{ op: 'set_focus', focus: null }])]);
    const changes = Object.fromEntries(mapping.map(([name]) => [name, String(data.get(name) || '').trim()]));changes[form.dataset.list === 'people' ? 'people_page' : 'jobs_page'] = null;if (form.dataset.list !== 'relations') Object.assign(changes, { left_group: null, right_group: null, focus_side: null, focus_id: null });return go(peopleJobsUrl(location.search, changes));
  }
  function destroy() { generation++;requestVersion++;cleanup?.();cleanup = null;requestController?.abort();remoteController?.abort();clearTimeout(remoteTimer);clearTimeout(patchTimer);pendingCommands = [];initialPublish = null;renderedRemote = null;snapshot = null;host = null; }
  return { page, mount, destroy, submit, getViewState: () => structuredClone(view), applyViewCommands: commands => change(commands) };
}
