const POLL_MS = 100;
const DIALOG_SCRIPT_ID = "webterm-dialogs";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * Runs inside the device page (serialised by chrome.scripting, so it must not
 * reference anything outside its own body). Finds one visible element and
 * applies `action` to it.
 *
 * Selectors: CSS, `text=Exact visible text`, or `label=Label text` for the
 * form control that label belongs to.
 */
function pageAction(action, selector, arg) {
  const visible = (el) =>
    el.getClientRects().length > 0 && getComputedStyle(el).visibility !== "hidden";
  const textOf = (el) =>
    (el instanceof HTMLInputElement ? el.value : el.textContent).trim().replace(/\s+/g, " ");

  const byText = (candidates, wanted) => {
    let best = null;
    for (const el of candidates) {
      if (textOf(el) !== wanted || !visible(el)) continue;
      // Prefer the innermost element carrying the text.
      if (!best || best.contains(el)) best = el;
    }
    return best;
  };

  const find = () => {
    if (selector.startsWith("text=")) {
      return byText(document.querySelectorAll("body *"), selector.slice(5).trim());
    }
    if (selector.startsWith("label=")) {
      const label = byText(document.querySelectorAll("label"), selector.slice(6).trim());
      return label?.control ?? null;
    }
    for (const el of document.querySelectorAll(selector)) {
      if (visible(el)) return el;
    }
    return null;
  };

  const changed = (el) => {
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  };

  if (action === "pageText") return { found: true, value: document.body?.innerText ?? "" };

  let el;
  try {
    el = find();
  } catch (error) {
    return { found: true, error: `bad selector ${selector}: ${error.message}` };
  }
  if (!el) return { found: false };

  switch (action) {
    case "probe":
      return { found: true };
    case "highlight":
      el.scrollIntoView({ block: "center" });
      el.style.outline = "3px solid #ff3b7f";
      el.style.outlineOffset = "2px";
      return { found: true };
    case "click":
      el.scrollIntoView({ block: "center" });
      el.click();
      return { found: true };
    case "fill": {
      el.focus();
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
        // Go through the prototype setter so frameworks that wrap `value` notice.
        const proto = Object.getPrototypeOf(el);
        Object.getOwnPropertyDescriptor(proto, "value").set.call(el, arg);
      } else if (el.isContentEditable) {
        el.textContent = arg;
      } else {
        return { found: true, error: `${selector} is not a text field` };
      }
      changed(el);
      return { found: true };
    }
    case "select": {
      if (!(el instanceof HTMLSelectElement)) {
        return { found: true, error: `${selector} is not a <select>` };
      }
      const option = [...el.options].find((o) => o.value === arg || o.text.trim() === arg);
      if (!option) return { found: true, error: `${selector} has no option ${arg}` };
      el.value = option.value;
      changed(el);
      return { found: true };
    }
    case "check":
      if (el.checked !== arg) el.click();
      return { found: true };
    case "text":
      return { found: true, value: (el.innerText ?? el.textContent).trim() };
    case "value":
      return { found: true, value: el.type === "checkbox" || el.type === "radio" ? el.checked : el.value };
    case "attribute":
      return { found: true, value: el.getAttribute(arg) };
    default:
      return { found: true, error: `unknown action ${action}` };
  }
}

/**
 * A device web page driven by script injection. Every action waits for its
 * element to appear (in any frame) until the timeout, so page loads and
 * redirects between steps need no explicit waiting.
 */
export class WebPage {
  #tabId = null;
  #baseUrl;
  #signal;
  #timeout;
  #slowMo;

  /** `slowMo` (ms) pauses before each action and outlines its element, for watching a run. */
  constructor({ baseUrl = "", signal, timeout = 10_000, slowMo = 0 } = {}) {
    this.#baseUrl = baseUrl;
    this.#signal = signal;
    this.#timeout = timeout;
    this.#slowMo = slowMo;
  }

  /** Tab showing the device page, or null before the first goto(). */
  get tabId() {
    return this.#tabId;
  }

  #checkStopped() {
    if (this.#signal?.aborted) throw new Error("stopped");
  }

