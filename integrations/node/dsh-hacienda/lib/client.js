/**
 * Client half of @hacienda/dsh-hacienda.
 *
 * Registers the "Files" (Artifacts) view in the `conversation.view` ring. It
 * renders a Finder-style directory listing fed by the package-private
 * `list-workspace` RPC (see lib/host.js). This is the working dynamic-plugin
 * client, packaged so it survives a web-bundle rebuild.
 *
 * The full Studio Finder component and the @extend-ai / CodeMirror viewers are
 * the next slice (C1.3–C1.5); this establishes the view seat + data path.
 *
 * Plain Cordis Client body — `React`, `host.call`, `styles.insert`, `ctx` are
 * provided by the restricted browser evaluator. No JSX.
 */
export default {
  name: 'hacienda-artifacts-client',
  apply(ctx) {
    const slots = ctx.get('slots')
    if (slots === undefined) {
      console.error('[hacienda-artifacts] slots unavailable')
      return
    }

    function ArtifactsView(props) {
      // eslint-disable-next-line no-unused-vars
      const sessionId = props.sessionId
      const [state, setState] = React.useState({ loading: true, items: [], error: null, path: '' })
      const [target, setTarget] = React.useState('')

      React.useEffect(function () {
        let cancelled = false
        setState({ loading: true, items: [], error: null, path: '' })
        host
          .call('list-workspace', { path: target })
          .then(function (res) {
            if (cancelled) return
            if (res && res.error) {
              setState({ loading: false, items: [], error: res.error, path: target })
            } else {
              setState({ loading: false, items: res.items || [], error: null, path: res.path })
            }
          })
          .catch(function (err) {
            if (!cancelled)
              setState({
                loading: false,
                items: [],
                error: String((err && err.message) || err),
                path: target,
              })
          })
        return function () {
          cancelled = true
        }
      }, [target])

      const openFolder = function (item) {
        if (item.kind === 'folder') setTarget(item.path)
      }

      const refresh = function () {
        setState({ loading: true, items: [], error: null, path: '' })
        host
          .call('list-workspace', { path: state.path })
          .then(function (res) {
            const items = res && res.items ? res.items : []
            setState({ loading: false, items, error: null, path: res ? res.path : state.path })
          })
          .catch(function (err) {
            setState({
              loading: false,
              items: [],
              error: String((err && err.message) || err),
              path: state.path,
            })
          })
      }

      return React.createElement(
        'div',
        {
          style: {
            padding: '12px',
            fontFamily: 'ui-monospace, monospace',
            fontSize: '12px',
            height: '100%',
            overflow: 'auto',
          },
        },
        React.createElement(
          'div',
          { style: { fontWeight: 600, marginBottom: '8px' } },
          'Artifacts ▸ ' + (state.path || '/'),
          React.createElement(
            'button',
            { onClick: function () { setTarget('') }, style: { marginLeft: '8px' } },
            'root',
          ),
          React.createElement(
            'button',
            { onClick: refresh, style: { marginLeft: '4px' } },
            'refresh',
          ),
        ),
        state.loading ? React.createElement('div', null, 'Loading…') : null,
        state.error
          ? React.createElement(
              'div',
              { style: { color: 'var(--dsw-alias-state-error-primary, #e5484d)' } },
              String(state.error),
            )
          : null,
        React.createElement(
          'ul',
          { style: { listStyle: 'none', margin: 0, padding: 0 } },
          state.items.map(function (item) {
            const isFolder = item.kind === 'folder'
            return React.createElement(
              'li',
              {
                key: item.path,
                onClick: function () {
                  if (isFolder) openFolder(item)
                },
                style: {
                  cursor: isFolder ? 'pointer' : 'default',
                  padding: '2px 4px',
                  borderBottom: '1px solid var(--dsw-alias-border-l2, #eee)',
                },
              },
              React.createElement('span', null, isFolder ? '📁' : '📄'),
              React.createElement('span', { style: { marginLeft: '6px' } }, item.path),
              item.size !== undefined
                ? React.createElement(
                    'span',
                    {
                      style: {
                        color: 'var(--dsw-alias-label-tertiary, #888)',
                        marginLeft: '8px',
                      },
                    },
                    String(item.size),
                  )
                : null,
            )
          }),
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
