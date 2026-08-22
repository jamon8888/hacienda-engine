/**
 * Host half of @hacienda/dsh-hacienda.
 *
 * Registers the Artifacts-view RPC surface (list-workspace, read-file-text,
 * file-download-url, scan-artifacts) and the `scan_folder` model tool over
 * `ctx.fs`. This is the working dynamic-plugin code, packaged so it survives a
 * web-bundle rebuild (Approach A: bundled profile, not a fork).
 *
 * Plain CommonJS/ESM-agnostic Cordis plugin body. `harness`, `console`, `ctx`
 * are provided by the restricted Host evaluator.
 */
export default {
  name: 'hacienda-artifacts-host',
  apply(ctx) {
    const fsSvc = ctx.get('fs')
    if (fsSvc === undefined) {
      console.error('[hacienda-artifacts] ctx.fs unavailable; RPCs will fail')
    }

    // `fsSvc.resolve` only uses `cwd` as a default base for RELATIVE paths — it
    // is not a containment boundary (dsh-fs-local's own docs: "a resolution
    // default, NOT a containment boundary"), and reads are never fenced by a
    // sandboxing backend either (only writeText/editText are). An absolute path
    // or a `../` traversal from the client would otherwise resolve straight
    // through to any host file readable by the process. `contains` gives us
    // the same canonical-identity check the write-side sandbox uses, so every
    // RPC below rejects a target outside the workspace root before touching it.
    let rootTargetPromise = null
    function workspaceRoot() {
      if (!rootTargetPromise) rootTargetPromise = fsSvc.resolve('.', {})
      return rootTargetPromise
    }

    async function resolvePath(p) {
      const target = await fsSvc.resolve(p, {})
      const root = await workspaceRoot()
      if (!fsSvc.contains(root, target)) {
        const err = new Error('path escapes the workspace root')
        err.code = 'FS_OUTSIDE_WORKSPACE'
        throw err
      }
      return target
    }

    // Both `read-file-text` and `scan-artifacts` read the same file's full text
    // independently (the client fires them back-to-back for a preview), and
    // `readText` has no size cap of its own — a large file would otherwise be
    // buffered twice with no bound. One shared limit, checked via `stat` before
    // either read touches file content.
    const MAX_TEXT_READ_BYTES = 10 * 1024 * 1024
    async function readTextBounded(target) {
      const info = await fsSvc.stat(target)
      if (info && typeof info.size === 'number' && info.size > MAX_TEXT_READ_BYTES) {
        const err = new Error('file too large to preview (' + info.size + ' bytes)')
        err.code = 'FS_TOO_LARGE'
        throw err
      }
      return fsSvc.readText(target)
    }

    function entryKind(e) {
      if (e && typeof e.isDirectory === 'function') return e.isDirectory() ? 'folder' : 'file'
      if (e && typeof e.isDirectory === 'boolean') return e.isDirectory ? 'folder' : 'file'
      return 'file'
    }

    // RPC 1 — list a directory as owned JSON FileSystemItem[] (Finder-style).
    harness.handle('list-workspace', async (args) => {
      if (!fsSvc) return { error: 'ctx.fs unavailable' }
      const path = (args && args.path) || ''
      try {
        const entries = await fsSvc.listDir(await resolvePath(path), undefined)
        const items = entries.map(function (e) {
          const kind = entryKind(e)
          return {
            kind,
            path: (path ? path + '/' : '') + e.name,
            name: e.name,
            parentPath: path || '',
            hidden: !!e.hidden,
            size: kind === 'file' && typeof e.size === 'number' ? e.size : undefined,
            updatedAt: e.mtime ? String(e.mtime) : undefined,
          }
        })
        return { path, items, parentPath: path ? '' : null }
      } catch (err) {
        return { error: String((err && err.message) || err) }
      }
    })

    // RPC 2 — read a text file (CodeMirror / markdown).
    harness.handle('read-file-text', async (args) => {
      if (!fsSvc) return { error: 'ctx.fs unavailable' }
      const path = args && args.path
      if (!path) return { error: 'missing path' }
      try {
        return { path, text: await readTextBounded(await resolvePath(path)) }
      } catch (err) {
        return { error: String((err && err.message) || err) }
      }
    })

    // RPC 3 — a download URL for a large binary. Small-file data-URL now; a
    // ctx.webServer GET route replaces it for large PDF/DOCX in a later slice.
    harness.handle('file-download-url', async (args) => {
      if (!fsSvc) return { error: 'ctx.fs unavailable' }
      const path = args && args.path
      if (!path) return { error: 'missing path' }
      try {
        const bytes = await fsSvc.readBytes(await resolvePath(path), undefined, 4 * 1024 * 1024)
        let bin = ''
        for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i])
        return { path, url: 'data:application/octet-stream;base64,' + btoa(bin) }
      } catch (err) {
        return { error: String((err && err.message) || err) }
      }
    })

    // RPC 4 — PII span overlay for a file. Regex feedback tier (no P7 dep) so
    // the Artifacts preview can highlight detected PII now. The AUTHORITATIVE
    // pass (P7-corrected HaciendaFacade / GLiNER2 worker) replaces this later;
    // in-preview highlighting is the interactive feedback tier (§11.4).
    const PII_PATTERNS = [
      { category: 'email', re: /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g, conf: 0.95 },
      { category: 'iban', re: /\b[A-Z]{2}[0-9]{2}(?:[ ]?[A-Z0-9]{4}){3,7}\b/g, conf: 0.9 },
      { category: 'url', re: /https?:\/\/[^\s]+/g, conf: 0.9 },
      { category: 'phone', re: /(\+?\d{1,3}[-.\s]?)?\(?\d{3}\)?[-.\s]?\d{3}[-.\s]?\d{4}\b/g, conf: 0.8 },
      { category: 'card', re: /\b(?:4[0-9]{12}(?:[0-9]{3})?|5[1-5][0-9]{14}|3[47][0-9]{13}|6(?:011|5[0-9]{2})[0-9]{12})(?:[ ]?[0-9]{4})?\b/g, conf: 0.85 },
    ]
    function scanText(text) {
      if (!text) return []
      const spans = []
      for (const p of PII_PATTERNS) {
        p.re.lastIndex = 0
        let m
        while ((m = p.re.exec(text)) !== null) {
          spans.push({ start: m.index, end: m.index + m[0].length, category: p.category, confidence: p.conf })
        }
      }
      spans.sort(function (a, b) { return a.start - b.start })
      return spans
    }
    harness.handle('scan-artifacts', async (args) => {
      if (!fsSvc) return { error: 'ctx.fs unavailable' }
      const path = args && args.path
      if (!path) return { error: 'missing path' }
      try {
        const isTextLike = !/(\.(png|jpe?g|gif|webp|svg|pdf|docx|xlsx|pptx|zip|gz))$/i.test(path)
        if (!isTextLike) return { path, spans: [], binary: true }
        const text = await readTextBounded(await resolvePath(path))
        return { path, spans: scanText(text), detector: 'regex-feedback-tier' }
      } catch (err) {
        return { error: String((err && err.message) || err) }
      }
    })

    // Model-facing tool: scan_folder (S2.1 shape).
    harness.registerTool(
      ctx,
      harness.defineTool({
        name: 'scan_folder',
        description:
          'List the files in a folder with type/size/mtime (Artifacts view + redaction scanning).',
        parameters: {
          type: 'object',
          additionalProperties: true,
          properties: { path: { type: 'string' } },
          required: ['path'],
        },
        output: {
          schema: { type: 'object', additionalProperties: true },
          render(_args, value) {
            const items = (value && value.items) || []
            const lines = items.map(function (i) {
              return (
                (i.kind === 'folder' ? '[d] ' : '    ') +
                i.path +
                (i.size !== undefined ? '  (' + i.size + ')' : '')
              )
            })
            return [{ type: 'text', text: lines.length ? lines.join('\n') : '(empty)' }]
          },
        },
        async execute(args) {
          if (!fsSvc) return { error: 'ctx.fs unavailable' }
          const path = (args && args.path) || ''
          try {
            const entries = await fsSvc.listDir(await resolvePath(path), undefined)
            const items = entries.map(function (e) {
              const kind = entryKind(e)
              return {
                kind,
                path: (path ? path + '/' : '') + e.name,
                name: e.name,
                size: kind === 'file' && typeof e.size === 'number' ? e.size : undefined,
                hidden: !!e.hidden,
              }
            })
            return { path, count: items.length, items }
          } catch (err) {
            return { error: String((err && err.message) || err) }
          }
        },
      }),
    )
  },
}
