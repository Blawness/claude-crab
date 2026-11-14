const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const stage = $('stage');
const panel = $('panel');

const settings = { remaining: false, interval: 300 };
// Data counts as stale once two checks in a row have been missed.
const staleMs = () => (settings.interval * 2 + 120) * 1000;
const CPU_HOT = 85;
const CPU_HOT_TICKS = 3; // x 2s
const RAM_FULL = 90;

const view = {
  usage: null,      // last good usage payload
  usageAt: 0,
  usageState: 'waiting',
  usageStatus: null,
  history: [],      // [{ t, pct }] of session utilization, last hour
  hotTicks: 0,
  danceUntil: 0,
  lastSessionPct: null,
};

/* ---------- helpers ---------- */
function setRing(id, pct) {
  const el = $(id);
  const r = el.r.baseVal.value;
  const c = 2 * Math.PI * r;
  const p = Math.round(Math.max(0, Math.min(100, pct || 0)));
  if (el.dataset.p === String(p)) return;
  el.dataset.p = String(p);
  el.style.strokeDasharray = `${(c * p) / 100} ${c}`;
}

const shown = (pct) => (settings.remaining ? 100 - pct : pct);
const pctText = (pct) => (pct == null ? '-' : `${Math.round(shown(pct))}%${settings.remaining ? ' left' : ''}`);
// Ring color by how much is used, like Claude Code: orange above 75%, red above 95%.
const levelColor = (pct) => (pct >= 95 ? '#e0483a' : pct >= 75 ? '#ff9a1f' : '');

const pctOf = (bucket) =>
  bucket && typeof bucket.utilization === 'number' ? bucket.utilization : null;

function untilText(iso) {
  if (!iso) return '';
  const ms = new Date(iso).getTime() - Date.now();
  if (!isFinite(ms)) return '';
  if (ms <= 0) return 'resets any moment';
  const m = Math.round(ms / 60000);
  if (m < 60) return `resets in ${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return `resets in ${h} h ${m % 60} min`;
  return 'resets ' + new Date(iso).toLocaleString('en-GB', {
    weekday: 'short', hour: '2-digit', minute: '2-digit',
  });
}

const clock = (t) =>
  new Date(t).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });

function etaText(session) {
  const pct = pctOf(session);
  if (pct == null) return '-';
  if (pct >= 100) return 'limit reached';
  const h = view.history;
  if (h.length < 2) return 'collecting data...';
  const first = h[0];
  const last = h[h.length - 1];
  const rate = (last.pct - first.pct) / (last.t - first.t); // pct per ms
  if (!(rate > 0)) return 'steady, not rising';
  const eta = Date.now() + (100 - pct) / rate;
  const reset = session.resets_at ? new Date(session.resets_at).getTime() : Infinity;
  if (eta >= reset) return 'safe until reset';
  return `runs out around ${clock(eta)}`;
}

/* ---------- render ---------- */
function renderUsage() {
  const fresh = view.usage && Date.now() - view.usageAt < staleMs();
  const session = view.usage && view.usage.five_hour;
  const week = view.usage && view.usage.seven_day;
  const sPct = pctOf(session);
  const wPct = pctOf(week);

  stage.classList.toggle('no-usage', !fresh);
  setRing('ring-claude', sPct == null ? 0 : shown(sPct));
  $('ring-claude').style.stroke = fresh && sPct != null ? levelColor(sPct) : '';

  $('p-session').textContent = pctText(sPct);
  $('p-session-sub').textContent = session ? untilText(session.resets_at) : '';
  $('p-week').textContent = pctText(wPct);
  $('p-week-sub').textContent = week ? untilText(week.resets_at) : '';
  $('p-eta').textContent = etaText(session);

  // Any other limit buckets the account has (per-model weekly limits and the like).
  const extra = $('p-extra');
  extra.textContent = '';
  if (view.usage) {
    for (const [key, bucket] of Object.entries(view.usage)) {
      if (key === 'five_hour' || key === 'seven_day' || pctOf(bucket) == null) continue;
      const row = document.createElement('div');
      row.className = 'row';
      const dot = document.createElement('span');
      dot.className = 'dot claude dim';
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = key.replace(/^seven_day_/, 'Weekly ').replace(/_/g, ' ');
      const value = document.createElement('span');
      value.className = 'value';
      value.textContent = pctText(pctOf(bucket));
      row.append(dot, label, value);
      extra.append(row);
    }
  }

  const needLogin = view.usageState === 'login';
  $('login-btn').hidden = !needLogin;
  let status = '';
  if (needLogin) status = 'Not signed in to claude.ai in this widget.';
  else if (view.usageState === 'error') status = `Usage data unavailable (code ${view.usageStatus}).`;
  else if (view.usageState === 'waiting') status = 'Waiting for usage data...';
  else if (!fresh) status = `Stale data, last updated ${clock(view.usageAt)}.`;
  else status = `Updated ${clock(view.usageAt)}`;
  $('p-status').textContent = status;

  const tip = [];
  if (sPct != null) tip.push(`5-hour session: ${pctText(sPct)}, ${untilText(session.resets_at)}`);
  if (wPct != null) tip.push(`Weekly: ${pctText(wPct)}, ${untilText(week.resets_at)}`);
  if (!tip.length) tip.push(status);
  const title = tip.join('\n');
  if (stage.title !== title) stage.title = title;

  renderMood(sPct, fresh);
  fitPanel();
}

function renderMood(sPct, fresh) {
  mood = 'calm';
  let badge = '';
  if (Date.now() < view.danceUntil) mood = 'dance';
  else if (!fresh || sPct == null) mood = 'calm';
  else if (sPct >= 100) mood = 'sleep';
  else if (sPct >= 80) mood = 'panic';
  else if (sPct >= 50) mood = 'warm';

  if (mood === 'sleep') {
    const iso = view.usage.five_hour.resets_at;
    const m = iso ? Math.max(0, Math.round((new Date(iso) - Date.now()) / 60000)) : null;
    if (m != null) badge = m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`;
  } else if (view.usageState === 'login') {
    badge = 'sign in';
  }
  if (!badge && (mood === 'calm' || mood === 'warm')) badge = taskBadge();
  for (const c of [...stage.classList]) if (c.startsWith('mood-')) stage.classList.remove(c);
  stage.classList.add('mood-' + mood);
  $('badge').textContent = badge;
}

