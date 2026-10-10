import { VectorIndex } from "../embed/client.js";
import { loadEmbedder } from "./model.js";

const RESULT_COUNT = 8;
const BATCH_LINES = 16;
// Lines with less text than this (blank lines, bare prompts) are not indexed.
const MIN_LINE_CHARS = 8;
const LOADED_KEY = "embedLoaded";

const panel = document.getElementById("search");
const toggleButton = document.getElementById("search-toggle");
const loadButton = document.getElementById("search-load");
const statusLabel = document.getElementById("search-status");
const form = document.getElementById("search-form");
const queryInput = document.getElementById("search-query");
const resultList = document.getElementById("search-results");

/**
 * Search by meaning over what the device has printed: every received line is
 * embedded on the GPU, and a query in any language is matched against them.
 *
 * `terminal.reveal(text)` scrolls the terminal to a line; it returns false
 * when the line is no longer in the scrollback.
 *
 * Returns `{ addText }`, to be fed the plain text the device sends.
 */
export function initSearch(terminal) {
  // One entry per distinct line; a repeated line only bumps its count.
  const lines = [];
  const byText = new Map();
  const queue = [];
  let partial = "";
  let embedder = null;
  let index = null;
  let indexing = false;

  function showStatus(text, isError = false) {
    statusLabel.textContent = text;
    statusLabel.classList.toggle("error", isError);
  }

  function showProgress() {
    if (!embedder) return;
    const done = lines.length - queue.length;
    showStatus(
      queue.length || indexing
        ? `Indexing ${done} of ${lines.length} lines…`
        : `${lines.length} lines indexed on ${embedder.device}`,
    );
  }

  function addLine(raw) {
    const text = raw.trim();
    if (text.length < MIN_LINE_CHARS) return;
    const known = byText.get(text);
    if (known !== undefined) {
      lines[known].count += 1;
      return;
    }
    byText.set(text, lines.length);
    queue.push(lines.length);
    lines.push({ text, count: 1 });
  }

  async function indexQueued() {
    if (indexing || !embedder) return;
    indexing = true;
    try {
      while (queue.length) {
        const ids = queue.slice(0, BATCH_LINES);
        const vectors = await embedder.embed(ids.map((id) => lines[id].text));
        ids.forEach((id, i) => index.add(id, vectors[i]));
        queue.splice(0, ids.length);
        showProgress();
      }
    } catch (error) {
      showStatus(`indexing failed: ${error.message}`, true);
    } finally {
      indexing = false;
    }
    showProgress();
  }

  async function load() {
    if (embedder) return;
    loadButton.disabled = true;
    try {
      embedder = await loadEmbedder(showStatus);
    } catch (error) {
      showStatus(error.message, true);
      loadButton.disabled = false;
      return;
    }
    index = new VectorIndex(embedder.dimensions);
    localStorage.setItem(LOADED_KEY, "1");
    loadButton.hidden = true;
    queryInput.disabled = false;
    queryInput.focus();
    showProgress();
    await indexQueued();
  }

  function showResults(hits) {
    resultList.replaceChildren();
    if (!hits.length) {
      resultList.textContent = "Nothing indexed yet. Connect and let the device print something.";
      return;
    }
    for (const { id, score } of hits) {
      const { text, count } = lines[id];
      const row = document.createElement("button");
      row.className = "hit";
      row.title = "Scroll the terminal to this line";
      const meta = document.createElement("span");
      meta.className = "score";
      meta.textContent = score.toFixed(2) + (count > 1 ? ` ×${count}` : "");
      const line = document.createElement("span");
      line.className = "line";
      line.textContent = text;
      row.append(meta, line);
      row.addEventListener("click", () => {
        if (!terminal.reveal(text)) showStatus("That line has left the scrollback");
      });
      resultList.append(row);
    }
  }

  async function search() {
    const query = queryInput.value.trim();
    if (!query || !embedder) return;
    try {
      // Lines that arrived just before the query should be searchable too.
      await indexQueued();
      const started = performance.now();
      const vector = await embedder.embed(query, { kind: "query" });
      showResults(index.search(vector, RESULT_COUNT));
      showStatus(
        `${lines.length} lines searched in ${Math.round(performance.now() - started)} ms on ${embedder.device}`,
      );
    } catch (error) {
      showStatus(`search failed: ${error.message}`, true);
    }
  }

  toggleButton.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
    // The first load downloads the model, so it only starts unasked once the
    // user has loaded it before.
    if (!panel.hidden && !embedder && localStorage.getItem(LOADED_KEY)) load();
  });
  loadButton.addEventListener("click", load);
  form.addEventListener("submit", (event) => {
    event.preventDefault();
    search();
  });

  return {
    addText(text) {
      const parts = (partial + text).split("\n");
      partial = parts.pop();
      parts.forEach(addLine);
      if (parts.length) indexQueued();
    },
  };
}
