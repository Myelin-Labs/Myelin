// Drive an isolated headless test browser with Node's built-in CDP WebSocket.
// A pending page, navigation failure or WASM exception cannot count as success.
import { spawn } from 'node:child_process';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';

const [browser, url, profile] = process.argv.slice(2);
if (!browser || !url || !profile || typeof WebSocket !== 'function') throw Error('requires browser, URL, profile and Node 22+');
const child = spawn(browser, ['--headless', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
  '--disable-dev-shm-usage', '--disable-extensions', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'],
  { stdio: ['ignore', 'ignore', 'inherit'] });
const exited = new Promise(resolve => child.once('exit', resolve));
let ws;
const pending = new Map();
const deadline = setTimeout(() => { console.error('browser conformance timed out'); child.kill('SIGKILL'); process.exitCode = 1; ws?.close(); }, 110_000);
try {
  let port;
  for (let n = 0; n < 200; n++) {
    if (child.exitCode !== null || child.signalCode !== null) throw Error('browser exited before debugging endpoint opened');
    try { port = (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; break; }
    catch { await delay(50); }
  }
  if (!port) throw Error('browser debugging endpoint did not open');
  const target = await (await fetch(`http://127.0.0.1:${port}/json/new?about:blank`, {method: 'PUT'})).json();
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.addEventListener('open', resolve, {once:true}); ws.addEventListener('error', reject, {once:true}); });
  ws.addEventListener('message', event => {
    const reply = JSON.parse(event.data);
    const request = pending.get(reply.id);
    if (request) { pending.delete(reply.id); reply.error ? request.reject(Error(JSON.stringify(reply.error))) : request.resolve(reply.result); }
  });
  ws.addEventListener('close', () => { for (const request of pending.values()) request.reject(Error('browser connection closed')); pending.clear(); });
  let nextId = 0;
  const call = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++nextId; pending.set(id, {resolve, reject}); ws.send(JSON.stringify({id, method, params}));
  });
  await call('Page.enable');
  const loaded = new Promise(resolve => {
    const listener = event => { if (JSON.parse(event.data).method === 'Page.loadEventFired') { ws.removeEventListener('message', listener); resolve(); } };
    ws.addEventListener('message', listener);
  });
  const navigation = await call('Page.navigate', {url});
  if (navigation.errorText) throw Error(navigation.errorText);
  await loaded;
  const response = await call('Runtime.evaluate', {
    expression: `new Promise(resolve => {
      const check = () => {
        const element = document.getElementById('result');
        if (element && element.textContent !== 'pending') resolve({result: JSON.parse(element.textContent), html: document.documentElement.outerHTML});
        else setTimeout(check, 25);
      }; check();
    })`, awaitPromise: true, returnByValue: true,
  });
  if (response.exceptionDetails) throw Error(JSON.stringify(response.exceptionDetails));
  if (response.result.value.result.status !== 'passed') throw Error(JSON.stringify(response.result.value.result));
  process.stdout.write(JSON.stringify(response.result.value) + '\n');
} finally {
  clearTimeout(deadline);
  ws?.close();
  child.kill('SIGTERM');
  const kill = setTimeout(() => child.kill('SIGKILL'), 5000);
  await exited;
  clearTimeout(kill);
}
