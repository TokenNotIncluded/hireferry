/* One directory for people, roles and the real introductions between them. */
const fields = ['people_q', 'people_role', 'people_status', 'people_page', 'job_q', 'job_status', 'jobs_page', 'graph_status'];

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

export function createPeopleJobsUI({ state, api, t, esc, icon, avatar, money, date, shell, badge, matchText, go }) {
  let generation = 0, snapshot = null, cleanup = null;
  const owner = () => `${state.admin}:${state.owner}:${state.staff?.id || ''}:${state.user?.id || ''}:${state.csrf || ''}`;
  const currentParams = () => new URLSearchParams(peopleJobsUrl(location.search).split('?')[1]);
  const url = changes => peopleJobsUrl(location.search, changes);
  const roleName = role => role === 'seeker' ? t('求职者', 'Job seeker') : role === 'employer' ? t('招聘方', 'Employer') : t('待选择身份', 'Role not selected');
  const viewUrl = (view, extra = {}) => '/admin?' + new URLSearchParams({ tab: 'matches', view, ...extra });
  const option = (value, label, selected) => `<option value="${esc(value)}"${value === selected ? ' selected' : ''}>${esc(label)}</option>`;

  function pager(kind, data) {
    const key = kind === 'people' ? 'people_page' : 'jobs_page';
    const page = Math.max(1, Number(data.page) || 1), size = Math.max(1, Number(data.page_size) || 50);
    const count = Number(data.total) || 0;
    const link = (next, label, name) => `<a class="icon-button" href="${esc(url({ [key]: next }))}" data-nav aria-label="${esc(label)}">${icon(name)}</a>`;
    return `<nav class="pj-pagination" aria-label="${kind === 'people' ? t('用户分页', 'People pages') : t('岗位分页', 'Role pages')}"><span>${t(`本页 ${data.items.length} / 共 ${count}`, `${data.items.length} here / ${count} total`)}</span><div>${page > 1 ? link(page - 1, t('上一页', 'Previous page'), 'back') : ''}${page * size < count ? link(page + 1, t('下一页', 'Next page'), 'arrow') : ''}</div></nav>`;
  }

  function filters(kind, p) {
    const people = kind === 'people';
    const key = people ? 'people_q' : 'job_q';
    return `<form data-form="people-jobs-filter" data-list="${kind}" class="pj-search"><div class="pj-search-input">${icon('search')}<input class="input" name="${key}" value="${esc(p.get(key) || '')}" maxlength="100" aria-label="${people ? t('搜索用户名称或邮箱', 'Search people by name or email') : t('搜索岗位标题或内容', 'Search role titles or descriptions')}" placeholder="${people ? t('搜索用户', 'Search people') : t('搜索岗位', 'Search roles')}"><button class="icon-button" type="submit" aria-label="${t('搜索', 'Search')}">${icon('arrow')}</button></div><div class="pj-filter-fields">${people ? `<select class="input" name="people_role" aria-label="${t('用户身份', 'Person role')}">${[['', t('全部用户', 'All people')], ['seeker', t('求职者', 'Job seekers')], ['employer', t('招聘方', 'Employers')]].map(([value, label]) => option(value, label, p.get('people_role') || '')).join('')}</select><select class="input" name="people_status" aria-label="${t('账户状态', 'Account access')}">${[['', t('全部状态', 'All access')], ['active', t('正常', 'Active')], ['suspended', t('已封禁', 'Suspended')]].map(([value, label]) => option(value, label, p.get('people_status') || '')).join('')}</select>` : `<select class="input" name="job_status" aria-label="${t('岗位状态', 'Role status')}">${[['', t('全部岗位', 'All roles')], ['published', t('已发布', 'Published')], ['draft', t('草稿', 'Drafts')], ['closed', t('已关闭', 'Closed')], ['deleted', t('已删除', 'Deleted')]].map(([value, label]) => option(value, label, p.get('job_status') || '')).join('')}</select>`}</div></form>`;
  }

  function personCard(person, relations, unavailable) {
    const count = unavailable ? '—' : relations.filter(match => match.seeker_id === person.id).length;
    return `<article class="pj-card pj-person" data-graph-node="person:${esc(person.id)}"><button type="button" class="pj-node-select" data-graph-select="person" data-id="${esc(person.id)}" aria-pressed="false" aria-label="${esc(t(`选择用户 ${person.name}`, `Select person ${person.name}`))}">${avatar(person, 'small')}<span><strong>${esc(person.name || person.email)}</strong><small>${esc(roleName(person.role))}${person.banned ? ` · ${t('已封禁', 'Suspended')}` : ''}</small></span><span class="pj-port" aria-hidden="true"></span></button><p class="pj-card-meta">${esc(person.email)}</p><div class="pj-card-foot"><span class="pj-link-count">${icon('link')}${count} ${t('条已显示关系', 'shown links')}</span><button type="button" class="text-button" data-action="admin-edit-user" data-id="${esc(person.id)}">${t('资料', 'Profile')}</button>${state.owner || state.staff ? `<button type="button" class="text-button pj-access" data-action="admin-user-access" data-id="${esc(person.id)}">${person.banned ? t('恢复', 'Restore') : t('封禁', 'Suspend')}</button>` : ''}</div></article>`;
  }

  function jobCard(job, relations, people, unavailable) {
    const count = unavailable ? '—' : relations.filter(match => match.job_id === job.id).length;
    const employer = people.find(person => person.id === job.owner_id);
    const status = job.status === 'published' && Number(job.headcount) === 0 ? t('已暂停', 'Paused') : ({ published: t('招聘中', 'Hiring'), draft: t('草稿', 'Draft'), closed: t('已关闭', 'Closed'), deleted: t('已删除', 'Deleted') })[job.status] || job.status;
    return `<article class="pj-card pj-job" data-graph-node="job:${esc(job.id)}"><button type="button" class="pj-node-select" data-graph-select="job" data-id="${esc(job.id)}" aria-pressed="false" aria-label="${esc(t(`选择岗位 ${job.title}`, `Select role ${job.title}`))}"><span class="pj-port" aria-hidden="true"></span><span class="pj-job-icon">${icon('briefcase')}</span><span><strong>${esc(job.title || t('已永久删除的岗位', 'Permanently deleted role'))}</strong><small>${esc(status)} · ${Number(job.headcount) || 0} ${t('人', 'people')}</small></span></button><p class="pj-card-meta"><strong>${money(job.salary_cents, job)}</strong><span> / ${t('月底薪', 'mo base')}</span>${employer ? `<span class="pj-employer">${esc(employer.name)}</span>` : ''}</p><div class="pj-card-foot"><span class="pj-link-count">${icon('link')}${count} ${t('条已显示关系', 'shown links')}</span><button class="text-button" type="button" data-action="admin-edit-job" data-id="${esc(job.id)}">${state.owner || state.staff ? t('管理', 'Manage') : t('详情', 'Details')}</button><a class="text-button" href="${esc(viewUrl('search', { job: job.id }))}" data-nav ${job.status !== 'published' || Number(job.headcount) <= 0 ? 'hidden' : ''}>${t('找人', 'Find people')}</a></div></article>`;
  }

  function relationList(relations) {
    return relations.map(match => `<li data-graph-relation="${esc(match.id)}"><span><strong>${esc(match.seeker_name)}</strong><span aria-hidden="true">${icon('arrow')}</span><strong>${esc(match.job_title)}</strong></span><span class="status ${match.status === 'declined' ? 'closed' : ''}">${esc(matchText(match.status))}</span><button type="button" class="text-button" data-action="match-detail" data-id="${esc(match.id)}">${t('跟进', 'Follow up')}${icon('arrow')}</button></li>`).join('');
  }

  async function page() {
    const version = ++generation, identity = owner(), params = currentParams();
    const peopleQuery = new URLSearchParams(), jobsQuery = new URLSearchParams();
    for (const [from, to] of [['people_q', 'q'], ['people_role', 'role'], ['people_status', 'status'], ['people_page', 'page']]) if (params.get(from)) peopleQuery.set(to, params.get(from));
    for (const [from, to] of [['job_q', 'q'], ['job_status', 'status'], ['jobs_page', 'page']]) if (params.get(from)) jobsQuery.set(to, params.get(from));
    const [people, jobs] = await Promise.all([api('/api/admin/users?' + peopleQuery), api('/api/admin/jobs?' + jobsQuery)]);
    if (version !== generation || identity !== owner()) return '';
    people.items ||= []; jobs.items ||= [];
    let result = { items: [], total: 0 }, relationError = false;
    if (people.items.some(person => person.role === 'seeker') && jobs.items.length) {
      try { result = await api('/api/admin/workbench/matches?' + graphMatchQuery(people.items, jobs.items, params.get('graph_status') || '')); }
      catch (error) { if ([401, 403].includes(error.status)) throw error; relationError = true; }
    }
    if (version !== generation || identity !== owner()) return '';
    const relations = graphRelations(people.items, jobs.items, result.items || []);
    const truncated = Number(result.total) > relations.length;
    // Keep connected roles adjacent to the first related person on the left.
    const order = new Map(people.items.map((person, index) => [person.id, index]));
    const rank = job => Math.min(people.items.length, ...relations.filter(match => match.job_id === job.id).map(match => order.get(match.seeker_id)));
    const orderedJobs = [...jobs.items].sort((a, b) => rank(a) - rank(b));
    snapshot = { version, identity, people: people.items, jobs: orderedJobs, relations };
    const noRows = label => `<div class="pj-empty">${icon('search')}<strong>${label}</strong><span>${t('试试其他筛选条件', 'Try another filter')}</span></div>`;
    const statusFilters = [['', t('全部关系', 'All stages')], ['introduced', t('已介绍', 'Introduced')], ['interview', t('面试中', 'Interview')], ['hired', t('已录用', 'Hired')], ['declined', t('已结束', 'Ended')]];
    return shell(`<div class="people-jobs-workspace" data-graph-page="${version}"><header class="pj-page-header"><div><h1>${t('用户与岗位', 'People & roles')}</h1><p>${t('选择两侧卡片查看关联，实线表示已有匹配。', 'Select cards on either side. Solid lines show existing introductions.')}</p></div><a class="button secondary small" href="${esc(viewUrl('search'))}" data-nav>${icon('plus')}${t('建立匹配', 'Make introduction')}</a></header><nav class="wb-tabs pj-view-tabs" aria-label="${t('匹配工作台视图', 'Matching workspace views')}"><a href="${esc(url({}))}" data-nav aria-current="page">${icon('link')}${t('关系视图', 'Connections')}</a><a href="${esc(viewUrl('search'))}" data-nav>${icon('search')}${t('筛选候选人', 'Find candidates')}</a><a href="${esc(viewUrl('followups'))}" data-nav>${icon('board')}${t('匹配跟进', 'Follow-ups')}</a></nav><section class="pj-board"><header class="pj-board-bar"><div class="pj-board-legend"><span><i></i>${t('已有匹配', 'Introduced')}</span><span><i class="ended"></i>${t('已结束', 'Ended')}</span><span data-graph-preview-legend hidden><i class="preview"></i>${t('待确认', 'Pending review')}</span>${truncated ? `<small class="pj-relation-limit">${t(`显示前 ${relations.length} / ${Number(result.total)} 条`, `First ${relations.length} of ${Number(result.total)} links`)}</small>` : ''}</div><form data-form="people-jobs-filter" data-list="relations"><select class="input" name="graph_status" aria-label="${t('匹配阶段', 'Introduction stage')}">${statusFilters.map(([value, label]) => option(value, label, params.get('graph_status') || '')).join('')}</select><button class="text-button" type="submit">${t('筛选', 'Filter')}</button></form></header><div class="pj-scroll" tabindex="0" role="region" aria-label="${t('用户与岗位双列关系图', 'People and roles connection map')}"><div class="pj-stage"><section class="pj-column pj-people-column" aria-labelledby="pj-people-title"><header class="pj-column-heading"><h2 id="pj-people-title">${icon('person')}${t('用户', 'People')}</h2><span>${Number(people.total) || 0}</span></header>${filters('people', params)}<div class="pj-nodes" data-graph-scroll="people">${people.items.length ? people.items.map(person => personCard(person, relations, relationError)).join('') : noRows(t('没有符合条件的用户', 'No people found'))}</div>${pager('people', people)}</section><div class="pj-crossing" aria-hidden="true">${icon('link')}</div><section class="pj-column pj-jobs-column" aria-labelledby="pj-jobs-title"><header class="pj-column-heading"><h2 id="pj-jobs-title">${icon('briefcase')}${t('岗位', 'Roles')}</h2><span>${Number(jobs.total) || 0}</span></header>${filters('jobs', params)}<div class="pj-nodes" data-graph-scroll="jobs">${jobs.items.length ? orderedJobs.map(job => jobCard(job, relations, people.items, relationError)).join('') : noRows(t('没有符合条件的岗位', 'No roles found'))}</div>${pager('jobs', jobs)}</section><svg class="pj-connections" aria-hidden="true"></svg></div></div><footer class="pj-composer"><div class="pj-selection"><span data-graph-person-label>${t('选择一个求职者', 'Select a job seeker')}</span>${icon('link')}<span data-graph-job-label>${t('选择一个岗位', 'Select a role')}</span></div><div><button type="button" class="text-button" data-graph-clear hidden>${t('清除选择', 'Clear selection')}</button><button type="button" class="button secondary small" data-graph-followup hidden>${t('查看已有匹配', 'View introduction')}</button><a class="button small" data-graph-introduce data-nav hidden>${t('查看并匹配', 'Review & introduce')}${icon('arrow')}</a><span class="help" data-graph-hint role="status">${t('选择双方后，核对资料再确认匹配', 'Select both sides, then review before introducing')}</span></div></footer></section><div class="pj-scope-note" role="status">${relationError ? `<span class="pj-load-error">${t('匹配关系暂未读取成功；卡片上的数量暂不可用。', 'Introductions could not be loaded; card counts are temporarily unavailable.')}</span><button type="button" class="text-button" data-action="retry">${t('重试', 'Retry')}</button>` : `<span>${t(`本页显示 ${relations.length} 条关系${truncated ? `（共 ${Number(result.total)} 条）` : ''}；连线仅连接当前两列中的记录。`, `Showing ${relations.length}${truncated ? ` of ${Number(result.total)}` : ''} introductions between records on these pages.`)}</span>${truncated ? `<a class="text-button" href="${esc(viewUrl('followups'))}" data-nav>${t('查看全部跟进', 'View all follow-ups')}${icon('arrow')}</a>` : ''}`}</div><details class="pj-relation-list"${relations.length ? ' open' : ''}><summary>${t('匹配记录', 'Introduction records')}<span>${relationError ? '—' : relations.length}</span></summary><ul>${relationList(relations)}</ul>${!relations.length ? `<p class="help">${relationError ? t('读取失败，请重试。', 'Could not load. Try again.') : t('当前两页之间没有符合筛选的匹配。可切换用户或岗位页，或查看全部跟进。', 'No introductions between these pages match the selected stage. Change a page or open all follow-ups.')}</p>` : ''}</details></div>`, 'matches', true);
  }

  function mount(root = document) {
    cleanup?.(); cleanup = null;
    const host = root.querySelector?.('.people-jobs-workspace');
    const data = snapshot;
    if (!host || !data || Number(host.dataset.graphPage) !== data.version || data.identity !== owner()) return;
    const stage = host.querySelector('.pj-stage'), svg = host.querySelector('.pj-connections');
    const nodes = new Map([...host.querySelectorAll('[data-graph-node]')].map(node => [node.dataset.graphNode, node]));
    const scrolls = [...host.querySelectorAll('[data-graph-scroll]')];
    let selectedPerson = '', selectedJob = '', frame = null, alive = true;
    const related = match => (!selectedPerson || match.seeker_id === selectedPerson) && (!selectedJob || match.job_id === selectedJob);
    const bounds = node => {
      const box = node.querySelector('.pj-port').getBoundingClientRect();
      const clip = node.closest('.pj-nodes').getBoundingClientRect();
      const y = box.top + box.height / 2;
      return y < clip.top || y > clip.bottom ? null : { x: box.left + box.width / 2, y };
    };
    function draw() {
      frame = null;
      if (!alive || data.identity !== owner()) return;
      const box = stage.getBoundingClientRect();
      svg.setAttribute('viewBox', `0 0 ${Math.max(1, box.width)} ${Math.max(1, box.height)}`);
      let lines = data.relations.map(match => {
        const left = nodes.get('person:' + match.seeker_id), right = nodes.get('job:' + match.job_id);
        const a = left && bounds(left), b = right && bounds(right);
        if (!a || !b) return '';
        const start = { x: a.x - box.left, y: a.y - box.top }, end = { x: b.x - box.left, y: b.y - box.top };
        return `<path d="${connectionPath(start, end)}" class="pj-connection ${match.status === 'declined' ? 'is-ended' : ''} ${selectedPerson || selectedJob ? related(match) ? 'is-highlighted' : 'is-muted' : ''}" data-action="match-detail" data-id="${esc(match.id)}"><title>${esc(match.seeker_name)} → ${esc(match.job_title)} · ${esc(matchText(match.status))}</title></path>`;
      }).join('');
      const preview = graphPreview(data.people, data.jobs, data.relations, selectedPerson, selectedJob);
      if (preview) {
        const a = bounds(nodes.get('person:' + preview.seeker_id)), b = bounds(nodes.get('job:' + preview.job_id));
        if (a && b) {
          const start = { x: a.x - box.left, y: a.y - box.top }, end = { x: b.x - box.left, y: b.y - box.top };
          const x = (start.x + end.x) / 2, y = (start.y + end.y) / 2;
          lines += `<g data-graph-preview><path d="${connectionPath(start, end)}" class="pj-connection is-preview"/><g class="pj-preview-label" transform="translate(${x},${y})"><rect x="-22" y="-11" width="44" height="22" rx="6"/><text y="3.5" text-anchor="middle">${t('待确认', 'Review')}</text></g></g>`;
        }
      }
      svg.innerHTML = lines;
    }
    const schedule = () => { if (frame === null) frame = requestAnimationFrame(draw); };
    function refresh() {
      const person = data.people.find(item => item.id === selectedPerson), job = data.jobs.find(item => item.id === selectedJob);
      for (const [key, node] of nodes) {
        const selected = key === 'person:' + selectedPerson || key === 'job:' + selectedJob;
        node.classList.toggle('is-selected', selected);
        node.querySelector('[data-graph-select]').setAttribute('aria-pressed', String(selected));
        node.classList.toggle('is-related', !selected && data.relations.some(match => related(match) && (key === 'person:' + match.seeker_id || key === 'job:' + match.job_id)) && !!(selectedPerson || selectedJob));
      }
      host.querySelector('[data-graph-person-label]').textContent = person?.name || person?.email || t('选择一个求职者', 'Select a job seeker');
      host.querySelector('[data-graph-job-label]').textContent = job?.title || t('选择一个岗位', 'Select a role');
      host.querySelector('[data-graph-clear]').hidden = !selectedPerson && !selectedJob;
      const existing = person && job && data.relations.find(match => match.seeker_id === person.id && match.job_id === job.id);
      const followup = host.querySelector('[data-graph-followup]');
      followup.hidden = !existing;
      if (existing) { followup.dataset.action = 'match-detail'; followup.dataset.id = existing.id; }
      else { delete followup.dataset.action; delete followup.dataset.id; }
      const introduce = host.querySelector('[data-graph-introduce]');
      const canReview = person?.role === 'seeker' && !person.banned && job?.status === 'published' && Number(job.headcount) > 0;
      introduce.hidden = !!existing || !canReview;
      if (canReview) introduce.href = viewUrl('search', { job: job.id, candidate: person.id });
      else introduce.removeAttribute('href');
      const preview = graphPreview(data.people, data.jobs, data.relations, selectedPerson, selectedJob);
      host.querySelector('[data-graph-preview-legend]').hidden = !preview;
      const hint = host.querySelector('[data-graph-hint]');
      hint.hidden = !!existing;
      hint.textContent = preview ? t('待确认 · 打开双方资料核对', 'Pending review · check both profiles') : t('选择双方后，核对资料再确认匹配', 'Select both sides, then review before introducing');
      for (const row of host.querySelectorAll('[data-graph-relation]')) {
        const match = data.relations.find(item => item.id === row.dataset.graphRelation);
        row.hidden = !related(match);
      }
      schedule();
    }
    function onClick(event) {
      if (!alive || data.identity !== owner()) return;
      const card = event.target.closest('[data-graph-node]');
      const button = event.target.closest('[data-graph-select]') || (!event.target.closest('a,button,input,select,textarea') && card?.querySelector('[data-graph-select]'));
      if (button && host.contains(button)) {
        const kind = button.dataset.graphSelect, id = button.dataset.id;
        if (kind === 'person') selectedPerson = selectedPerson === id ? '' : id;
        else selectedJob = selectedJob === id ? '' : id;
        refresh();
      } else if (event.target.closest('[data-graph-clear]')) { selectedPerson = ''; selectedJob = ''; refresh(); }
    }
    function onChange(event) { const form = event.target.closest('form[data-form="people-jobs-filter"]'); if (event.target.tagName === 'SELECT' && form) form.requestSubmit(); }
    host.addEventListener('click', onClick);
    host.addEventListener('change', onChange);
    scrolls.forEach(node => node.addEventListener('scroll', schedule, { passive: true }));
    window.addEventListener('resize', schedule);
    const observer = globalThis.ResizeObserver ? new ResizeObserver(schedule) : null;
    observer?.observe(stage);
    for (const node of nodes.values()) observer?.observe(node);
    schedule();
    cleanup = () => {
      alive = false; if (frame !== null) cancelAnimationFrame(frame);
      observer?.disconnect(); window.removeEventListener('resize', schedule);
      host.removeEventListener('click', onClick); host.removeEventListener('change', onChange);
      scrolls.forEach(node => node.removeEventListener('scroll', schedule));
    };
  }

  function submit(form, data) {
    const allowed = form.dataset.list === 'people' ? ['people_q', 'people_role', 'people_status'] : form.dataset.list === 'jobs' ? ['job_q', 'job_status'] : ['graph_status'];
    const changes = Object.fromEntries(allowed.map(key => [key, String(data.get(key) || '').trim()]));
    if (form.dataset.list === 'people') changes.people_page = null;
    if (form.dataset.list === 'jobs') changes.jobs_page = null;
    return go(url(changes));
  }

  function destroy() { generation += 1; cleanup?.(); cleanup = null; snapshot = null; }
  return { page, mount, destroy, submit };
}
