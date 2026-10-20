export default {
  extends: ['stylelint-config-standard', 'stylelint-config-css-modules'],
  plugins: ['stylelint-declaration-strict-value'],
  ignoreFiles: [
    'node_modules/**',
    'dist/**',
    ...(process.env.WEB_LINT_FIXTURES ? [] : ['lint-fixtures/**']),
  ],
  rules: {
    'scale-unlimited/declaration-strict-value': [
      [
        '/color$/',
        '/^background/',
        '/radius$/',
        '/shadow$/',
        'font-size',
        '/^border($|-(top|right|bottom|left)$)/',
        'outline',
        'text-decoration',
        'column-rule',
        'fill',
        'stroke',
      ],
      { ignoreFunctions: false, disableFix: true },
    ],
  },
  overrides: [
    {
      files: ['src/design/tokens.css'],
      rules: { 'scale-unlimited/declaration-strict-value': null },
    },
    {
      files: ['src/features/**/*.module.css', 'lint-fixtures/features/**/*.module.css'],
      rules: {
        'property-allowed-list': [
          'display',
          '/^gap$/',
          '/^row-gap$/',
          '/^column-gap$/',
          '/^grid/',
          '/^margin/',
          '/^padding/',
          '/^width$/',
        ],
      },
    },
  ],
};
