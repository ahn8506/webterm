// Page-side API of the small language model; generation runs in ./llm-worker.js.
export const LLM_MODEL_ID = "onnx-community/Qwen3.5-0.8B-Text-ONNX";
export const LLM_DEFAULT_DTYPE = "q4f16";

export class Llm {
  #worker;
  #pending = new Map();
  #nextId = 1;
  #onProgress;
  #info = null;

  constructor(worker, onProgress) {
    this.#worker = worker;
    this.#onProgress = onProgress;
    worker.onmessage = (event) => this.#onMessage(event.data);
    worker.onerror = (event) => {
      const error = new Error(`Language model worker failed: ${event.message || "could not start"}`);
      for (const { reject } of this.#pending.values()) reject(error);
      this.#pending.clear();
    };
  }

  // Starts the worker and loads the model (WebGPU only). onProgress receives
  // { status, file, loaded, total }.
  static async create({ onProgress, modelId = LLM_MODEL_ID, dtype = LLM_DEFAULT_DTYPE } = {}) {
    const worker = new Worker(new URL("./llm-worker.js", import.meta.url), {
      type: "module",
      name: "webterm-llm",
    });
    const llm = new Llm(worker, onProgress);
    try {
      llm.#info = await llm.#request("load", { modelId, dtype });
    } catch (error) {
      worker.terminate();
      throw error;
    }
    return llm;
  }

  // { device, modelId, dtype }
  get info() {
    return this.#info;
  }

  // messages: [{ role: "system" | "user" | "assistant", content }]. Returns
  // the reply text; decoding is greedy, so it is repeatable.
  generate(messages, { maxTokens = 48 } = {}) {
    return this.#request("generate", { messages, maxTokens });
  }

  dispose() {
    this.#worker.terminate();
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
      this.#onProgress?.(message.data);
      return;
    }
    const waiting = this.#pending.get(message.id);
    if (!waiting) return;
    this.#pending.delete(message.id);
    if (message.type === "error") waiting.reject(new Error(message.message));
    else waiting.resolve(message.data);
  }
}