function onUsage(msg) {
  if (!msg) return;
  view.usageState = msg.state;
  view.usageStatus = msg.status ?? null;
  if (msg.state === 'ok' && msg.data) {
    view.usage = msg.data;
    view.usageAt = msg.at || Date.now();
    const pct = pctOf(msg.data.five_hour);
    if (pct != null) {
      if (view.lastSessionPct != null && view.lastSessionPct >= 50 && pct < 10) {
        view.danceUntil = Date.now() + 6000;
        view.history = [];
        setTimeout(renderUsage, 6100);
      }
      view.lastSessionPct = pct;
      const now = Date.now();
      view.history.push({ t: now, pct });
      view.history = view.history.filter((s) => now - s.t <= 60 * 60 * 1000);
    }
  }
  renderUsage();
}

function onSys({ cpu, memUsed, memTotal }) {
  const ram = memTotal ? (memUsed / memTotal) * 100 : 0;
  setRing('ring-cpu', cpu);
  setRing('ring-ram', ram);
  view.hotTicks = cpu >= CPU_HOT ? view.hotTicks + 1 : 0;
  stage.classList.toggle('cpu-hot', view.hotTicks >= CPU_HOT_TICKS);
  stage.classList.toggle('ram-full', ram >= RAM_FULL);
  if (!panel.hidden) {
    const gb = (b) => (b / 1073741824).toFixed(1);
    $('p-cpu').textContent = `${Math.round(cpu)}%`;
    $('p-ram').textContent = `${Math.round(ram)}%`;
    $('p-ram-sub').textContent = `${gb(memUsed)} of ${gb(memTotal)} GB`;
  }
}

/* ---------- interaction ---------- */
let press = null;
stage.addEventListener('mousedown', (e) => {
  if (e.button === 0) press = { x: e.screenX, y: e.screenY, dragged: false };
});
window.addEventListener('mousemove', (e) => {
  if (!press || press.dragged) return;
  if (Math.abs(e.screenX - press.x) + Math.abs(e.screenY - press.y) > 4) {
    press.dragged = true;
    invoke('start_drag');
  }
});
window.addEventListener('mouseup', (e) => {
  if (e.button !== 0 || !press) return;
  const wasDrag = press.dragged;
  press = null;
  if (!wasDrag && stage.contains(e.target)) {
    if (canAct()) play("hop");
    togglePanel();
  }
});
window.addEventListener('blur', () => { press = null; });

