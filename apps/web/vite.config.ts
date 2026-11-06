import { defineConfig } from 'vite';
import react, { reactCompilerPreset } from '@vitejs/plugin-react';
import babel from '@rolldown/plugin-babel';
export default defineConfig({
  server: {
    proxy: {
      '/api': {
        target: process.env.MEERKAT_RUNTIME_ORIGIN ?? 'http://127.0.0.1:7440',
        changeOrigin: true,
        ws: true,
        configure(proxy, options) {
          const origin = new URL(String(options.target)).origin;
          proxy.on('proxyReq', (request) => request.setHeader('Origin', origin));
          proxy.on('proxyReqWs', (request) => request.setHeader('Origin', origin));
        },
      },
    },
  },
  plugins: [react(), babel({ presets: [reactCompilerPreset()] })],
});
