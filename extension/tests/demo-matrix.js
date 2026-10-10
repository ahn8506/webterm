// Table-driven consistency test for the mock device. Each row of `settings`
// is checked for every value in both directions (web>term and term>web), so
// covering another setting means adding a row, not writing a script.
//
// To test a real device, copy this file and replace login, prompt and the rows.
// Run it against the mock with `node demo/device-server.mjs`, the
// "Demo device (mock)" port and http://127.0.0.1:8791.

import { runMatrix } from "../automation/matrix.js";

const device = {
  async login(web) {
    await web.goto("/login");
    await web.fill("label=Username", "admin");
    await web.fill("label=Password", "admin");
    await web.click("text=Log in");
  },

  // What the console prints when it is ready for the next command.
  prompt: /# $/,

  settings: [
    {
      name: "VLAN ID",
      page: "/network",
      field: "label=VLAN ID",
      saved: "Settings saved",
      show: "show vlan",
      read: /VLAN (\d+)/,
      set: (value) => `set vlan ${value}`,
      values: ["100", "200", "4094"],
    },
    {
      name: "Mode",
      page: "/network",
      field: "label=Mode",
      type: "select",
      saved: "Settings saved",
      show: "show vlan",
      read: /mode (\w+)/,
      set: (value) => `set mode ${value}`,
      values: ["trunk", "access"],
    },
    {
      // The two sides spell this differently: a checkbox on the web,
      // active/shutdown on the console.
      name: "Enabled",
      page: "/network",
      field: "label=Enabled",
      type: "checkbox",
      saved: "Settings saved",
      show: "show vlan",
      read: /VLAN \d+\s+(active|shutdown)/,
      set: (value) => `set enabled ${value === "active" ? "on" : "off"}`,
      values: [
        { web: false, serial: "shutdown" },
        { web: true, serial: "active" },
      ],
    },
    {
      name: "Hostname",
      page: "/network",
      field: "label=Hostname",
      saved: "Settings saved",
      show: "show running-config",
      read: /hostname (\S+)/,
      set: (value) => `set hostname ${value}`,
      values: ["lab-sw-01", "demo-switch"],
    },
  ],
};

export default (context) => runMatrix(context, device);
