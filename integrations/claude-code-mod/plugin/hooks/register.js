import { installRenderer } from '../renderer.js';
import { createModProjection } from '../mod-adapter.js';

export function register(on) {
  let bridge;
  let starting;
  const rendererBridge = Object.freeze({
    project(text) {
      return bridge?.project(text) ?? [{ text: String(text ?? ''), revealed: false }];
    },
  });

  // Initialize during prompt.submit, before Claude Code dispatches the user
  // message, so Hacienda has the projection session before redaction begins.
  on('prompt.submit', async ($, event, next) => {
    if (!bridge?.isActive?.()) {
      if (!starting) {
        starting = createModProjection($)
          .then(value => { bridge = value; })
          .catch(() => { bridge = undefined; })
          .finally(() => { starting = undefined; });
      }
      await starting;
    }
    return next(event);
  }).catch(async ($, event, next) => {
    // Projection setup is optional; a hook/runtime error still lets the
    // pseudonymized prompt continue instead of blocking the user's turn.
    return next(event);
  });

  // Bounded one-shot poll before each assistant message is drawn. The server
  // atomically drains at most 32 mappings, avoiding a streaming response body.
  on('ui.render', { component: 'AssistantMessage' }, async ($, event, next) => {
    await bridge?.poll?.();
    return next(event);
  });

  // Registered last so the poll hook runs before projection/rendering.
  installRenderer(on, rendererBridge);
}
