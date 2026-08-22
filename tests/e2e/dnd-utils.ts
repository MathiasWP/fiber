import type { Locator, Page } from '@playwright/test';

/**
 * Where the pointer lands on the target during the drop, which is what the
 * hitbox uses to pick an edge (or, for a row that isn't edge-aware, simply
 * lands anywhere inside it).
 */
export type DropSpot = 'top' | 'bottom' | 'center';

/**
 * Simulates a native HTML5 drag from `source` to `source`, landing at `spot`
 * on `target`.
 *
 * Pragmatic drag-and-drop (the library behind the sidebar's reordering) reads
 * real `DragEvent`s and a shared `DataTransfer`, not pointer events — so
 * Playwright's own `dragTo` (which only dispatches mouse events) never
 * triggers it. This dispatches the same event sequence a browser would during
 * a drag: `dragstart` on the source, then `dragenter`/`dragover`/`drop` on the
 * target at a specific point, then `dragend` on the source. The y-coordinate
 * for `dragover`/`drop` determines which edge the closest-edge hitbox reports.
 */
export async function dragAndDrop(
	page: Page,
	source: Locator,
	target: Locator,
	spot: DropSpot = 'center'
): Promise<void> {
	await source.waitFor();
	await target.waitFor();
	const src = await source.elementHandle();
	const tgt = await target.elementHandle();
	if (!src || !tgt) throw new Error('dragAndDrop: source or target element not found');

	await page.evaluate(
		([srcEl, tgtEl, spot]) => {
			const dt = new DataTransfer();
			const rect = (tgtEl as Element).getBoundingClientRect();
			const x = rect.left + rect.width / 2;
			const y =
				spot === 'top'
					? rect.top + Math.min(2, rect.height / 4)
					: spot === 'bottom'
						? rect.bottom - Math.min(2, rect.height / 4)
						: rect.top + rect.height / 2;

			const fire = (el: Element, type: string) => {
				const ev = new DragEvent(type, {
					bubbles: true,
					cancelable: true,
					clientX: x,
					clientY: y
				});
				Object.defineProperty(ev, 'dataTransfer', { value: dt });
				el.dispatchEvent(ev);
			};

			fire(srcEl as Element, 'dragstart');
			fire(tgtEl as Element, 'dragenter');
			fire(tgtEl as Element, 'dragover');
			fire(tgtEl as Element, 'drop');
			fire(srcEl as Element, 'dragend');
		},
		[src, tgt, spot]
	);
}

/**
 * Holds a drag over `target` without dropping, so a test can inspect the
 * drop-indicator classes the sidebar draws while the pointer hovers.
 *
 * Returns a `release` function that fires `dragend` on the source, cancelling
 * the drag without moving anything — mirroring letting go outside any valid
 * target, or pressing Escape.
 */
export async function dragOver(
	page: Page,
	source: Locator,
	target: Locator,
	spot: DropSpot = 'center'
): Promise<() => Promise<void>> {
	await source.waitFor();
	await target.waitFor();
	const src = await source.elementHandle();
	const tgt = await target.elementHandle();
	if (!src || !tgt) throw new Error('dragOver: source or target element not found');

	await page.evaluate(
		([srcEl, tgtEl, spot]) => {
			const dt = new DataTransfer();
			const rect = (tgtEl as Element).getBoundingClientRect();
			const x = rect.left + rect.width / 2;
			const y =
				spot === 'top'
					? rect.top + Math.min(2, rect.height / 4)
					: spot === 'bottom'
						? rect.bottom - Math.min(2, rect.height / 4)
						: rect.top + rect.height / 2;

			const fire = (el: Element, type: string) => {
				const ev = new DragEvent(type, {
					bubbles: true,
					cancelable: true,
					clientX: x,
					clientY: y
				});
				Object.defineProperty(ev, 'dataTransfer', { value: dt });
				el.dispatchEvent(ev);
			};

			(window as unknown as Record<string, unknown>).__dndScratchDataTransfer = dt;
			fire(srcEl as Element, 'dragstart');
			fire(tgtEl as Element, 'dragenter');
			fire(tgtEl as Element, 'dragover');
		},
		[src, tgt, spot]
	);

	return async () => {
		await page.evaluate((srcEl) => {
			const dt = (window as unknown as Record<string, unknown>).__dndScratchDataTransfer as DataTransfer;
			const ev = new DragEvent('dragend', { bubbles: true, cancelable: true });
			Object.defineProperty(ev, 'dataTransfer', { value: dt });
			(srcEl as Element).dispatchEvent(ev);
		}, src);
	};
}
