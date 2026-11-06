# Meerkat site

The landing page at `meerkat.shens.dev`. A static Astro site, independent of the app.

```bash
pnpm install
pnpm dev      # http://127.0.0.1:4321
pnpm build    # outputs dist/
```

- Copy lives in `src/i18n/index.ts` (Chinese at `/`, English at `/en/`).
- Repository and download links live in `src/site.ts`.
- Art in `src/assets` is copied from the app; the site never imports app code.
- Fonts are bundled: Geist Mono for Latin, Noto Sans SC for Chinese, both OFL.
  After changing copy, run `pnpm fonts` (needs `uv`) to re-subset them into `public/fonts`.
- The Feishu QR code in the setup section is real and opens the repository.
