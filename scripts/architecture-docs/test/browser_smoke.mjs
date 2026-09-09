#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const USAGE = 'Usage: browser_smoke.mjs --endpoint WS_URL --rfc HTML_PATH --diagram HTML_PATH';

function parseArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index];
    const value = argv[index + 1];
    if (!['--endpoint', '--rfc', '--diagram'].includes(option) || value === undefined || values.has(option)) {
      throw new Error('arguments_invalid');
    }
    values.set(option, value);
  }
  if (values.size !== 3) throw new Error('arguments_invalid');

  const endpoint = new URL(values.get('--endpoint'));
  const loopback = ['127.0.0.1', 'localhost', '[::1]'].includes(endpoint.hostname);
  if (endpoint.protocol !== 'ws:' || !loopback) throw new Error('endpoint_not_loopback_websocket');

  const files = {};
  for (const name of ['rfc', 'diagram']) {
    const file = values.get(`--${name}`);
    if (!path.isAbsolute(file)) throw new Error(`${name}_path_not_absolute`);
    let stat;
    try { stat = fs.statSync(file); } catch (_error) { throw new Error(`${name}_path_not_file`); }
    if (!stat.isFile()) throw new Error(`${name}_path_not_file`);
    files[name] = pathToFileURL(file).href;
  }
  return { endpoint: endpoint.href, ...files };
}

class Cdp {
  constructor(endpoint) {
    this.socket = new WebSocket(endpoint);
    this.nextId = 1;
    this.pending = new Map();
    this.listeners = [];
  }

  async connect() {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('cdp_connection_timeout')), 10_000);
      this.socket.addEventListener('open', () => {
        clearTimeout(timer);
        resolve();
      }, { once: true });
      this.socket.addEventListener('error', () => {
        clearTimeout(timer);
        reject(new Error('cdp_connection_failed'));
      }, { once: true });
    });
    this.socket.addEventListener('message', event => this.receive(JSON.parse(event.data)));
    this.socket.addEventListener('close', () => {
      for (const { reject } of this.pending.values()) reject(new Error('cdp_connection_closed'));
      this.pending.clear();
    });
  }

  receive(message) {
    if (message.id) {
      const pending = this.pending.get(message.id);
      if (!pending) return;
      this.pending.delete(message.id);
      if (message.error) pending.reject(new Error(`cdp_error ${message.error.message}`));
      else pending.resolve(message.result || {});
      return;
    }
    for (const listener of this.listeners) listener(message);
  }

  send(method, params = {}, sessionId = undefined) {
    const id = this.nextId++;
    const message = { id, method, params };
    if (sessionId) message.sessionId = sessionId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`cdp_command_timeout method=${method}`));
      }, 10_000);
      this.pending.set(id, {
        resolve: value => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: error => {
          clearTimeout(timer);
          reject(error);
        }
      });
      this.socket.send(JSON.stringify(message));
    });
  }

  onEvent(listener) {
    this.listeners.push(listener);
  }

  close() {
    this.socket.close();
  }
}

function assert(condition, code) {
  if (!condition) throw new Error(code);
}

async function waitFor(cdp, sessionId, expression, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const result = await cdp.send('Runtime.evaluate', {
      expression,
      returnByValue: true,
      awaitPromise: true
    }, sessionId);
    if (result.result?.value) return result.result.value;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error('document_ready_timeout');
}

async function evaluate(cdp, sessionId, expression, options = {}) {
  const result = await cdp.send('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
    userGesture: options.userGesture === true
  }, sessionId);
  if (result.exceptionDetails) throw new Error('browser_evaluation_failed');
  return result.result?.value;
}

async function navigate(cdp, sessionId, url) {
  await cdp.send('Page.navigate', { url }, sessionId);
  await waitFor(
    cdp,
    sessionId,
    `location.href === ${JSON.stringify(url)} && document.documentElement?.dataset.documentReady === 'complete'`
  );
}

