import ts from 'typescript-eslint';
import { createTypeScriptImportResolver } from 'eslint-import-resolver-typescript';
import react from 'eslint-plugin-react';
import hooks from 'eslint-plugin-react-hooks';
import importX from 'eslint-plugin-import-x';
import i18next from 'eslint-plugin-i18next';
import comments from '@eslint-community/eslint-plugin-eslint-comments';
import { zones, native, subscriptions, path } from './lint-config/constraints.js';
export default ts.config(
  {
    ignores: [
      'node_modules/**',
      'dist/**',
      'src/lib/api/generated/**',
      ...(process.env.WEB_LINT_FIXTURES ? [] : ['lint-fixtures/**']),
    ],
  },
  ...ts.configs.recommended,
  {
    files: ['**/*.{ts,tsx,js}'],
    plugins: {
      react,
      'react-hooks': hooks,
      'import-x': importX,
      i18next,
      'eslint-comments': comments,
    },
    settings: {
      react: { version: '19.3' },
      'import-x/resolver-next': [createTypeScriptImportResolver()],
      'import-x/parsers': { '@typescript-eslint/parser': ['.ts', '.tsx'] },
      'import-x/extensions': ['.js', '.ts', '.tsx'],
    },
    rules: {
      ...Object.fromEntries(
        Object.keys(hooks.configs.recommended.rules).map((rule) => [rule, 'error']),
      ),
      'react/no-danger': 'error',
      'import-x/no-cycle': 'error',
      'eslint-comments/require-description': 'error',
      'react/jsx-max-depth': ['error', { max: 6 }],
      'max-lines': ['error', { max: 200, skipBlankLines: false, skipComments: false }],
      'import-x/no-restricted-paths': ['error', { basePath: import.meta.dirname, zones }],
      'no-restricted-syntax': [
        'error',
        { selector: path, message: 'Use registered icons or assets.' },
        ...subscriptions,
      ],
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_' },
      ],
    },
  },
  {
    files: ['src/{shell,features}/**/*.tsx', 'lint-fixtures/{shell,features}/**/*.tsx'],
    rules: {
      'no-restricted-syntax': [
        'error',
        { selector: native, message: 'Use design or scene components.' },
        { selector: path, message: 'Use registered icons.' },
        ...subscriptions,
      ],
      'i18next/no-literal-string': ['error', { mode: 'jsx-text-only' }],
    },
  },
  {
    files: ['src/design/**/*.{ts,tsx}', 'lint-fixtures/design/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            '.',
            './index',
            './index.ts',
            './index.tsx',
            '../index',
            '../index.ts',
            '../design',
          ],
          patterns: [
            {
              regex: '^(?!(react(/|$)|@base-ui/react(/|$)|qrcode$|lucide-react$|\\.{1,2}/)).+',
              message: 'Design only depends on design and approved UI packages.',
            },
          ],
        },
      ],
    },
  },
  {
    files: ['src/scene/**/*.{ts,tsx}', 'lint-fixtures/scene/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            '.',
            './index',
            './index.ts',
            './index.tsx',
            '../index',
            '../index.ts',
            '../scene',
          ],
          patterns: [
            {
              regex: '^(?!(react(/|$)|\\.{1,2}/)).+',
              message: 'Scene composes design components.',
            },
          ],
        },
      ],
    },
  },
  {
    files: ['src/{features,shell}/**/*.{ts,tsx}', 'lint-fixtures/{features,shell}/**/*.{ts,tsx}'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          paths: [
            {
              name: 'react',
              importNames: ['useEffect', 'useLayoutEffect'],
              message: 'Use a resource hook or document an exception.',
            },
          ],
        },
      ],
      'no-restricted-syntax': [
        'error',
        { selector: native, message: 'Use design or scene components.' },
        { selector: path, message: 'Use registered icons.' },
        ...subscriptions,
        {
          selector: 'CallExpression[callee.name=/^(useEffect|useLayoutEffect)$/]',
          message: 'Document effect exceptions.',
        },
        {
          selector: 'MemberExpression[property.name=/^(useEffect|useLayoutEffect)$/]',
          message: 'Document effect exceptions.',
        },
        {
          selector:
            'MemberExpression[computed=true][property.value=/^(useEffect|useLayoutEffect)$/]',
          message: 'Document effect exceptions.',
        },
      ],
    },
  },
  {
    files: [
      'src/lib/**/*.{ts,tsx}',
      'src/scene/drag/*.ts',
      'src/design/Toast.tsx',
      'src/design/Command.tsx',
    ],
    rules: {
      'no-restricted-syntax': [
        'error',
        { selector: path, message: 'Use registered icons or assets.' },
      ],
    },
  },
);
