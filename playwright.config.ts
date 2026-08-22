import { defineConfig, devices } from '@playwright/test';

/**
 * The app runs against a fake Tauri backend (see `tests/e2e/mock-ipc.ts`), so
 * this is the built frontend in a normal browser — no packaged binary, and it
 * runs the same on every platform.
 *
 * Chromium only, deliberately. Tauri renders in one webview per platform and
 * none of them is Firefox, so a second browser would buy coverage of something
 * the app never runs in.
 */
export default defineConfig({
	testDir: 'tests/e2e',
	fullyParallel: true,
	forbidOnly: !!process.env.CI,
	retries: process.env.CI ? 1 : 0,
	reporter: process.env.CI ? 'github' : 'list',
	use: {
		baseURL: 'http://localhost:4173',
		trace: 'on-first-retry',
		permissions: ['clipboard-read', 'clipboard-write']
	},
	projects: [{ name: 'chromium', use: { ...devices['Desktop Chrome'] } }],
	webServer: {
		command: 'pnpm build && pnpm preview --port 4173 --strictPort',
		url: 'http://localhost:4173',
		reuseExistingServer: !process.env.CI,
		timeout: 120_000
	}
});
