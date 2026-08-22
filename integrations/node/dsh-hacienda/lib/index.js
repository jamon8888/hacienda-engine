/**
 * @hacienda/dsh-hacienda main entry.
 *
 * Defers to the Host plugin (lib/host.js). The browser half ships as a
 * prebuilt client bundle produced by the C3 web build (see cordis.patch.yml's
 * `dsh.client` note and the README); it is delivered via the client-modules
 * roster path, not imported here.
 */
export { default } from './host.js'
