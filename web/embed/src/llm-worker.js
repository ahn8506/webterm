// Dedicated module worker for the small language model. Like worker.js, all
// code is shipped next to this file; only weights come from the network.
import { AutoModelForCausalLM, AutoTokenizer, env } from "@huggingface/transformers";

const ORT_DIR = new URL("./ort/", import.meta.url);

env.allowLocalModels = false;
env.allowRemoteModels = true;
env.useBrowserCache = true;
env.useWasmCache = false;
env.backends.onnx.wasm.wasmPaths = {
  mjs: new URL("ort-wasm-simd-threaded.asyncify.mjs", ORT_DIR).href,
  wasm: new URL("ort-wasm-simd-threaded.asyncify.wasm", ORT_DIR).href,
};
env.backends.onnx.wasm.proxy = false;
if (!self.crossOriginIsolated) env.backends.onnx.wasm.numThreads = 1;

let tokenizer = null;
let model = null;

function progressCallback(event) {
  if (!event || !event.file) return;
  self.postMessage({
    type: "progress",
    data: { status: event.status, file: event.file, loaded: event.loaded ?? 0, total: event.total ?? 0 },
  });
}

async function load({ modelId, dtype }) {
  if (!("gpu" in navigator) || !(await navigator.gpu.requestAdapter())) {
    throw new Error("WebGPU unavailable: the language model needs it");
  }
  tokenizer = await AutoTokenizer.from_pretrained(modelId, { progress_callback: progressCallback });
  model = await AutoModelForCausalLM.from_pretrained(modelId, {
    device: "webgpu",
    dtype,
    progress_callback: progressCallback,
  });
  // Compiles the WebGPU pipelines so the first real request is not the slow one.
  await generate({ messages: [{ role: "user", content: "hi" }], maxTokens: 1 });
  return { device: "webgpu", modelId, dtype };
}

async function generate({ messages, maxTokens }) {
  const inputs = tokenizer.apply_chat_template(messages, {
    add_generation_prompt: true,
    return_dict: true,
    enable_thinking: false,
  });
  const prompt = inputs.input_ids.dims[1];
  // Greedy: the same request always gives the same answer.
  const output = await model.generate({ ...inputs, max_new_tokens: maxTokens, do_sample: false });
  const [text] = tokenizer.batch_decode(output.slice(null, [prompt, null]), {
    skip_special_tokens: true,
  });
  return text;
}

// Requests are handled strictly one at a time, in arrival order.
let queue = Promise.resolve();

self.onmessage = (event) => {
  const { id, type, payload } = event.data;
  queue = queue.then(async () => {
    try {
      let data;
      if (type === "load") data = await load(payload);
      else if (type === "generate") {
        if (!model) throw new Error("Model is not loaded");
        data = await generate(payload);
      } else throw new Error(`Unknown request "${type}"`);
      self.postMessage({ type: "result", id, data });
    } catch (error) {
      self.postMessage({ type: "error", id, message: String(error?.message ?? error) });
    }
  });
};
