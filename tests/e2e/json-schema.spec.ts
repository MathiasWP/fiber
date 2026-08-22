import { expect, test } from '@playwright/test';
import { validateJsonBody } from '../../src/lib/json-schema';

test.describe('validateJsonBody', () => {
	test('reports a type mismatch', () => {
		expect(validateJsonBody({ type: 'boolean' }, 'null')).toEqual([
			'$ must be boolean, not null.'
		]);
	});

	test('requires listed properties', () => {
		expect(
			validateJsonBody({ type: 'object', required: ['enabled'] }, '{ "other": true }')
		).toEqual(['$.enabled is required.']);
	});

	test('treats oneOf as exactly one match, not anyOf', () => {
		const schema = { oneOf: [{ type: 'number' }, { type: 'integer' }] };
		expect(validateJsonBody(schema, '1')).toEqual(['$ must match exactly one allowed schema.']);
		expect(validateJsonBody(schema, '1.5')).toEqual([]);
	});

	test('anyOf still accepts the first matching branch', () => {
		const schema = { anyOf: [{ type: 'number' }, { type: 'integer' }] };
		expect(validateJsonBody(schema, '1')).toEqual([]);
	});

	test('allOf applies every branch', () => {
		const schema = {
			allOf: [{ type: 'object' }, { required: ['id'] }, { properties: { id: { type: 'string' } } }]
		};
		expect(validateJsonBody(schema, '{ "id": 1 }')).toEqual(['$.id must be string, not number.']);
		expect(validateJsonBody(schema, '{}')).toEqual(['$.id is required.']);
	});

	test('enum and const reject other values', () => {
		expect(validateJsonBody({ enum: ['a', 'b'] }, '"c"')).toEqual(['$ must be one of: "a", "b".']);
		expect(validateJsonBody({ const: 3 }, '2')).toEqual(['$ must equal 3.']);
	});

	test('additionalProperties can forbid or check extras', () => {
		expect(
			validateJsonBody({ type: 'object', additionalProperties: false }, '{ "x": 1 }')
		).toEqual(['$.x is not allowed.']);
		expect(
			validateJsonBody(
				{ type: 'object', additionalProperties: { type: 'number' } },
				'{ "x": "no" }'
			)
		).toEqual(['$.x must be number, not string.']);
	});

	test('array items are checked by index', () => {
		expect(validateJsonBody({ type: 'array', items: { type: 'number' } }, '[1, "x"]')).toEqual([
			'$[1] must be number, not string.'
		]);
	});

	test('nullable accepts null without dropping the other type', () => {
		expect(validateJsonBody({ type: 'string', nullable: true }, 'null')).toEqual([]);
		expect(validateJsonBody({ type: ['string', 'null'] }, 'null')).toEqual([]);
		expect(validateJsonBody({ type: 'string', nullable: true }, '1')).toEqual([
			'$ must be string or null, not number.'
		]);
	});

	test('a key that is not a JS identifier is addressed in brackets', () => {
		expect(
			validateJsonBody(
				{ type: 'object', properties: { 'content-type': { type: 'number' } } },
				'{ "content-type": true }'
			)
		).toEqual(['$["content-type"] must be number, not boolean.']);
	});

	test('invalid or empty JSON is not a schema error', () => {
		expect(validateJsonBody({ type: 'object' }, '')).toEqual([]);
		expect(validateJsonBody({ type: 'object' }, '{')).toEqual([]);
		expect(validateJsonBody(null, '{}')).toEqual([]);
	});

	test('oversized bodies are not parsed', () => {
		const huge = `{"x":"${'a'.repeat(2 * 1024 * 1024)}"}`;
		expect(validateJsonBody({ type: 'object' }, huge)).toEqual([]);
	});
});
