/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  server: {
    // Docker(特にWindows/macOSホスト上のDocker Desktop)経由のbindマウントでは
    // ネイティブのファイルシステムイベントが伝播しないことが多いため、
    // ポーリング監視にフォールバックする(docker-compose.dev.yml参照)。
    // ローカルで直接`npm run dev`する分には無害(ポーリングが少し重いだけ)
    watch: {
      usePolling: true,
    },
    // Traefik経由でLAN内の任意のドメイン(SAHAI_DOMAIN、ユーザーごとに異なる)で
    // アクセスされうるため、Viteの既定のHostヘッダーチェック(DNSリバインディング
    // 対策)を無効化する。この開発サーバー自体は外部公開せずTraefik配下のみ
    // からのアクセスに限られるため、無効化しても外部からは到達できない)
    allowedHosts: true,
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    globals: true,
  },
})
