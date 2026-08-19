import type { CaptureKind } from './api';

/**
 * Where well-known auth providers keep their credentials.
 *
 * A signed-in session can easily hold fifty values. Naming the ones we
 * recognise — and floating them to the top — turns "scroll and guess" into
 * "click the row that says Auth0".
 *
 * Patterns are drawn from each SDK's documented storage format. The SDK-shaped
 * ones (Auth0, Supabase, Firebase, MSAL, Cognito, Okta, Clerk, Auth.js) were
 * checked against their docs; the rest are long-standing framework session
 * cookie names.
 */
interface Rule {
	provider: string;
	where: CaptureKind;
	/** Matches the storage key or cookie name. */
	key: RegExp;
	/** When the value is JSON, the leaf path holding the credential. */
	path?: RegExp;
	/** Added to the ranking score. Higher means "we're more sure". */
	weight: number;
}

const RULES: Rule[] = [
	// ---- localStorage, SDK-specific ------------------------------------------
	{
		provider: 'Auth0',
		where: 'localStorage',
		// @@auth0spajs@@::<clientId>::<audience>::<scope>
		key: /^@@auth0spajs@@/,
		path: /(^|\.)(access_token|id_token)$/,
		weight: 50
	},
	{ provider: 'Auth0', where: 'localStorage', key: /^(_legacy_)?auth0(spajs)?[.@]/, weight: 25 },
	{
		provider: 'Supabase',
		where: 'localStorage',
		// sb-<project-ref>-auth-token
		key: /^sb-.+-auth-token/,
		path: /(^|\.)access_token$/,
		weight: 50
	},
	{ provider: 'Supabase', where: 'localStorage', key: /^sb-.+-auth-token/, weight: 25 },
	{
		provider: 'Firebase',
		where: 'localStorage',
		// firebase:authUser:<apiKey>:[DEFAULT]
		key: /^firebase:authUser:/,
		path: /stsTokenManager\.accessToken$/,
		weight: 50
	},

	// ---- IndexedDB -----------------------------------------------------------
	{
		provider: 'Firebase',
		where: 'indexedDb',
		// Where the JS SDK actually puts it by default:
		// firebaseLocalStorageDb/firebaseLocalStorage/firebase:authUser:<apiKey>
		key: /^firebaseLocalStorageDb\//,
		path: /stsTokenManager\.accessToken$/,
		weight: 50
	},
	{ provider: 'Firebase', where: 'indexedDb', key: /^firebaseLocalStorageDb\//, weight: 25 },
	{
		provider: 'Microsoft (MSAL)',
		where: 'localStorage',
		// <homeAccountId>.<environment>-accesstoken-<clientId>--<scopes>
		key: /-accesstoken-|^msal\./,
		path: /(^|\.)secret$/,
		weight: 50
	},
	{ provider: 'Microsoft (MSAL)', where: 'localStorage', key: /-accesstoken-|^msal\./, weight: 25 },
	{
		provider: 'AWS Cognito',
		where: 'localStorage',
		// CognitoIdentityServiceProvider.<clientId>.<username>.accessToken
		key: /^CognitoIdentityServiceProvider\..*\.(accessToken|idToken)$/,
		weight: 45
	},
	{
		provider: 'Okta',
		where: 'localStorage',
		key: /^okta-token-storage/,
		path: /accessToken\.accessToken$/,
		weight: 50
	},
	{ provider: 'Okta', where: 'localStorage', key: /^okta-token-storage/, weight: 25 },
	{ provider: 'Clerk', where: 'localStorage', key: /^__clerk_client_jwt$/, weight: 45 },
	{ provider: 'Keycloak', where: 'localStorage', key: /^kc-(access|token)$/, weight: 40 },
	{ provider: 'SuperTokens', where: 'localStorage', key: /st-access-token/i, weight: 40 },
	{ provider: 'Stytch', where: 'localStorage', key: /^stytch_session_jwt$/, weight: 40 },

	// ---- cookies, SDK-specific -----------------------------------------------
	{ provider: 'Clerk', where: 'cookie', key: /^__session$/, weight: 45 },
	{ provider: 'Clerk', where: 'cookie', key: /^__client$/, weight: 30 },
	{
		provider: 'Auth.js / NextAuth',
		where: 'cookie',
		key: /^(__Secure-)?(next-auth|authjs)\.session-token/,
		weight: 45
	},
	{ provider: 'Auth0', where: 'cookie', key: /^appSession$/, weight: 45 },
	{ provider: 'Keycloak', where: 'cookie', key: /^(KEYCLOAK_(IDENTITY|SESSION)|kc-access)$/, weight: 40 },
	{ provider: 'SuperTokens', where: 'cookie', key: /^sAccessToken$/, weight: 40 },
	{ provider: 'Ory', where: 'cookie', key: /^ory_(kratos_)?session/, weight: 40 },
	{ provider: 'Stytch', where: 'cookie', key: /^stytch_session(_jwt)?$/, weight: 40 },
	{ provider: 'Okta', where: 'cookie', key: /^(sid|idx)$/, weight: 30 },
	{ provider: 'Descope', where: 'cookie', key: /^DSR?$/, weight: 30 },
	{ provider: 'WorkOS', where: 'cookie', key: /^wos-session$/, weight: 40 },

	// ---- cookies, framework sessions -----------------------------------------
	{ provider: 'Django', where: 'cookie', key: /^sessionid$/, weight: 30 },
	{ provider: 'Laravel', where: 'cookie', key: /^laravel_session$/, weight: 30 },
	{ provider: 'PHP', where: 'cookie', key: /^PHPSESSID$/, weight: 30 },
	{ provider: 'Java', where: 'cookie', key: /^JSESSIONID$/, weight: 30 },
	{ provider: 'Rails', where: 'cookie', key: /^_.+_session$/, weight: 30 },
	{ provider: 'Express', where: 'cookie', key: /^connect\.sid$/, weight: 30 },
	{ provider: 'ASP.NET', where: 'cookie', key: /^(\.AspNetCore\.|ASP\.NET_SessionId$)/, weight: 30 }
];

export interface Identified {
	provider: string;
	weight: number;
}

/**
 * Names the provider a value belongs to, if we recognise it. The best-weighted
 * match wins, so a rule that also pins down the path beats one that only
 * matched the key.
 */
export function identify(
	where: CaptureKind,
	key: string,
	path: string
): Identified | null {
	let best: Identified | null = null;

	for (const rule of RULES) {
		if (rule.where !== where) continue;
		if (!rule.key.test(key)) continue;
		if (rule.path && !rule.path.test(path)) continue;
		if (!best || rule.weight > best.weight) {
			best = { provider: rule.provider, weight: rule.weight };
		}
	}

	return best;
}
