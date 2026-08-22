import { expect, test } from '@playwright/test';
import { applyPathParams, pathParamNames } from '../../src/lib/api';

test.describe('path params', () => {
	test('names come from placeholders in the path, not the query', () => {
		expect(pathParamNames('/pet/{petId}?pretty=1')).toEqual(['petId']);
		expect(pathParamNames('/store/{orderId}/items/{itemId}')).toEqual(['orderId', 'itemId']);
	});

	test('empty values stay visible as placeholders', () => {
		expect(applyPathParams('/pet/{petId}', [{ name: 'petId', value: '' }])).toBe('/pet/{petId}');
		expect(applyPathParams('/pet/{petId}', [])).toBe('/pet/{petId}');
	});

	test('a slash in a value does not become another path component', () => {
		expect(applyPathParams('/pet/{petId}', [{ name: 'petId', value: 'a/b' }])).toBe('/pet/a%2Fb');
	});
});
