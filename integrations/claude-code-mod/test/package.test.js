import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = fileURLToPath(new URL('..', import.meta.url));

test('package command emits a standalone Claude plugin marketplace', async () => {
  const temp = await mkdtemp(join(tmpdir(), 'hacienda-claude-package-'));
  const output = join(temp, 'marketplace');
  try {
    const result = spawnSync(process.execPath, [join(here, 'package-plugin.mjs'), output], {
      encoding: 'utf8',
    });
    assert.equal(result.status, 0, result.stderr);

    const marketplace = JSON.parse(await readFile(join(output, '.claude-plugin/marketplace.json'), 'utf8'));
    assert.equal(marketplace.name, 'hacienda-local');
    assert.equal(marketplace.plugins[0].source, './plugins/hacienda-projection-renderer');

    const plugin = join(output, 'plugins/hacienda-projection-renderer');
    const manifest = JSON.parse(await readFile(join(plugin, '.claude-plugin/plugin.json'), 'utf8'));
    const hooks = JSON.parse(await readFile(join(plugin, 'hooks/hooks.json'), 'utf8'));
    assert.equal(manifest.name, 'hacienda-projection-renderer');
    assert.deepEqual(hooks.modules, ['./register.js']);
    for (const file of ['renderer.js', 'hooks/register.js', 'README.md', 'LICENSE']) {
      await stat(join(plugin, file));
    }
    const register = await readFile(join(plugin, 'hooks/register.js'), 'utf8');
    assert.match(register, /from '\.\.\/renderer\.js'/);
    assert.match(register, /from '\.\.\/mod-adapter\.js'/);
    // The Mod runtime has no Node built-ins. Reject only `node:` import
    // specifiers; the local `mod-adapter.js` is the supported adapter.
    assert.doesNotMatch(register, /(?:\bfrom\s*|\bimport\s*\()\s*['"]node:[^'"]+['"]/);
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});
