/**
 * Which editor a size belongs to.
 *
 * Two sizes rather than one, because the panes are not read the same way: a
 * response is usually skimmed and wants to fit, while a body is typed into and
 * wants to be legible. Wanting the response smaller than the request is the
 * common case, not an edge one.
 */
export type EditorScope = 'request' | 'response';

/** Matches the size the editors were fixed at before this was adjustable. */
export const DEFAULT_FONT_SIZE = 13;
/** Below this it stops being readable; above it, a response is all scrollbar. */
const MIN = 9;
const MAX = 28;
const STEP = 1;

const storageKey = (scope: EditorScope) => `fiber:editor-font-size:${scope}`;

class EditorFont {
	#sizes = $state<Record<EditorScope, number>>({
		request: DEFAULT_FONT_SIZE,
		response: DEFAULT_FONT_SIZE
	});

	/**
	 * The editor ⌘+ acts on: whichever was last focused.
	 *
	 * Defaults to the response because that is the one people reach for the
	 * shortcut to shrink, and because on a fresh window neither has been touched
	 * yet — so the shortcut has to mean *something* before you have clicked
	 * anywhere.
	 */
	active = $state<EditorScope>('response');

	size(scope: EditorScope): number {
		return this.#sizes[scope];
	}

	/** Reads both stored sizes. Call once, from an effect. */
	init(): void {
		for (const scope of ['request', 'response'] as const) {
			const stored = Number(localStorage.getItem(storageKey(scope)));
			if (Number.isFinite(stored) && stored >= MIN && stored <= MAX) {
				this.#sizes[scope] = stored;
			}
		}
	}

	#set(scope: EditorScope, next: number): void {
		const size = Math.min(MAX, Math.max(MIN, next));
		this.#sizes[scope] = size;
		localStorage.setItem(storageKey(scope), String(size));
	}

	bigger(): void {
		this.#set(this.active, this.#sizes[this.active] + STEP);
	}

	smaller(): void {
		this.#set(this.active, this.#sizes[this.active] - STEP);
	}

	reset(): void {
		this.#set(this.active, DEFAULT_FONT_SIZE);
	}
}

export const editorFont = new EditorFont();
