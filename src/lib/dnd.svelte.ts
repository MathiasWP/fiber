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
import type { Attachment } from 'svelte/attachments';

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

/**
 * What's under the pointer right now.
 *
 * Reported from a single monitor rather than by each row, because "below row 1"
 * and "above row 2" are the same gap: letting each row decide made the line jump
 * between two names for one position as the pointer crossed the midpoint.
 */
export type DragHint =
	| { kind: 'request'; sectionId: string; requestId: string; edge: VerticalEdge }
	| { kind: 'section'; sectionId: string; edge: VerticalEdge }
	| { kind: 'into'; sectionId: string }
	| null;

function isRequest(data: Record<string, unknown>): data is Record<string, unknown> & RequestRef {
	return data[REQUEST] === true;
}

function isSection(data: Record<string, unknown>): data is Record<string, unknown> & SectionRef {
	return data[SECTION] === true;
}

/** A request row: draggable, and a target for reordering around it. */
export function requestRow(ref: RequestRef): Attachment<HTMLElement> {
	return (node) =>
		combine(
			draggable({
				element: node,
				getInitialData: () => ({ [REQUEST]: true, ...ref })
			}),
			dropTargetForElements({
				element: node,
				canDrop: ({ source }) => isRequest(source.data),
				getData: ({ input, element }) =>
					attachClosestEdge(
						{ [REQUEST]: true, ...ref },
						{
							input,
							element,
							allowedEdges: ['top', 'bottom']
						}
					)
			})
		);
}

/**
 * A collection header: draggable to reorder, and a target for both collections
 * (reorder around it) and requests (move into it).
 */
export function sectionHeader(ref: SectionRef): Attachment<HTMLElement> {
	return (node) =>
		combine(
			draggable({
				element: node,
				getInitialData: () => ({ [SECTION]: true, ...ref })
			}),
			dropTargetForElements({
				element: node,
				canDrop: ({ source }) => isRequest(source.data) || isSection(source.data),
				getData: ({ input, element }) =>
					attachClosestEdge(
						{ [SECTION]: true, ...ref },
						{
							input,
							element,
							allowedEdges: ['top', 'bottom']
						}
					)
			})
		);
}

/** What the target under the pointer means, or null when there isn't one. */
function hintFrom(
	source: Record<string, unknown>,
	target: { data: Record<string, unknown> } | undefined
): DragHint {
	if (!target) return null;

	if (isRequest(target.data)) {
		// A row hovering itself would offer to move it where it already is.
		if (isRequest(source) && source.requestId === target.data.requestId) return null;
		const edge = verticalEdge(target.data);
		return edge
			? {
					kind: 'request',
					sectionId: target.data.sectionId,
					requestId: target.data.requestId,
					edge
				}
			: null;
	}

	if (isSection(target.data)) {
		if (isRequest(source)) {
			// A request dropped on a header joins that collection at the end, so
			// there's no gap to point at — the header itself is the target.
			return source.sectionId === target.data.sectionId
				? null
				: { kind: 'into', sectionId: target.data.sectionId };
		}
		if (isSection(source) && source.sectionId === target.data.sectionId) return null;
		const edge = verticalEdge(target.data);
		return edge ? { kind: 'section', sectionId: target.data.sectionId, edge } : null;
	}

	return null;
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
 * One monitor for the whole sidebar rather than a handler per row: both the
 * indicator and the drop are resolved in a single place, from the source and
 * whichever target is under the pointer.
 */
export function watchDrag(handlers: {
	onHint: (hint: DragHint) => void;
	onDrop: (outcome: DropOutcome) => void;
}): () => void {
	const { onHint, onDrop } = handlers;

	return monitorForElements({
		onDrag: ({ source, location }) =>
			onHint(hintFrom(source.data, location.current.dropTargets[0])),
		onDragStart: () => onHint(null),
		onDrop: ({ source, location }) => {
			onHint(null);
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
