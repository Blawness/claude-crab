const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const make = (tag, cls, text) => {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text != null) n.textContent = text;
  return n;
};
const minutes = (from, to = Date.now()) => Math.max(0, Math.round((to - from) / 60000));

const open = new Set(); // agents whose output is expanded
let sessions = {};      // live hook state by session id

function showError(message) {
  $('form-error').textContent = message || '';
  $('form-error').hidden = !message;
}

$('browse').addEventListener('click', async () => {
  const picked = await invoke('pick_folder');
  if (picked) $('folder').value = picked;
});

$('run').addEventListener('click', async () => {
  showError('');
  $('run').disabled = true;
  try {
    await invoke('agent_spawn', {
      folder: $('folder').value.trim(),
      prompt: $('prompt').value,
      worktree: $('worktree').checked,
      board: fromBoard,
    });
    $('prompt').value = '';
    fromBoard = false;
    await refresh();
  } catch (e) {
    showError(String(e));
  } finally {
    $('run').disabled = false;
  }
});

/* ---------- Board tasks ---------- */
let fromBoard = false; // the prompt in the form came from a Board task, so the agent may report back
$('prompt').addEventListener('input', () => { if (!$('prompt').value.trim()) fromBoard = false; });

function boardPrompt(t) {
  return [
    `Board task: ${t.title}`,
    t.description || '',
    '',
    `When you finish, add a comment on this Board task (id ${t.id}, workspace ${t.workspace}; find it by title if the id does not match) that summarises what you did. Do not change the task's status.`,
  ].join('\n');
}

function renderBoard(tasks, when) {
  const box = $('board');
  box.textContent = '';
  $('board-note').textContent = tasks.length
    ? `${tasks.length} open tasks, loaded ${new Date(when).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })}.`
    : 'No open tasks on Board.';
  for (const t of tasks) {
    const card = make('div', 'agent');
    card.append(make('div', 'agent-title', t.title));
    card.title = t.description || '';
    const meta = [t.status, t.workspace];
    if (t.due) meta.push('due ' + t.due);
    if (t.assignee) meta.push(t.assignee);
    card.append(make('div', 'agent-meta', meta.filter(Boolean).join(' · ')));
    const actions = make('div', 'agent-actions');
    const assign = make('button', '', 'Assign to Claude');
    assign.addEventListener('click', () => {
      $('prompt').value = boardPrompt(t);
      fromBoard = true;
      showError('');
      $('new-task').scrollIntoView({ behavior: 'smooth' });
      ($('folder').value.trim() ? $('run') : $('folder')).focus();
    });
    actions.append(assign);
    card.append(actions);
    box.append(card);
  }
}

$('board-refresh').addEventListener('click', async () => {
  $('board-refresh').disabled = true;
  $('board-note').textContent = 'Asking Claude for the task list. This takes about a minute…';
  try {
    const tasks = await invoke('board_tasks');
    const saved = { tasks, when: Date.now() };
    try { localStorage.setItem('board', JSON.stringify(saved)); } catch (e) {}
    renderBoard(tasks, saved.when);
  } catch (e) {
    $('board-note').textContent = String(e);
  } finally {
    $('board-refresh').disabled = false;
  }
});

// Show the last list right away; it is only as fresh as its timestamp says.
try {
  const saved = JSON.parse(localStorage.getItem('board') || 'null');
  if (saved && Array.isArray(saved.tasks)) renderBoard(saved.tasks, saved.when);
} catch (e) {}

// What an agent is doing right now: its live hook state while it runs, its exit after.
function statusOf(agent) {
  const live = sessions[agent.id];
  if (agent.status === 'running') {
    if (live && live.state === 'waiting') return ['waiting', 'waiting for you'];
    return ['working', live && live.tool ? `using ${live.tool}` : 'working'];
  }
  if (agent.status === 'done') return ['done', 'finished'];
  if (agent.status === 'stopped') return ['stopped', 'stopped'];
  if (agent.status === 'detached') return ['stopped', 'lost track (widget was restarted)'];
  return ['failed', `failed${agent.exit != null ? ' (exit ' + agent.exit + ')' : ''}`];
}

