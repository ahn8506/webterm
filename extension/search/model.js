// extension/embed is build output that a fresh checkout does not have, so it
// is only imported when a model is first asked for.
export async function importBuilt(file) {
  try {
    return await import(`../embed/${file}`);
  } catch {
    throw new Error("extension/embed is missing; run npm install and npm run build in web/embed");
  }
}

// terminal.html?dtype=q8 loads another precision of the model, to compare.
const DTYPE = new URLSearchParams(location.search).get("dtype") ?? undefined;

let loading = null;
const listeners = new Set();

/**
 * The one embedding model shared by everything on the page. The first call
 * starts loading it (a download of about 210 MB the first time ever); every
 * caller's `onStatus` gets the progress text.
 */
export function loadEmbedder(onStatus) {
  if (onStatus) listeners.add(onStatus);
  loading ??= importBuilt("client.js")
    .then(({ Embedder }) =>
      Embedder.create({
        dtype: DTYPE,
        onProgress({ file, loaded, total }) {
          const text =
            file && total ? `Loading ${file}: ${Math.round((100 * loaded) / total)}%` : "Loading model…";
          for (const listener of listeners) listener(text);
        },
      }),
    )
    .catch((error) => {
      // Let a later call try again.
      loading = null;
      throw error;
    });
  return loading.finally(() => listeners.delete(onStatus));
}
