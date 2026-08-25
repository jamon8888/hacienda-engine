# hacienda-build-optimizer Plugin Package
# Cordis plugin for faster build, clippy, and test execution in hacienda-engine
# Provides incremental, parallel, and cached commands

name: hacienda-build-optimizer
version: "1.0.0"
plugin: hacienda-build-optimizer
description: |
  Build optimizer for hacienda-engine. Provides faster clippy, test, and build
  commands through incremental checks, parallel execution, and smart caching.

# Host-side implementation
host: |
  return {
    name: "hacienda-build-optimizer",
    inject: ["workspace", "shell", "git"],
    async apply(ctx) {
      const { workspace, shell, git } = ctx;
      const root = workspace.root;

      // ============================================
      // SERVICE: hacienda.BuildOptimizer
      // ============================================
      ctx.provide("hacienda.BuildOptimizer", {
        // Configuration
        config: {
          // Default crates to check for fast mode (core crates only)
          fastCrates: ["hacienda", "hacienda-core"],
          // Full workspace crates for complete check
          allCrates: ["hacienda", "hacienda-core", "hacienda-api", "hacienda-cli", "hacienda-mcp", "hacienda-rag", "hacienda-wasm"],
          // Features to enable for clippy
          clippyFeatures: ["ner-candle"],
          // Parallel jobs (0 = auto)
          jobs: 0,
        },

        // Get list of changed crates since last commit or base branch
        async getChangedCrates(baseRef = "HEAD~1") {
          try {
            // Get changed files
            const { stdout } = await shell.exec(`git diff --name-only ${baseRef}`, { cwd: root });
            const changedFiles = stdout.trim().split("\n").filter(f => f);

            // Map files to crates
            const crateMap = {
              "hacienda/": "hacienda",
              "hacienda-core/": "hacienda-core",
              "hacienda-api/": "hacienda-api",
              "hacienda-cli/": "hacienda-cli",
              "hacienda-mcp/": "hacienda-mcp",
              "crates/hacienda-rag/": "hacienda-rag",
              "crates/hacienda-wasm/": "hacienda-wasm",
            };

            const changedCrates = new Set();
            for (const file of changedFiles) {
              for (const [prefix, crate] of Object.entries(crateMap)) {
                if (file.startsWith(prefix)) {
                  changedCrates.add(crate);
                  break;
                }
              }
              // Also check Cargo.toml changes
              if (file === "Cargo.toml" || file.endsWith("/Cargo.toml")) {
                // If workspace Cargo.toml changed, all crates might be affected
                return this.config.allCrates;
              }
            }

            return Array.from(changedCrates).length > 0 ? Array.from(changedCrates) : this.config.fastCrates;
          } catch {
            // Fallback to fast crates on error
            return this.config.fastCrates;
          }
        },

        // Build clippy command with optimized flags
        buildClippyCommand(crates, options = {}) {
          const { fix = false, denyWarnings = true, allTargets = true, features = this.config.clippyFeatures } = options;
          
          const crateArgs = crates.map(c => `-p ${c}`).join(" ");
          const featuresArg = features.length > 0 ? `--features ${features.join(",")}` : "";
          const targetsArg = allTargets ? "--all-targets" : "";
          const fixArg = fix ? "--fix" : "";
          const denyArg = denyWarnings ? "-- -D warnings" : "";
          const jobsArg = this.config.jobs > 0 ? `--jobs ${this.config.jobs}` : "";

          return `cargo clippy ${crateArgs} ${featuresArg} ${targetsArg} ${fixArg} ${jobsArg} ${denyArg}`.trim();
        },

        // Build test command with optimized flags
        buildTestCommand(crates, options = {}) {
          const { release = false, nocapture = false, jobs = this.config.jobs } = options;
          
          const crateArgs = crates.map(c => `-p ${c}`).join(" ");
          const releaseArg = release ? "--release" : "";
          const nocaptureArg = nocapture ? "-- --nocapture" : "";
          const jobsArg = jobs > 0 ? `--jobs ${jobs}` : "";

          return `cargo test ${crateArgs} ${releaseArg} ${jobsArg} ${nocaptureArg}`.trim();
        },

        // Build check command (type-check only, fastest)
        buildCheckCommand(crates, options = {}) {
          const { allTargets = true, features = this.config.clippyFeatures } = options;
          
          const crateArgs = crates.map(c => `-p ${c}`).join(" ");
          const featuresArg = features.length > 0 ? `--features ${features.join(",")}` : "";
          const targetsArg = allTargets ? "--all-targets" : "";
          const jobsArg = this.config.jobs > 0 ? `--jobs ${this.config.jobs}` : "";

          return `cargo check ${crateArgs} ${featuresArg} ${targetsArg} ${jobsArg}`.trim();
        },

        // Fast clippy - only core crates, no workspace-wide
        async fastClippy(options = {}) {
          const crates = this.config.fastCrates;
          const cmd = this.buildClippyCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Incremental clippy - only changed crates
        async incrementalClippy(baseRef = "HEAD~1", options = {}) {
          const crates = await this.getChangedCrates(baseRef);
          const cmd = this.buildClippyCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Full clippy - all workspace crates
        async fullClippy(options = {}) {
          const crates = this.config.allCrates;
          const cmd = this.buildClippyCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Fast test - only core crates
        async fastTest(options = {}) {
          const crates = this.config.fastCrates;
          const cmd = this.buildTestCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Incremental test - only changed crates
        async incrementalTest(baseRef = "HEAD~1", options = {}) {
          const crates = await this.getChangedCrates(baseRef);
          const cmd = this.buildTestCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Full test - all workspace crates
        async fullTest(options = {}) {
          const crates = this.config.allCrates;
          const cmd = this.buildTestCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Fast check - type check only on core crates (fastest)
        async fastCheck(options = {}) {
          const crates = this.config.fastCrates;
          const cmd = this.buildCheckCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Incremental check - type check only on changed crates
        async incrementalCheck(baseRef = "HEAD~1", options = {}) {
          const crates = await this.getChangedCrates(baseRef);
          const cmd = this.buildCheckCommand(crates, options);
          return shell.exec(cmd, { cwd: root });
        },

        // Show sccache stats
        async sccacheStats() {
          try {
            return await shell.exec("sccache --show-stats", { cwd: root });
          } catch {
            return { stdout: "sccache not available", stderr: "", code: 1 };
          }
        },

        // Show build timing info
        async timingInfo() {
          try {
            return await shell.exec("cargo build --timings", { cwd: root });
          } catch (e) {
            return { stdout: "", stderr: e.message, code: 1 };
          }
        },

        // Get available commands for the model
        getCommands() {
          return {
            // Clippy commands
            "clippy:fast": "Fast clippy on core crates only (~3s vs ~19s)",
            "clippy:incremental": "Clippy only on changed crates since HEAD~1",
            "clippy:full": "Full workspace clippy (all crates, all targets)",
            "clippy:fix": "Auto-fix clippy warnings on core crates",
            
            // Test commands
            "test:fast": "Fast test on core crates only",
            "test:incremental": "Test only changed crates",
            "test:full": "Full workspace test suite",
            
            // Check commands (type-check only, fastest)
            "check:fast": "Fast type-check on core crates",
            "check:incremental": "Type-check only changed crates",
            
            // Utility
            "sccache:stats": "Show sccache hit/miss statistics",
            "build:timings": "Show cargo build timing breakdown",
          };
        },
      });

      // ============================================
      // TOOLS: Register CLI tools for the model
      // ============================================
      const optimizer = ctx.consume("hacienda.BuildOptimizer");
      
      // Fast clippy tool
      ctx.provide("tool:hacienda_clippy_fast", {
        name: "hacienda_clippy_fast",
        description: "Run fast clippy on core crates only (hacienda, hacienda-core) - ~3s vs ~19s for full workspace",
        async handler({ fix = false }) {
          const result = await optimizer.fastClippy({ fix });
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            fix: { type: "boolean", description: "Auto-fix clippy warnings", default: false },
          },
        },
      });

      // Incremental clippy tool
      ctx.provide("tool:hacienda_clippy_incremental", {
        name: "hacienda_clippy_incremental",
        description: "Run clippy only on crates with changes since base ref (default HEAD~1)",
        async handler({ baseRef = "HEAD~1", fix = false }) {
          const result = await optimizer.incrementalClippy(baseRef, { fix });
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            baseRef: { type: "string", description: "Git ref to compare against", default: "HEAD~1" },
            fix: { type: "boolean", description: "Auto-fix clippy warnings", default: false },
          },
        },
      });

      // Full clippy tool
      ctx.provide("tool:hacienda_clippy_full", {
        name: "hacienda_clippy_full",
        description: "Run full workspace clippy on all crates with all targets",
        async handler({ fix = false }) {
          const result = await optimizer.fullClippy({ fix });
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            fix: { type: "boolean", description: "Auto-fix clippy warnings", default: false },
          },
        },
      });

      // Fast test tool
      ctx.provide("tool:hacienda_test_fast", {
        name: "hacienda_test_fast",
        description: "Run fast tests on core crates only (hacienda, hacienda-core)",
        async handler({ release = false, nocapture = false }) {
          const result = await optimizer.fastTest({ release, nocapture });
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            release: { type: "boolean", description: "Run in release mode", default: false },
            nocapture: { type: "boolean", description: "Show test output", default: false },
          },
        },
      });

      // Incremental test tool
      ctx.provide("tool:hacienda_test_incremental", {
        name: "hacienda_test_incremental",
        description: "Run tests only on crates with changes since base ref",
        async handler({ baseRef = "HEAD~1", release = false, nocapture = false }) {
          const result = await optimizer.incrementalTest(baseRef, { release, nocapture });
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            baseRef: { type: "string", description: "Git ref to compare against", default: "HEAD~1" },
            release: { type: "boolean", description: "Run in release mode", default: false },
            nocapture: { type: "boolean", description: "Show test output", default: false },
          },
        },
      });

      // Full test tool
      ctx.provide("tool:hacienda_test_full", {
        name: "hacienda_test_full",
        description: "Run full workspace test suite on all crates",
        async handler({ release = false, nocapture = false }) {
          const result = await optimizer.fullTest({ release, nocapture });
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            release: { type: "boolean", description: "Run in release mode", default: false },
            nocapture: { type: "boolean", description: "Show test output", default: false },
          },
        },
      });

      // Fast check tool (type-check only)
      ctx.provide("tool:hacienda_check_fast", {
        name: "hacienda_check_fast",
        description: "Run fast type-check on core crates only (fastest correctness signal)",
        async handler() {
          const result = await optimizer.fastCheck({});
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: { type: "object", properties: {} },
      });

      // Incremental check tool
      ctx.provide("tool:hacienda_check_incremental", {
        name: "hacienda_check_incremental",
        description: "Run type-check only on crates with changes since base ref",
        async handler({ baseRef = "HEAD~1" }) {
          const result = await optimizer.incrementalCheck(baseRef, {});
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: {
          type: "object",
          properties: {
            baseRef: { type: "string", description: "Git ref to compare against", default: "HEAD~1" },
          },
        },
      });

      // SCCache stats tool
      ctx.provide("tool:hacienda_sccache_stats", {
        name: "hacienda_sccache_stats",
        description: "Show sccache hit/miss statistics for build caching effectiveness",
        async handler() {
          const result = await optimizer.sccacheStats();
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: { type: "object", properties: {} },
      });

      // Build timings tool
      ctx.provide("tool:hacienda_build_timings", {
        name: "hacienda_build_timings",
        description: "Show cargo build timing breakdown to identify bottlenecks",
        async handler() {
          const result = await optimizer.timingInfo();
          return { stdout: result.stdout, stderr: result.stderr, code: result.code };
        },
        schema: { type: "object", properties: {} },
      });

      // Smart pipeline: check -> clippy -> test (fast mode)
      ctx.provide("tool:hacienda_verify_fast", {
        name: "hacienda_verify_fast",
        description: "Fast verification pipeline: check + clippy + test on core crates only (~30-60s)",
        async handler({ fix = false }) {
          const results = [];
          
          // 1. Fast check
          const checkResult = await optimizer.fastCheck({});
          results.push({ step: "check", ...checkResult });
          if (checkResult.code !== 0) return { results, success: false };
          
          // 2. Fast clippy
          const clippyResult = await optimizer.fastClippy({ fix });
          results.push({ step: "clippy", ...clippyResult });
          if (clippyResult.code !== 0) return { results, success: false };
          
          // 3. Fast test
          const testResult = await optimizer.fastTest({});
          results.push({ step: "test", ...testResult });
          
          return { results, success: testResult.code === 0 };
        },
        schema: {
          type: "object",
          properties: {
            fix: { type: "boolean", description: "Auto-fix clippy warnings", default: false },
          },
        },
      });

      // Smart pipeline: incremental
      ctx.provide("tool:hacienda_verify_incremental", {
        name: "hacienda_verify_incremental",
        description: "Incremental verification pipeline: check + clippy + test only on changed crates",
        async handler({ baseRef = "HEAD~1", fix = false }) {
          const results = [];
          
          // 1. Incremental check
          const checkResult = await optimizer.incrementalCheck(baseRef, {});
          results.push({ step: "check", ...checkResult });
          if (checkResult.code !== 0) return { results, success: false };
          
          // 2. Incremental clippy
          const clippyResult = await optimizer.incrementalClippy(baseRef, { fix });
          results.push({ step: "clippy", ...clippyResult });
          if (clippyResult.code !== 0) return { results, success: false };
          
          // 3. Incremental test
          const testResult = await optimizer.incrementalTest(baseRef, {});
          results.push({ step: "test", ...testResult });
          
          return { results, success: testResult.code === 0 };
        },
        schema: {
          type: "object",
          properties: {
            baseRef: { type: "string", description: "Git ref to compare against", default: "HEAD~1" },
            fix: { type: "boolean", description: "Auto-fix clippy warnings", default: false },
          },
        },
      });
    },
  };