  async #requireAccess(url) {
    const origin = `${new URL(url).origin}/*`;
    if (!(await chrome.permissions.contains({ origins: [origin] }))) {
      throw new Error(
        `web: no access to ${new URL(url).origin}. Put that address in the Device URL field and run again.`,
      );
    }
    return origin;
  }

  /** Opens `url` (absolute, or relative to the Device URL) and waits for it to load. */
  async goto(url) {
    this.#checkStopped();
    if (!this.#baseUrl && !/^https?:/i.test(url)) {
      throw new Error(`web: ${url} is relative but the Device URL field is empty`);
    }
    const target = new URL(url, this.#baseUrl || undefined).href;
    const origin = await this.#requireAccess(target);

    if (this.#tabId === null) {
      await chrome.scripting.unregisterContentScripts({ ids: [DIALOG_SCRIPT_ID] }).catch(() => {});
      await chrome.scripting.registerContentScripts([
        {
          id: DIALOG_SCRIPT_ID,
          matches: [origin],
          js: ["automation/dialogs.js"],
          runAt: "document_start",
          world: "MAIN",
          allFrames: true,
          persistAcrossSessions: false,
        },
      ]);
      // Its own window on the right half of the screen, so the terminal stays
      // visible on the left while the test drives the page.
      const half = Math.floor(screen.availWidth / 2);
      const created = await chrome.windows.create({
        url: target,
        left: screen.availLeft + half,
        top: screen.availTop,
        width: half,
        height: screen.availHeight,
      });
      this.#tabId = created.tabs[0].id;
    } else {
      await chrome.tabs.update(this.#tabId, { url: target });
    }
    await this.waitForLoad();
  }

  async waitForLoad({ timeout = this.#timeout } = {}) {
    const deadline = Date.now() + timeout;
    // Give the navigation a moment to start before trusting "complete".
    await sleep(POLL_MS);
    for (;;) {
      this.#checkStopped();
      const tab = await chrome.tabs.get(this.#tabId);
      if (tab.status === "complete") return;
      if (Date.now() > deadline) throw new Error(`web: page did not finish loading in ${timeout} ms`);
      await sleep(POLL_MS);
    }
  }

  async #inject(target, action, selector, arg) {
    return chrome.scripting.executeScript({
      target: { tabId: this.#tabId, ...target },
      func: pageAction,
      args: [action, selector, arg ?? null],
    });
  }

  /** Polls until the element exists in some frame, then applies the action there. */
  async #act(action, selector, arg, { timeout = this.#timeout } = {}) {
    if (this.#tabId === null) throw new Error("web: call goto() first");
    const deadline = Date.now() + timeout;
    let lastError = "";
    for (;;) {
      this.#checkStopped();
      try {
        const probes = await this.#inject({ allFrames: true }, "probe", selector);
        const hit = probes.find((probe) => probe.result?.found);
        if (hit?.result.error) throw new Error(`web: ${hit.result.error}`);
        if (hit) {
          if (this.#slowMo && action !== "probe") {
            await this.#inject({ frameIds: [hit.frameId] }, "highlight", selector);
            await sleep(this.#slowMo);
          }
          const [{ result }] = await this.#inject({ frameIds: [hit.frameId] }, action, selector, arg);
          if (result?.error) throw new Error(`web: ${result.error}`);
          if (result?.found) {
            // A click often submits a form; let that navigation finish so the
            // next step (a goto in particular) does not cancel it midway.
            if (action === "click") await this.waitForLoad({ timeout }).catch(() => {});
            return result.value;
          }
        }
      } catch (error) {
        if (error.message.startsWith("web: ")) throw error;
        // The frame went away mid-call (navigation) or is not ready yet; retry.
        lastError = error.message;
      }
      if (Date.now() > deadline) {
        throw new Error(
          `web: ${selector} not found within ${timeout} ms` + (lastError ? ` (${lastError})` : ""),
        );
      }
      await sleep(POLL_MS);
    }
  }

  click(selector, options) {
    return this.#act("click", selector, null, options);
  }

  fill(selector, value, options) {
    return this.#act("fill", selector, String(value), options);
  }

  /** Chooses a `<select>` option by its value or its visible text. */
  select(selector, option, options) {
    return this.#act("select", selector, String(option), options);
  }

  check(selector, checked = true, options) {
    return this.#act("check", selector, Boolean(checked), options);
  }

  text(selector, options) {
    return this.#act("text", selector, null, options);
  }

  /** Current value of a form control (boolean for checkboxes and radios). */
  value(selector, options) {
    return this.#act("value", selector, null, options);
  }

  attribute(selector, name, options) {
    return this.#act("attribute", selector, name, options);
  }

  waitFor(selector, options) {
    return this.#act("probe", selector, null, options);
  }

  /** Waits until `text` (string or RegExp) appears anywhere on the page, in any frame. */
  async waitForText(text, { timeout = this.#timeout } = {}) {
    const deadline = Date.now() + timeout;
    for (;;) {
      this.#checkStopped();
      const frames = await this.#inject({ allFrames: true }, "pageText", "").catch(() => []);
      for (const { result } of frames) {
        const body = result?.value ?? "";
        const found = text instanceof RegExp ? body.match(text) : body.includes(text) ? text : null;
        if (found) return found;
      }
      if (Date.now() > deadline) throw new Error(`web: ${text} did not appear within ${timeout} ms`);
      await sleep(POLL_MS);
    }
  }

  /** Runs `fn(...args)` in the page's own JavaScript world and returns its result. */
  async evaluate(fn, ...args) {
    this.#checkStopped();
    const [{ result }] = await chrome.scripting.executeScript({
      target: { tabId: this.#tabId },
      world: "MAIN",
      func: fn,
      args,
    });
    return result;
  }

  /** alert/confirm/prompt calls answered automatically since the last call to this. */
  async dialogs() {
    // Frames of one origin share the tab's sessionStorage, so the top frame has them all.
    const [{ result }] = await chrome.scripting.executeScript({
      target: { tabId: this.#tabId },
      func: () => {
        const seen = JSON.parse(sessionStorage.getItem("__webtermDialogs") ?? "[]");
        sessionStorage.removeItem("__webtermDialogs");
        return seen;
      },
    });
    return result ?? [];
  }

  async url() {
    return (await chrome.tabs.get(this.#tabId)).url;
  }

  /** Stops answering dialogs; with `closeWindow` also closes the device window. */
  async close({ closeWindow = true } = {}) {
    await chrome.scripting.unregisterContentScripts({ ids: [DIALOG_SCRIPT_ID] }).catch(() => {});
    if (this.#tabId !== null && closeWindow) {
      await chrome.tabs.remove(this.#tabId).catch(() => {});
    }
    this.#tabId = null;
  }
}
