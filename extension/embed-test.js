import { Embedder, VectorIndex } from "./embed/client.js";

const SAMPLE_LINES = [
  "[    0.000000] Booting Linux on physical CPU 0x0000000000 [0x410fd034]",
  "[    1.284113] mmc0: new high speed SDHC card at address aaaa",
  "[    3.912004] EXT4-fs (mmcblk0p2): mounted filesystem with ordered data mode",
  "[   12.501877] eth0: Link is Up - 1Gbps/Full - flow control rx/tx",
  "[  214.775310] eth0: Link is Down",
  "[  214.990142] wlan0: deauthenticating from 3c:84:6a:12:9f:10 by local choice (Reason: 3=DEAUTH_LEAVING)",
  "dhclient: DHCPDISCOVER on eth0 to 255.255.255.255 port 67 interval 8",
  "sshd[812]: Failed password for invalid user admin from 203.0.113.44 port 51122 ssh2",
  "sshd[845]: Accepted publickey for pi from 192.168.1.20 port 60214 ssh2",
  "[  301.118220] Out of memory: Killed process 1422 (python3) total-vm:1893452kB, anon-rss:902112kB",
  "[  455.002931] thermal thermal_zone0: critical temperature reached (105 C), shutting down",
  "[  512.660018] usb 1-1.2: new full-speed USB device number 5 using dwc_otg",
  "[  512.801455] cdc_acm 1-1.2:1.0: ttyACM0: USB ACM device",
  "E (10342) wifi: esp_wifi_connect failed: ESP_ERR_WIFI_SSID",
  "I (512) boot: Loaded app from partition at offset 0x10000",
  "Guru Meditation Error: Core 0 panic'ed (LoadProhibited). Exception was unhandled.",
  "rst:0xc (SW_CPU_RESET),boot:0x13 (SPI_FAST_FLASH_BOOT)",
  "U-Boot 2023.04 (Jul 11 2023 - 09:12:44 +0000) DRAM: 1 GiB",
  "systemd[1]: nginx.service: Main process exited, code=exited, status=1/FAILURE",
  "nginx: [emerg] bind() to 0.0.0.0:80 failed (98: Address already in use)",
  "$ df -h /  ->  /dev/root  29G  28G  120M  100% /",
  "write error: No space left on device",
  "[  733.410071] I2C: bme280 0-0076: failed to read chip id (-121)",
  "CRC mismatch in firmware image: expected 0x8fa3c21d, got 0x00000000",
];

const params = new URLSearchParams(location.search);
const options = {
  device: params.get("device") ?? "webgpu",
  dimensions: Number(params.get("dims") ?? 768),
};
if (params.get("dtype")) options.dtype = params.get("dtype");
if (params.get("file")) options.modelFileName = params.get("file");
if (params.get("fallback") === "1") options.allowFallback = true;

const $ = (id) => document.getElementById(id);
const ms = (value) => `${value.toFixed(0)} ms`;
const mb = (bytes) => `${(bytes / 1e6).toFixed(1)} MB`;

// Read by automated checks; also handy from DevTools.
const state = { phase: "idle", options, timings: {}, results: null, error: null };
window.__embedTest = state;

let embedder = null;
let index = null;

function setStatus(text, cls = "") {
  $("status").textContent = text;
  $("status").className = cls;
}

function fact(name, value) {
  const row = $("facts").insertRow();
  row.insertCell().textContent = name;
  row.insertCell().textContent = value;
}

const fileRows = new Map();
function onProgress({ status, file, loaded, total }) {
  if (!file) {
    setStatus(`${status}…`);
    return;
  }
  let row = fileRows.get(file);
  if (!row) {
    const tr = $("files").insertRow();
    tr.insertCell().textContent = file;
    const bar = document.createElement("progress");
    bar.max = 1;
    bar.value = 0;
    tr.insertCell().append(bar);
    row = { bar, text: tr.insertCell(), total: 0 };
    row.text.className = "num";
    fileRows.set(file, row);
  }
  if (total) {
    row.total = total;
    row.bar.value = loaded / total;
    row.text.textContent = `${mb(loaded)} / ${mb(total)}`;
  }
  if (status === "done") {
    row.bar.value = 1;
    if (row.total) row.text.textContent = mb(row.total);
  }
  setStatus("Loading model files…");
}

