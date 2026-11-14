// End-to-end test: runs the real release build and drives it through the WebView2 debug port.
//
//   npm run build && npm run e2e            quiet: runs off-screen, never takes focus
//   npm run e2e -- --full                   also opens a terminal and checks it comes to the front
//
// Isolation: LOCALAPPDATA points at a temp folder, so sessions, settings and agents are fake and
// the user's own are never touched. Only one crab can run at a time, so a running one is closed
// first and started again at the end. Windows only. Needs Windows Terminal (wt.exe) for the
// terminal-focus test; that test is skipped without it.
//
// shortcut: the claude.ai usage page and the chat link (opens a browser tab) are not exercised.

import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const ROOT = join(dirname(fileURLToPath(import.meta.url)), '..');
const EXE = join(ROOT, 'src-tauri', 'target', 'release', 'crab-widget.exe');
const PORT = 9335;
const AGENT_ID = '00000000-e2e0-4000-8000-000000000001';
// Quiet by default: nothing appears on screen or takes focus. --full adds the terminal-focus test.
const FULL = process.argv.includes('--full');
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/* ---------- tiny CDP client ---------- */
async function pages() {
  try { return await (await fetch(`http://127.0.0.1:${PORT}/json`, { signal: AbortSignal.timeout(3000) })).json(); } catch { return []; }
}
async function waitFor(what, test, ms = 15000) {
  const end = Date.now() + ms;
  for (;;) {
    const v = await test();
    if (v) return v;
    if (Date.now() > end) throw new Error(`timed out waiting for ${what}`);
    await sleep(200);
  }
}
async function connect(url) {
  const ws = new WebSocket(url);
  await new Promise((ok, fail) => { ws.onopen = ok; ws.onerror = fail; });
  let next = 0;
  const waiting = new Map();
  ws.onmessage = (m) => {
    const r = JSON.parse(m.data);
    if (waiting.has(r.id)) { waiting.get(r.id)(r); waiting.delete(r.id); }
  };
  // every call fails after 10 s instead of hanging the whole run
  const send = (method, params = {}) => new Promise((ok, fail) => {
    const id = ++next;
    const timer = setTimeout(() => { waiting.delete(id); fail(new Error(`${method} got no answer`)); }, 10000);
    waiting.set(id, (r) => { clearTimeout(timer); ok(r); });
    ws.send(JSON.stringify({ id, method, params }));
  });
  const run = async (expression) => {
    const r = await send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (r.result.exceptionDetails) throw new Error(r.result.exceptionDetails.exception?.description || 'page error');
    return r.result.result.value;
  };
  return { send, run, close: () => ws.close() };
}
// Only our own pages: the hidden claude.ai page shares the debug port and is never opened.
const page = async (match) => {
  const p = await waitFor('a page', async () => (await pages()).find((x) => x.type === 'page' && match(x)));
  return connect(p.webSocketDebuggerUrl);
};
const invoke = (cmd, args = {}) => `window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)})`;

/* ---------- test runner ---------- */
const results = [];
async function test(name, fn) {
  try {
    await fn();
    results.push(['PASS', name]);
    console.log(`  ok   ${name}`);
  } catch (e) {
    results.push(['FAIL', name, e.message]);
    console.log(`  FAIL ${name}\n       ${e.message}`);
  }
}
function expect(cond, message) { if (!cond) throw new Error(message); }

