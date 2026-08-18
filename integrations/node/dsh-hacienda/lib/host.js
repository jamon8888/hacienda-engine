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

    async function resolvePath(p) {
      return fsSvc.resolve(p, {})
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
        return { path, text: await fsSvc.readText(await resolvePath(path)) }
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

    // RPC 4 — PII span overlay for a file. Placeholder until the P7-corrected
    // facade / GLiNER2 worker is wired (authoritative pass stays server-side).
    harness.handle('scan-artifacts', async (args) => ({
      path: args && args.path,
      spans: [],
      status: 'review-only-not-wired',
    }))

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
