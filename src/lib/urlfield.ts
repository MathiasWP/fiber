/**
 * Word jumps that stop where a URL has joints.
 *
 * macOS treats `app.staging.example.com` as a single word, so ⌥→ from the
 * middle of a host skips the whole thing and lands at the end. A browser's
 * address bar doesn't do that — it stops at every dot, slash and dash, which is
 * what makes a long URL editable rather than something you retype. This gives
 * the fields that hold a URL the address bar's behaviour instead.
 *
 * Movement only. ⌥⌫ is deliberately left alone: deleting by these boundaries
 * would mean rewriting the value ourselves, which takes the edit out of the
 * browser's own undo stack, and losing ⌘Z is the worse trade.
 *
 * Only `alt` is intercepted. That is the modifier macOS uses, and it is the one
 * platform where the native behaviour is wrong — Windows and Linux jump on
 * `ctrl` and already stop at punctuation, so they are left as they are.
 */

import type { Attachment } from 'svelte/attachments';

/** Letters and digits make up a word; every other character is a joint. */
const WORD = /[\p{L}\p{N}]/u;

/** The next boundary at or after `from`: past any joints, then past a word. */
export function forward(value: string, from: number): number {
	let i = from;
	while (i < value.length && !WORD.test(value[i])) i++;
	while (i < value.length && WORD.test(value[i])) i++;
	return i;
}

/** The previous boundary at or before `from`, by the same rule, reversed. */
export function backward(value: string, from: number): number {
	let i = from;
	while (i > 0 && !WORD.test(value[i - 1])) i--;
	while (i > 0 && WORD.test(value[i - 1])) i--;
	return i;
}

export const urlField: Attachment<HTMLInputElement> = (node) => {
	function onKeydown(event: KeyboardEvent) {
		if (!event.altKey || event.metaKey || event.ctrlKey) return;
		if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;

		const { value, selectionStart, selectionEnd, selectionDirection } = node;
		if (selectionStart === null || selectionEnd === null) return;

		const back = event.key === 'ArrowLeft';

		// Which end of the selection moves, and which one stays behind. Shift
		// drags one end and pins the other; without it the caret just lands.
		const reversed = selectionDirection === 'backward' && selectionStart !== selectionEnd;
		const focus = event.shiftKey
			? reversed
				? selectionStart
				: selectionEnd
			: back
				? selectionStart
				: selectionEnd;
		const anchor = reversed ? selectionEnd : selectionStart;

		const next = back ? backward(value, focus) : forward(value, focus);

		event.preventDefault();
		if (event.shiftKey) {
			node.setSelectionRange(
				Math.min(anchor, next),
				Math.max(anchor, next),
				next < anchor ? 'backward' : 'forward'
			);
		} else {
			node.setSelectionRange(next, next);
		}
	}

	node.addEventListener('keydown', onKeydown);
	return () => node.removeEventListener('keydown', onKeydown);
};