async function load() {
  $("load").disabled = true;
  state.phase = "loading";
  try {
    const t0 = performance.now();
    embedder = await Embedder.create({ ...options, onProgress });
    state.timings.loadMs = performance.now() - t0;
    state.info = embedder.info;
    state.downloadBytes = [...fileRows.values()].reduce((sum, row) => sum + row.total, 0);

    const info = embedder.info;
    fact("Device in use", info.device);
    if (info.fallbackReason) fact("WebGPU not used because", info.fallbackReason);
    if (info.adapter) {
      const a = info.adapter;
      fact("GPU adapter", [a.vendor, a.architecture, a.device, a.description].filter(Boolean).join(" / ")
        + (a.isFallbackAdapter ? " (software fallback adapter)" : ""));
    }
    fact("Model", `${info.modelId} (${info.modelFileName}, dtype ${info.dtype})`);
    fact("Dimensions", String(embedder.dimensions));
    fact("Model files", mb(state.downloadBytes));
    fact("Load time (download or cache + session + warm-up)", ms(state.timings.loadMs));

    setStatus("Embedding sample log lines…");
    index = new VectorIndex(embedder.dimensions);
    // One call of exactly one batch (16) first, so the number is a clean per-batch latency.
    const t1 = performance.now();
    const first = await embedder.embed(SAMPLE_LINES.slice(0, 16));
    state.timings.firstBatchMs = performance.now() - t1;
    const t2 = performance.now();
    const again = await embedder.embed(SAMPLE_LINES.slice(0, 16));
    state.timings.warmBatchMs = performance.now() - t2;
    const rest = await embedder.embed(SAMPLE_LINES.slice(16));
    [...first, ...rest].forEach((vector, i) => index.add(i, vector));

    let drift = 0;
    for (let i = 0; i < first.length; i++) {
      let dot = 0;
      for (let j = 0; j < first[i].length; j++) dot += first[i][j] * again[i][j];
      drift = Math.max(drift, 1 - dot);
    }
    state.repeatDrift = drift;
    state.vectors = [...first, ...rest];

    fact("Batch of 16 lines, first run", ms(state.timings.firstBatchMs));
    fact("Batch of 16 lines, repeated", `${ms(state.timings.warmBatchMs)} (${(state.timings.warmBatchMs / 16).toFixed(1)} ms per line)`);
    fact("Indexed lines", String(index.size));

    $("go").disabled = false;
    setStatus(
      info.device === "webgpu" ? "Ready on WebGPU." : `Ready, but running on ${info.device}, not WebGPU.`,
      info.device === "webgpu" ? "ok" : "warn",
    );
    await search();
    state.phase = "ready";
  } catch (error) {
    state.phase = "error";
    state.error = String(error?.stack ?? error);
    setStatus(state.error, "err");
    $("load").disabled = false;
  }
}

async function search() {
  const query = $("query").value.trim();
  if (!query || !embedder) return;
  const t0 = performance.now();
  const [vector] = await embedder.embed([query], { kind: "query" });
  const embedMs = performance.now() - t0;
  const t1 = performance.now();
  const hits = index.search(vector, 8);
  const searchMs = performance.now() - t1;

  state.timings.queryMs = embedMs;
  state.queryVector = vector;
  state.results = hits.map((hit) => ({ ...hit, line: SAMPLE_LINES[hit.id] }));

  const table = $("results");
  table.replaceChildren();
  const head = table.insertRow();
  for (const title of ["#", "Score", `Line (query embedded in ${ms(embedMs)}, search ${searchMs.toFixed(2)} ms)`]) {
    const th = document.createElement("th");
    th.textContent = title;
    head.append(th);
  }
  state.results.forEach((hit, rank) => {
    const row = table.insertRow();
    row.insertCell().textContent = String(rank + 1);
    const score = row.insertCell();
    score.className = "num";
    score.textContent = hit.score.toFixed(4);
    const line = row.insertCell();
    line.className = "line";
    line.textContent = hit.line;
  });
}

$("load").addEventListener("click", load);
$("search").addEventListener("submit", (event) => {
  event.preventDefault();
  search().catch((error) => setStatus(String(error), "err"));
});
if (params.get("auto") === "1") load();