/* ---------- fixtures ---------- */
function makeData() {
  const base = mkdtempSync(join(tmpdir(), 'crab-e2e-'));
  const app = join(base, 'crab-widget');
  const sessions = join(app, 'sessions');
  const agents = join(app, 'agents');
  mkdirSync(sessions, { recursive: true });
  mkdirSync(agents, { recursive: true });
  writeFileSync(join(app, 'settings.json'), JSON.stringify({ sound: false, nudge: false, chats: false, mini: false, remaining: false, smooth: false }));
  const now = Date.now();
  writeFileSync(join(agents, 'agents.json'), JSON.stringify([{
    id: AGENT_ID, status: 'done', prompt: 'e2e agent\nchecks the swarm page', folder: base,
    worktree: null, started: now - 120000, ended: now - 60000, exit: 0,
  }]));
  writeFileSync(join(agents, `${AGENT_ID}.out`), 'e2e agent output');
  const session = (id, extra) => writeFileSync(join(sessions, `${id}.json`), JSON.stringify({
    session: id, tool: '', coding: false, cwd: '', last: '', done: 0, total: 0, agents: 0, since: now, updated: now, ...extra,
  }));
  session('chat-e2e', { kind: 'chat', chat: 'e2e', state: 'working', project: 'Claude chat', title: 'e2e chat' });
  session('e2e-working', { state: 'working', tool: 'Edit', project: 'e2e', title: 'e2e working session' });
  session('e2e-done', { state: 'done', project: 'e2e', title: 'e2e finished session' });
  return { base, sessions, session };
}

/* ---------- process control ---------- */
const running = () => {
  try { return execFileSync('tasklist', ['/FI', 'IMAGENAME eq crab-widget.exe', '/NH'], { encoding: 'utf8' }).includes('crab-widget.exe'); } catch { return false; }
};
const killCrab = () => { try { execFileSync('taskkill', ['/IM', 'crab-widget.exe', '/T', '/F'], { stdio: 'ignore' }); } catch {} };
const ps = (cmd) => execFileSync('powershell', ['-NoProfile', '-Command', cmd], { encoding: 'utf8' }).trim();

