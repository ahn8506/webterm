// Injected into the device page's own world before its scripts run. Native
// alert/confirm/prompt dialogs block the page and cannot be answered by an
// injected script, so they are answered here and recorded for the test.
(() => {
  // sessionStorage, because a confirm() is usually followed by a navigation
  // that would wipe anything kept on `window`.
  const KEY = "__webtermDialogs";
  const record = (type, message) => {
    try {
      const seen = JSON.parse(sessionStorage.getItem(KEY) ?? "[]");
      seen.push({ type, message: String(message) });
      sessionStorage.setItem(KEY, JSON.stringify(seen));
    } catch {
      // Storage is unavailable in sandboxed frames; answering still works.
    }
  };
  window.alert = (message) => record("alert", message);
  window.confirm = (message) => {
    record("confirm", message);
    return true;
  };
  window.prompt = (message, fallback) => {
    record("prompt", message);
    return fallback ?? "";
  };
})();