let panelHeight = 0;
// Grows or shrinks the window so the open panel fits exactly.
function fitPanel() {
  if (panel.hidden) return;
  const height = Math.ceil(panel.offsetTop + panel.offsetHeight + 8);
  if (height === panelHeight) return;
  panelHeight = height;
  invoke('set_panel', { open: true, height });
}
function togglePanel() {
  if (panel.hidden) {
    panel.hidden = false;
    panelHeight = 0;
    renderUsage();
    renderBoard();
  } else {
    panel.hidden = true;
    invoke('set_panel', { open: false });
  }
}

window.addEventListener('contextmenu', (e) => {
  e.preventDefault();
  invoke('show_menu');
});
$('login-btn').addEventListener('click', () => invoke('open_login'));
$('usage-link').addEventListener('click', () => invoke('open_usage_page'));
$('manager-btn').addEventListener('click', () => invoke('open_manager'));

/* ---------- Claude Code task status (from hooks) ---------- */
let task = { state: 'idle', count: 0, sessions: [] };
const busy = () => task.state === 'working' || task.state === 'waiting';

// Tools where Claude is actively producing something: shell, file edits, driving a browser.
const TYPING_TOOL = /^(Edit|Write|NotebookEdit|Bash|PowerShell)$|claude-in-chrome|Claude_Browser|computer-use/;
function toolLabel(tool) {
  if (!tool) return 'thinking';
  if (/claude-in-chrome|Claude_Browser|computer-use/.test(tool)) return 'using browser';
  if (/^(Edit|Write|NotebookEdit)$/.test(tool)) return 'editing';
  if (/^(Bash|PowerShell)$/.test(tool)) return 'running command';
  if (/^(Read|Grep|Glob)$/.test(tool)) return 'reading code';
  if (/^Web/.test(tool)) return 'browsing';
  if (/^(Agent|Task)$/.test(tool)) return 'delegating';
  return 'using a tool';
}
function taskBadge() {
  const others = task.count > 1 ? ` +${task.count - 1}` : '';
  if (task.state === 'waiting') return 'needs you!' + others;
  if (task.state === 'working') return (task.kind === 'chat' ? 'chatting' : toolLabel(task.tool)) + others;
  return '';
}
const minutes = (from, to = Date.now()) => Math.max(0, Math.round((to - from) / 60000));
const node = (cls, text) => {
  const n = document.createElement('div');
  n.className = cls;
  if (text != null) n.textContent = text;
  return n;
};
// One row per session: what it was asked, what it is doing, how far along.
function sessionRow(s) {
  const chat = s.kind === 'chat';
  const row = node('sess ' + s.state);
  row.append(node('sess-dot'), node('sess-title', s.title || s.project || (chat ? 'Untitled chat' : 'Session')));
  row.title = (s.title ? s.title + '\n' : '') + (chat ? 'Click to open this chat' : 'Click to open this session');
  row.addEventListener('click', () => invoke('open_session', { session: s.session }));

  const meta = [];
  if (!chat && s.project) meta.push(s.project);
  if (s.state === 'waiting') meta.push('waiting for you');
  else if (s.state === 'working') meta.push(chat ? 'replying' : toolLabel(s.tool));
  else meta.push('done');
  if (s.state === 'done') meta.push(`${minutes(s.updated)} min ago`);
  else meta.push(`${minutes(s.since)} min`);
  if (s.total) meta.push(`step ${s.done}/${s.total}`);
  if (s.agents) meta.push(`${s.agents} agent${s.agents > 1 ? 's' : ''}`);
  row.append(node('sess-meta', meta.join(' · ')));

  if (s.total && s.state !== 'done') {
    const bar = node('sess-bar');
    const fill = document.createElement('i');
    fill.style.width = `${Math.min(100, (s.done / s.total) * 100)}%`;
    bar.append(fill);
    row.append(bar);
  }
  if (s.state === 'done' && s.last) row.append(node('sess-last', s.last));
  return row;
}
// Claude Code sessions and regular chats are listed under their own headings.
function renderBoard() {
  const all = task.sessions || [];
  const groups = [
    { name: 'Claude Code', list: all.filter((s) => s.kind !== 'chat'), empty: 'No active sessions.' },
    { name: 'Claude Chat', list: all.filter((s) => s.kind === 'chat'), empty: 'No recent chats.' },
  ];
  const box = $('sessions');
  box.textContent = '';
  for (const group of groups) {
    const count = (state) => group.list.filter((s) => s.state === state).length;
    const parts = [];
    if (count('waiting')) parts.push(`${count('waiting')} need you`);
    if (count('working')) parts.push(`${count('working')} working`);
    const head = node('board-head');
    head.append(node('', group.name), node('board-count', parts.join(' · ')));
    const rows = node('board-rows');
    if (group.list.length) group.list.forEach((s) => rows.append(sessionRow(s)));
    else rows.append(node('empty', group.empty));
    box.append(head, rows);
  }
  fitPanel();
}
function onTask(next) {
  const before = task.state;
  task = next || { state: 'idle', count: 0 };
  if (task.state === 'done' && before !== 'done' && (mood === 'calm' || mood === 'warm')) play('hop');
  renderBoard();
  renderUsage();
}

