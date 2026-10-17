import { fileURLToPath } from 'node:url'

import { configDefaults, defineConfig, mergeConfig } from 'vitest/config'

import viteConfig from './vite.config.ts'

export default mergeConfig(
  viteConfig,
  defineConfig({
    test: {
      environment: 'jsdom',
      include: ['src/**/*.test.ts', 'uiharu/compiler/src/**/*.test.ts'],
      exclude: [...configDefaults.exclude, 'src-tauri/**', 'crates/**'],
      root: fileURLToPath(new URL('./', import.meta.url)),
      passWithNoTests: true,
    },
  }),
)
