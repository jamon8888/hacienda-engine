import { cp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../..');
const output = resolve(process.argv[2] ?? join(repo, 'dist/hacienda-claude-marketplace'));
const plugin = join(output, 'plugins/hacienda-projection-renderer');

await rm(output, { recursive: true, force: true });
await mkdir(join(output, '.claude-plugin'), { recursive: true });
await mkdir(join(plugin, 'hooks'), { recursive: true });
await mkdir(join(plugin, '.claude-plugin'), { recursive: true });

await cp(join(repo, 'claude-mod/src/renderer.js'), join(plugin, 'renderer.js'));
await cp(join(here, 'mod-adapter.js'), join(plugin, 'mod-adapter.js'));
await cp(join(here, 'plugin/hooks/register.js'), join(plugin, 'hooks/register.js'));
await cp(join(here, 'plugin/hooks/hooks.json'), join(plugin, 'hooks/hooks.json'));
await cp(join(here, 'plugin/.claude-plugin/plugin.json'), join(plugin, '.claude-plugin/plugin.json'));
await cp(join(repo, 'LICENSE'), join(plugin, 'LICENSE'));
await writeFile(join(plugin, 'README.md'), `# Hacienda Projection Renderer\n\nThis Claude Code Mod bootstraps Hacienda from the prompt.submit hook before a prompt is sent, then polls a bounded batch of authenticated loopback mappings before rendering assistant messages. It requires Claude Code v2.1.326 or later for the documented Mods hook environment API. It uses no Node.js APIs and fails closed to pseudonyms if setup, authentication, polling, or validation fails. Revealed values are visual-only and remain in memory; they do not change Claude's stored transcript, model context, provider traffic, or later turns. See the Hacienda repository's claude-mod/README.md for setup and security details.\n`);
await writeFile(join(plugin, 'package.json'), JSON.stringify({ type: 'module' }, null, 2) + '\n');

const adapterPath = join(plugin, 'mod-adapter.js');
const adapter = await readFile(adapterPath, 'utf8');
const rendererImport = "../../claude-mod/src/renderer.js";
if (!adapter.includes(rendererImport)) throw new Error('Unexpected Mod adapter renderer import');
await writeFile(adapterPath, adapter.replace(rendererImport, './renderer.js'));

await writeFile(join(output, '.claude-plugin/marketplace.json'), JSON.stringify({
  name: 'hacienda-local',
  description: 'Hacienda local projection renderer for Claude Code',
  owner: { name: 'Hacienda' },
  plugins: [{ name: 'hacienda-projection-renderer', source: './plugins/hacienda-projection-renderer' }],
}, null, 2) + '\n');

console.log(`Packaged Claude Mod marketplace at ${output}`);
