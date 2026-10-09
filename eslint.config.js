import { vueTsConfigs, withVueTs } from '@vue/eslint-config-typescript'
import prettierConfig from 'eslint-config-prettier'
import { globalIgnores } from 'eslint/config'
import pluginVue from 'eslint-plugin-vue'
import globals from 'globals'

export default withVueTs(
  { rootDir: import.meta.dirname },

  globalIgnores([
    '**/dist/**',
    '**/coverage/**',
    '**/target/**',
    'src-tauri/gen/**',
    'src/ipc/bindings/**',
    '.build/**',
    '.hsync/**',
  ]),

  pluginVue.configs['flat/recommended'],
  vueTsConfigs.recommendedTypeChecked,

  {
    name: 'hsync/node-config-files',
    files: ['*.config.{js,ts}'],
    languageOptions: {
      globals: globals.node,
    },
  },

  {
    name: 'hsync/rules',
    rules: {
      eqeqeq: ['error', 'smart'],
      'no-console': ['warn', { allow: ['warn', 'error'] }],

      '@typescript-eslint/consistent-type-imports': 'error',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],

      'vue/block-lang': ['error', { script: { lang: 'ts' } }],
      'vue/component-api-style': ['error', ['script-setup']],
      'vue/define-macros-order': 'error',
      'vue/multi-word-component-names': ['error', { ignores: ['App'] }],
      'vue/no-unused-refs': 'error',
      'vue/no-useless-v-bind': 'error',
      'vue/prefer-true-attribute-shorthand': 'error',
    },
  },

  // Formatting is owned by Prettier.
  prettierConfig,
)
