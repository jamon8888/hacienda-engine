import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(new URL("./prepare_onnxruntime.py", import.meta.url));
const configuredPython = process.env.PYTHON;
const candidates = configuredPython
  ? [[configuredPython, []]]
  : process.platform === "win32"
    ? [
        ["py", ["-3"]],
        ["python", []],
      ]
    : [
        ["python3", []],
        ["python", []],
      ];

for (const [command, prefixArgs] of candidates) {
  const result = spawnSync(command, [...prefixArgs, script], { stdio: "inherit" });
  if (result.error?.code === "ENOENT") continue;
  if (result.error) {
    console.error(`Could not run ${command}: ${result.error.message}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

console.error("Python 3 is required to stage ONNX Runtime for Tauri.");
process.exit(1);
