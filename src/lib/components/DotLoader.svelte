<script lang="ts">
	/**
	 * "Half Helix" — a 5×5 dot matrix with a sine strand travelling down it, the
	 * bright dots blooming. After sv-matrix's `square-17`, reimplemented here
	 * rather than vendored: that component pulls in seventeen files, a global
	 * stylesheet and `clsx` to support patterns and geometry we never ask for,
	 * and it assumes a shadcn-svelte layout this app doesn't have. The maths and
	 * the bloom below are its, unchanged.
	 *
	 * Colour comes from `currentColor`, so it inherits whatever text colour it
	 * lands in — and the glow with it.
	 */

	interface Props {
		/** Box size in px. The dots and their gaps are derived from it. */
		size?: number;
		/** Higher is faster; 1 is one 1600ms loop. */
		speed?: number;
		class?: string;
	}

	let { size = 28, speed = 2.5, class: className = '' }: Props = $props();

	// Five columns of dots and four gaps between them, in the ratio the original
	// uses at its default size (29px box, 5px dots).
	const dot = $derived((size * 5) / 29);
	const gap = $derived(size / 29);

	const ROWS = [0, 1, 2, 3, 4];
	const STEP_COUNT = 20;
	const HELIX_LOOP_RADIANS = (Math.PI * 2) / (STEP_COUNT - 1);
	const BASE_OPACITY = 0.08;
	const NEAR_STRAND_OPACITY = 0.24;
	const STRAND_OPACITY = 1;
	const CYCLE_MS = 1600;

	/** Position in the loop, 0–1. */
	let phase = $state(0);
	let reduced = $state(false);

	$effect(() => {
		const query = matchMedia('(prefers-reduced-motion: reduce)');
		reduced = query.matches;
		const onChange = (event: MediaQueryListEvent) => (reduced = event.matches);
		query.addEventListener('change', onChange);
		return () => query.removeEventListener('change', onChange);
	});

	$effect(() => {
		// A still frame is the whole animation when motion is unwelcome.
		if (reduced) {
			phase = 0;
			return;
		}

		const cycle = CYCLE_MS / speed;
		let frame = 0;
		const start = performance.now();

		const tick = (now: number) => {
			phase = ((now - start) % cycle) / cycle;
			frame = requestAnimationFrame(tick);
		};
		frame = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(frame);
	});

	/** Which column the strand passes through on this row, right now. */
	function strandColumn(row: number): number {
		const rowPhase = phase * STEP_COUNT * HELIX_LOOP_RADIANS + row * 1.24;
		return Math.round(2 + 2 * Math.sin(rowPhase));
	}

	function opacityAt(row: number, col: number): number {
		const strand = strandColumn(row);
		if (col === strand) return STRAND_OPACITY;
		if (Math.abs(col - strand) === 1) return NEAR_STRAND_OPACITY;
		return BASE_OPACITY;
	}
</script>

<span
	class="dmx inline-grid align-middle {className}"
	style="width:{size}px; height:{size}px; gap:{gap}px; --dmx-dot:{dot}px"
	role="status"
	aria-label="Loading"
>
	{#each ROWS as row (row)}
		{#each ROWS as col (col)}
			{@const level = opacityAt(row, col)}
			<span
				class="dmx-dot rounded-full bg-current"
				style="opacity:{level}; --dmx-level:{level}"
			></span>
		{/each}
	{/each}
</span>

<style>
	.dmx {
		grid-template-columns: repeat(5, minmax(0, 1fr));
		grid-template-rows: repeat(5, minmax(0, 1fr));
	}

	/* The bloom, verbatim from the original: two stacked drop-shadows scaled by
	   how bright the dot currently is, so only the strand actually glows. */
	.dmx-dot {
		filter: drop-shadow(0 0 calc(var(--dmx-dot) * 0.75 * var(--dmx-level, 0)) currentColor)
			drop-shadow(0 0 calc(var(--dmx-dot) * 1.35 * var(--dmx-level, 0)) currentColor);
		will-change: opacity, filter;
	}
</style>
