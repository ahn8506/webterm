// Dedicated module worker: owns the tokenizer and the ONNX Runtime session.
// Everything it executes is shipped next to this file; only model weights
// (data) are fetched from the network and kept in the Cache API.
import { AutoConfig, AutoModel, AutoTokenizer, env } from "@huggingface/transformers";

const ORT_DIR = new URL("./ort/", import.meta.url);

env.allowLocalModels = false;
env.allowRemoteModels = true;
env.useBrowserCache = true;
// The runtime files are local, so there is nothing to cache, and the cache
// path would turn the factory into a blob: URL that the page CSP rejects.
env.useWasmCache = false;
env.backends.onnx.wasm.wasmPaths = {
  mjs: new URL("ort-wasm-simd-threaded.asyncify.mjs", ORT_DIR).href,
  wasm: new URL("ort-wasm-simd-threaded.asyncify.wasm", ORT_DIR).href,
};
env.backends.onnx.wasm.proxy = false;
if (!self.crossOriginIsolated) env.backends.onnx.wasm.numThreads = 1;

// Prompt prefixes from the EmbeddingGemma 2 model card.
const PREFIXES = {
  query: "task: search result | query: ",
  document: "title: none | text: ",
};

let tokenizer = null;
let model = null;
let config = null;

async function probeWebGPU() {
  if (!("gpu" in navigator)) return { ok: false, reason: "navigator.gpu is not available" };
  try {
    const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
    if (!adapter) return { ok: false, reason: "no WebGPU adapter" };
    const info = adapter.info ?? {};
    return {
      ok: true,
      adapter: {
        vendor: info.vendor ?? "",
        architecture: info.architecture ?? "",
        device: info.device ?? "",
        description: info.description ?? "",
        isFallbackAdapter: info.isFallbackAdapter ?? adapter.isFallbackAdapter ?? false,
      },
    };
  } catch (error) {
    return { ok: false, reason: String(error?.message ?? error) };
  }
}

function progressCallback(event) {
  // Transformers.js reports per-file initiate/download/progress/done events.
  if (!event || !event.file) return;
  self.postMessage({
    type: "progress",
    data: {
      status: event.status,
      file: event.file,
      loaded: event.loaded ?? 0,
      total: event.total ?? 0,
    },
  });
}

function stage(status) {
  self.postMessage({ type: "progress", data: { status, file: "", loaded: 0, total: 0 } });
}

async function runBatch(texts, kind, dimensions) {
  const prefix = PREFIXES[kind];
  if (prefix === undefined) throw new Error(`Unknown kind "${kind}" (use "query" or "document")`);
  const inputs = tokenizer(
    texts.map((t) => prefix + t),
    { padding: true, truncation: true, max_length: config.maxTokens },
  );
  const { sentence_embedding } = await model(inputs);
  const full = sentence_embedding.dims[1];
  const raw = sentence_embedding.data;
  const out = new Float32Array(texts.length * dimensions);
  for (let r = 0; r < texts.length; r++) {
    // Matryoshka: keep the leading `dimensions` values, then re-normalise.
    let sum = 0;
    for (let i = 0; i < dimensions; i++) {
      const v = raw[r * full + i];
      sum += v * v;
    }
    if (!Number.isFinite(sum) || sum === 0) {
      throw new Error(`Model returned a non-finite or zero embedding on ${config.device}`);
    }
    const scale = 1 / Math.sqrt(sum);
    for (let i = 0; i < dimensions; i++) out[r * dimensions + i] = raw[r * full + i] * scale;
  }
  sentence_embedding.dispose?.();
  return out;
}

async function load(options) {
  const { modelId, dtype, modelFileName, maxTokens, device: wanted, allowFallback } = options;
  const gpu = await probeWebGPU();

  const attempts = [];
  if (wanted !== "wasm") {
    if (gpu.ok) attempts.push("webgpu");
    else if (!allowFallback) throw new Error(`WebGPU unavailable: ${gpu.reason}`);
  }
  if (wanted === "wasm" || allowFallback) attempts.push("wasm");

  tokenizer = await AutoTokenizer.from_pretrained(modelId, { progress_callback: progressCallback });

  // Text only: without these two the vision and audio encoders (about 300 MB
  // more at q4) are neither downloaded nor loaded.
  const modelConfig = await AutoConfig.from_pretrained(modelId);
  modelConfig.vision_config = modelConfig.audio_config = null;

  let fallbackReason = gpu.ok ? null : gpu.reason;
  let lastError = null;
  for (const device of attempts) {
    try {
      stage(`creating ${device} session`);
      const file = modelFileName ?? "model";
      model = await AutoModel.from_pretrained(modelId, {
        config: modelConfig,
        device,
        dtype,
        model_file_name: file,
        progress_callback: progressCallback,
      });
      config = { device, maxTokens, file };
      // Warm-up: compiles the WebGPU pipelines and proves the model produces
      // finite output on this device before we report it as usable.
      stage("warming up");
      await runBatch(["warm up"], "document", 768);
      lastError = null;
      break;
    } catch (error) {
      lastError = error;
      fallbackReason = String(error?.message ?? error);
      try { await model?.dispose(); } catch {}
      model = null;
    }
  }
  if (lastError) throw lastError;

  return {
    device: config.device,
    requestedDevice: wanted,
    fallbackReason: config.device === "webgpu" ? null : fallbackReason,
    adapter: gpu.ok ? gpu.adapter : null,
    modelId,
    dtype,
    modelFileName: config.file,
    maxTokens,
    threads: env.backends.onnx.wasm.numThreads ?? null,
  };
}

// Requests are handled strictly one at a time, in arrival order.
let queue = Promise.resolve();

self.onmessage = (event) => {
  const { id, type, payload } = event.data;
  queue = queue.then(async () => {
    try {
      if (type === "load") {
        self.postMessage({ type: "result", id, data: await load(payload) });
      } else if (type === "embed") {
        if (!model) throw new Error("Model is not loaded");
        const vectors = await runBatch(payload.texts, payload.kind, payload.dimensions);
        self.postMessage({ type: "result", id, data: vectors }, [vectors.buffer]);
      } else if (type === "dispose") {
        try { await model?.dispose(); } catch {}
        model = null;
        tokenizer = null;
        self.postMessage({ type: "result", id, data: null });
      } else {
        throw new Error(`Unknown request "${type}"`);
      }
    } catch (error) {
      self.postMessage({ type: "error", id, message: String(error?.message ?? error) });
    }
  });
};
