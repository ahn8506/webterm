// Table-driven checks that a setting stays consistent between the device's
// web UI and its serial console, in both directions:
//
//   web>term   set the value in the web UI, read it back on the console
//   term>web   set the value on the console, read it back in the web UI
//
// A device is described once (see tests/demo-matrix.js); every row of its
// settings table then yields one check per value and direction.

const WEB_TO_TERM = "web>term";
const TERM_TO_WEB = "term>web";

/** A value is either one literal used on both sides, or `{ web, serial }`. */
function sides(value) {
  return value !== null && typeof value === "object" ? value : { web: value, serial: value };
}

function same(actual, expected) {
  return typeof expected === "boolean" ? actual === expected : String(actual) === String(expected);
}

function mismatch(where, actual, expected) {
  return new Error(`${where} shows ${JSON.stringify(actual)}, expected ${JSON.stringify(expected)}`);
}

/**
 * Orders the checks for one setting so that every value is tried in every
 * direction and no check asks for the value the device already has (which
 * would pass even if nothing worked).
 */
function plan(values, directions, current) {
  // Start from a value the device does not currently hold.
  const first = values.findIndex((value) => !same(current, sides(value).serial));
  const ordered = first > 0 ? [...values.slice(first), ...values.slice(0, first)] : values;

  const checks = [];
  for (let pass = 0; pass < directions.length; pass++) {
    ordered.forEach((value, index) => {
      checks.push({ value, direction: directions[(index + pass) % directions.length] });
    });
  }
  return checks;
}

/**
 * @param context   what a test's default export receives
 * @param device    { login(web), prompt, timeout?, settings: [...] }
 *
 * Each setting:
 *   name      label for the log
 *   page      web path holding the field
 *   field     selector of the field
 *   type      "text" (default) | "select" | "checkbox"
 *   apply     selector of the save button (default "text=Apply")
 *   saved     text the page shows once saved (optional)
 *   show      console command that prints the setting
 *   read      RegExp whose first group captures the value in that output
 *   set       (serialValue) => console command that changes it; omit for
 *             settings that can only be changed from the web
 *   values    two or more values; each a literal or { web, serial }
 */
export async function runMatrix(context, device) {
  const { web, term, step, log } = context;
  const timeout = device.timeout ?? 5000;

  const readSerial = async (setting) => {
    await term.send(`${setting.show}\r`);
    const match = await term.expect(setting.read, { timeout });
    await term.expect(device.prompt, { timeout });
    return match[1];
  };

  const setWeb = async (setting, value) => {
    await web.goto(setting.page);
    if (setting.type === "select") await web.select(setting.field, value);
    else if (setting.type === "checkbox") await web.check(setting.field, value);
    else await web.fill(setting.field, value);
    await web.click(setting.apply ?? "text=Apply");
    if (setting.saved) await web.waitForText(setting.saved);
  };

  const checks = {
    async [WEB_TO_TERM](setting, value) {
      await setWeb(setting, value.web);
      const actual = await readSerial(setting);
      if (!same(actual, value.serial)) throw mismatch("console", actual, value.serial);
      return actual;
    },
    async [TERM_TO_WEB](setting, value) {
      await term.send(`${setting.set(value.serial)}\r`);
      await term.expect(device.prompt, { timeout });
      await web.goto(setting.page);
      const actual = await web.value(setting.field);
      if (!same(actual, value.web)) throw mismatch("web page", actual, value.web);
      return value.serial;
    },
  };

  if (device.login) await step("log in", () => device.login(web));
  // Flush whatever the console printed so far and make sure it answers.
  await term.send("\r");
  await term.expect(device.prompt, { timeout });

  let total = 0;
  let inconclusive = 0;
  const failures = [];
  for (const setting of device.settings) {
    const directions = setting.set ? [WEB_TO_TERM, TERM_TO_WEB] : [WEB_TO_TERM];
    if (setting.values.length < 2) {
      log(`  ${setting.name}: needs at least two values to prove a change; skipped`);
      continue;
    }
    let current;
    try {
      current = await readSerial(setting);
    } catch (error) {
      if (error.message === "stopped") throw error;
      total += 1;
      failures.push(`${setting.name}: ${error.message}`);
      log(`✗ ${setting.name}: cannot read the current value (${error.message})`);
      continue;
    }

    for (const { value, direction } of plan(setting.values, directions, current)) {
      const pair = sides(value);
      const name = `${setting.name} = ${pair.serial}  [${direction}]`;
      total += 1;
      // After a failure the device may already hold the next value; setting
      // it again would pass without proving anything.
      if (current !== null && same(current, pair.serial)) {
        inconclusive += 1;
        log(`– ${name}: not checked, the device already holds this value`);
        continue;
      }
      try {
        current = await step(name, () => checks[direction](setting, pair));
      } catch (error) {
        if (error.message === "stopped") throw error;
        failures.push(`${name}: ${error.message}`);
        log(`    ${error.message}`);
        // Re-read so the next check knows where the device really stands.
        current = await readSerial(setting).catch(() => null);
      }
    }
  }

  const passed = total - failures.length - inconclusive;
  log(`${passed} of ${total} checks passed` + (inconclusive ? `, ${inconclusive} not checked` : ""));
  if (failures.length) throw new Error(`${failures.length} of ${total} checks failed`);
}