function agentCard(agent) {
  const [cls, label] = statusOf(agent);
  const live = sessions[agent.id] || {};
  const card = make('div', 'agent ' + cls);
  card.dataset.id = agent.id;

  const head = make('div', 'agent-head');
  head.append(make('span', 'dot'), make('span', 'agent-title', agent.prompt.split('\n')[0]));
  card.append(head);
  card.title = agent.prompt;

  const meta = [label];
  meta.push(agent.status === 'running' ? `${minutes(agent.started)} min` : `ran ${minutes(agent.started, agent.ended || Date.now())} min`);
  if (live.total && agent.status === 'running') meta.push(`step ${live.done}/${live.total}`);
  meta.push(agent.worktree ? `worktree ${agent.worktree}` : 'in place');
  meta.push(agent.folder);
  card.append(make('div', 'agent-meta', meta.join(' · ')));

  const actions = make('div', 'agent-actions');
  const toggle = make('button', '', open.has(agent.id) ? 'Hide output' : 'Show output');
  toggle.addEventListener('click', () => {
    open.has(agent.id) ? open.delete(agent.id) : open.add(agent.id);
    refresh();
  });
  const folder = make('button', '', 'Open folder');
  folder.addEventListener('click', () => invoke('open_folder', { path: agent.folder }));
  actions.append(toggle, folder);
  if (agent.status === 'running') {
    const stop = make('button', 'danger', 'Stop');
    stop.addEventListener('click', async () => { await invoke('agent_stop', { id: agent.id }); refresh(); });
    actions.append(stop);
  } else {
    const remove = make('button', '', 'Remove');
    remove.addEventListener('click', async () => { await invoke('agent_remove', { id: agent.id }); refresh(); });
    actions.append(remove);
  }
  card.append(actions);

  if (open.has(agent.id)) card.append(make('pre', '', agent.output || '(nothing printed yet)'));
  return card;
}

let lastKey = '';
async function refresh() {
  const [agents, state] = await Promise.all([invoke('agent_list'), invoke('get_state')]);

  sessions = {};
  const folders = new Set();
  for (const s of (state.task && state.task.sessions) || []) {
    sessions[s.session] = s;
    if (s.cwd) folders.add(s.cwd);
  }
  agents.forEach((a) => folders.add(a.folder));

  const data = (state.usage && state.usage.data) || {};
  const pct = pctOf(data.five_hour);
  $('usage').textContent = pct == null ? 'Usage: -' : `5-hour: ${pct}% used`;
  $('usage').classList.toggle('high', pct != null && pct >= 75);
  renderHome(data, (state.task && state.task.sessions) || []);

  // Rebuild only when something visible changed, so open output keeps its scroll position.
  const key = JSON.stringify([agents, [...open], Object.values(sessions).map((s) => [s.session, s.state, s.tool, s.done, s.total]), Math.floor(Date.now() / 60000)]);
  if (key === lastKey) return;
  lastKey = key;

  const running = agents.filter((a) => a.status === 'running').length;
  $('agent-count').textContent = agents.length ? `(${running} running)` : '';
  const box = $('agents');
  box.textContent = '';
  if (!agents.length) box.append(make('div', 'empty', 'No agents yet. Start one above.'));
  agents.forEach((a) => box.append(agentCard(a)));

  const list = $('recent-folders');
  list.textContent = '';
  for (const f of folders) {
    const option = document.createElement('option');
    option.value = f;
    list.append(option);
  }
}