/* ---------- crab animation: one 10 fps ticker, repaint only when the pose changes ---------- */
const FRAME_MS = 100;
const el = {
  mover: $('mover'),
  bob: $('bob'),
  armL: document.querySelector('.arm-l'),
  armR: document.querySelector('.arm-r'),
  legs: [...document.querySelectorAll('.leg')],
  eyes: document.querySelector('.eyes'),
  eye: [...document.querySelectorAll('.eye')],
  sweatA: document.querySelector('.sweat-a'),
  sweatB: document.querySelector('.sweat-b'),
  smoke: [...document.querySelectorAll('#smoke rect')],
  zzz: $('zzz'),
  laptop: $('laptop'),
  lid: document.querySelector('.lap-lid'),
  logo: document.querySelector('.lap-logo'),
  hands: [...document.querySelectorAll('.hand')],
  backdrop: document.querySelector('.backdrop'),
  bubble: $('bubble'),
  bubTail: document.querySelector('.bub-tail'),
  bubThink: [...document.querySelectorAll('.bub-think')],
  dots: [...document.querySelectorAll('.bub-dot')],
};
const LID_H = 20;      // open lid height
const LID_BOTTOM = 46; // where the lid meets the base
const LID_STEPS = 4;   // frames to open or close
let lidStep = 0;
const EYE_H = 13;
const EYE_Y = 10;

// Writes a style or attribute only when its value actually changed.
const cache = new Map();
function put(node, key, value, attr = false) {
  let c = cache.get(node);
  if (!c) cache.set(node, (c = {}));
  if (c[key] === value) return;
  c[key] = value;
  if (attr) node.setAttribute(key, value);
  else node.style[key] = value;
}
const shift = (node, x, y) => put(node, 'transform', `translate(${x}px, ${y}px)`);
// A particle that steps through `ys`, then disappears until the next cycle.
function particle(node, frame, ys, x = 0) {
  if (frame == null || frame >= ys.length) return put(node, 'display', 'none');
  put(node, 'display', 'block');
  shift(node, x, ys[frame]);
}

const look = { x: 0, y: 0, inside: false };
window.addEventListener('mousemove', (e) => {
  const r = $('crab').getBoundingClientRect();
  const clamp = (v, m) => Math.max(-m, Math.min(m, Math.round(v)));
  look.inside = true;
  look.x = clamp((e.clientX - (r.left + r.width / 2)) / 12, 4);
  look.y = clamp((e.clientY - (r.top + r.height / 2)) / 12, 3);
});
document.documentElement.addEventListener('mouseleave', () => {
  look.inside = false;
  look.x = look.y = 0;
});

// Acts: short scripted moves while calm or warm. Each returns pose tweaks for frame f.
const ACTS = {
  walk: { frames: 24, at(f, p) {
    const path = f < 6 ? -f * 1.5 : f < 18 ? -9 + (f - 6) * 1.5 : 9 - (f - 18) * 1.5;
    p.mx = Math.round(path);
    const up = f % 2 === 0;
    p.legs = up ? [-5, 0, -5, 0] : [0, -5, 0, -5];
  } },
  wave: { frames: 14, at(f, p) { p.ar = Math.floor(f / 2) % 2 === 0 ? -11 : -4; } },
  hop: { frames: 7, at(f, p) {
    p.my = [3, -6, -12, -13, -9, -3, 0][f];
    if (f >= 1 && f <= 5) { p.al = p.ar = -8; p.legs = [-5, -5, -5, -5]; }
  } },
  look: { frames: 18, at(f, p) { if (!look.inside) p.ex = f < 7 ? -4 : f < 14 ? 4 : 0; } },
  // claws up, tap together in front three times, with a little bounce and happy eyes
  clap: { frames: 14, at(f, p) {
    const shut = f >= 2 && f < 14 && Math.floor(f / 2) % 2 === 1;
    p.al = p.ar = -9;
    p.alx = shut ? 4 : 0;
    p.arx = shut ? -4 : 0;
    p.my = shut ? -2 : 0;
    p.eyeH = 6;
  } },
};
let frame = 0;
let act = null;
let actStart = 0;
let mood = 'calm';

