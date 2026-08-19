import { combine } from '@atlaskit/pragmatic-drag-and-drop/combine';
import {
	draggable,
	dropTargetForElements,
	monitorForElements
} from '@atlaskit/pragmatic-drag-and-drop/element/adapter';
import {
	attachClosestEdge,
	extractClosestEdge
} from '@atlaskit/pragmatic-drag-and-drop-hitbox/closest-edge';

/**
 * The hitbox reports all four edges; only these two mean anything in a vertical
 * list, so they're narrowed here rather than being cast at every call site.
 */
export type VerticalEdge = 'top' | 'bottom';

function verticalEdge(data: Record<string, unknown>): VerticalEdge | null {
	const edge = extractClosestEdge(data);
	return edge === 'top' || edge === 'bottom' ? edge : null;
}

/**
 * Dragging in the sidebar.
 *
 * Two kinds of thing move: a request, which can be reordered within its
 * collection or dropped into another one, and a collection, which can be
 * reordered among the others.
 *
 * Loaded endpoints deliberately aren't draggable. Their order comes from the
 * loader and is regenerated on every refresh, so any order you dragged them
 * into would silently disappear the next time it ran.
 */

const REQUEST = 'fiber/request';
const SECTION = 'fiber/section';

export interface RequestRef {
	sectionId: string;
	requestId: string;
}

export interface SectionRef {
	sectionId: string;
}

/** Where a dragged thing would land, for drawing the line. */
export type DropHint = { edge: VerticalEdge } | null;

function isRequest(data: Record<string, unknown>): data is Record<string, unknown> & RequestRef {
	return data[REQUEST] === true;
}

function isSection(data: Record<string, unknown>): data is Record<string, unknown> & SectionRef {
	return data[SECTION] === true;
}

interface RowOptions {
	ref: RequestRef;
	/** Called with the edge a drop would land on, or null when not hovered. */
	onHint: (hint: DropHint) => void;
}

/** A request row: draggable, and a target for reordering around it. */
export function requestRow(node: HTMLElement, options: RowOptions) {
	let current = options;

	const cleanup = combine(
		draggable({
			element: node,
			getInitialData: () => ({ [REQUEST]: true, ...current.ref })
		}),
		dropTargetForElements({
			element: node,
			canDrop: ({ source }) => isRequest(source.data),
			getData: ({ input, element }) =>
				attachClosestEdge({ [REQUEST]: true, ...current.ref }, {
					input,
					element,
					allowedEdges: ['top', 'bottom']
				}),
			onDrag: ({ self, source }) => {
				// Hovering a row over itself would draw a line either side of the
				// thing you're holding, which reads as a move that isn't one.
				if (isRequest(source.data) && source.data.requestId === current.ref.requestId) {
					current.onHint(null);
					return;
				}
				const edge = verticalEdge(self.data);
				current.onHint(edge ? { edge } : null);
			},
			onDragLeave: () => current.onHint(null),
			onDrop: () => current.onHint(null)
		})
	);

	return {
		update(next: RowOptions) {
			current = next;
		},
		destroy: cleanup
	};
}

interface HeaderOptions {
	ref: SectionRef;
	/** Reordering collections, or dropping a request into this one. */
	onHint: (hint: DropHint | 'into') => void;
}

/**
 * A collection header: draggable to reorder, and a target for both collections
 * (reorder around it) and requests (move into it).
 */
export function sectionHeader(node: HTMLElement, options: HeaderOptions) {
	let current = options;

	const cleanup = combine(
		draggable({
			element: node,
			getInitialData: () => ({ [SECTION]: true, ...current.ref })
		}),
		dropTargetForElements({
			element: node,
			canDrop: ({ source }) => isRequest(source.data) || isSection(source.data),
			getData: ({ input, element }) =>
				attachClosestEdge({ [SECTION]: true, ...current.ref }, {
					input,
					element,
					allowedEdges: ['top', 'bottom']
				}),
			onDrag: ({ self, source }) => {
				if (isRequest(source.data)) {
					// A request dropped on a header joins that collection at the end,
					// so there's no edge to show — highlight the whole header instead.
					current.onHint(source.data.sectionId === current.ref.sectionId ? null : 'into');
					return;
				}
				if (isSection(source.data) && source.data.sectionId === current.ref.sectionId) {
					current.onHint(null);
					return;
				}
				const edge = verticalEdge(self.data);
				current.onHint(edge ? { edge } : null);
			},
			onDragLeave: () => current.onHint(null),
			onDrop: () => current.onHint(null)
		})
	);

	return {
		update(next: HeaderOptions) {
			current = next;
		},
		destroy: cleanup
	};
}

export interface DropOutcome {
	/** Moving a request, either within a collection or into another. */
	request?: {
		from: RequestRef;
		/** Land before this request, or at the end of `sectionId` when absent. */
		to: { sectionId: string; requestId?: string; edge?: VerticalEdge };
	};
	/** Reordering collections. */
	section?: { movedId: string; targetId: string; edge: VerticalEdge };
}

/**
 * One monitor for the whole sidebar rather than a handler per row: the drop is
 * resolved once, from the source and the target that was under the pointer.
 */
export function watchDrops(onDrop: (outcome: DropOutcome) => void): () => void {
	return monitorForElements({
		onDrop: ({ source, location }) => {
			const target = location.current.dropTargets[0];
			if (!target) return;

			if (isRequest(source.data)) {
				const from: RequestRef = {
					sectionId: source.data.sectionId,
					requestId: source.data.requestId
				};

				if (isRequest(target.data)) {
					if (target.data.requestId === from.requestId) return;
					onDrop({
						request: {
							from,
							to: {
								sectionId: target.data.sectionId,
								requestId: target.data.requestId,
								edge: verticalEdge(target.data) ?? undefined
							}
						}
					});
					return;
				}

				if (isSection(target.data) && target.data.sectionId !== from.sectionId) {
					onDrop({ request: { from, to: { sectionId: target.data.sectionId } } });
				}
				return;
			}

			if (isSection(source.data) && isSection(target.data)) {
				if (source.data.sectionId === target.data.sectionId) return;
				const edge = verticalEdge(target.data);
				if (!edge) return;
				onDrop({
					section: {
						movedId: source.data.sectionId,
						targetId: target.data.sectionId,
						edge
					}
				});
			}
		}
	});
}
