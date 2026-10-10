// Builds extension/embed: client.js, worker.js and the ONNX Runtime files.
import { build } from "esbuild";
import { copyFile, mkdir, readFile, readdir, rm, stat, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const outDir = path.resolve(here, "../../extension/embed");
const require = createRequire(import.meta.url);

// Resolved from Transformers.js so we ship the exact runtime it was built for.
const transformersEntry = require.resolve("@huggingface/transformers");
const ortDist = path.join(
  path.dirname(createRequire(transformersEntry).resolve("onnxruntime-web")),
);
const ORT_FILES = ["ort-wasm-simd-threaded.asyncify.mjs", "ort-wasm-simd-threaded.asyncify.wasm"];

await rm(outDir, { recursive: true, force: true });
await mkdir(path.join(outDir, "ort"), { recursive: true });

await build({
  absWorkingDir: here,
  entryPoints: {
    client: "src/client.js",
    worker: "src/worker.js",
    "llm-client": "src/llm-client.js",
    "llm-worker": "src/llm-worker.js",
  },
  outdir: outDir,
  bundle: true,
  format: "esm",
  platform: "browser",
  target: "chrome120",
  // Makes onnxruntime-web resolve to the build that loads its .mjs/.wasm from
  // wasmPaths instead of the one with the factory inlined as a string.
  conditions: ["onnxruntime-web-use-extern-wasm"],
  minify: true,
  legalComments: "none",
  logLevel: "info",
});

for (const file of ORT_FILES) {
  await copyFile(path.join(ortDist, file), path.join(outDir, "ort", file));
}

// Transformers.js carries a CDN default for the runtime files. worker.js
// overrides it, but an MV3 package should not even contain the URL.
for (const name of ["worker.js", "llm-worker.js"]) {
  const workerPath = path.join(outDir, name);
  const worker = await readFile(workerPath, "utf8");
  await writeFile(workerPath, worker.replaceAll("https://cdn.jsdelivr.net/npm/", "./ort/unused/"));
}

// Fail the build if anything that could load remote code is left.
const REMOTE_CODE = [
  /cdn\.jsdelivr\.net/, /unpkg\.com/, /cdnjs\.cloudflare\.com/, /esm\.sh/,
  /importScripts\s*\(\s*["'`]https?:/, /import\s*\(\s*["'`]https?:/,
  /https?:\/\/[^"'`\s]+\.(?:m?js|wasm)(?=["'`?#])/,
];
const problems = [];
let total = 0;
for (const entry of await readdir(outDir, { recursive: true, withFileTypes: true })) {
  if (!entry.isFile()) continue;
  const file = path.join(entry.parentPath, entry.name);
  const size = (await stat(file)).size;
  total += size;
  console.log(`  ${path.relative(outDir, file).padEnd(44)} ${(size / 1e6).toFixed(2)} MB`);
  if (/\.m?js$/.test(entry.name)) {
    const text = await readFile(file, "utf8");
    for (const pattern of REMOTE_CODE) {
      const match = text.match(pattern);
      if (match) problems.push(`${entry.name}: ${match[0]}`);
    }
  }
}
console.log(`  total ${(total / 1e6).toFixed(2)} MB -> ${outDir}`);
if (problems.length) {
  console.error("Remote code references found:\n  " + problems.join("\n  "));
  process.exit(1);
}