/* ---------- main ---------- */
expect(existsSync(EXE), `no release build at ${EXE}; run npm run build first`);
const wasRunning = running();
killCrab();
await sleep(800);
const data = makeData();
const child = spawn(EXE, [], {
  // CRAB_QUIET_WINDOWS: the crab and its app window sit off-screen and never take focus
  env: { ...process.env, LOCALAPPDATA: data.base, CRAB_QUIET_WINDOWS: '1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${PORT}` },
  detached: true, stdio: 'ignore',
});
child.unref();
let terminalShell = null;
let terminalStop = null;

try {
  console.log('crab e2e');
  const widget = await page((p) => p.url === 'http://tauri.localhost/');
  await waitFor('widget script', () => widget.run('!!window.__TAURI__ && document.readyState === "complete"'));

  await test('widget sees the fake sessions', async () => {
    const ids = await waitFor('sessions', async () => {
      const s = await widget.run(`${invoke('get_state')}.then(s => (s.task.sessions || []).map(x => x.session))`);
      return s.length >= 3 && s;
    });
    expect(ids.includes('chat-e2e') && ids.includes('e2e-working'), `got ${ids}`);
  });

  await test('hover opens the horizontal panel with only live sessions', async () => {
    await widget.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 75, y: 75 });
    await waitFor('panel open', () => widget.run('!document.getElementById("panel").hidden && innerWidth > 400'), 3000);
    const titles = await widget.run('[...document.querySelectorAll("#sessions .sess-title")].map(n => n.textContent)');
    expect(titles.includes('e2e working session'), `panel rows: ${titles}`);
    expect(!titles.includes('e2e finished session'), 'a finished session is listed in the panel');
    expect(await widget.run('document.getElementById("p-session") !== null'), 'usage column missing');
  });

  await test('leaving the widget closes the panel', async () => {
    await widget.run('document.documentElement.dispatchEvent(new MouseEvent("mouseleave")), true');
    await waitFor('panel closed', () => widget.run('document.getElementById("panel").hidden && innerWidth < 200'), 3000);
  });

  await test('chat ✕ removes the chat from the list', async () => {
    // the virtual pointer never left the crab, so a second real hover would not fire mouseenter
    await widget.run('document.getElementById("stage").dispatchEvent(new MouseEvent("mouseenter")), true');
    await waitFor('panel open', () => widget.run('!document.getElementById("panel").hidden'), 3000);
    const clicked = await widget.run('(() => { const b = document.querySelector("#sessions .sess-close"); if (b) b.click(); return !!b; })()');
    expect(clicked, 'no ✕ on the chat row');
    await waitFor('chat record removed', () => !existsSync(join(data.sessions, 'chat-e2e.json')), 3000);
    await widget.run('document.documentElement.dispatchEvent(new MouseEvent("mouseleave")), true');
  });

  await test('clicking the crab opens the app window on Home', async () => {
    await sleep(600);
    await widget.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 75, y: 75, button: 'left', clickCount: 1 });
    await widget.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 75, y: 75, button: 'left', clickCount: 1 });
    const win = await page((p) => p.url.startsWith('http://tauri.localhost/manager.html'));
    await waitFor('home page', () => win.run('location.hash === "#home" && !document.getElementById("page-home").hidden'));
    const rows = await waitFor('home rows', () => win.run('[...document.querySelectorAll("#h-sessions .agent-title")].map(n => n.textContent)').then((r) => r.length && r));
    expect(rows.includes('e2e finished session'), `home rows: ${rows}`);
    win.close();
  });

  const clickCrab = async () => {
    await widget.send('Input.dispatchMouseEvent', { type: 'mousePressed', x: 75, y: 75, button: 'left', clickCount: 1 });
    await widget.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x: 75, y: 75, button: 'left', clickCount: 1 });
  };
  await test('clicking the crab again minimizes the window, and once more brings it back', async () => {
    const win = await page((p) => p.url.startsWith('http://tauri.localhost/manager.html'));
    await widget.run(`${invoke('open_manager', { page: 'settings' })}.then(() => true)`);
    await waitFor('settings shown', () => win.run('location.hash === "#settings" && document.visibilityState === "visible"'), 5000);
    await sleep(600);
    // WebView2 keeps visibilityState "visible" when minimized, so ask Tauri about the window.
    const minimized = () => widget.run(`window.__TAURI__.window.Window.getByLabel('manager').then(w => w.isMinimized())`);
    await clickCrab();
    await waitFor('window minimized', minimized, 3000);
    await sleep(600);
    await clickCrab();
    await waitFor('window restored', async () => !(await minimized()), 3000);
    expect((await win.run('location.hash')) === '#settings', 'restoring changed the page');
    win.close();
  });

  await test('settings page shows and changes settings', async () => {
    const win = await page((p) => p.url.startsWith('http://tauri.localhost/manager.html'));
    await win.run('location.hash = "#settings", true');
    await waitFor('settings page', () => win.run('!document.getElementById("page-settings").hidden'));
    const before = await win.run(`${invoke('get_settings')}.then(s => s.remaining)`);
    await win.run('document.querySelector("[data-key=remaining]").click(), true');
    await waitFor('setting flipped', async () => (await win.run(`${invoke('get_settings')}.then(s => s.remaining)`)) === !before);
    await waitFor('checkbox follows', async () => (await win.run('document.querySelector("[data-key=remaining]").checked')) === !before);
    await win.run('document.querySelector("[data-key=remaining]").click(), true');
    await waitFor('setting restored', async () => (await win.run(`${invoke('get_settings')}.then(s => s.remaining)`)) === before);
    win.close();
  });

  await test('sound style can be picked and is saved', async () => {
    const win = await page((p) => p.url.startsWith('http://tauri.localhost/manager.html'));
    const styles = await waitFor('style options', () => win.run('[...document.querySelectorAll("#sound-style option")].map(o => o.value)').then((v) => v.length && v));
    expect(styles.includes('arcade') && styles.includes('windows'), `styles: ${styles}`);
    await win.run('(() => { const s = document.getElementById("sound-style"); s.value = "arcade"; s.dispatchEvent(new Event("change")); })(), true');
    await waitFor('style saved', () => readFileSync(join(data.base, 'crab-widget', 'settings.json'), 'utf8').includes('"sound_style":"arcade"'), 3000);
    expect((await win.run(`${invoke('get_settings')}.then(s => s.style)`)) === 'arcade', 'style not applied');
    win.close();
  });

  await test('clicking an agent opens Swarm with that agent expanded', async () => {
    const how = await widget.run(invoke('open_session', { session: AGENT_ID }));
    expect(how === 'manager', `open_session returned ${how}`);
    const win = await page((p) => p.url.startsWith('http://tauri.localhost/manager.html'));
    await waitFor('agent expanded', () => win.run(`location.hash === "#swarm/${AGENT_ID}" && !document.getElementById("page-swarm").hidden && !!document.querySelector('.agent[data-id="${AGENT_ID}"] pre')`));
    win.close();
  });

  const hasWt = (() => { try { ps('Get-Command wt.exe -ErrorAction Stop | Out-Null'); return true; } catch { return false; } })();
  if (!FULL) {
    // it has to open a real terminal and pull it to the front, which interrupts whoever is typing
    results.push(['SKIP', 'terminal focus (run with --full)']);
    console.log('  skip terminal focus (opens a terminal and takes focus; run npm run e2e -- --full)');
  } else if (!hasWt) {
    results.push(['SKIP', 'terminal focus (no wt.exe)']);
    console.log('  skip terminal focus (no wt.exe)');
  } else {
    await test('clicking a terminal session brings its window to the front', async () => {
      const marker = `crab-e2e-${process.pid}`;
      // The shell exits by itself, with code 0, once the stop file appears: Windows Terminal only
      // closes a tab on a clean exit, so killing it would leave the window open.
      terminalStop = join(data.base, 'stop-terminal');
      // wt.exe treats a bare ";" as "next wt command", so each one is escaped as "\;"
      const script = `$host.UI.RawUI.WindowTitle='${marker}'; while (-not (Test-Path '${terminalStop}')) { Start-Sleep -Milliseconds 200 }; exit 0`;
      spawn('wt.exe', ['-w', 'new', 'powershell', '-NoProfile', '-Command', script.replaceAll(';', '\\;')],
        { detached: true, stdio: 'ignore' }).unref();
      // the new shell is the one whose command line carries the marker (not this query's own shell)
      terminalShell = Number(await waitFor('test terminal', () => ps(
        `(Get-CimInstance Win32_Process -Filter "Name='powershell.exe'" | Where-Object { $_.CommandLine -like '*${marker}*' -and $_.ProcessId -ne $PID } | Select-Object -First 1).ProcessId`,
      ) || null));
      data.session('e2e-terminal', { state: 'done', ppid: terminalShell, project: 'e2e', title: 'e2e terminal' });
      await sleep(1500);
      const how = await widget.run(invoke('open_session', { session: 'e2e-terminal' }));
      expect(how === 'window', `open_session returned ${how} (expected the terminal to be in front)`);
    });
  }

  widget.close();
} catch (e) {
  results.push(['FAIL', 'setup', e.message]);
  console.log(`  FAIL setup\n       ${e.message}`);
} finally {
  if (terminalStop) {
    try { writeFileSync(terminalStop, ''); } catch {}
    // give the shell a moment to exit on its own; kill it only if it is somehow still there
    for (let i = 0; i < 15 && terminalShell && ps(`[bool](Get-Process -Id ${terminalShell} -ErrorAction SilentlyContinue)`) === 'True'; i++) await sleep(200);
  }
  if (terminalShell) { try { process.kill(terminalShell); } catch {} }
  killCrab();
  await sleep(800);
  try { rmSync(data.base, { recursive: true, force: true }); } catch {}
  // put the user's own crab back, without the debug port
  if (wasRunning) {
    const env = { ...process.env };
    delete env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS;
    spawn(EXE, [], { env, detached: true, stdio: 'ignore' }).unref();
  }
}

const failed = results.filter((r) => r[0] === 'FAIL').length;
console.log(`\n${results.filter((r) => r[0] === 'PASS').length} passed, ${failed} failed, ${results.filter((r) => r[0] === 'SKIP').length} skipped`);
process.exit(failed ? 1 : 0);
