/// <reference types="@xberg-io/xberg-wasm" />

declare module "*.wasm?url" {
  const url: string;
  export default url;
}

declare module "*?raw" {
  const content: string;
  export default content;
}
