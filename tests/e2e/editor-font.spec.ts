import { expect, test } from '@playwright/test';
import { install, response, savedRequest, section } from './mock-ipc';

/**
 * ⌘+/⌘-/⌘0 resize whichever editor last had focus (`src/lib/editor.svelte.ts`).
 * `request.spec.ts` already covers the basic grow-then-reset round trip; this
 * covers the rest: shrinking, clamping at both ends, persistence across a
 * reload, and that the request and response editors size independently.
 */

function fontSize(page: import('@playwright/test').Page, index: number) {
	return page.locator('.cm-editor').nth(index).evaluate((node) => getComputedStyle(node).fontSize);
}

test('⌘- shrinks the last-focused editor', async ({ page }) => {
	await install(page, {
		sections: [
			section({ requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })] })
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	const editor = page.locator('.cm-editor').first();
	await editor.click();
	const before = await fontSize(page, 0);
	await page.keyboard.press('ControlOrMeta+-');
	const after = await fontSize(page, 0);
	expect(parseFloat(after)).toBeLessThan(parseFloat(before));
});

test('shrinking stops at the minimum size rather than going unreadable', async ({ page }) => {
	await install(page, {
		sections: [
			section({ requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })] })
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	const editor = page.locator('.cm-editor').first();
	await editor.click();
	for (let i = 0; i < 40; i++) {
		await page.keyboard.press('ControlOrMeta+-');
	}
	const min = await fontSize(page, 0);

	// One more press changes nothing further — the floor holds.
	await page.keyboard.press('ControlOrMeta+-');
	await expect.poll(() => fontSize(page, 0)).toBe(min);
});

test('growing stops at the maximum size', async ({ page }) => {
	await install(page, {
		sections: [
			section({ requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })] })
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	const editor = page.locator('.cm-editor').first();
	await editor.click();
	for (let i = 0; i < 40; i++) {
		await page.keyboard.press('ControlOrMeta+=');
	}
	const max = await fontSize(page, 0);

	await page.keyboard.press('ControlOrMeta+=');
	await expect.poll(() => fontSize(page, 0)).toBe(max);
});

test('the size survives a reload', async ({ page }) => {
	await install(page, {
		sections: [
			section({ requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })] })
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();

	const editor = page.locator('.cm-editor').first();
	await editor.click();
	const before = await fontSize(page, 0);
	await page.keyboard.press('ControlOrMeta+=');
	await page.keyboard.press('ControlOrMeta+=');
	const grown = await fontSize(page, 0);
	expect(grown).not.toBe(before);

	await page.reload();
	await page.getByText('List users').click();
	await expect.poll(() => fontSize(page, 0)).toBe(grown);
});

test('the request and response editors size independently', async ({ page }) => {
	await install(page, {
		sendResponse: response({ body: '{"hello":"world"}' }),
		sections: [
			section({ requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })] })
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByText('200 OK')).toBeVisible();

	const requestEditor = page.locator('.cm-editor').nth(0);
	const responseEditor = page.locator('.cm-editor').nth(1);
	const requestBefore = await fontSize(page, 0);
	const responseBefore = await fontSize(page, 1);

	// Grow only the response pane.
	await responseEditor.click();
	await page.keyboard.press('ControlOrMeta+=');
	await page.keyboard.press('ControlOrMeta+=');

	await expect.poll(() => fontSize(page, 1)).not.toBe(responseBefore);
	expect(await fontSize(page, 0)).toBe(requestBefore);

	// Growing the request pane now leaves the response pane where it is.
	await requestEditor.click();
	const responseGrown = await fontSize(page, 1);
	await page.keyboard.press('ControlOrMeta+=');
	await expect.poll(() => fontSize(page, 0)).not.toBe(requestBefore);
	expect(await fontSize(page, 1)).toBe(responseGrown);
});

test('⌘0 resets only the last-focused editor to the default', async ({ page }) => {
	await install(page, {
		sendResponse: response({ body: '{"hello":"world"}' }),
		sections: [
			section({ requests: [savedRequest({ method: 'POST', path: '/users', body: '{"a":1}' })] })
		]
	});
	await page.goto('/');
	await page.getByText('List users').click();
	await page.getByRole('button', { name: 'Send' }).click();
	await expect(page.getByText('200 OK')).toBeVisible();

	const requestEditor = page.locator('.cm-editor').nth(0);
	const responseEditor = page.locator('.cm-editor').nth(1);

	await requestEditor.click();
	const requestDefault = await fontSize(page, 0);
	await page.keyboard.press('ControlOrMeta+=');
	await page.keyboard.press('ControlOrMeta+=');

	await responseEditor.click();
	const responseDefault = await fontSize(page, 1);
	await page.keyboard.press('ControlOrMeta+=');

	await responseEditor.click();
	await page.keyboard.press('ControlOrMeta+0');
	await expect.poll(() => fontSize(page, 1)).toBe(responseDefault);
	// The request pane, grown earlier and never reset, keeps its size.
	expect(await fontSize(page, 0)).not.toBe(requestDefault);
});