async function checkRfc(cdp, sessionId, url) {
  await navigate(cdp, sessionId, url);
  const structure = await evaluate(cdp, sessionId, `(() => ({
    status: document.documentElement.dataset.status,
    headings: document.querySelectorAll('#document-content h1,#document-content h2,#document-content h3,#document-content h4,#document-content h5,#document-content h6').length,
    tables: document.querySelectorAll('#document-content table').length,
    fences: document.querySelectorAll('#document-content pre > code[class^="language-"]').length,
    toc: document.querySelectorAll('#table-of-contents a').length,
    sourceBytes: atob(document.getElementById('markdown-source').textContent).length,
    metadataBytes: JSON.parse(document.getElementById('build-metadata').textContent).source.bytes
  }))()`);
  assert(structure.status === 'PROVISIONAL', 'rfc_status_invalid');
  assert(structure.headings === 117, 'rfc_heading_count_invalid');
  assert(structure.tables === 54, 'rfc_table_count_invalid');
  assert(structure.fences === 2, 'rfc_fence_count_invalid');
  assert(structure.toc === 117, 'rfc_toc_count_invalid');
  assert(structure.sourceBytes === structure.metadataBytes, 'rfc_source_bytes_invalid');

  const interactions = await evaluate(cdp, sessionId, `(() => {
    const search = document.getElementById('doc-search');
    search.value = '完整 52 Unit 附录';
    search.dispatchEvent(new Event('input', { bubbles: true }));
    const visibleToc = Array.from(document.querySelectorAll('#table-of-contents li')).filter(item => item.dataset.searchHidden !== 'true').length;
    const hiddenToc = document.querySelectorAll('#table-of-contents li[data-search-hidden="true"]').length;
    document.getElementById('theme-toggle').click();
    const theme = document.documentElement.dataset.theme;
    document.getElementById('collapse-all').click();
    const collapsed = document.querySelectorAll('#document-content [hidden]').length;
    document.getElementById('expand-all').click();
    const expanded = document.querySelectorAll('#document-content [hidden]').length;
    return { visibleToc, hiddenToc, theme, collapsed, expanded };
  })()`);
  assert(interactions.visibleToc >= 1 && interactions.hiddenToc >= 1, 'rfc_search_failed');
  assert(interactions.theme === 'dark', 'rfc_theme_failed');
  assert(interactions.collapsed >= 1 && interactions.expanded === 0, 'rfc_collapse_failed');

  await cdp.send('Emulation.setEmulatedMedia', { media: 'print' }, sessionId);
  const print = await evaluate(cdp, sessionId, `(() => {
    window.dispatchEvent(new Event('beforeprint'));
    return {
      toolbar: getComputedStyle(document.querySelector('.toolbar')).display,
      hidden: document.querySelectorAll('#document-content [hidden]').length
    };
  })()`);
  assert(print.toolbar === 'none' && print.hidden === 0, 'rfc_print_expansion_failed');
  await cdp.send('Emulation.setEmulatedMedia', { media: '' }, sessionId);
  return structure;
}

