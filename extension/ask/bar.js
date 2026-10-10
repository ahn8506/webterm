import { loadEmbedder } from "../search/model.js";
import { CATALOGS } from "./catalog.js";
import { loadLlm, pickCommand } from "./llm.js";

const CATALOG_KEY = "askCatalog";
const LLM_KEY = "askLlm";
const CHOICES = 3;
// How many of the closest commands the language model chooses among.
const LLM_CANDIDATES = 6;
// The best match is only sent unasked when it beats the runner-up by this much.
const MIN_MARGIN = 0.04;

const form = document.getElementById("ask");
const input = document.getElementById("ask-input");
const catalogSelect = document.getElementById("ask-catalog");
const llmCheckbox = document.getElementById("ask-llm");
const hint = document.getElementById("ask-hint");

const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** The catalog entry `command` is an instance of, if any. */
function entryFor(command, entries) {
  return entries.find((entry) => {
    const [before, after] = entry.command.split("{}");
    if (after === undefined) return entry.command === command;
    const value = entry.arg?.source ?? "\\S+";
    return new RegExp(`^${escapeRegExp(before)}(?:${value})${escapeRegExp(after)}$`).test(command);
  });
}

function dot(a, b) {
  let sum = 0;
  for (let i = 0; i < a.length; i++) sum += a[i] * b[i];
  return sum;
}

/**
 * The bar under the terminal: a sentence typed there is matched by meaning
 * against the chosen catalog and the closest command is sent to the device.
 *
 * `terminal.send(text)` writes to the port and throws when none is connected.
 */
export function initAsk(terminal) {
  // Catalog name -> one unit vector per phrase, each tagged with its entry.
  const embedded = new Map();
  // A command that changes the device, waiting for a second Enter.
  let pending = null;

  function show(text, isError = false) {
    hint.textContent = text;
    hint.classList.toggle("error", isError);
  }

  async function phrasesOf(name, embedder) {
    if (!embedded.has(name)) {
      const phrases = CATALOGS[name].flatMap((entry) => entry.say.map((text) => ({ entry, text })));
      // Phrases and the typed sentence are the same kind of text, so both get
      // the same prefix.
      const vectors = await embedder.embed(phrases.map((phrase) => phrase.text), { kind: "query" });
      phrases.forEach((phrase, i) => (phrase.vector = vectors[i]));
      embedded.set(name, phrases);
    }
    return embedded.get(name);
  }

  /** Catalog entries ordered by how close their best phrase is to `sentence`. */
  async function rank(sentence) {
    const embedder = await loadEmbedder(show);
    const phrases = await phrasesOf(catalogSelect.value, embedder);
    const vector = await embedder.embed(sentence, { kind: "query" });
    const best = new Map();
    for (const { entry, vector: phrase } of phrases) {
      const score = dot(vector, phrase);
      if (score > (best.get(entry) ?? -Infinity)) best.set(entry, score);
    }
    return [...best].map(([entry, score]) => ({ entry, score })).sort((a, b) => b.score - a.score);
  }

  function transmit(command) {
    terminal.send(`${command}\r`);
    input.value = "";
  }

  async function submit() {
    const sentence = input.value.trim();
    if (!sentence) return;
    if (pending && pending.sentence === sentence) {
      const { command } = pending;
      pending = null;
      transmit(command);
      show(`Sent: ${command}`);
      return;
    }
    pending = null;
    // Something that already is a command goes out as typed.
    if (CATALOGS[catalogSelect.value].some((entry) => entry.command === sentence)) {
      transmit(sentence);
      show(`Sent: ${sentence}`);
      return;
    }

    const ranked = await rank(sentence);
    if (llmCheckbox.checked) return submitWithLlm(sentence, ranked);
    const { entry, score } = ranked[0];
    const tail = `(${score.toFixed(2)}, next ${ranked[1].score.toFixed(2)})`;

    if (score - ranked[1].score < MIN_MARGIN) {
      // Too close to call: sending the wrong command is worse than asking.
      offer("Not sure which one you mean; pick one: ", ranked, sentence);
      return;
    }

    const command = fill(entry, sentence);
    if (command === null) {
      // Leave the command in the box for the user to complete.
      input.value = entry.command.replace("{}", "");
      show(`Closest is "${entry.command}" but no value was given; complete it and press Enter ${tail}`);
      return;
    }
    if (entry.confirm) {
      pending = { sentence, command };
      show(`Press Enter again to send: ${command}  ${tail}`);
      return;
    }
    transmit(command);
    show(`Sent: ${command}  ${tail}`);
  }

  function offer(message, ranked, sentence) {
    show(message);
    for (const { entry, score } of ranked.slice(0, CHOICES)) {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = `${fill(entry, sentence) ?? entry.command}  ${score.toFixed(2)}`;
      button.addEventListener("click", () => choose(entry, sentence));
      hint.append(button);
    }
  }

  // The embedding narrows the catalog to a few commands; the language model
  // picks one and fills in its value. Its answer is only used when it is one
  // of those commands.
  async function submitWithLlm(sentence, ranked) {
    const llm = await loadLlm(show);
    const candidates = ranked.slice(0, LLM_CANDIDATES).map(({ entry }) => entry);
    show("Thinking…");
    const started = performance.now();
    const command = await pickCommand(llm, candidates, sentence);
    const tail = `(language model, ${Math.round(performance.now() - started)} ms)`;
    const entry = command && entryFor(command, candidates);
    if (!entry) {
      const said = command ? `The model answered "${command}", which is not in the list` : "The model found no fitting command";
      offer(`${said}; pick one: `, ranked, sentence);
      return;
    }
    if (entry.confirm) {
      pending = { sentence, command };
      show(`Press Enter again to send: ${command}  ${tail}`);
      return;
    }
    transmit(command);
    show(`Sent: ${command}  ${tail}`);
  }

  /** The entry's command with its argument taken from the sentence; null when it is missing. */
  function fill(entry, sentence) {
    if (!entry.command.includes("{}")) return entry.command;
    const value = entry.arg && sentence.match(entry.arg)?.[0];
    return value ? entry.command.replace("{}", value) : null;
  }

  // Clicking a choice is as deliberate as a second Enter, so it sends at once.
  function choose(entry, sentence) {
    const command = fill(entry, sentence);
    try {
      if (command === null) {
        input.value = entry.command.replace("{}", "");
        input.focus();
        show(`Complete "${entry.command}" and press Enter`);
        return;
      }
      transmit(command);
      show(`Sent: ${command}`);
    } catch (error) {
      show(error.message, true);
    }
  }

  for (const name of Object.keys(CATALOGS)) catalogSelect.append(new Option(name, name));
  const saved = localStorage.getItem(CATALOG_KEY);
  if (saved in CATALOGS) catalogSelect.value = saved;
  catalogSelect.addEventListener("change", () => {
    localStorage.setItem(CATALOG_KEY, catalogSelect.value);
    pending = null;
  });

  llmCheckbox.checked = localStorage.getItem(LLM_KEY) === "1";
  llmCheckbox.addEventListener("change", () => {
    localStorage.setItem(LLM_KEY, llmCheckbox.checked ? "1" : "0");
    pending = null;
  });

  let busy = false;
  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (busy) return;
    busy = true;
    try {
      await submit();
    } catch (error) {
      show(error.message, true);
    } finally {
      busy = false;
    }
  });
}
