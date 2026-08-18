/**
 * Client half of @hacienda/dsh-hacienda.
 *
 * Registers the "Files" (Artifacts) view in the `conversation.view` ring. This
 * is a Finder-style directory browser fed by the package-private `list-workspace`
 * RPC (lib/host.js): filetype-aware icons, formatted sizes, folder-first sort,
 * and parent/root navigation.
 *
 * Development note: this is plain JS using only the `React`/`host.call`
 * builtins. The full Hacienda Studio Finder component (`@pierre/trees`) and the
 * `@extend-ai` DOCX/PPTX/XLSX/PDF viewers + CodeMirror are the C1.3-C1.5 slice
 * and must be bundled by the C3 web build (they cannot be imported in this
 * restricted evaluator); this view is the durable, dependency-free base.
 */
export default {
  name: 'hacienda-artifacts-client',
  apply(ctx) {
    const slots = ctx.get('slots')
    if (slots === undefined) {
      console.error('[hacienda-artifacts] slots unavailable')
      return
    }

    // ---- file-type presentation (extension-based; no deps) ----
    const KIND_META = {
      md: { glyph: 'M↓', color: '#1a7f37', kind: 'markdown' },
      docx: { glyph: 'W', color: '#2563eb', kind: 'document' },
      xlsx: { glyph: 'X', color: '#0f766e', kind: 'spreadsheet' },
      pptx: { glyph: 'P', color: '#c2410c', kind: 'presentation' },
      pdf: { glyph: 'PDF', color: '#b91c1c', kind: 'document' },
      txt: { glyph: 'T', color: '#6b7280', kind: 'text' },
      json: { glyph: '{ }', color: '#7c3aed', kind: 'code' },
      yml: { glyph: 'Y', color: '#0f766e', kind: 'code' },
      yaml: { glyph: 'Y', color: '#0f766e', kind: 'code' },
      ts: { glyph: 'TS', color: '#2563eb', kind: 'code' },
      tsx: { glyph: 'TSX', color: '#2563eb', kind: 'code' },
      js: { glyph: 'JS', color: '#b45309', kind: 'code' },
      py: { glyph: 'PY', color: '#1d4ed8', kind: 'code' },
      rs: { glyph: 'RS', color: '#b45309', kind: 'code' },
      zip: { glyph: 'ZIP', color: '#7c2d12', kind: 'archive' },
      gz: { glyph: 'TAR', color: '#7c2d12', kind: 'archive' },
    }
    const FOLDER = { glyph: '⬛', color: '#3b82f6', kind: 'folder' }
    const IMAGE_RE = /\.(png|jpe?g|gif|webp|svg|bmp|tiff?)$/i
    const DEFAULT = { glyph: '•', color: '#9ca3af', kind: 'other' }

    function metaFor(name, isFolder) {
      if (isFolder) return FOLDER
      if (IMAGE_RE.test(name)) return { glyph: 'IMG', color: '#be185d', kind: 'image' }
      const dot = name.lastIndexOf('.')
      const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : ''
      return KIND_META[ext] || DEFAULT
    }

    function formatBytes(n) {
      if (n === undefined || n === null) return ''
      if (n < 1024) return n + ' B'
      const units = ['KB', 'MB', 'GB', 'TB']
      let v = n
      for (const u of units) {
        v /= 1024
        if (v < 1024 || u === 'TB') return (v >= 100 ? Math.round(v) : v.toFixed(1)) + ' ' + u
      }
      return n + ' B'
    }

    function parentOf(path) {
      if (!path) return null
      const trimmed = path.replace(/\/+$/, '')
      const idx = trimmed.lastIndexOf('/')
      return idx <= 0 ? '' : trimmed.slice(0, idx)
    }

    function ArtifactsView(props) {
      // eslint-disable-next-line no-unused-vars
      const sessionId = props.sessionId
      const [state, setState] = React.useState({ loading: true, items: [], error: null, path: '' })
      const [target, setTarget] = React.useState('')
      const [selected, setSelected] = React.useState(null)

      const load = React.useCallback(function (path) {
        setState({ loading: true, items: [], error: null, path: '' })
        host
          .call('list-workspace', { path })
          .then(function (res) {
            setState({
              loading: false,
              items: (res && res.items) || [],
              error: res && res.error ? res.error : null,
              path: res ? res.path : path,
            })
            setSelected(null)
          })
          .catch(function (err) {
            setState({
              loading: false,
              items: [],
              error: String((err && err.message) || err),
              path,
            })
          })
      }, [])

      React.useEffect(function () {
        load(target)
      }, [target, load])

      const sorted = React.useMemo(
        function () {
          const items = state.items.slice()
          items.sort(function (a, b) {
            if (a.kind !== b.kind) return a.kind === 'folder' ? -1 : 1
            return String(a.name || a.path).localeCompare(String(b.name || b.path), undefined, {
              numeric: true,
              sensitivity: 'base',
            })
          })
          return items
        },
        [state.items],
      )

      const up = function () {
        const p = parentOf(state.path)
        if (p !== null) setTarget(p)
      }

      const openItem = function (item) {
        setSelected(item.path)
        // Folders navigate; files open later via read-file-text / download URL
        // (viewer wiring is the C1.4 slice). No-op for files for now.
        if (item.kind === 'folder') setTarget(item.path)
      }

      return React.createElement(
        'div',
        {
          style: {
            padding: '12px',
            fontFamily: 'ui-sans-serif, system-ui, sans-serif',
            fontSize: '13px',
            height: '100%',
            overflow: 'auto',
          },
        },
        // Header / breadcrumb
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
          React.createElement('span', { style: { marginLeft: 'auto', display: 'flex', gap: '4px' } },
            React.createElement('button', { onClick: function () { setTarget('') } }, 'root'),
            React.createElement('button', { onClick: up, disabled: !state.path }, 'up'),
            React.createElement('button', { onClick: function () { load(state.path) } }, 'refresh'),
          ),
        ),
        state.loading ? React.createElement('div', { style: { padding: '8px', color: 'var(--dsw-alias-label-secondary, #666)' } }, 'Loading…') : null,
        state.error
          ? React.createElement(
              'div',
              { style: { color: 'var(--dsw-alias-state-error-primary, #e5484d)', padding: '8px' } },
              String(state.error),
            )
          : null,
        // Listing
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
              const isSel = selected === item.path
              return React.createElement(
                'tr',
                {
                  key: item.path,
                  onClick: function () { openItem(item) },
                  onDoubleClick: function () { if (item.kind === 'folder') setTarget(item.path) },
                  style: {
                    cursor: 'pointer',
                    background: isSel ? 'var(--dsw-alias-bg-module-hover, #f3f4f6)' : undefined,
                  },
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
                React.createElement(
                  'td',
                  { style: { padding: '2px 6px', textAlign: 'right', color: 'var(--dsw-alias-label-tertiary, #888)' } },
                  item.kind === 'folder' ? '—' : formatBytes(item.size),
                ),
                React.createElement(
                  'td',
                  { style: { padding: '2px 6px', color: 'var(--dsw-alias-label-secondary, #666)' } },
                  m.kind,
                ),
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
