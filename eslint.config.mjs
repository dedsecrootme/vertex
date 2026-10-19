import js from '@eslint/js';
import globals from 'globals';

export default [
  { ignores: ['**/node_modules/**', '**/target/**', '**/vendor/**', '**/gen/**', '**/.wrangler/**', '**/.vercel/**', 'database/**', 'temp/**', 'test-results/**', 'playwright-report/**'] },
  js.configs.recommended,
  { files: ['**/*.js', '**/*.mjs'], languageOptions: { ecmaVersion: 'latest' }, rules: {
    'no-unused-vars': ['error', { argsIgnorePattern: '^_', caughtErrors: 'none' }],
    'no-empty': ['error', { allowEmptyCatch: true }],
  } },
  { files: ['apps/web/*.js', 'apps/landing/public/*.js'], languageOptions: { globals: globals.browser } },
  { files: ['scripts/**/*.mjs', 'tests/**/*.mjs', '*.mjs'], languageOptions: { globals: { ...globals.node, ...globals.browser } } },
  { files: ['apps/landing/worker.mjs'], languageOptions: { globals: globals.serviceworker } },
];
