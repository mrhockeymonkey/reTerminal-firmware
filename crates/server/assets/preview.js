// Glue between the browser and web_preview.wasm (crates/web-preview).
//
// The wasm module exposes a handful of plain functions over two static
// buffers: we copy the JSON from GET /screen into `wp_spec_buf_ptr()`, call
// `wp_render(len)`, then read `wp_width()*wp_height()*4` RGBA bytes from
// `wp_rgba_ptr()` straight into an ImageData. No wasm-bindgen involved.

'use strict';

const canvas = document.getElementById('panel');
const ctx = canvas.getContext('2d');
const statusEl = document.getElementById('status');
const jsonEl = document.getElementById('json');
const putResult = document.getElementById('put-result');
const flashEl = document.getElementById('flash');

const POLL_MS = 2000;
const STATUS_TEXT = { 0: 'ok', 1: 'invalid spec', 2: 'unsupported version', '-1': 'document too large' };

let wasm = null;
let lastEtag = null;
let lastRenderedText = null;

function setStatus(text, isError) {
  statusEl.textContent = text;
  statusEl.className = isError ? 'err' : '';
}

function decodeError() {
  const len = wasm.wp_error_len();
  if (!len) return '';
  const bytes = new Uint8Array(wasm.memory.buffer, wasm.wp_error_ptr(), len);
  return new TextDecoder().decode(bytes);
}

// Draw the frame; optionally play the black/white flash a real refresh shows.
async function present(imageData) {
  if (flashEl.checked) {
    const frames = ['#000', '#f5f5f0', '#000', '#f5f5f0'];
    for (const fill of frames) {
      ctx.fillStyle = fill;
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      await new Promise((r) => setTimeout(r, 90));
    }
  }
  ctx.putImageData(imageData, 0, 0);
}

async function renderText(text, source) {
  const bytes = new TextEncoder().encode(text);
  const cap = wasm.wp_spec_buf_len();
  if (bytes.length > cap) {
    setStatus(`${source}: document is ${bytes.length} bytes, limit ${cap}`, true);
    return;
  }
  new Uint8Array(wasm.memory.buffer, wasm.wp_spec_buf_ptr(), bytes.length).set(bytes);
  const status = wasm.wp_render(bytes.length);
  const w = wasm.wp_width();
  const h = wasm.wp_height();
  // Copy out of wasm memory so the ImageData owns its buffer.
  const rgba = new Uint8ClampedArray(new Uint8Array(wasm.memory.buffer, wasm.wp_rgba_ptr(), w * h * 4));
  if (status !== -1) {
    await present(new ImageData(rgba, w, h));
  }
  const when = new Date().toLocaleTimeString();
  if (status === 0) {
    setStatus(`${source} · rendered ${when}`, false);
  } else {
    setStatus(`${source} · ${STATUS_TEXT[status] || status}: ${decodeError()}`, true);
  }
  lastRenderedText = text;
}

async function refresh(force) {
  if (!wasm) return;
  try {
    const headers = {};
    if (lastEtag && !force) headers['If-None-Match'] = lastEtag;
    const res = await fetch('/screen', { cache: 'no-store', headers });
    if (res.status === 304) return;
    if (!res.ok) {
      setStatus(`GET /screen failed: ${res.status}`, true);
      return;
    }
    lastEtag = res.headers.get('ETag');
    const text = await res.text();
    if (text === lastRenderedText && !force) return;
    if (document.activeElement !== jsonEl) jsonEl.value = text;
    await renderText(text, 'server');
  } catch (e) {
    setStatus(`GET /screen failed: ${e}`, true);
  }
}

async function putScreen() {
  const body = jsonEl.value;
  putResult.textContent = 'sending…';
  putResult.className = '';
  try {
    const res = await fetch('/screen', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body,
    });
    const text = await res.text();
    if (res.ok) {
      putResult.textContent = `accepted (${res.status})`;
      lastEtag = null;
      await refresh(true);
    } else {
      putResult.textContent = `${res.status}: ${text}`;
      putResult.className = 'err';
      // Show what the device would display for this document anyway.
      await renderText(body, 'local (rejected by server)');
    }
  } catch (e) {
    putResult.textContent = `PUT failed: ${e}`;
    putResult.className = 'err';
  }
}

async function main() {
  try {
    const { instance } = await WebAssembly.instantiateStreaming(
      fetch('web_preview.wasm', { cache: 'no-store' }),
      {},
    );
    wasm = instance.exports;
  } catch (e) {
    setStatus(`could not load web_preview.wasm (run scripts/build-web-preview.sh): ${e}`, true);
    return;
  }
  document.getElementById('put').addEventListener('click', putScreen);
  document.getElementById('reload').addEventListener('click', () => {
    lastEtag = null;
    lastRenderedText = null;
    refresh(true);
  });
  // Live-preview edits without touching the server: render on Ctrl/Cmd+Enter.
  jsonEl.addEventListener('keydown', (ev) => {
    if ((ev.ctrlKey || ev.metaKey) && ev.key === 'Enter') {
      ev.preventDefault();
      renderText(jsonEl.value, 'local (unsent)');
    }
  });
  await refresh(true);
  setInterval(() => refresh(false), POLL_MS);
}

main();
