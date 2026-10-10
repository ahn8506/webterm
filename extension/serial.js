const HOST_NAME = "com.webterm.serial";

/**
 * Connection to the native serial host. Events: "ports", "opened", "data"
 * (detail: Uint8Array), "closed", "error", and "disconnect" when the host
 * process itself goes away.
 */
export class SerialHost extends EventTarget {
  #port = null;

  #ensure() {
    if (this.#port) return this.#port;
    const port = chrome.runtime.connectNative(HOST_NAME);
    port.onMessage.addListener((msg) => this.#dispatch(msg));
    port.onDisconnect.addListener(() => {
      this.#port = null;
      const message = chrome.runtime.lastError?.message ?? "host exited";
      this.dispatchEvent(new CustomEvent("disconnect", { detail: message }));
    });
    this.#port = port;
    return port;
  }

  #dispatch(msg) {
    const detail =
      msg.type === "data" ? Uint8Array.fromBase64(msg.data) : msg;
    this.dispatchEvent(new CustomEvent(msg.type, { detail }));
  }

  list() {
    this.#ensure().postMessage({ type: "list" });
  }

  open(path, options = {}) {
    this.#ensure().postMessage({ type: "open", path, ...options });
  }

  write(bytes) {
    this.#ensure().postMessage({ type: "write", data: bytes.toBase64() });
  }

  setSignals(signals) {
    this.#ensure().postMessage({ type: "signals", ...signals });
  }

  close() {
    this.#port?.postMessage({ type: "close" });
  }
}
