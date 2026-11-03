// Rebuild from the repository root: node docs/design/preview/build.mjs
// Review locally: python3 docs/design/preview/serve.py
import { build } from "../../../apps/web/node_modules/vite/dist/node/index.js";
import react, {
  reactCompilerPreset,
} from "../../../apps/web/node_modules/@vitejs/plugin-react/dist/index.js";
import babel from "../../../apps/web/node_modules/@rolldown/plugin-babel/dist/index.mjs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { readFile, writeFile, mkdir } from "node:fs/promises";
const here = dirname(fileURLToPath(import.meta.url));
const web = resolve(here, "../../../apps/web");
const result = await build({
  configFile: false,
  root: web,
  publicDir: false,
  resolve: {
    alias: {
      react: resolve(web, "node_modules/react"),
      "react-dom": resolve(web, "node_modules/react-dom"),
      "@tanstack/react-query": resolve(
        web,
        "node_modules/@tanstack/react-query",
      ),
    },
  },
  define: { "process.env.NODE_ENV": JSON.stringify("production") },
  plugins: [
    {
      name: "design-only-services",
      enforce: "pre",
      resolveId(source, importer) {
        if (!importer || !source.startsWith(".")) return;
        const absolute = resolve(dirname(importer), source).replace(
          /\.(ts|tsx)$/,
          "",
        );
        for (const name of ["events", "storage", "desktopShell"])
          if (absolute === resolve(web, "src/lib", name))
            return resolve(here, `${name}.ts`);
      },
    },
    react(),
    babel({ presets: [reactCompilerPreset()] }),
  ],
  build: {
    write: false,
    assetsInlineLimit: Infinity,
    cssCodeSplit: false,
    minify: true,
    lib: {
      entry: resolve(here, "entry.ts"),
      formats: ["iife"],
      name: "MeerkatDesign",
    },
  },
});
const output = (Array.isArray(result) ? result : [result]).flatMap(
  (part) => part.output,
);
const js = output
  .filter((part) => part.type === "chunk")
  .map((part) => part.code)
  .join("\n");
const css = output
  .filter((part) => part.type === "asset" && part.fileName.endsWith(".css"))
  .map((part) => part.source)
  .join("\n");
await writeFile(
  resolve(here, "../prototype.html"),
  `<!doctype html>\n<!-- Generated from the current app by preview/build.mjs. Edit preview sources, then rebuild. -->\n<html lang="zh"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Meerkat</title><style>${css}</style></head><body><div id="root"></div><script>${js.replace(/<\/script/gi, "<\\/script")}</script></body></html>\n`,
);
console.log("Wrote docs/design/prototype.html");

// Compare the design against Vite's ordinary production app build with the same
// disposable content. This reference is written outside the repository.
if (process.argv.includes("--reference")) {
  const refResult = await build({
    configFile: false,
    root: web,
    publicDir: false,
    resolve: {
      alias: {
        react: resolve(web, "node_modules/react"),
        "@tanstack/react-query": resolve(
          web,
          "node_modules/@tanstack/react-query",
        ),
      },
    },
    define: { "process.env.NODE_ENV": JSON.stringify("production") },
    build: {
      write: false,
      assetsInlineLimit: Infinity,
      minify: true,
      lib: {
        entry: resolve(here, "transport.ts"),
        formats: ["iife"],
        name: "DesignData",
      },
    },
  });
  const referenceData = (Array.isArray(refResult) ? refResult : [refResult])
    .flatMap((r) => r.output)
    .filter((p) => p.type === "chunk")
    .map((p) => p.code)
    .join("\n");
  const prelude = `Object.defineProperty(navigator,'userAgent',{value:navigator.userAgent+' MeerkatDesktop/0.1'});localStorage.setItem('meerkat.language','"zh"');localStorage.setItem('meerkat.theme','"light"');localStorage.removeItem('meerkat.workspace');window.__TAURI_INTERNALS__={invoke:async()=>null};window.WebSocket=class{close(){}};`;
  const reference = (await readFile(resolve(web, "dist/index.html"), "utf8"))
    .replace(
      "<head>",
      "<head><script>" +
        prelude +
        referenceData.replace(/<\/script/gi, "<\\/script") +
        "</script>",
    )
    .replaceAll("/assets/", "/reference/assets/");
  await mkdir("/tmp/meerkat-design-review", { recursive: true });
  await writeFile("/tmp/meerkat-design-review/reference.html", reference);
}
