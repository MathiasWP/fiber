const STORAGE_KEY = 'fiber:editor-font-size';

/** Matches the size the editors were fixed at before this was adjustable. */
export const DEFAULT_FONT_SIZE = 13;
/** Below this it stops being readable; above it, a response is all scrollbar. */
const MIN = 9;
const MAX = 28;
const STEP = 1;

/**
 * How large text is in the body and response editors.
 *
 * One size for both rather than one per pane: they sit side by side, and a
 * response at 20px next to a request at 13px reads as a mistake. ⌘+, ⌘- and ⌘0
 * drive it from anywhere in the app, which is where the shortcuts are expected
 * to work from.
 */
class EditorFont {
	size = $state(DEFAULT_FONT_SIZE);

	/** Reads the stored size. Returns nothing; call it once, from an effect. */
	init(): void {
		const stored = Number(localStorage.getItem(STORAGE_KEY));
		if (Number.isFinite(stored) && stored >= MIN && stored <= MAX) {
			this.size = stored;
		}
	}

	#set(next: number): void {
		this.size = Math.min(MAX, Math.max(MIN, next));
		localStorage.setItem(STORAGE_KEY, String(this.size));
	}

	bigger(): void {
		this.#set(this.size + STEP);
	}

	smaller(): void {
		this.#set(this.size - STEP);
	}

	reset(): void {
		this.#set(DEFAULT_FONT_SIZE);
	}
}

export const editorFont = new EditorFont();
