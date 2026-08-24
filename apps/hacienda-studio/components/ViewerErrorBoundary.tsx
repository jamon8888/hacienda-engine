/**
 * Error boundary for the lazily-loaded docx/xlsx/pptx viewers in `pages/DocumentDetail.tsx`.
 *
 * Those viewers are `React.lazy`, which converts a module-evaluation failure into a
 * rejected import — and a rejected lazy import, or a render-time throw inside one of the
 * `@extend-ai/react-*` packages, propagates up and unmounts the whole `DocumentDetail`
 * tree. That takes the redacted output, the findings panel and the audit tab down with it,
 * over a *preview* of the source file the user does not strictly need. Lazy loading alone
 * moved the blast radius off first-app-load (see that file's import comment); this keeps
 * the failure inside the viewer pane itself.
 *
 * A class component because React has no hook equivalent — `componentDidCatch` /
 * `getDerivedStateFromError` are the only error-boundary API.
 */
import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  /** Shown in place of the viewer. Kept generic — the user's recourse is the Redacted or
   * Source tab, not a stack trace. */
  fileName?: string;
}

interface State {
  failed: boolean;
}

export class ViewerErrorBoundary extends Component<Props, State> {
  state: State = { failed: false };

  static getDerivedStateFromError(): State {
    return { failed: true };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // Logged rather than surfaced: the viewer packages throw opaque internal errors that
    // mean nothing to the user, but they are exactly what a developer needs when someone
    // reports "the preview is blank for this one file".
    console.error("[ViewerErrorBoundary] document viewer failed to render:", error, info);
  }

  render(): ReactNode {
    if (this.state.failed) {
      return (
        <div className="flex h-full min-h-[400px] flex-col items-center justify-center gap-2 p-6 text-center">
          <p className="text-sm font-medium text-foreground">
            This file&rsquo;s preview could not be displayed.
          </p>
          <p className="max-w-sm text-xs text-muted-foreground">
            The extracted text, detected PII and audit trail are unaffected — switch to the
            Redacted or Findings tab to keep working with
            {this.props.fileName ? ` ${this.props.fileName}` : " this document"}.
          </p>
        </div>
      );
    }
    return this.props.children;
  }
}
