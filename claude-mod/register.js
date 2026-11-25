import { installRenderer } from './src/renderer.js';

// No bridge is created or contacted by default. A host integration may inject
// an authenticated same-device bridge after establishing a Claude session.
installRenderer(globalThis.on, globalThis.haciendaProjectionBridge);
