/** @type {import('prettier').Config} */
export default {
  semi: false,
  singleQuote: true,
  printWidth: 100,
  trailingComma: 'all',
  endOfLine: 'lf',
  overrides: [
    {
      files: '*.md',
      options: { proseWrap: 'preserve' },
    },
  ],
}
