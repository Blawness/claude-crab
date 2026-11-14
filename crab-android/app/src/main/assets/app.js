// The crab screen. Data comes from the app (window.Crab bridge) as one state object:
// { latest, good, history, chats, settings, phone, checking }.
const bridge = window.Crab || {
  // Lets the page run in a plain browser for a quick look, with made-up numbers.
  state: () => JSON.stringify({
    latest: { state: 'ok', at: Date.now() },
    good: { state: 'ok', at: Date.now(), data: {
      five_hour: { utilization: 42, resets_at: new Date(Date.now() + 2.4e6 * 3).toISOString() },
      seven_day: { utilization: 18, resets_at: new Date(Date.now() + 3.2e8).toISOString() },
    } },
    history: [], chats: { at: Date.now(), list: [] }, checking: false,
    settings: { remaining: false, smooth: false, chats: true, background: true, notify: true, interval: 300,
      intervals: [[120, '2 minutes'], [300, '5 minutes'], [900, '15 minutes']] },
    phone: { memUsed: 5.1e9, memTotal: 8e9, battery: 76, temp: 31.5, charging: false },
    update: { current: '0.1.1 (build 1)', latest: { code: 2, name: '0.1.2' }, phase: 'idle', progress: -1, error: '', checkedAt: Date.now() },
  }),
  checkUpdate() {}, installUpdate() {},
  refresh() {}, login() {}, openUsagePage() {}, openChat() {}, haptic() {}, setFlag() {}, setInterval() {},
};

const $ = (id) => document.getElementById(id);
const stage = $('stage');

let s = JSON.parse(bridge.state());
const RAM_FULL = 90;
const HOT_C = 42; // battery temperature that makes the crab smoke

const view = { danceUntil: 0, lastSessionPct: null };

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

const remaining = () => !!s.settings.remaining;
const shown = (pct) => (remaining() ? 100 - pct : pct);
const pctText = (pct) => (pct == null ? '-' : `${Math.round(shown(pct))}%${remaining() ? ' left' : ''}`);
// Like Claude Code: orange above 75%, red above 95%.
const levelColor = (pct) => (pct >= 95 ? '#e0483a' : pct >= 75 ? '#ff9a1f' : '');
const pctOf = (bucket) => (bucket && typeof bucket.utilization === 'number' ? bucket.utilization : null);
const usage = () => (s.good && s.good.data) || null;
// Stale once two checks in a row were missed (the background check runs every 15 minutes).
const staleMs = () => (Math.max(s.settings.interval, 900) * 2 + 120) * 1000;
const fresh = () => !!(s.good && Date.now() - s.good.at < staleMs());

function untilText(iso) {
  if (!iso) return '';
  const ms = new Date(iso).getTime() - Date.now();
  if (!isFinite(ms)) return '';
  if (ms <= 0) return 'resets any moment';
  const m = Math.round(ms / 60000);
  if (m < 60) return `resets in ${m} min`;
  const h = Math.floor(m / 60);
  if (h < 24) return `resets in ${h} h ${m % 60} min`;
  return 'resets ' + new Date(iso).toLocaleString(undefined, { weekday: 'short', hour: '2-digit', minute: '2-digit' });
}
const clock = (t) => new Date(t).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' });

