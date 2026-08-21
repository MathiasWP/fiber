import { expect, test } from '@playwright/test';
import { install, section } from './mock-ipc';

async function openBody(page: import('@playwright/test').Page, body: string) {
	await install(page, {
		sections: [
			section({
				requests: [
					{ id: 'r1', name: 'Create', method: 'POST', path: '/users', body, headers: [] }
				]
			})
		]
	});
	await page.goto('/');
	await page.getByText('Create').click();
	await page.locator('.cm-content').first().click();
	await page.keyboard.press('Control+Home');
}

const doc = (page: import('@playwright/test').Page) =>
	page.locator('.cm-content').first().evaluate((el) => (el as HTMLElement).innerText);
const selected = (page: import('@playwright/test').Page) =>
	page.evaluate(() => window.getSelection()?.toString() ?? '');

test('the comma that means "next field" is not left in the body twice', async ({ page }) => {
	await openBody(page, '{\n  "age": number,\n  "size": number\n}');
	await page.keyboard.press('Tab');
	await page.keyboard.type('1');
	await page.keyboard.type(',');

	expect(await doc(page)).toBe('{\n  "age": 1,\n  "size": number\n}');
	expect(await selected(page)).toBe('number');
});

test('a comma inside a string value stays in the string', async ({ page }) => {
	await openBody(page, '{\n  "name": string,\n  "city": string\n}');
	await page.keyboard.press('Tab');
	await page.keyboard.type('"Ada, Lovelace');

	expect(await doc(page)).toBe('{\n  "name": "Ada, Lovelace",\n  "city": string\n}');
});

test('closing the quote is what finishes a string, and then a comma moves on', async ({
	page
}) => {
	await openBody(page, '{\n  "name": string,\n  "age": number\n}');
	await page.keyboard.press('Tab');
	// The opening quote wraps the placeholder; the closing one steps over the
	// auto-closed quote rather than adding a second.
	await page.keyboard.type('"Ada"');
	await page.keyboard.type(',');

	await expect.poll(() => doc(page)).toBe('{\n  "name": "Ada",\n  "age": number\n}');
	expect(await selected(page)).toBe('number');
});

test('unknown is a gap you can tab to', async ({ page }) => {
	await openBody(page, '{\n  "a": unknown,\n  "b": string\n}');
	await page.keyboard.press('Tab');
	expect(await selected(page)).toBe('unknown');
	await page.keyboard.press('Tab');
	expect(await selected(page)).toBe('string');
});

test('tab still selects the whole placeholder, not its end', async ({ page }) => {
	await openBody(page, '{\n  "a": string,\n  "b": number,\n  "c": boolean\n}');
	for (const expected of ['string', 'number', 'boolean', 'string']) {
		await page.keyboard.press('Tab');
		expect(await selected(page)).toBe(expected);
	}
});

/**
 * The field's own tint paints over CodeMirror's selection highlight, so a
 * forward selection looked like an untouched field with a caret parked after
 * it. The caret sitting at the front is what says "type here".
 */
test('the caret lands at the front of the field, not after it', async ({ page }) => {
	await openBody(page, '{\n  "a": string\n}');
	await page.keyboard.press('Tab');

	// Which side of the field, not which pixel: the runners disagree about font
	// metrics, and an exact left edge is a test about Chromium's text layout.
	// Polled because the caret element is positioned a frame after the
	// selection changes.
	await expect
		.poll(() =>
			page.evaluate(() => {
				const slot = document.querySelector('.cm-slot')!.getBoundingClientRect();
				const caret = document.querySelector('.cm-cursor-primary')!.getBoundingClientRect();
				const middle = (slot.left + slot.right) / 2;
				return caret.left < middle ? 'front' : 'back';
			})
		)
		.toBe('front');

	// Still the whole field, so typing replaces it.
	expect(await selected(page)).toBe('string');
});
