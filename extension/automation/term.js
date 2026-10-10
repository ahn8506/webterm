const MAX_CAPTURE_CHARS = 1_000_000;
const TAIL_CHARS = 400;

// CSI, OSC, charset selection and single-character escapes.
const ESCAPE = /\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[@-Z\\^_=>]/g;
// An escape sequence cut off by the end of a chunk.
const PARTIAL_ESCAPE = /\x1b(?:\[[0-?]*[ -/]*|\][^\x07\x1b]*\x1b?|[()])?$/;
const OTHER_CONTROLS = /[\x00-\x08\x0b-\x1f\x7f]/g;

function describe(pattern) {
  return pattern instanceof RegExp ? String(pattern) : JSON.stringify(pattern);
}

/**
 * Plain-text record of what the device sent, with escape sequences removed,
 * for tests to wait on. Matching only looks at output after the mark, which
 * moves past each match (and can be moved to the end with `mark()`).
 */
export class TermCapture {
  #text = "";
  #mark = 0;
  #pending = "";
  #decoder = new TextDecoder("utf-8");
  #waiters = new Set();
  /** Called with each piece of plain text as it arrives. */
  onText = null;

  push(bytes) {
    let chunk = this.#pending + this.#decoder.decode(bytes, { stream: true });
    const partial = chunk.match(PARTIAL_ESCAPE);
    this.#pending = partial ? partial[0] : "";
    if (partial) chunk = chunk.slice(0, partial.index);

    const plain = chunk.replace(ESCAPE, "").replace(OTHER_CONTROLS, "");
    this.#text += plain;
    if (plain) this.onText?.(plain);
    const excess = this.#text.length - MAX_CAPTURE_CHARS;
    if (excess > 0) {
      this.#text = this.#text.slice(excess);
      this.#mark = Math.max(0, this.#mark - excess);
    }
    for (const wake of this.#waiters) wake();
  }

  /** Ignore everything received so far. */
  mark() {
    this.#mark = this.#text.length;
  }

  #match(pattern) {
    const unread = this.#text.slice(this.#mark);
    if (pattern instanceof RegExp) {
      const once = new RegExp(pattern.source, pattern.flags.replace(/[gy]/g, ""));
      const found = once.exec(unread);
      if (!found) return null;
      this.#mark += found.index + found[0].length;
      return found;
    }
    const index = unread.indexOf(pattern);
    if (index < 0) return null;
    this.#mark += index + pattern.length;
    return pattern;
  }

  /**
   * Resolves with the match (a RegExp match array, or the string) once the
   * pattern shows up in output received after the mark.
   */
  expect(pattern, { timeout = 5000, signal } = {}) {
    return new Promise((resolve, reject) => {
      const finish = (settle, value) => {
        clearTimeout(timer);
        this.#waiters.delete(check);
        signal?.removeEventListener("abort", onAbort);
        settle(value);
      };
      const check = () => {
        const found = this.#match(pattern);
        if (found !== null) finish(resolve, found);
      };
      const onAbort = () => finish(reject, new Error("stopped"));
      const timer = setTimeout(() => {
        const tail = this.#text.slice(this.#mark).slice(-TAIL_CHARS);
        finish(
          reject,
          new Error(
            `serial: no ${describe(pattern)} within ${timeout} ms. ` +
              (tail ? `Output since the last match:\n${tail}` : "Nothing was received."),
          ),
        );
      }, timeout);

      if (signal?.aborted) return onAbort();
      signal?.addEventListener("abort", onAbort);
      this.#waiters.add(check);
      check();
    });
  }
}
