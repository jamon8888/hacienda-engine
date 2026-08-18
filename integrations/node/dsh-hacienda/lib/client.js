/**
 * Client half of @hacienda/dsh-hacienda.
 *
 * Registers the "Files" (Artifacts) view in the `conversation.view` ring. This
 * version adds **file-open preview**: clicking a markdown/text/code file shows
 * its content (via the `read-file-text` RPC); images render inline; other files
 * get a download link (via `file-download-url`). Folders navigate.
 *
 * Plain JS using only the `React`/`host.call` builtins. The full Studio Finder
 * component (`@pierre/trees`), the `@extend-ai` DOCX/PPTX/XLSX/PDF viewers, and
 * CodeMirror are the C1.3–C1.5 slice and must be bundled by the C3 web build.
 */
export default {
  name: 'hacienda-artifacts-client',
  apply(ctx) {
    const slots = ctx.get('slots')
    if (slots === undefined) {
      console.error('[hacienda-artifacts] slots unavailable')
      return
    }

    const TEXT_EXT = {
      md: 1, mdx: 1, txt: 1, markdown: 1, json: 1, yml: 1, yaml: 1,
      ts: 1, tsx: 1, js: 1, jsx: 1, py: 1, rs: 1, sh: 1, css: 1, html: 1, sql: 1, toml: 1, go: 1,
    }
    const IMG_RE = /\.(png|jpe?g|gif|webp|svg|bmp|tiff?)$/i
    const KIND_META = {
      md: { glyph: 'M↓', color: '#1a7f37' },
      docx: { glyph: 'W', color: '#2563eb' },
      xlsx: { glyph: 'X', color: '#0f766e' },
      pptx: { glyph: 'P', color: '#c2410c' },
      pdf: { glyph: 'PDF', color: '#b91c1c' },
      txt: { glyph: 'T', color: '#6b7280' },
      json: { glyph: '{ }', color: '#7c3aed' },
      yml: { glyph: 'Y', color: '#0f766e' },
      yaml: { glyph: 'Y', color: '#0f766e' },
      ts: { glyph: 'TS', color: '#2563eb' },
      tsx: { glyph: 'TSX', color: '#2563eb' },
      js: { glyph: 'JS', color: '#b45309' },
      py: { glyph: 'PY', color: '#1d4ed8' },
      rs: { glyph: 'RS', color: '#b45309' },
      zip: { glyph: 'ZIP', color: '#7c2d12' },
      gz: { glyph: 'TAR', color: '#7c2d12' },
    }
    const FOLDER = { glyph: '⬛', color: '#3b82f6' }
    const DEFAULT = { glyph: '•', color: '#9ca3af' }

    function metaFor(name, isFolder) {
      if (isFolder) return FOLDER
      if (IMG_RE.test(name)) return { glyph: 'IMG', color: '#be185d' }
      const dot = name.lastIndexOf('.')
      const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
      return KIND_META[ext] || DEFAULT
    }
    function isText(name) {
      const dot = name.lastIndexOf('.')
      const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
      return !!TEXT_EXT[ext]
    }
    function formatBytes(n) {
      if (n == null) return ''
      if (n < 1024) return n + ' B'
      const u = ['KB', 'MB', 'GB', 'TB']
      let v = n
      for (const k of u) {
        v /= 1024
        if (v < 1024 || k === 'TB') return (v >= 100 ? Math.round(v) : v.toFixed(1)) + ' ' + k
      }
      return n + ' B'
    }
    function parentOf(path) {
      if (!path) return null
      const t = path.replace(/\/+$/, '')
      const i = t.lastIndexOf('/')
      return i <= 0 ? '' : t.slice(0, i)
    }

    // PII span highlight (regex feedback tier from scan-artifacts).
    const CAT_COLOR = {
      email: '#e5484d',
      phone: '#f76b15',
      iban: '#8e4ec6',
      url: '#1d4ed8',
      card: '#d1242f',
    }
    function renderText(text, spans) {
      if (!spans || spans.length === 0)
        return React.createElement(
          'pre',
          { style: { flex: 1, overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontSize: '12px', padding: '8px' } },
          text,
        )
      const sorted = spans.slice().sort(function (a, b) { return a.start - b.start })
      const parts = []
      let idx = 0
      for (const s of sorted) {
        if (s.start < idx) continue
        if (s.start > idx) parts.push(React.createElement('span', { key: idx }, text.slice(idx, s.start)))
        const color = CAT_COLOR[s.category] || '#e5484d'
        parts.push(
          React.createElement(
            'mark',
            {
              key: 's' + s.start,
              style: {
                background: color + '33',
                borderBottom: '2px solid ' + color,
                borderRadius: '3px',
                padding: '0 1px',
              },
              title: (s.category || 'pii') + ' conf=' + (typeof s.confidence === 'number' ? s.confidence.toFixed(2) : '?'),
            },
            text.slice(s.start, s.end),
          ),
        )
        idx = s.end
      }
      if (idx < text.length) parts.push(React.createElement('span', { key: 'end' }, text.slice(idx)))
      return React.createElement(
        'pre',
        { style: { flex: 1, overflow: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontSize: '12px', padding: '8px' } },
        parts,
      )
    }

    function ArtifactsView(props) {
      const [state, setState] = React.useState({ loading: true, items: [], error: null, path: '' })
      const [target, setTarget] = React.useState('')
      const [preview, setPreview] = React.useState(null) // { path, kind:'text'|'img'|'download', text?, url? }
      const [err, setErr] = React.useState(null)

      const load = React.useCallback(function (path) {
        setState({ loading: true, items: [], error: null, path: '' })
        setPreview(null)
        setErr(null)
        host
          .call('list-workspace', { path })
          .then(function (res) {
            setState({
              loading: false,
              items: (res && res.items) || [],
              error: (res && res.error) || null,
              path: res ? res.path : path,
            })
          })
          .catch(function (e) {
            setState({ loading: false, items: [], error: String((e && e.message) || e), path })
          })
      }, [])
      React.useEffect(function () {
        load(target)
      }, [target, load])

      function openFile(item) {
        setErr(null)
        if (IMG_RE.test(item.name || item.path)) {
          host
            .call('file-download-url', { path: item.path })
            .then(function (r) {
              setPreview(r && r.url ? { path: item.path, kind: 'img', url: r.url } : { path: item.path, kind: 'download', url: r && r.url })
            })
            .catch(function (e) {
              setErr(String((e && e.message) || e))
            })
        } else if (isText(item.name || item.path)) {
          host
            .call('read-file-text', { path: item.path })
            .then(function (r) {
              if (r && r.error) {
                setErr(r.error)
                return
              }
              // Fetch PII spans for the preview highlight (regex feedback tier).
              host
                .call('scan-artifacts', { path: item.path })
                .then(function (s) {
                  setPreview({ path: item.path, kind: 'text', text: r.text, spans: (s && s.spans) || [] })
                })
                .catch(function () {
                  setPreview({ path: item.path, kind: 'text', text: r.text, spans: [] })
                })
            })
            .catch(function (e) {
              setErr(String((e && e.message) || e))
            })
        } else {
          host
            .call('file-download-url', { path: item.path })
            .then(function (r) {
              setPreview({ path: item.path, kind: 'download', url: r && r.url })
            })
            .catch(function (e) {
              setErr(String((e && e.message) || e))
            })
        }
      }

      // ---- preview pane ----
      if (preview) {
        return React.createElement(
          'div',
          {
            style: {
              padding: '12px',
              fontFamily: 'ui-sans-serif, system-ui',
              fontSize: '13px',
              height: '100%',
              display: 'flex',
              flexDirection: 'column',
            },
          },
          React.createElement(
            'div',
            {
              style: {
                display: 'flex',
                alignItems: 'center',
                gap: '8px',
                paddingBottom: '8px',
                borderBottom: '1px solid var(--dsw-alias-border-l2, #eee)',
              },
            },
            React.createElement('button', { onClick: function () { setPreview(null) } }, '← back'),
            React.createElement('span', { style: { fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } }, preview.path),
            preview.kind === 'text' && preview.spans && preview.spans.length
              ? React.createElement('span', { style: { marginLeft: 'auto', color: 'var(--dsw-alias-state-warn-primary, #f76b15)', fontSize: '11px', fontWeight: 600 } }, preview.spans.length + ' PII detected')
              : preview.kind === 'download' && preview.url
                ? React.createElement('a', { href: preview.url, download: true, style: { marginLeft: 'auto' } }, 'download')
                : null,
          ),
          preview.kind === 'text'
            ? renderText(preview.text, preview.spans)
            : preview.kind === 'img'
              ? React.createElement('img', { src: preview.url, style: { maxWidth: '100%', marginTop: '8px' } })
              : React.createElement('div', { style: { padding: '8px', color: 'var(--dsw-alias-label-secondary, #666)' } }, 'Binary file — a viewer is wired in the C1.4 slice.'),
        )
      }

      const sorted = React.useMemo(function () {
        const items = state.items.slice()
        items.sort(function (a, b) {
          if (a.kind !== b.kind) return a.kind === 'folder' ? -1 : 1
          return String(a.name || a.path).localeCompare(String(b.name || b.path), undefined, {
            numeric: true,
            sensitivity: 'base',
          })
        })
        return items
      }, [state.items])

      // ---- directory listing ----
      return React.createElement(
        'div',
        {
          style: {
            padding: '12px',
            fontFamily: 'ui-sans-serif, system-ui',
            fontSize: '13px',
            height: '100%',
            overflow: 'auto',
          },
        },
        React.createElement(
          'div',
          {
            style: {
              display: 'flex',
              alignItems: 'center',
              gap: '8px',
              fontWeight: 600,
              paddingBottom: '8px',
              borderBottom: '1px solid var(--dsw-alias-border-l2, #eee)',
              marginBottom: '6px',
            },
          },
          React.createElement('span', null, 'Artifacts'),
          React.createElement('span', { style: { color: 'var(--dsw-alias-label-tertiary, #888)' } }, '▸'),
          React.createElement('span', { style: { overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' } }, state.path || '/'),
          React.createElement(
            'span',
            { style: { marginLeft: 'auto', display: 'flex', gap: '4px' } },
            React.createElement('button', { onClick: function () { setTarget('') } }, 'root'),
            React.createElement('button', { onClick: function () { const p = parentOf(state.path); if (p !== null) setTarget(p) }, disabled: !state.path }, 'up'),
            React.createElement('button', { onClick: function () { load(state.path) } }, 'refresh'),
          ),
        ),
        state.loading ? React.createElement('div', { style: { padding: '8px', color: 'var(--dsw-alias-label-secondary, #666)' } }, 'Loading…') : null,
        state.error ? React.createElement('div', { style: { color: 'var(--dsw-alias-state-error-primary, #e5484d)', padding: '8px' } }, String(state.error)) : null,
        err ? React.createElement('div', { style: { color: 'var(--dsw-alias-state-error-primary, #e5484d)', padding: '8px' } }, String(err)) : null,
        React.createElement(
          'table',
          { style: { width: '100%', borderCollapse: 'collapse' } },
          React.createElement(
            'thead',
            null,
            React.createElement(
              'tr',
              { style: { textAlign: 'left', color: 'var(--dsw-alias-label-tertiary, #888)' } },
              React.createElement('th', { style: { padding: '2px 6px', fontWeight: 500 } }, 'Name'),
              React.createElement('th', { style: { padding: '2px 6px', fontWeight: 500, textAlign: 'right' } }, 'Size'),
              React.createElement('th', { style: { padding: '2px 6px', fontWeight: 500 } }, 'Kind'),
            ),
          ),
          React.createElement(
            'tbody',
            null,
            sorted.map(function (item) {
              const m = metaFor(item.name || item.path, item.kind === 'folder')
              return React.createElement(
                'tr',
                {
                  key: item.path,
                  onClick: function () {
                    if (item.kind === 'folder') setTarget(item.path)
                    else openFile(item)
                  },
                  onDoubleClick: function () {
                    if (item.kind === 'folder') setTarget(item.path)
                    else openFile(item)
                  },
                  style: { cursor: 'pointer' },
                },
                React.createElement(
                  'td',
                  { style: { padding: '2px 6px' } },
                  React.createElement(
                    'span',
                    {
                      style: {
                        display: 'inline-block',
                        width: '34px',
                        height: '20px',
                        lineHeight: '20px',
                        textAlign: 'center',
                        borderRadius: '4px',
                        fontSize: '10px',
                        fontWeight: 700,
                        color: '#fff',
                        background: m.color,
                        marginRight: '8px',
                        verticalAlign: 'middle',
                      },
                    },
                    m.glyph,
                  ),
                  React.createElement('span', { style: { verticalAlign: 'middle' } }, item.name),
                ),
                React.createElement('td', { style: { padding: '2px 6px', textAlign: 'right', color: 'var(--dsw-alias-label-tertiary, #888)' } }, item.kind === 'folder' ? '—' : formatBytes(item.size)),
                React.createElement('td', { style: { padding: '2px 6px', color: 'var(--dsw-alias-label-secondary, #666)' } }, m.kind),
              )
            }),
          ),
        ),
      )
    }

    slots.inject('conversation.view', function () {
      return slots.register(
        { name: 'conversation.view', id: 'artifacts', order: 20, label: 'Files' },
        function (props) {
          return React.createElement(ArtifactsView, props)
        },
      )
    })
  },
}