async function checkDiagrams(cdp, sessionId, url) {
  await navigate(cdp, sessionId, url);
  const diagrams = await evaluate(cdp, sessionId, `(() => {
    const figures = Array.from(document.querySelectorAll('.diagram'));
    const rendered = figures.filter(figure => figure.dataset.diagramState === 'rendered');
    const failed = figures.filter(figure => figure.dataset.diagramState === 'error');
    const sourcePreserved = figures.every(figure => figure.querySelector('.mermaid-source code')?.textContent.trim().length > 0);
    const sourceVisible = figures.every(figure => getComputedStyle(figure.querySelector('.mermaid-source')).display !== 'none');
    const svgCount = rendered.filter(figure => figure.querySelector('.mermaid-render svg')).length;
    const readableFailure = failed.every(figure => figure.querySelector('.mermaid-render').textContent.includes('图表渲染失败'));
    const zoomFigure = rendered[0];
    const beforeZoom = zoomFigure?.style.getPropertyValue('--diagram-scale');
    zoomFigure?.querySelector('[data-diagram-zoom="in"]').click();
    const afterZoom = zoomFigure?.style.getPropertyValue('--diagram-scale');
    const fullscreenControls = figures.filter(figure => figure.querySelector('[data-diagram-fullscreen]')).length;
    return { total: figures.length, rendered: rendered.length, failed: failed.length, sourcePreserved, sourceVisible, svgCount, readableFailure, beforeZoom, afterZoom, fullscreenControls };
  })()`);
  assert(diagrams.total >= 4, 'diagram_fixture_incomplete');
  assert(diagrams.rendered >= 3 && diagrams.svgCount === diagrams.rendered, 'diagram_svg_render_failed');
  assert(diagrams.failed >= 1 && diagrams.readableFailure, 'diagram_failure_fallback_invalid');
  assert(diagrams.sourcePreserved && diagrams.sourceVisible, 'diagram_source_not_preserved');
  assert(diagrams.beforeZoom !== diagrams.afterZoom, 'diagram_zoom_failed');
  assert(diagrams.fullscreenControls === diagrams.total, 'diagram_fullscreen_control_missing');

  const fullscreen = await evaluate(cdp, sessionId, `(async () => {
    const figure = Array.from(document.querySelectorAll('.diagram')).find(item => item.dataset.diagramState === 'rendered');
    const button = figure?.querySelector('[data-diagram-fullscreen]');
    if (!figure || !button || typeof figure.requestFullscreen !== 'function') {
      return { supported: false, entered: false, exited: false };
    }
    button.click();
    for (let index = 0; index < 40 && document.fullscreenElement !== figure; index += 1) {
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    const entered = document.fullscreenElement === figure;
    if (entered) await document.exitFullscreen();
    for (let index = 0; index < 40 && document.fullscreenElement !== null; index += 1) {
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    return { supported: true, entered, exited: document.fullscreenElement === null };
  })()`, { userGesture: true });
  assert(fullscreen.supported, 'diagram_fullscreen_api_unavailable');
  assert(fullscreen.entered, 'diagram_fullscreen_entry_failed');
  assert(fullscreen.exited, 'diagram_fullscreen_exit_failed');

  const injection = await evaluate(cdp, sessionId, `(() => {
    const content = document.getElementById('document-content');
    const text = content.textContent;
    return {
      probeVisible: text.includes('window.fixtureExecuted') && text.includes('<img'),
      probeExecuted: globalThis.fixtureExecuted === true,
      executableProbeElements: content.querySelectorAll('script,img').length
    };
  })()`);
  assert(injection.probeVisible, 'diagram_injection_probe_not_visible');
  assert(!injection.probeExecuted, 'diagram_script_probe_executed');
  assert(injection.executableProbeElements === 0, 'diagram_raw_resource_probe_created');
  return diagrams;
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  assert(typeof WebSocket === 'function', 'native_websocket_unavailable');
  const cdp = new Cdp(options.endpoint);
  const networkAttempts = [];
  const targets = [];
  try {
    await cdp.connect();
    cdp.onEvent(message => {
      if (message.method === 'Network.requestWillBeSent') {
        const url = message.params?.request?.url || '';
        if (/^https?:/i.test(url)) networkAttempts.push(url);
      }
    });
    const created = await cdp.send('Target.createTarget', { url: 'about:blank' });
    targets.push(created.targetId);
    const attached = await cdp.send('Target.attachToTarget', { targetId: created.targetId, flatten: true });
    const sessionId = attached.sessionId;
    await cdp.send('Page.enable', {}, sessionId);
    await cdp.send('Runtime.enable', {}, sessionId);
    await cdp.send('Network.enable', {}, sessionId);
    await cdp.send('Network.setBlockedURLs', { urls: ['http://*', 'https://*'] }, sessionId);

    const rfc = await checkRfc(cdp, sessionId, options.rfc);
    const diagrams = await checkDiagrams(cdp, sessionId, options.diagram);
    assert(
      networkAttempts.length === 0,
      `external_network_attempts=${networkAttempts.length} urls=${JSON.stringify(networkAttempts)}`
    );
    process.stdout.write(`browser_smoke_ok headings=${rfc.headings} tables=${rfc.tables} diagrams=${diagrams.rendered} diagram_errors=${diagrams.failed} fullscreen=entered_exited injection=blocked network_attempts=0\n`);
  } finally {
    for (const targetId of targets) {
      try { await cdp.send('Target.closeTarget', { targetId }); } catch (_error) { /* best effort */ }
    }
    cdp.close();
  }
}

main().catch(error => {
  process.stderr.write(`${error.message}\n${USAGE}\n`);
  process.exitCode = error.message === 'arguments_invalid' ? 2 : 1;
});
