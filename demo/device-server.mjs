// Mock device for trying WebTerm without hardware: a small web admin UI plus
// the state the extension's "Demo device" serial console reads back.
//
//   node demo/device-server.mjs        then use http://127.0.0.1:8791

import http from "node:http";

const PORT = 8791;
const state = { hostname: "demo-switch", vlan: 1, mode: "access", enabled: true };

/** Updates whichever settings are present in `params`. */
function applySettings(params) {
  const vlan = Number(params.get("vlan"));
  if (params.has("vlan") && Number.isInteger(vlan) && vlan >= 1 && vlan <= 4094) state.vlan = vlan;
  if (params.get("hostname")) state.hostname = params.get("hostname").slice(0, 32);
  if (params.has("mode")) state.mode = params.get("mode") === "trunk" ? "trunk" : "access";
  if (params.has("enabled")) state.enabled = params.get("enabled") === "on";
}

const escapeHtml = (text) =>
  String(text).replace(/[&<>"]/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[ch]);

const page = (title, body) => `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<title>${title} — ${escapeHtml(state.hostname)}</title>
<style>
  body { font: 15px/1.5 system-ui, sans-serif; margin: 0; background: #f3f5f8; color: #1c2430; }
  header { background: #1f3a5f; color: #fff; padding: 12px 24px; font-weight: 600; }
  main { max-width: 460px; margin: 32px auto; background: #fff; padding: 24px 28px; border-radius: 8px; box-shadow: 0 1px 4px #0002; }
  h1 { font-size: 20px; margin: 0 0 16px; }
  label { display: block; margin: 14px 0 4px; font-weight: 600; }
  input[type=text], input[type=password], select { width: 100%; box-sizing: border-box; padding: 7px 9px; font: inherit; border: 1px solid #b8c0cc; border-radius: 4px; }
  .inline { display: flex; align-items: center; gap: 8px; margin-top: 14px; }
  .inline label { margin: 0; }
  button, input[type=submit] { margin-top: 20px; padding: 8px 18px; font: inherit; color: #fff; background: #2f6fd6; border: 0; border-radius: 4px; cursor: pointer; }
  .saved { background: #e3f6e5; color: #1d6b2a; padding: 8px 12px; border-radius: 4px; margin-bottom: 12px; }
</style>
<header>${escapeHtml(state.hostname)} · Web Admin</header>
<main>${body}</main>
</html>`;

const loginPage = () =>
  page(
    "Login",
    `<h1>Login</h1>
<form method="post" action="/login">
  <label for="user">Username</label><input id="user" name="user" type="text">
  <label for="pass">Password</label><input id="pass" name="pass" type="password">
  <button id="go">Log in</button>
</form>`,
  );

const selected = (value) => (state.mode === value ? "selected" : "");

const networkPage = (saved) =>
  page(
    "Network",
    `<h1>Network</h1>
${saved ? '<div class="saved">Settings saved</div>' : ""}
<form method="get" action="/network/apply" onsubmit="return confirm('Apply network settings?')">
  <label for="hostname">Hostname</label><input id="hostname" name="hostname" type="text" value="${escapeHtml(state.hostname)}">
  <label for="vlan">VLAN ID</label><input id="vlan" name="vlan" type="text" value="${state.vlan}">
  <label for="mode">Mode</label>
  <select id="mode" name="mode">
    <option value="access" ${selected("access")}>Access</option>
    <option value="trunk" ${selected("trunk")}>Trunk</option>
  </select>
  <div class="inline"><input id="enabled" name="enabled" type="checkbox" ${state.enabled ? "checked" : ""}><label for="enabled">Enabled</label></div>
  <input type="submit" value="Apply">
</form>`,
  );

http
  .createServer((request, response) => {
    const url = new URL(request.url, `http://${request.headers.host}`);
    const send = (status, body, headers = {}) => {
      response.writeHead(status, { "content-type": "text/html; charset=utf-8", ...headers });
      response.end(body);
    };
    const redirect = (location, headers = {}) => send(302, "", { location, ...headers });
    const loggedIn = (request.headers.cookie ?? "").includes("session=1");

    // Read by the extension's demo serial console.
    if (url.pathname === "/api/state") {
      return send(200, JSON.stringify(state), {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
      });
    }
    // Written by the demo console's "set" commands.
    if (url.pathname === "/api/set") {
      applySettings(url.searchParams);
      return send(200, JSON.stringify(state), {
        "content-type": "application/json",
        "access-control-allow-origin": "*",
      });
    }
    if (url.pathname === "/login" && request.method === "POST") {
      return redirect("/network", { "set-cookie": "session=1; Path=/" });
    }
    if (url.pathname === "/login") return send(200, loginPage());
    if (!loggedIn) return redirect("/login");

    if (url.pathname === "/network/apply") {
      applySettings(url.searchParams);
      // An unchecked box is simply absent from a submitted form.
      state.enabled = url.searchParams.has("enabled");
      console.log("applied from web:", JSON.stringify(state));
      return redirect("/network?saved=1");
    }
    if (url.pathname === "/" || url.pathname === "/network") {
      return send(200, networkPage(url.searchParams.has("saved")));
    }
    send(404, page("Not found", "<h1>Not found</h1>"));
  })
  .listen(PORT, "127.0.0.1", () => {
    console.log(`Mock device web UI: http://127.0.0.1:${PORT}  (any username/password)`);
  });
