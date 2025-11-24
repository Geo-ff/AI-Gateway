import { fileURLToPath, URL } from 'node:url'

import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import vueDevTools from 'vite-plugin-vue-devtools'

export default defineConfig({
  plugins: [
    vue(),
    vueDevTools(),
  ],
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    },
  },
  server: {
    proxy: {
      '/auth': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      '/providers': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      '/models': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      '/admin': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
      '/v1': {
        target: 'http://127.0.0.1:8080',
        changeOrigin: true,
      },
    },
  },
})