function play(name) {
  act = name;
  actStart = frame;
}
function canAct() {
  return (mood === 'calm' || mood === 'warm') && !busy() && !document.body.classList.contains('mini');
}

function tick() {
  frame++;
  if (document.body.classList.contains('mini')) return;
  const n = frame;
  const p = {
    mx: 0, my: 0, bob: 0, al: 0, ar: 0, alx: 0, arx: 0, legs: [0, 0, 0, 0],
    ex: look.x, ey: look.y, eyeH: EYE_H, sweatA: null, sweatB: null, zzz: null,
  };
  const blink = n % 46 >= 44;
  const alt = n % 2 === 0;

  if (mood === 'calm') {
    p.bob = Math.floor(n / 14) % 2 ? 2 : 0;
    if (blink) p.eyeH = 2;
  } else if (mood === 'warm') {
    p.bob = Math.floor(n / 7) % 2 ? 2 : 0;
    if (blink) p.eyeH = 2;
    p.sweatA = Math.floor(n / 3) % 8;
  } else if (mood === 'panic') {
    const k = n % 8;
    p.mx = [0, -4, -7, -4, 0, 4, 7, 4][k];
    p.my = alt ? -1 : 1;
    p.legs = alt ? [-5, 0, -5, 0] : [0, -5, 0, -5];
    p.al = alt ? -11 : 0;
    p.ar = alt ? 0 : -11;
    p.eyeH = 16;
    p.sweatA = n % 6;
    p.sweatB = (n + 3) % 6;
  } else if (mood === 'sleep') {
    p.my = 9;
    p.legs = [-10, -10, -10, -10];
    p.bob = Math.floor(n / 22) % 2 ? 2 : 0;
    p.eyeH = 2;
    p.ex = 0;
    p.ey = 3;
    p.zzz = Math.floor(n / 5) % 7;
  } else if (mood === 'dance') {
    const k = n % 6;
    p.mx = [0, -7, -7, 0, 7, 7][k];
    p.my = [0, -9, -4, 0, -9, -4][k];
    p.al = alt ? -11 : 0;
    p.ar = alt ? 0 : -11;
    p.legs = alt ? [-5, 0, -5, 0] : [0, -5, 0, -5];
    p.eyeH = 6;
  }

  if (mood === 'calm' || mood === 'warm') {
    if (task.state === 'working' && task.kind === 'chat') {
      // a regular chat: speech bubble with typing dots, and a small gesture now and then
      p.chatting = true;
      p.bob = Math.floor(n / 6) % 2 ? 2 : 0;
      if (Math.floor(n / 12) % 3 === 0) p.ar = Math.floor(n / 3) % 2 ? -9 : -5;
    } else if (task.state === 'working') {
      const typing = TYPING_TOOL.test(task.tool || '');
      // the laptop is out for the whole turn once Claude has touched code
      p.coding = !!task.coding || typing;
      if (typing) {
        // arms drop to the sides of the keyboard; the hands in front do the tapping
        if (lidStep >= LID_STEPS) {
          p.typing = true;
          p.al = 12;
          p.ar = 12;
          p.tap = Math.floor(n / 2) % 2 === 0;
          p.ex = (Math.floor(n / 5) % 3) - 1;
          p.ey = 1;
        }
      } else if (!task.tool) {
        // thinking: looks up, one arm to the head, thought bubble
        p.thinking = true;
        p.bob = Math.floor(n / 10) % 2 ? 2 : 0;
        p.ex = Math.floor(n / 14) % 2 ? 3 : -3;
        p.ey = -3;
        p.ar = -8;
      } else {
        // reading or searching: eyes scan, no typing
        p.ex = (Math.floor(n / 4) % 5) - 2;
        p.ey = p.coding ? 1 : 0;
      }
    } else if (task.state === 'waiting') {
      // both arms up, waving for attention
      const up = Math.floor(n / 3) % 2 === 0;
      p.al = up ? -13 : -7;
      p.ar = up ? -7 : -13;
      p.my = up ? -3 : 0;
      p.ex = 0;
      p.ey = -2;
    }
  }

  if (act) {
    const f = n - actStart;
    if (f >= ACTS[act].frames || busy()) act = null;
    else ACTS[act].at(f, p);
  }

  shift(el.mover, p.mx, p.my);
  shift(el.bob, 0, p.bob);
  shift(el.armL, p.alx, p.al);
  shift(el.armR, p.arx, p.ar);

  // laptop: the lid swings up over a few frames, and folds back down when coding stops
  lidStep = Math.max(0, Math.min(LID_STEPS, lidStep + (p.coding ? 1 : -1)));
  put(el.laptop, 'display', lidStep > 0 ? 'block' : 'none');
  if (lidStep > 0) {
    const lidH = Math.round((LID_H * lidStep) / LID_STEPS);
    put(el.lid, 'height', String(lidH), true);
    put(el.lid, 'y', String(LID_BOTTOM - lidH), true);
    // the logo on the lid glows on and off while typing
    const lit = lidStep >= LID_STEPS && Math.floor(n / 8) % 4 !== 0;
    put(el.logo, 'display', lit ? 'block' : 'none');
    const typing = lidStep >= LID_STEPS && !!p.typing;
    el.hands.forEach((hand, i) => {
      put(hand, 'display', typing ? 'block' : 'none');
      if (typing) shift(hand, 0, (i === 0) === p.tap ? 4 : 0);
    });
  }

  // a red edge pulses while any session waits on the user
  const alarm = task.waiting > 0 && Math.floor(n / 4) % 2 === 0;
  put(el.backdrop, 'stroke', alarm ? '#e0483a' : 'none');

  // speech bubble: dots fill in one by one, then start over
  // a speech bubble while chatting, a thought bubble (trailing dots, no tail) while thinking
  const bubbleOn = p.chatting || p.thinking;
  put(el.bubble, 'display', bubbleOn ? 'block' : 'none');
  if (bubbleOn) {
    put(el.bubTail, 'display', p.chatting ? 'block' : 'none');
    el.bubThink.forEach((dot) => put(dot, 'display', p.thinking ? 'block' : 'none'));
    const shownDots = Math.floor(n / 4) % 4;
    el.dots.forEach((dot, i) => put(dot, 'display', i < shownDots ? 'block' : 'none'));
  }
  el.legs.forEach((leg, i) => shift(leg, 0, p.legs[i]));
  shift(el.eyes, p.ex, p.ey);
  for (const eye of el.eye) {
    put(eye, 'height', String(p.eyeH), true);
    put(eye, 'y', String(EYE_Y + (EYE_H - p.eyeH) / 2), true);
  }
  const DROP = [0, 5, 10, 16, 22];
  particle(el.sweatA, p.sweatA, DROP);
  particle(el.sweatB, p.sweatB, DROP);
  particle(el.zzz, p.zzz, [6, 1, -4, -9, -14], p.zzz == null ? 0 : p.zzz * 2);
  const hot = stage.classList.contains('cpu-hot');
  el.smoke.forEach((puff, i) =>
    particle(puff, hot ? (Math.floor(n / 3) + i * 2) % 6 : null, [8, 3, -2, -7]));
}
setInterval(tick, FRAME_MS);

(function schedule() {
  setTimeout(() => {
    if (canAct() && !act) {
      const pool = ['walk', 'walk', 'wave', 'look', 'hop', 'clap'];
      play(pool[Math.floor(Math.random() * pool.length)]);
    }
    schedule();
  }, 4500 + Math.random() * 4500);
})();

/* Countdown texts and staleness move on their own. */
setInterval(() => { renderUsage(); renderBoard(); }, 30000);

listen('sys', (e) => onSys(e.payload));
listen('usage', (e) => onUsage(e.payload));
listen('task', (e) => onTask(e.payload));
function applyMode(m) {
  if (!m) return;
  document.body.classList.toggle('mini', !!m.mini);
  document.body.classList.toggle('smooth', !!m.smooth);
  settings.remaining = !!m.remaining;
  if (m.interval) settings.interval = m.interval;
  renderUsage();
}
listen('mode', (e) => applyMode(e.payload));

invoke('get_state').then((s) => {
  applyMode(s.mode);
  onUsage(s.usage);
  onTask(s.task);
  if (s.openPanel) togglePanel(); // CRAB_OPEN_PANEL=1, for checking the panel
});
