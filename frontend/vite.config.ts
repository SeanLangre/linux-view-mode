import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    host: '127.0.0.1',
    // The Rust backend (`cargo run` in the repo root).
    proxy: { '/api': 'http://127.0.0.1:7878' },
  },
})
