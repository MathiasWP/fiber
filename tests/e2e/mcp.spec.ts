import { expect, test, type Page } from '@playwright/test';
import { commands, install, mcpClient, section, type MockOptions } from './mock-ipc';

async function openMcp(page: Page, options: MockOptions = {}) {
	await install(page, { sections: [section()], ...options });
	await page.goto('/');
	await page.getByRole('button', { name: 'MCP', exact: true }).click();
}

test.describe('the MCP tab', () => {
	test('reads the client configs only once the tab is opened', async ({ page }) => {
		await install(page, { sections: [section()], mcpClients: [mcpClient()] });
		await page.goto('/');
		await expect(page.getByText('List users')).toBeHidden();

		// Nothing on this tab is worth touching another program's files for
		// at startup.
		expect(await commands(page, 'mcp_clients')).toHaveLength(0);

		await page.getByRole('button', { name: 'MCP', exact: true }).click();
		await expect(page.getByText('~/.claude.json')).toBeVisible();
		expect(await commands(page, 'mcp_clients')).toHaveLength(1);
	});

	test('Add writes the entry and the row turns into Remove', async ({ page }) => {
		await openMcp(page, { mcpClients: [mcpClient(), mcpClient({ id: 'cursor', name: 'Cursor' })] });

		const row = page.locator('div', { hasText: '~/.claude.json' });
		await page.getByRole('button', { name: 'Add' }).first().click();

		expect(await commands(page, 'mcp_install')).toEqual([
			{ cmd: 'mcp_install', args: { id: 'claude-code' } }
		]);
		await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible();
		// Cursor is untouched: one row changing does not re-read the rest.
		await expect(page.getByRole('button', { name: 'Add' })).toHaveCount(1);
		await expect(row.first()).toBeVisible();
	});

	test('Remove takes it out again', async ({ page }) => {
		await openMcp(page, { mcpClients: [mcpClient({ state: 'installed' })] });

		await page.getByRole('button', { name: 'Remove' }).click();
		expect(await commands(page, 'mcp_uninstall')).toEqual([
			{ cmd: 'mcp_uninstall', args: { id: 'claude-code' } }
		]);
		await expect(page.getByRole('button', { name: 'Add' })).toBeVisible();
	});

	test('an entry pointing at another copy offers an update and says what it runs', async ({
		page
	}) => {
		await openMcp(page, {
			mcpClients: [mcpClient({ state: 'outdated', command: '/Users/old/Fiber.app/f' })]
		});

		await expect(page.getByText('/Users/old/Fiber.app/f')).toBeVisible();
		await page.getByRole('button', { name: 'Update' }).click();
		expect(await commands(page, 'mcp_install')).toHaveLength(1);
		await expect(page.getByRole('button', { name: 'Remove' })).toBeVisible();
	});

	test('a config that cannot be parsed offers the snippet instead of a button', async ({
		page
	}) => {
		await openMcp(page, {
			mcpClients: [
				mcpClient({
					id: 'vscode',
					name: 'VS Code',
					path: '~/Library/Application Support/Code/User/mcp.json',
					state: 'unreadable',
					message: 'mcp.json could not be parsed, so it was left alone'
				})
			]
		});

		await expect(page.getByText('could not be parsed')).toBeVisible();
		await expect(page.getByRole('button', { name: 'Add' })).toHaveCount(0);

		await page.getByRole('button', { name: 'Copy JSON' }).first().click();
		await expect
			.poll(() => page.evaluate(() => navigator.clipboard.readText()))
			.toContain('"command": "/Applications/Fiber.app/Contents/MacOS/fiber"');
	});

	test('a client that is not installed is still listed, marked as not found', async ({ page }) => {
		await openMcp(page, { mcpClients: [mcpClient({ id: 'windsurf', name: 'Windsurf', detected: false })] });

		await expect(page.getByText('not found')).toBeVisible();
		await expect(page.getByRole('button', { name: 'Add' })).toBeEnabled();
	});

	test('the snippet carries this binary, and the path copies on its own', async ({ page }) => {
		await openMcp(page, { mcpClients: [], mcpBinary: '/opt/fiber/fiber' });

		await expect(page.getByText('"args": [')).toBeVisible();
		await page.getByRole('button', { name: 'Copy path' }).click();
		await expect
			.poll(() => page.evaluate(() => navigator.clipboard.readText()))
			.toBe('/opt/fiber/fiber');
	});

	test('ToolHive is offered as a command, not a button', async ({ page }) => {
		await openMcp(page, { mcpClients: [] });

		// It needs ToolHive and a container runtime, so the tab hands over the
		// command rather than pretending it can run it.
		await expect(page.getByText('runs as a container under ToolHive')).toBeVisible();
		await page.getByRole('button', { name: 'Copy command' }).click();
		await expect
			.poll(() => page.evaluate(() => navigator.clipboard.readText()))
			.toBe(
				'curl -fsSL https://raw.githubusercontent.com/MathiasWP/fiber/main/scripts/toolhive.sh | bash'
			);

		await page.getByRole('button', { name: 'Read the guide' }).click();
		expect(await commands(page, 'plugin:opener|open_url')).toEqual([
			{
				cmd: 'plugin:opener|open_url',
				args: {
					url: 'https://github.com/MathiasWP/fiber/blob/main/deploy/toolhive.md',
					with: undefined
				}
			}
		]);
	});

	test('a failed write is reported rather than swallowed', async ({ page }) => {
		await openMcp(page, {
			mcpClients: [mcpClient()],
			mcpWriteError: 'could not write ~/.claude.json: permission denied'
		});

		await page.getByRole('button', { name: 'Add' }).click();
		await expect(page.getByText('permission denied')).toBeVisible();
		// The row keeps its old state: nothing pretends the write happened.
		await expect(page.getByRole('button', { name: 'Add' })).toBeEnabled();
	});

	test('leaving History for MCP stops the entry overriding the response pane', async ({ page }) => {
		await install(page, { sections: [section()], mcpClients: [mcpClient()] });
		await page.goto('/');

		await page.getByRole('button', { name: 'History', exact: true }).click();
		await page.getByRole('button', { name: 'MCP', exact: true }).click();
		await expect(page.getByText('~/.claude.json')).toBeVisible();
	});
});
