import { DEMO_LOG } from "./log.js";

/** Port name the terminal offers for the mock device. */
export const DEMO_PORT = "demo:";

const SERVER = "http://127.0.0.1:8791";
const GREEN = "\x1b[32m";
const RED = "\x1b[31m";
const BOLD = "\x1b[1m";
const RESET = "\x1b[0m";

const SHELL = {
  ls: "boot.cfg  crash.log  startup-config  switch-os-1.0.0.bin\r\n",
  pwd: "/flash\r\n",
  "df -h": "Filesystem  Size  Used  Avail  Use%  Mounted on\r\n/dev/flash  1.0G  612M   412M   60%  /flash\r\n",
  uptime: " 17:00:00 up 9:00,  1 user,  load average: 0.08, 0.03, 0.01\r\n",
  date: "Sat Oct 10 17:00:00 UTC 2026\r\n",
};

/**
 * Stand-in for the native serial host: a mock device console for trying the
 * terminal and the test panel without hardware. Same methods and events as
 * SerialHost. Its `show` commands report the settings held by
 * demo/device-server.mjs, so changes made in that web UI show up here.
 */
export class DemoDevice extends EventTarget {
  #open = false;
  #line = "";
  #hostname = "demo-switch";
  #encoder = new TextEncoder();
  #decoder = new TextDecoder();

  #emit(type, detail) {
    queueMicrotask(() => this.dispatchEvent(new CustomEvent(type, { detail })));
  }

  #print(text) {
    if (this.#open) this.#emit("data", this.#encoder.encode(text));
  }

  #prompt() {
    this.#print(`${BOLD}${this.#hostname}#${RESET} `);
  }

  open(path) {
    this.#open = true;
    this.#line = "";
    this.#emit("opened", { path });
    this.#print("\r\nMock device console. Type 'help' for commands.\r\n\r\n");
    this.#prompt();
  }

  close() {
    if (!this.#open) return;
    this.#open = false;
    this.#emit("closed", { reason: null });
  }

  setSignals() {}

  write(bytes) {
    if (!this.#open) return;
    for (const ch of this.#decoder.decode(bytes, { stream: true })) {
      if (ch === "\r") {
        const command = this.#line.trim();
        this.#line = "";
        this.#print("\r\n");
        this.#run(command);
      } else if (ch === "\x7f" || ch === "\b") {
        if (this.#line) {
          this.#line = this.#line.slice(0, -1);
          this.#print("\b \b");
        }
      } else if (ch >= " ") {
        this.#line += ch;
        this.#print(ch);
      }
    }
  }

  async #run(command) {
    if (command) this.#print(await this.#respond(command));
    this.#prompt();
  }

  async #respond(command) {
    if (command === "help") {
      return (
        "  show vlan\r\n  show version\r\n  show running-config\r\n  show log\r\n" +
        "  set vlan <1-4094>\r\n  set mode <access|trunk>\r\n" +
        "  set enabled <on|off>\r\n  set hostname <name>\r\n"
      );
    }
    if (command === "show version") {
      return "Mock Switch OS 1.0.0 (demo)\r\nUptime: 0 days, 1 hour\r\n";
    }
    // A few shell commands, so the ask bar's "Linux shell" catalog has something to hit.
    if (command in SHELL) return SHELL[command];
    if (command === "show log") return DEMO_LOG.map((line) => `${line}\r\n`).join("");
    const set = command.match(/^set (vlan|mode|enabled|hostname) (\S+)$/);
    if (!set && command !== "show vlan" && command !== "show running-config") {
      return `${RED}% Unknown command: ${command}${RESET}\r\n`;
    }

    let state;
    try {
      const url = set
        ? `${SERVER}/api/set?${set[1]}=${encodeURIComponent(set[2])}`
        : `${SERVER}/api/state`;
      state = await (await fetch(url, { cache: "no-store" })).json();
    } catch {
      return `${RED}% Mock web server is not running (node demo/device-server.mjs)${RESET}\r\n`;
    }
    this.#hostname = state.hostname;
    if (set) return "";
    const status = state.enabled ? `${GREEN}active${RESET}` : `${RED}shutdown${RESET}`;
    if (command === "show vlan") {
      return `VLAN ${state.vlan}   ${status}   mode ${state.mode}\r\n`;
    }
    return (
      `hostname ${state.hostname}\r\n` +
      `interface vlan ${state.vlan}\r\n` +
      ` switchport mode ${state.mode}\r\n` +
      (state.enabled ? "" : " shutdown\r\n") +
      "end\r\n"
    );
  }
}
