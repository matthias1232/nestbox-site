import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

// The terminal and desktop libraries are large and only needed once a console opens, so they
// get their own chunks; solid-js is split out so it stays cached across app releases.
export default defineConfig({
  plugins: [solid()],
  build: {
    chunkSizeWarningLimit: 600,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes('/node_modules/@xterm/')) return 'xterm';
          if (id.includes('/node_modules/@novnc/')) return 'novnc';
          if (id.includes('/node_modules/solid-js/')) return 'solid';
        },
      },
    },
  },
});