/* ---------- Home ---------- */
const pctOf = (b) => (b && typeof b.utilization === 'number' ? Math.round(b.utilization) : null);
function untilText(iso) {
  if (!iso) return '';
  const mins = minutes(Date.now(), new Date(iso).getTime());
  return mins >= 60 ? `resets in ${Math.floor(mins / 60)} h ${mins % 60} min` : `resets in ${mins} min`;
}
function stat(id, bucket) {
  const p = pctOf(bucket);
  $(id).textContent = p == null ? '-' : `${p}%`;
  $(id + '-bar').style.width = `${p == null ? 0 : Math.min(100, p)}%`;
  $(id + '-bar').classList.toggle('high', p != null && p >= 75);
  $(id + '-sub').textContent = bucket ? untilText(bucket.resets_at) : 'No data yet';
}
function renderHome(data, list) {
  stat('h-session', data.five_hour);
  stat('h-week', data.seven_day);
  const waiting = list.filter((s) => s.state === 'waiting').length;
  const working = list.filter((s) => s.state === 'working').length;
  $('h-count').textContent = list.length ? `(${working} working, ${waiting} need you)` : '';
  const box = $('h-sessions');
  box.textContent = '';
  if (!list.length) box.append(make('div', 'empty', 'No active sessions.'));
  for (const s of list) {
    const chat = s.kind === 'chat';
    const cls = s.state === 'waiting' ? 'waiting' : s.state === 'working' ? 'working' : 'done';
    const row = make('div', 'agent clickable ' + cls);
    const head = make('div', 'agent-head');
    head.append(make('span', 'dot'), make('span', 'agent-title', s.title || s.project || (chat ? 'Untitled chat' : 'Session')));
    row.append(head);
    const meta = [chat ? 'Claude Chat' : 'Claude Code'];
    if (!chat && s.project) meta.push(s.project);
    meta.push(s.state === 'waiting' ? 'waiting for you' : s.state === 'working' ? (s.tool ? `using ${s.tool}` : 'working') : 'done');
    if (s.total) meta.push(`step ${s.done}/${s.total}`);
    row.append(make('div', 'agent-meta', meta.join(' · ')));
    row.title = 'Click to open';
    row.addEventListener('click', () => invoke('open_session', { session: s.session }));
    if (chat) {
      const close = make('button', 'close', '✕');
      close.title = 'Remove this chat from the list';
      close.addEventListener('click', (e) => {
        e.stopPropagation();
        row.remove();
        invoke('dismiss_session', { session: s.session });
      });
      head.append(close);
    }
    box.append(row);
  }
}

/* ---------- Settings: each switch runs the tray menu entry with the same id ---------- */
async function loadSettings() {
  const s = await invoke('get_settings');
  document.querySelectorAll('[data-key]').forEach((box) => { box.checked = !!s[box.dataset.key]; });
  const sel = $('interval');
  if (!sel.options.length) {
    for (const [secs, label] of s.intervals) sel.append(new Option(label, secs));
  }
  sel.value = String(s.interval);
  const style = $('sound-style');
  if (!style.options.length) {
    for (const [id, label] of s.styles) style.append(new Option(label, id));
  }
  style.value = s.style;
}
document.querySelectorAll('[data-key]').forEach((box) =>
  box.addEventListener('change', async () => { await invoke('set_setting', { id: box.dataset.key }); loadSettings(); }));
$('interval').addEventListener('change', () => invoke('set_setting', { id: `every-${$('interval').value}` }));
$('sound-style').addEventListener('change', () => invoke('set_setting', { id: `style-${$('sound-style').value}` }));
document.querySelectorAll('[data-set]').forEach((btn) =>
  btn.addEventListener('click', () => invoke('set_setting', { id: btn.dataset.set })));

/* ---------- navigation: the page follows the #hash, so Rust can open any page ---------- */
// #swarm/<agent id> (a click on an agent in the widget) expands that agent and scrolls to it.
async function route() {
  const [name, agentId] = location.hash.slice(1).split('/');
  const page = ['home', 'swarm', 'settings'].includes(name) ? name : 'home';
  document.querySelectorAll('.page').forEach((p) => { p.hidden = p.id !== 'page-' + page; });
  document.querySelectorAll('nav a').forEach((a) => a.classList.toggle('active', a.dataset.page === page));
  if (page === 'settings') loadSettings();
  if (page === 'swarm' && agentId) {
    open.add(agentId);
    await refresh();
    const card = document.querySelector(`.agent[data-id="${CSS.escape(agentId)}"]`);
    if (card) {
      card.scrollIntoView({ block: 'center' });
      card.classList.add('flash');
      setTimeout(() => card.classList.remove('flash'), 1500);
    }
  }
}
window.addEventListener('hashchange', route);
route();

refresh();
setInterval(() => {
  refresh();
  if (location.hash === '#settings' && !document.activeElement.matches('select')) loadSettings(); // tray changes show up here too
}, 2000);
