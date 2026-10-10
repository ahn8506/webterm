// Page-side API. Inference runs in ./worker.js (a dedicated module worker),
// so nothing here blocks the caller's render loop.
export { VectorIndex } from "./vector-index.js";

export const MODEL_ID = "onnx-community/embeddinggemma-2-ONNX";
export const DEFAULT_DTYPE = "q4";
export const MATRYOSHKA_DIMENSIONS = Object.freeze([768, 512, 256, 128]);

export class Embedder {
  #worker;
  #pending = new Map();
  #nextId = 1;
  #onProgress;
  #info = null;
  #dimensions;
  #batchSize;
  #disposed = false;

  constructor(worker, { onProgress, dimensions, batchSize }) {
    this.#worker = worker;
    this.#onProgress = onProgress;
    this.#dimensions = dimensions;
    this.#batchSize = batchSize;
    worker.onmessage = (event) => this.#onMessage(event.data);
    worker.onerror = (event) => {
      this.#failAll(new Error(`Embedding worker failed: ${event.message || "could not start"}`));
    };
  }

  // Starts the worker and loads the model. onProgress receives
  // { status, file, loaded, total }.
  //   device        "webgpu" (default) or "wasm"
  //   allowFallback fall back to WASM if WebGPU is missing or fails. Off by
  //                 default: the CPU path has not been tried with this model
  //                 and will be far slower (single-threaded). When it is
  //                 used, `device` is "wasm" and `info.fallbackReason` says why.
  //   dtype         ONNX quantisation of the model repo (default "q4")
  //   batchSize     texts per inference call (default 16)
  //   maxTokens     inputs are truncated to this many tokens (default 256)
  static async create({
    onProgress,
    dimensions = 768,
    device = "webgpu",
    allowFallback = false,
    modelId = MODEL_ID,
    dtype = DEFAULT_DTYPE,
    modelFileName,
    batchSize = 16,
    maxTokens = 256,
  } = {}) {
    if (!Number.isInteger(dimensions) || dimensions < 1 || dimensions > 768) {
      throw new RangeError("Embedder: dimensions must be an integer in 1..768");
    }
    const worker = new Worker(new URL("./worker.js", import.meta.url), {
      type: "module",
      name: "webterm-embed",
    });
    const embedder = new Embedder(worker, { onProgress, dimensions, batchSize });
    try {
      embedder.#info = await embedder.#request("load", {
        modelId, dtype, modelFileName, maxTokens, device, allowFallback,
      });
    } catch (error) {
      embedder.dispose();
      throw error;
    }
    return embedder;
  }

  // "webgpu" or "wasm": what the session is actually running on.
  get device() {
    return this.#info.device;
  }

  get dimensions() {
    return this.#dimensions;
  }

  // { device, requestedDevice, fallbackReason, adapter, modelId, dtype, ... }
  get info() {
    return this.#info;
  }

  // kind selects the EmbeddingGemma prompt prefix: "query" or "document".
  // Returns one L2-normalised Float32Array per input, in input order.
  async embed(texts, { kind = "document", dimensions = this.#dimensions } = {}) {
    if (this.#disposed) throw new Error("Embedder has been disposed");
    const single = typeof texts === "string";
    const list = single ? [texts] : Array.from(texts, String);
    const out = new Array(list.length);

    // Similar lengths share a batch so short lines are not padded to long ones.
    const order = list.map((_, i) => i).sort((a, b) => list[a].length - list[b].length);
    for (let start = 0; start < order.length; start += this.#batchSize) {
      const chunk = order.slice(start, start + this.#batchSize);
      const flat = await this.#request("embed", {
        texts: chunk.map((i) => list[i]),
        kind,
        dimensions,
      });
      chunk.forEach((index, row) => {
        out[index] = flat.slice(row * dimensions, (row + 1) * dimensions);
      });
    }
    return single ? out[0] : out;
  }

  dispose() {
    if (this.#disposed) return;
    this.#disposed = true;
    this.#worker.terminate();
    this.#failAll(new Error("Embedder has been disposed"));
  }

  #request(type, payload) {
    return new Promise((resolve, reject) => {
      const id = this.#nextId++;
      this.#pending.set(id, { resolve, reject });
      this.#worker.postMessage({ id, type, payload });
    });
  }

  #onMessage(message) {
    if (message.type === "progress") {
      try { this.#onProgress?.(message.data); } catch (error) { console.error(error); }
      return;
    }
    const entry = this.#pending.get(message.id);
    if (!entry) return;
    this.#pending.delete(message.id);
    if (message.type === "error") entry.reject(new Error(message.message));
    else entry.resolve(message.data);
  }

  #failAll(error) {
    for (const { reject } of this.#pending.values()) reject(error);
    this.#pending.clear();
  }
}
