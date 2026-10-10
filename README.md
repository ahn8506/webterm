# WebTerm

A serial terminal that runs in the browser as a Chrome extension. It does not
use the Web Serial API: a small native program opens the COM port, so cables
with vendor-specific drivers work too.

- **Terminal:** [alacritty_terminal](https://github.com/alacritty/alacritty)
  compiled to WebAssembly, drawn with WebGPU.
- **Serial port:** a Rust native-messaging host (`host/`).
- **Test panel:** drives a device's web UI by script injection and checks the
  result on its serial console, in both directions (`extension/tests/`).
- **Search:** finds lines in the device output by meaning, with
  EmbeddingGemma 2 running on WebGPU in the browser.
- **Ask bar:** turns a sentence into a command from a catalog
  (`extension/ask/catalog.js`), optionally with a small language model
  (Qwen3.5 0.8B).

Status: an experiment. Tried on Windows with Chrome only.

## Layout

| Path | Contents |
|---|---|
| `extension/` | The unpacked extension, including the built terminal WebAssembly |
| `host/` | Native messaging host (Rust) |
| `install/` | Registers the host for Chrome and Edge (PowerShell) |
| `web/core`, `web/frame`, `web/renderer`, `web/app` | Terminal core, renderer and their WebAssembly glue (Rust) |
| `web/embed` | Embedding and language model workers (npm) |
| `demo/` | Mock device web UI for trying things without hardware |

## Run

Requires Windows, Chrome 140 or later, Rust and Node.js.

```powershell
cd host; cargo build --release; cd ..
.\install\install.ps1
cd web\embed; npm install; npm run build; cd ..\..
```

The last line builds `extension\embed` (the model workers and ONNX Runtime),
which is not in the repository. The terminal works without it; search and the
ask bar do not.

Then open `chrome://extensions`, enable Developer mode, choose "Load unpacked"
and select the `extension` folder. Click the extension's icon to open the
terminal.

To try it without a device:

```powershell
node demo\device-server.mjs
```

Pick "Demo device (mock)" as the port, and use `http://127.0.0.1:8791` as the
device URL in the test panel.

## Rebuild the terminal WebAssembly

```powershell
.\web\app\build.ps1   # -> extension\wasm
```

The models themselves are not in the repository; the browser downloads them
from Hugging Face on first use.

## Third-party code

`web/core/vendor/alacritty_terminal` is a trimmed copy of alacritty_terminal,
under the Apache License 2.0 (see the license file in that folder).