function etaText(session) {
  const pct = pctOf(session);
  if (pct == null) return '-';
  if (pct >= 100) return 'limit reached';
  const h = s.history || [];
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

/* ---------- chats ---------- */
const IDLE_WORDS = /idle|settled|complete|done|none|stopped|finished|ended/i;
function chatState(c) {
  if (c.needs) return 'waiting';
  const liveActive = !!c.live && !IDLE_WORDS.test(c.live);
  const unsettled = c.u && (c.s == null || c.u > c.s + 5000) && Date.now() - c.u < 15 * 60 * 1000;
  return liveActive || unsettled ? 'working' : 'idle';
}
const chatList = () => (s.chats && s.chats.list) || [];
function chatSummary() {
  const states = chatList().map(chatState);
  return { working: states.filter((x) => x === 'working').length, waiting: states.filter((x) => x === 'waiting').length };
}
function ago(t) {
  if (!t) return '';
  const m = Math.max(0, Math.round((Date.now() - t) / 60000));
  if (m < 1) return 'just now';
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h} h ago`;
  return new Date(t).toLocaleDateString(undefined, { day: 'numeric', month: 'short' });
}
function renderChats() {
  const card = $('chats-card');
  card.hidden = !s.settings.chats;
  if (card.hidden) return;
  const box = $('chats');
  box.textContent = '';
  const list = chatList();
  const sum = chatSummary();
  const parts = [];
  if (sum.waiting) parts.push(`${sum.waiting} need you`);
  if (sum.working) parts.push(`${sum.working} replying`);
  $('chats-count').textContent = parts.length ? '· ' + parts.join(' · ') : '';
  if (!list.length) {
    const empty = document.createElement('div');
    empty.className = 'empty';
    empty.textContent = s.chats ? 'No recent chats.' : 'Chats appear after the next check.';
    box.append(empty);
    return;
  }
  for (const c of list) {
    const state = chatState(c);
    const row = document.createElement('div');
    row.className = 'chat ' + state;
    const dot = document.createElement('div');
    dot.className = 'chat-dot';
    const title = document.createElement('div');
    title.className = 'chat-title';
    title.textContent = c.name || 'Untitled chat';
    const meta = document.createElement('div');
    meta.className = 'chat-meta';
    meta.textContent = state === 'waiting' ? 'waiting for you' : state === 'working' ? 'replying...' : ago(c.u);
    row.append(dot, title, meta);
    row.addEventListener('click', () => bridge.openChat(c.id));
    box.append(row);
  }
}

/* ---------- render ---------- */
function renderUsage() {
  const data = usage();
  const ok = fresh();
  const session = data && data.five_hour;
  const week = data && data.seven_day;
  const sPct = pctOf(session);
  const wPct = pctOf(week);

  stage.classList.toggle('no-usage', !ok);
  setRing('ring-session', sPct == null ? 0 : shown(sPct));
  setRing('ring-week', wPct == null ? 0 : shown(wPct));
  $('ring-session').style.stroke = ok && sPct != null ? levelColor(sPct) : '';

  $('p-session').textContent = pctText(sPct);
  $('p-session-sub').textContent = session ? untilText(session.resets_at) : '';
  $('bar-session').style.width = `${sPct == null ? 0 : shown(sPct)}%`;
  $('bar-session').style.background = sPct != null ? levelColor(sPct) : '';
  $('p-week').textContent = pctText(wPct);
  $('p-week-sub').textContent = week ? untilText(week.resets_at) : '';
  $('bar-week').style.width = `${wPct == null ? 0 : shown(wPct)}%`;
  $('p-eta').textContent = etaText(session);

  // Any other limit buckets the account has (per-model weekly limits and the like).
  const extra = $('p-extra');
  extra.textContent = '';
  if (data) {
    for (const [key, bucket] of Object.entries(data)) {
      if (key === 'five_hour' || key === 'seven_day' || pctOf(bucket) == null) continue;
      const row = document.createElement('div');
      row.className = 'row';
      const label = document.createElement('span');
      label.className = 'label';
      label.textContent = key.replace(/^seven_day_/, 'Weekly ').replace(/_/g, ' ');
      const value = document.createElement('span');
      value.className = 'value';
      value.textContent = pctText(pctOf(bucket));
      row.append(label, value);
      extra.append(row);
    }
  }

  const state = s.latest ? s.latest.state : 'waiting';
  const needLogin = state === 'login';
  $('login-btn').hidden = !needLogin;
  let status;
  if (s.checking) status = 'Checking...';
  else if (needLogin) status = 'Not signed in to claude.ai in this app.';
  else if (state === 'error') status = s.good ? `Can't reach claude.ai, last data ${clock(s.good.at)}` : `Usage data unavailable (code ${s.latest.status}).`;
  else if (!s.good) status = 'Waiting for usage data...';
  else if (!ok) status = `Stale data, last updated ${clock(s.good.at)}`;
  else status = `Updated ${clock(s.good.at)}`;
  $('p-status').textContent = status;
  $('refresh').classList.toggle('spin', !!s.checking);

  let line = status;
  if (sPct != null && !needLogin) {
    line = sPct >= 100 ? `Limit reached, ${untilText(session.resets_at)}`
      : `${pctText(sPct)} of your 5-hour session${remaining() ? '' : ' used'}`;
  }
  $('hero-line').textContent = line;

  renderMood(sPct, ok, needLogin);
}

let mood = 'calm';
function renderMood(sPct, ok, needLogin) {
  let next = 'calm';
  let badge = '';
  if (Date.now() < view.danceUntil) next = 'dance';
  else if (!ok || sPct == null) next = 'calm';
  else if (sPct >= 100) next = 'sleep';
  else if (sPct >= 80) next = 'panic';
  else if (sPct >= 50) next = 'warm';

  const sum = chatSummary();
  if (next === 'sleep') {
    const iso = usage().five_hour.resets_at;
    const m = iso ? Math.max(0, Math.round((new Date(iso) - Date.now()) / 60000)) : null;
    if (m != null) badge = m >= 60 ? `${Math.floor(m / 60)}h ${m % 60}m` : `${m}m`;
  } else if (needLogin) {
    badge = 'sign in';
  } else if (sum.waiting) {
    badge = 'needs you!';
  } else if (sum.working) {
    badge = 'chatting';
  }
  mood = next;
  for (const c of [...stage.classList]) if (c.startsWith('mood-')) stage.classList.remove(c);
  stage.classList.add('mood-' + mood);
  $('badge').textContent = badge;
}

