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
});
