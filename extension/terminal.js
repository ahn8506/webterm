import { initRunner } from "./automation/runner.js";
import { TermCapture } from "./automation/term.js";
import { DEMO_PORT, DemoDevice } from "./demo/device.js";
import { initAsk } from "./ask/bar.js";
import { initSearch } from "./search/panel.js";
import { SerialHost } from "./serial.js";
import init, { App } from "./wasm/webterm_app.js";

const FONT_FAMILY = "Consolas, 'Cascadia Mono', 'Malgun Gothic', monospace";
const FONT_SIZE_PX = 14;
const WHEEL_LINES = 3;
// Select value for typing a port name that enumeration did not find.
const CUSTOM_PORT = "";

const portSelect = document.getElementById("port");
const portCustom = document.getElementById("port-custom");
const baudSelect = document.getElementById("baud");
const connectButton = document.getElementById("connect");
const refreshButton = document.getElementById("refresh");
const autoCrCheckbox = document.getElementById("auto-cr");
const statusLabel = document.getElementById("status");
const countersLabel = document.getElementById("counters");
const screen = document.getElementById("screen");
const canvas = document.getElementById("canvas");
const input = document.getElementById("input");

const nativeHost = new SerialHost();
const demoHost = new DemoDevice();
// Whichever of the two owns the current connection.
let host = nativeHost;
// Once the user picks a port, a rescan must not move the selection.
let portChosen = false;

function on(type, handler) {
  nativeHost.addEventListener(type, handler);
  demoHost.addEventListener(type, handler);
}
const encoder = new TextEncoder();
const capture = new TermCapture();
let app = null;
let connected = false;
// Whether the previous chunk ended in CR, so CR LF split across chunks stays intact.
let lastWasCr = false;
let txBytes = 0;
let rxBytes = 0;

function showCounters() {
  countersLabel.textContent = `TX ${txBytes} · RX ${rxBytes}`;
}

function setStatus(text, isError = false) {
  statusLabel.textContent = text;
  statusLabel.classList.toggle("error", isError);
}

function setConnected(value) {
  connected = value;
  connectButton.textContent = value ? "Disconnect" : "Connect";
  portSelect.disabled = portCustom.disabled = baudSelect.disabled = value;
}

function send(text) {
  if (!connected || !text) return;
  app?.scroll_to_bottom();
  const bytes = encoder.encode(text);
  host.write(bytes);
  txBytes += bytes.length;
  showCounters();
}

function addCarriageReturns(bytes) {
  const out = [];
  for (const byte of bytes) {
    if (byte === 0x0a && !lastWasCr) out.push(0x0d);
    out.push(byte);
    lastWasCr = byte === 0x0d;
  }
  return new Uint8Array(out);
}

function receive(bytes) {
  rxBytes += bytes.length;
  showCounters();
  capture.push(bytes);
  if (!app) return;
  app.feed(autoCrCheckbox.checked ? addCarriageReturns(bytes) : bytes);
  // Replies to terminal queries (cursor position, device attributes).
  const reply = app.take_output();
  if (reply.length && connected) host.write(reply);
}

function fit() {
  app?.fit(canvas.clientWidth, canvas.clientHeight, window.devicePixelRatio);
}

function frame() {
  try {
    app.draw();
  } catch (error) {
    setStatus(`render: ${error}`, true);
    return;
  }
  requestAnimationFrame(frame);
}

async function startTerminal() {
  if (!navigator.gpu) {
    setStatus("WebGPU is not available in this browser", true);
    return;
  }
  try {
    await init();
    app = await App.create(canvas, FONT_FAMILY, FONT_SIZE_PX, window.devicePixelRatio);
  } catch (error) {
    setStatus(`terminal: ${error}`, true);
    return;
  }
  new ResizeObserver(fit).observe(canvas);
  fit();
  requestAnimationFrame(frame);
}

function showPorts(ports) {
  const previous = portChosen ? portSelect.value : null;
  const options = ports.map(({ path, description }) => {
    if (!description) return new Option(path, path);
    // Driver names such as "USB Serial Port(COM6)" already carry the port.
    const text = description.includes(path) ? description : `${path} — ${description}`;
    return new Option(text, path);
  });
  options.push(new Option("Demo device (mock)", DEMO_PORT));
  options.push(new Option("Other…", CUSTOM_PORT));
  portSelect.replaceChildren(...options);
  // Keep the current choice across a rescan when it still exists.
  if (options.some((option) => option.value === previous)) portSelect.value = previous;
  portCustom.hidden = portSelect.value !== CUSTOM_PORT;
}

on("ports", ({ detail }) => showPorts(detail.ports));

on("opened", ({ detail }) => {
  lastWasCr = false;
  txBytes = rxBytes = 0;
  showCounters();
  setConnected(true);
  setStatus(`${detail.path} @ ${baudSelect.value}`);
  input.focus();
});