function renderPhone(p) {
  if (!p) return;
  const ram = p.memTotal ? (p.memUsed / p.memTotal) * 100 : 0;
  setRing('ring-ram', ram);
  stage.classList.toggle('ram-full', ram >= RAM_FULL);
  stage.classList.toggle('hot', p.temp != null && p.temp >= HOT_C);
  const gb = (b) => (b / 1073741824).toFixed(1);
  $('p-ram').textContent = `${Math.round(ram)}%  ·  ${gb(p.memUsed)} of ${gb(p.memTotal)} GB`;
  $('p-battery').textContent = p.battery == null ? '-' : `${Math.round(p.battery)}%${p.charging ? ', charging' : ''}`;
  $('p-temp').textContent = p.temp == null ? '-' : `${p.temp.toFixed(1)} °C`;
}

function renderSettings() {
  document.body.classList.toggle('smooth', !!s.settings.smooth);
  for (const input of document.querySelectorAll('[data-flag]')) input.checked = !!s.settings[input.dataset.flag];
  const box = $('intervals');
  if (!box.childElementCount) {
    for (const [secs, label] of s.settings.intervals) {
      const b = document.createElement('button');
      b.textContent = label.replace(' minutes', ' min');
      b.dataset.secs = secs;
      b.addEventListener('click', () => bridge.setInterval(secs));
      box.append(b);
    }
  }
  for (const b of box.children) b.classList.toggle('on', Number(b.dataset.secs) === s.settings.interval);
}

function renderUpdate() {
  const u = s.update;
  if (!u) return;
  $('version').textContent = u.current;
  const busy = u.phase === 'downloading' || u.phase === 'installing';
  const card = $('update-card');
  card.hidden = !u.latest && !(u.phase === 'error' && u.error);
  if (u.latest) {
    $('update-title').textContent = `Update available: build ${u.latest.code}`;
    let sub = `You have ${u.current}`;
    if (u.phase === 'downloading') sub = u.progress >= 0 ? `Downloading... ${u.progress}%` : 'Downloading...';
    else if (u.phase === 'installing') sub = 'Opening the installer...';
    else if (u.phase === 'error') sub = u.error;
    $('update-sub').textContent = sub;
  } else {
    $('update-title').textContent = 'Update';
    $('update-sub').textContent = u.error;
  }
  $('update-btn').hidden = !u.latest;
  $('update-btn').disabled = busy;
  $('update-btn').textContent = u.phase === 'error' ? 'Try again' : 'Update';
  $('update-bar').hidden = u.phase !== 'downloading';
  $('update-fill').style.width = `${Math.max(0, u.progress)}%`;

  let status = '';
  if (u.phase === 'checking') status = 'Checking...';
  else if (u.latest) status = `Build ${u.latest.code} is available.`;
  else if (u.phase === 'error') status = u.error;
  else if (u.checkedAt) status = `Up to date. Checked ${clock(u.checkedAt)}.`;
  $('update-status').textContent = status;
  $('update-check').disabled = u.phase === 'checking' || busy;
}

function render() {
  renderUpdate();
  renderSettings();
  renderUsage();
  renderChats();
  renderPhone(s.phone);
}

// Called by the app whenever anything changes.
window.crabUpdate = (next) => {
  const pct = pctOf(next.good && next.good.data && next.good.data.five_hour);
  if (pct != null) {
    // The session just reset: a little dance.
    if (view.lastSessionPct != null && view.lastSessionPct >= 50 && pct < 10) {
      view.danceUntil = Date.now() + 6000;
      setTimeout(renderUsage, 6100);
    }
    view.lastSessionPct = pct;
  }
  s = next;
  render();
};
window.crabPhone = (p) => { s.phone = p; renderPhone(p); };

/* ---------- interaction ---------- */
$('refresh').addEventListener('click', () => bridge.refresh());
$('login-btn').addEventListener('click', () => bridge.login());
$('login-again').addEventListener('click', () => bridge.login());
$('update-btn').addEventListener('click', () => bridge.installUpdate());
$('update-check').addEventListener('click', () => bridge.checkUpdate());
$('usage-link').addEventListener('click', () => bridge.openUsagePage());
for (const input of document.querySelectorAll('[data-flag]')) {
  input.addEventListener('change', () => bridge.setFlag(input.dataset.flag, input.checked));
}
stage.addEventListener('click', () => {
  bridge.haptic();
  if (canAct()) play(Math.random() < 0.6 ? 'hop' : 'clap');
});

