// External module script so the page works under
// `script-src 'self' 'wasm-unsafe-eval'`.
import init from './pkg/webterm_renderer_demo.js';

init().catch((err) => {
  const status = document.getElementById('status');
  status.dataset.state = 'error';
  status.textContent = `wasm init failed: ${err}`;
});
