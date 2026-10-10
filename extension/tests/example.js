// Example test: change a setting in the device's web UI, then confirm it over
// the serial console. Copy this file, adjust the selectors and commands for
// your device, and pick the copy in the Tests panel.
//
// Before running: connect the serial port in the toolbar and enter the
// device's address (e.g. http://192.168.0.1) in the Device URL field.
//
// It runs as-is against the mock device: start `node demo/device-server.mjs`,
// connect the "Demo device (mock)" port and use http://127.0.0.1:8791.
//
//   web.goto(url)                 open a page (relative to the Device URL)
//   web.fill(sel, value)          type into a field
//   web.select(sel, option)       choose a <select> option by value or text
//   web.check(sel, true|false)    set a checkbox
//   web.click(sel)                click
//   web.text(sel) / web.value(sel)   read back
//   web.waitFor(sel) / web.waitForText(text)
//   web.dialogs()                 alert/confirm messages that were auto-accepted
//   term.send(text)               write to the serial port
//   term.expect(pattern, {timeout})  wait for output (string or RegExp)
//   term.screen()                 everything on the terminal as text
//
// Selectors are CSS, or "text=Apply" (exact visible text), or
// "label=VLAN ID" (the field belonging to that label).

export default async function ({ web, term, step, log }) {
  const vlan = "100";

  await step("log in", async () => {
    await web.goto("/login");
    await web.fill("label=Username", "admin");
    await web.fill("label=Password", "admin");
    await web.click("text=Log in");
  });

  await step("set VLAN in the web UI", async () => {
    await web.goto("/network");
    await web.fill("label=VLAN ID", vlan);
    await web.click("text=Apply");
    await web.waitForText("Settings saved");
  });

  await step("confirm on the serial console", async () => {
    await term.send("show vlan\r");
    const match = await term.expect(/VLAN (\d+)\s+active/, { timeout: 5000 });
    if (match[1] !== vlan) throw new Error(`device reports VLAN ${match[1]}, expected ${vlan}`);
    log(`device reports VLAN ${match[1]}`);
  });
}