// The eyes follow a finger on the screen.
const look = { x: 0, y: 0, inside: false };
function lookAt(x, y) {
  const r = $('crab').getBoundingClientRect();
  const clamp = (v, m) => Math.max(-m, Math.min(m, Math.round(v)));
  look.inside = true;
  look.x = clamp((x - (r.left + r.width / 2)) / 20, 4);
  look.y = clamp((y - (r.top + r.height / 2)) / 20, 3);
}
window.addEventListener('touchstart', (e) => lookAt(e.touches[0].clientX, e.touches[0].clientY), { passive: true });
window.addEventListener('touchmove', (e) => lookAt(e.touches[0].clientX, e.touches[0].clientY), { passive: true });
window.addEventListener('touchend', () => setTimeout(() => { look.inside = false; look.x = look.y = 0; }, 1200));

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
  backdrop: document.querySelector('.backdrop'),
  bubble: $('bubble'),
  dots: [...document.querySelectorAll('.bub-dot')],
};
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
function particle(node, frame, ys, x = 0) {
  if (frame == null || frame >= ys.length) return put(node, 'display', 'none');
  put(node, 'display', 'block');
  shift(node, x, ys[frame]);
}

// Acts: short scripted moves while calm or warm. Each sets pose tweaks for frame f.
const ACTS = {
  walk: { frames: 24, at(f, p) {
    const path = f < 6 ? -f * 1.5 : f < 18 ? -9 + (f - 6) * 1.5 : 9 - (f - 18) * 1.5;
    p.mx = Math.round(path);
    p.legs = f % 2 === 0 ? [-5, 0, -5, 0] : [0, -5, 0, -5];
  } },
  wave: { frames: 14, at(f, p) { p.ar = Math.floor(f / 2) % 2 === 0 ? -11 : -4; } },
  hop: { frames: 7, at(f, p) {
    p.my = [3, -6, -12, -13, -9, -3, 0][f];
    if (f >= 1 && f <= 5) { p.al = p.ar = -8; p.legs = [-5, -5, -5, -5]; }
  } },
  look: { frames: 18, at(f, p) { if (!look.inside) p.ex = f < 7 ? -4 : f < 14 ? 4 : 0; } },
  clap: { frames: 14, at(f, p) {
    const shut = f >= 2 && Math.floor(f / 2) % 2 === 1;
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

function play(name) { act = name; actStart = frame; }
const busy = () => { const sum = chatSummary(); return sum.working > 0 || sum.waiting > 0; };
const canAct = () => (mood === 'calm' || mood === 'warm') && !busy();

function tick() {
  if (document.hidden) return; // the app is in the background
  frame++;
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

  const sum = chatSummary();
  if (mood === 'calm' || mood === 'warm') {
    if (sum.waiting) {
      // both arms up, waving for attention
      const up = Math.floor(n / 3) % 2 === 0;
      p.al = up ? -13 : -7;
      p.ar = up ? -7 : -13;
      p.my = up ? -3 : 0;
      p.ex = 0;
      p.ey = -2;
    } else if (sum.working) {
      // a chat is replying: speech bubble with typing dots, and a small gesture now and then
      p.chatting = true;
      p.bob = Math.floor(n / 6) % 2 ? 2 : 0;
      if (Math.floor(n / 12) % 3 === 0) p.ar = Math.floor(n / 3) % 2 ? -9 : -5;
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

  // a red edge pulses while a chat waits on the user
  const alarm = sum.waiting > 0 && Math.floor(n / 4) % 2 === 0;
  put(el.backdrop, 'stroke', alarm ? '#e0483a' : 'none');

  put(el.bubble, 'display', p.chatting ? 'block' : 'none');
  if (p.chatting) {
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
  const hot = stage.classList.contains('hot');
  el.smoke.forEach((puff, i) => particle(puff, hot ? (Math.floor(n / 3) + i * 2) % 6 : null, [8, 3, -2, -7]));
}
setInterval(tick, FRAME_MS);

(function schedule() {
  setTimeout(() => {
    if (!document.hidden && canAct() && !act) {
      const pool = ['walk', 'walk', 'wave', 'look', 'hop', 'clap'];
      play(pool[Math.floor(Math.random() * pool.length)]);
    }
    schedule();
  }, 4500 + Math.random() * 4500);
})();

// Countdowns and staleness move on their own.
setInterval(() => { if (!document.hidden) { renderUsage(); renderChats(); } }, 30000);

render();
