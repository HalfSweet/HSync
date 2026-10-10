/**
 * Conventional Commits, see AGENTS.md "Commit conventions".
 *
 * @type {import('@commitlint/types').UserConfig}
 */
export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    'scope-enum': [
      2,
      'always',
      [
        // Rust crates
        'core',
        'capture',
        'driver',
        'dslogic',
        'demo',
        'decode',
        'fileio',
        'session',
        'app',
        // Frontend
        'ui',
        'timeline',
        'stream',
        'trigger',
        'render',
        'i18n',
        'theme',
        'ipc',
        'store',
        // Cross-cutting
        'deps',
        'tooling',
        'repo',
      ],
    ],
  },
}
