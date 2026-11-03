import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  base: './',
  server: { port: 1420, strictPort: true },
  build: { target: 'es2022' },
});
