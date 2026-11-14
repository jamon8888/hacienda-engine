/**
 * The only renderer-to-desktop operation that can disclose an original is the
 * capability-gated `open-original` command.  Keep this contract deliberately
 * small: a renderer may pass opaque references, never a File, bytes, markdown,
 * or a finding value.
 */

export const OPEN_ORIGINAL_COMMAND = "open-original" as const;

export interface OpenOriginalReferences {
  documentRef: string;
  versionRef: string;
  originalRef: string;
  viewerRef: string;
  capabilityRef: string;
}

/** Wire shape consumed by Tauri's `open-original` command. */
export interface OpenOriginalInvokePayload {
  document_ref: string;
  version_ref: string;
  original_ref: string;
  viewer_ref: string;
  capability_ref: string;
}

export type RendererInvoke = (
  command: typeof OPEN_ORIGINAL_COMMAND,
  payload: OpenOriginalInvokePayload,
) => Promise<unknown>;

export interface DesktopRendererBridge {
  readonly canOpenOriginal: boolean;
  openOriginal(references: OpenOriginalReferences): Promise<void>;
}

export class RendererBridgeUnavailableError extends Error {
  constructor() {
    super("Desktop original viewer is unavailable");
    this.name = "RendererBridgeUnavailableError";
  }
}

export function toOpenOriginalInvokePayload(
  references: OpenOriginalReferences,
): OpenOriginalInvokePayload {
  return {
    document_ref: references.documentRef,
    version_ref: references.versionRef,
    original_ref: references.originalRef,
    viewer_ref: references.viewerRef,
    capability_ref: references.capabilityRef,
  };
}

export function createDesktopRendererBridge(
  invoke?: RendererInvoke,
): DesktopRendererBridge {
  if (!invoke) {
    return {
      canOpenOriginal: false,
      openOriginal: async () => {
        throw new RendererBridgeUnavailableError();
      },
    };
  }

  return {
    canOpenOriginal: true,
    async openOriginal(references) {
      await invoke(
        OPEN_ORIGINAL_COMMAND,
        toOpenOriginalInvokePayload(references),
      );
    },
  };
}

interface TauriInternals {
  invoke?: RendererInvoke;
}

interface RendererGlobals {
  __TAURI_INTERNALS__?: TauriInternals;
}

/** Resolve Tauri v2's injected invoke function without requiring a web build dependency. */
export function createRendererBridge(
  globals: RendererGlobals = globalThis as RendererGlobals,
): DesktopRendererBridge {
  return createDesktopRendererBridge(globals.__TAURI_INTERNALS__?.invoke);
}
