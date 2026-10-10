import { WebPage } from "./web.js";

const TESTS_DIR = "tests";
const SLOW_MS = 700;

const panel = document.getElementById("tests");
const toggleButton = document.getElementById("tests-toggle");
const testSelect = document.getElementById("test-file");
const urlInput = document.getElementById("device-url");
const runButton = document.getElementById("test-run");
const slowCheckbox = document.getElementById("test-slow");
const logView = document.getElementById("test-log");

/** Names of the .js files in the extension's tests folder. */
function listTests() {
  return new Promise((resolve) => {
    chrome.runtime.getPackageDirectoryEntry((root) => {
      root.getDirectory(
        TESTS_DIR,
        {},
        (dir) =>
          dir.createReader().readEntries((entries) =>
            resolve(
              entries
                .filter((entry) => entry.isFile && entry.name.endsWith(".js"))
                .map((entry) => entry.name)
                .sort(),
            ),
          ),
        () => resolve([]),
      );
    });
  });
}

async function refreshTests() {
  const previous = testSelect.value;
  const names = await listTests();
  testSelect.replaceChildren(...names.map((name) => new Option(name, name)));
  if (names.includes(previous)) testSelect.value = previous;
  runButton.disabled = names.length === 0;

  // Both usually mean the page is newer than the loaded extension: Chrome
  // only picks up manifest permissions and new folders on an extension reload.
  if (!chrome.scripting) {
    runButton.disabled = true;
    log("The extension must be reloaded: open chrome://extensions and press the reload button on WebTerm Serial, then reopen this tab.", "fail");
  } else if (names.length === 0) {
    log(`No .js files found in the extension's ${TESTS_DIR} folder. If the folder exists, reload the extension at chrome://extensions.`, "fail");
  }
}

function log(text, kind = "") {
  const line = document.createElement("div");
  if (kind) line.className = kind;
  line.textContent = `${new Date().toLocaleTimeString("en-GB")}  ${text}`;
  logView.append(line);
  logView.scrollTop = logView.scrollHeight;
}

/**
 * Wires up the test panel. `terminal` supplies `capture` (a TermCapture),
 * `send(text)` which throws when no port is open, and `screenText()`.
 */
export function initRunner(terminal) {
  let controller = null;
  // Device window kept open after a failed run, closed when the next run starts.
  let leftoverTabId = null;

  async function run() {
    const baseUrl = urlInput.value.trim();
    localStorage.setItem("deviceUrl", baseUrl);
    logView.replaceChildren();

    // Must come first: the permission prompt needs the click's user gesture.
    if (baseUrl) {
      let origin;
      try {
        origin = `${new URL(baseUrl).origin}/*`;
      } catch {
        log(`Device URL is not a valid address: ${baseUrl}`, "fail");
        return;
      }
      if (!(await chrome.permissions.request({ origins: [origin] }))) {
        log(`Access to ${origin} was not granted`, "fail");
        return;
      }
    }

    if (leftoverTabId !== null) {
      await chrome.tabs.remove(leftoverTabId).catch(() => {});
      leftoverTabId = null;
    }

    controller = new AbortController();
    const { signal } = controller;
    runButton.textContent = "Stop";

    const web = new WebPage({ baseUrl, signal, slowMo: slowCheckbox.checked ? SLOW_MS : 0 });
    const term = {
      /** Sends text to the port; later `expect` calls only see output from here on. */
      async send(text) {
        // Marked before any pause, so a following expect() never sees older output.
        terminal.capture.mark();
        if (slowCheckbox.checked) await context.sleep(SLOW_MS);
        terminal.send(text);
      },
      expect: (pattern, options) => terminal.capture.expect(pattern, { ...options, signal }),
      /** Ignore everything received so far. */
      clear: () => terminal.capture.mark(),
      /** Scrollback plus screen as plain text, one grid row per line. */
      screen: () => terminal.screenText(),
    };
    const context = {
      web,
      term,
      baseUrl,
      log: (text) => log(String(text)),
      sleep: (ms) =>
        new Promise((resolve, reject) => {
          const timer = setTimeout(resolve, ms);
          signal.addEventListener("abort", () => {
            clearTimeout(timer);
            reject(new Error("stopped"));
          });
        }),
      /** Runs a named part of the test and logs how it went. */
      async step(name, body) {
        const started = performance.now();
        try {
          const result = await body();
          log(`✓ ${name} (${Math.round(performance.now() - started)} ms)`, "pass");
          return result;
        } catch (error) {
          log(`✗ ${name}`, "fail");
          throw error;
        }
      },
    };

    const name = testSelect.value;
    log(`Running ${name}`);
    let passed = false;
    try {
      // The query string defeats the module cache so edits apply without a reload.
      const module = await import(chrome.runtime.getURL(`${TESTS_DIR}/${name}`) + `?t=${Date.now()}`);
      if (typeof module.default !== "function") {
        throw new Error(`${name} has no default export function`);
      }
      await module.default(context);
      passed = true;
      log("PASSED", "pass");
    } catch (error) {
      log(error?.message ?? String(error), "fail");
      log(signal.aborted ? "STOPPED" : "FAILED", "fail");
    } finally {
      // Leave the device window up after a failure so its state can be inspected.
      if (!passed) leftoverTabId = web.tabId;
      await web.close({ closeWindow: passed });
      controller = null;
      runButton.textContent = "Run";
    }
  }

  toggleButton.addEventListener("click", () => {
    panel.hidden = !panel.hidden;
    if (!panel.hidden) refreshTests();
  });
  runButton.addEventListener("click", () => (controller ? controller.abort() : run()));
  urlInput.value = localStorage.getItem("deviceUrl") ?? "";
}