on("data", ({ detail }) => receive(detail));

on("closed", ({ detail }) => {
  setConnected(false);
  setStatus(detail.reason ?? "closed", Boolean(detail.reason));
});

on("error", ({ detail }) => setStatus(detail.message, true));

on("disconnect", ({ detail }) => {
  setConnected(false);
  setStatus(`native host: ${detail}`, true);
});

refreshButton.addEventListener("click", () => nativeHost.list());

portSelect.addEventListener("change", () => {
  portChosen = true;
  portCustom.hidden = portSelect.value !== CUSTOM_PORT;
  if (!portCustom.hidden) portCustom.focus();
});

connectButton.addEventListener("click", () => {
  if (connected) {
    host.close();
    return;
  }
  const path = (portSelect.value === CUSTOM_PORT ? portCustom.value : portSelect.value).trim();
  if (!path) {
    setStatus("enter a port name", true);
    return;
  }
  host = path === DEMO_PORT ? demoHost : nativeHost;
  host.open(path, { baud: Number(baudSelect.value) });
});

autoCrCheckbox.checked = localStorage.getItem("autoCr") === "1";
autoCrCheckbox.addEventListener("change", () => {
  localStorage.setItem("autoCr", autoCrCheckbox.checked ? "1" : "0");
});

const SPECIAL_KEYS = {
  Enter: "\r",
  Backspace: "\x7f",
  Tab: "\t",
  Escape: "\x1b",
  Home: "\x1b[H",
  End: "\x1b[F",
  Insert: "\x1b[2~",
  Delete: "\x1b[3~",
  PageUp: "\x1b[5~",
  PageDown: "\x1b[6~",
};

const ARROW_KEYS = { ArrowUp: "A", ArrowDown: "B", ArrowRight: "C", ArrowLeft: "D" };

// Printable characters are not handled here: they arrive through the
// textarea's input event so IME composition produces the final text.
function encodeKey(event) {
  if (event.key in ARROW_KEYS) {
    return (app?.app_cursor_keys() ? "\x1bO" : "\x1b[") + ARROW_KEYS[event.key];
  }
  if (event.key in SPECIAL_KEYS) return SPECIAL_KEYS[event.key];
  if (event.ctrlKey && !event.altKey && event.key.length === 1) {
    const code = event.key.toUpperCase().charCodeAt(0);
    // Ctrl+@ .. Ctrl+_ map onto the C0 control range.
    if (code >= 0x40 && code <= 0x5f) return String.fromCharCode(code - 0x40);
  }
  return null;
}

input.addEventListener("keydown", (event) => {
  if (event.isComposing || event.metaKey) return;
  // Leave Ctrl+Shift+C/V and plain Ctrl+V to the browser.
  if (event.ctrlKey && (event.shiftKey || event.key.toLowerCase() === "v")) return;
  const text = encodeKey(event);
  if (text === null) return;
  event.preventDefault();
  send(text);
});

input.addEventListener("input", (event) => {
  if (event.isComposing) return;
  send(input.value);
  input.value = "";
});

input.addEventListener("compositionend", () => {
  send(input.value);
  input.value = "";
});

input.addEventListener("paste", (event) => {
  event.preventDefault();
  const text = event.clipboardData.getData("text").replace(/\r?\n/g, "\r");
  if (!text) return;
  send(app?.bracketed_paste() ? `\x1b[200~${text}\x1b[201~` : text);
});

screen.addEventListener("mousedown", (event) => {
  event.preventDefault();
  input.focus();
});

screen.addEventListener(
  "wheel",
  (event) => {
    event.preventDefault();
    app?.scroll_lines(event.deltaY < 0 ? WHEEL_LINES : -WHEEL_LINES);
  },
  { passive: false },
);

initRunner({
  capture,
  send(text) {
    if (!connected) throw new Error("serial: no port is connected");
    send(text);
  },
  screenText: () => (app ? app.lines_text(0, app.line_count()).join("\n") : ""),
});

const search = initSearch({
  reveal(text) {
    if (!app) return false;
    const lines = app.lines_text(0, app.line_count());
    // A line wider than the terminal is stored as several rows; match its first.
    const row = lines.findLastIndex((line) => {
      const shown = line.trim();
      return shown === text || (shown.length >= 20 && text.startsWith(shown));
    });
    if (row < 0) return false;
    app.scroll_to_bottom();
    app.scroll_lines(Math.max(0, lines.length - 1 - row - Math.floor(app.rows() / 2)));
    return true;
  },
});
capture.onText = (text) => search.addText(text);

initAsk({
  send(text) {
    if (!connected) throw new Error("Connect to a port first");
    send(text);
  },
});

// The demo entry is available even when the native host is not installed.
showPorts([]);
await startTerminal();
nativeHost.list();
