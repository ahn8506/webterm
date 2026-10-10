import { importBuilt } from "../search/model.js";

let loading = null;

/**
 * The page's one language model; the first call starts loading it (a download
 * of about 490 MB the first time ever). `onStatus` gets the progress text.
 */
export function loadLlm(onStatus) {
  loading ??= importBuilt("llm-client.js")
    .then(({ Llm }) =>
      Llm.create({
        onProgress({ file, loaded, total }) {
          onStatus?.(
            file && total
              ? `Loading language model ${file}: ${Math.round((100 * loaded) / total)}%`
              : "Loading language model…",
          );
        },
      }),
    )
    .catch((error) => {
      // Let a later call try again.
      loading = null;
      throw error;
    });
  return loading;
}

const SYSTEM =
  "You turn a request into one command for a device console. " +
  "Use only a command from the list. Replace <value> with the value the request gives. " +
  "Reply with the command alone, nothing else. If no listed command does what is asked, reply NONE.";

/**
 * Asks the model which of `candidates` (catalog entries) the sentence means.
 * Returns the model's command text, unchecked, or null for "none of them".
 */
export async function pickCommand(llm, candidates, sentence) {
  const list = candidates
    .map((entry) => `${entry.command.replace("{}", "<value>")}  # ${entry.say.join(" / ")}`)
    .join("\n");
  const reply = await llm.generate(
    [
      { role: "system", content: SYSTEM },
      { role: "user", content: `Commands:\n${list}\n\nRequest: ${sentence}` },
    ],
    { maxTokens: 32 },
  );
  const line = reply.trim().split("\n")[0].replace(/^`+|`+$/g, "").trim();
  return !line || /^none\b/i.test(line) ? null : line;
}
