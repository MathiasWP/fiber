//! What an agent may call, and what needs a person first.
//!
//! Sharing a collection used to have one dial beyond on/off: anything but
//! GET, HEAD and OPTIONS needed a second switch. That works when the HTTP
//! method says what a call does. It says nothing at all about an API where
//! every operation is a POST — the read that lists your customers and the
//! write that refunds them are the same shape, so the switch is either off,
//! and the API is unusable, or on, and there is no guard left.
//!
//! What such an API does publish is its own vocabulary: an `x-kind` on each
//! operation, a `deprecated` flag, a scope. So the dial here is a **jq filter**
//! over that vocabulary, returning `"allow"`, `"ask"` or `"deny"` per endpoint.
//! Nothing in Fiber knows what `x-kind` means; the filter is where meaning is
//! attached, in one place a person can read and change.
//!
//! ```jq
//! if   .meta["x-kind"] == "query"   then "allow"
//! elif .meta["x-kind"] == "command" then "ask"
//! else "deny" end
//! ```
//!
//! jq for the same three reasons loaders use it: it cannot do anything but
//! transform, it can be re-run against the real endpoint list as you type, and
//! the people writing this already know it. A second matching language, made up
//! here, would buy nothing and would have to grow every time someone's rule
//! didn't fit.
//!
//! **Everything fails closed.** A filter that doesn't compile, throws, returns
//! a number, or returns nothing denies the call. A policy that is being edited
//! is not an open door.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// jq is not a place to write an essay.
const MAX_POLICY_CHARS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// Send it.
    Allow,
    /// Send it once a person has said so.
    Ask,
    /// Refuse, and say why.
    Deny,
}

/// What a policy filter is handed. This is a contract: a filter someone wrote
/// against these fields must keep working, so fields are added here and never
/// renamed or removed.
///
/// `known` is the one that isn't about the endpoint. `send_request` takes a
/// method and a path, not a catalogue entry, so an agent can name a path the
/// collection has never heard of — `POST /orders/17/refund` when the manifest
/// only lists `POST /orders/{id}`. When nothing in the catalogue matches, the
/// call still gets a decision, but with `known: false` and no metadata, so a
/// filter that keys off `.meta` lands in its own `else` branch. Which is the
/// point: an unrecognised path must not be able to inherit a recognised one's
/// permission.
#[derive(Debug, Clone)]
pub struct Facts<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub name: &'a str,
    pub description: &'a str,
    pub tag: &'a str,
    pub meta: &'a BTreeMap<String, serde_json::Value>,
    /// True when a loader reported this rather than a person typing it.
    pub loaded: bool,
    /// True when this describes an endpoint the collection actually lists.
    pub known: bool,
}

impl Facts<'_> {
    pub fn input(&self) -> serde_json::Value {
        serde_json::json!({
            "method": self.method,
            "path": self.path,
            "name": self.name,
            "description": self.description,
            "tag": self.tag,
            "meta": self.meta,
            "loaded": self.loaded,
            "known": self.known,
        })
    }
}

/// Runs a policy over a whole endpoint list in one pass.
///
/// One pass because the filter is compiled per call: deciding five hundred
/// endpoints one at a time would compile the same filter five hundred times.
/// Wrapping it in `[ .[] | ( … ) ]` costs nothing and jq allows a `def` inside
/// the parentheses, so a filter with helpers in it still works.
pub fn decide(policy: &str, endpoints: &[serde_json::Value]) -> Result<Vec<Access>, String> {
    if policy.trim().is_empty() {
        return Err("the access policy is empty".to_string());
    }
    if policy.len() > MAX_POLICY_CHARS {
        return Err(format!(
            "the access policy is longer than {MAX_POLICY_CHARS} characters"
        ));
    }
    if endpoints.is_empty() {
        return Ok(Vec::new());
    }

    let wrapped = format!("[ .[] | ( {policy} ) ]");
    let input = serde_json::Value::Array(endpoints.to_vec());
    let output = crate::loader::apply(&wrapped, &input).map_err(|err| err.to_string())?;

    let items = output.as_array().ok_or_else(|| {
        "the policy produced something other than one answer per endpoint".to_string()
    })?;
    if items.len() != endpoints.len() {
        return Err(format!(
            "the policy produced {} answers for {} endpoints — it must return exactly one of \
             \"allow\", \"ask\" or \"deny\" per endpoint",
            items.len(),
            endpoints.len()
        ));
    }

    items
        .iter()
        .map(|item| match item.as_str() {
            Some("allow") => Ok(Access::Allow),
            Some("ask") => Ok(Access::Ask),
            Some("deny") => Ok(Access::Deny),
            _ => Err(format!(
                "the policy answered {item} — it must return \"allow\", \"ask\" or \"deny\""
            )),
        })
        .collect()
}

/// The method-based rule, which is what a collection with no policy still uses.
pub fn is_read_only(method: &str) -> bool {
    matches!(
        method.trim().to_ascii_uppercase().as_str(),
        "GET" | "HEAD" | "OPTIONS"
    )
}

/// One endpoint a collection exposes, from either source.
///
/// Shared so that the editor's preview and the MCP server's answer are the same
/// computation over the same list. Two of these would drift, and the one that
/// drifted would be the one that said "allow".
#[derive(Debug, Clone)]
pub struct Entry {
    /// A saved request's id, or `METHOD /path` for a loaded endpoint.
    pub key: String,
    pub method: String,
    pub path: String,
    pub name: String,
    pub description: String,
    pub tag: String,
    pub meta: BTreeMap<String, serde_json::Value>,
    pub parameters: Vec<crate::openapi::SpecParam>,
    pub loaded: bool,
}

impl Entry {
    pub fn facts(&self) -> Facts<'_> {
        Facts {
            method: &self.method,
            path: &self.path,
            name: &self.name,
            description: &self.description,
            tag: &self.tag,
            meta: &self.meta,
            loaded: self.loaded,
            known: true,
        }
    }
}

/// Everything a collection exposes: what a person typed, then what the loader
/// last reported.
pub fn catalogue(
    section: &crate::store::Section,
    loaded: &[crate::loader::LoadedEndpoint],
) -> Vec<Entry> {
    let mut entries: Vec<Entry> = section
        .requests
        .iter()
        .map(|request| Entry {
            key: request.id.clone(),
            method: request.method.clone(),
            path: request.path.clone(),
            name: request.name.clone(),
            description: request.description.clone(),
            tag: request.tag.clone(),
            // A typed request has no manifest behind it, so a policy sees it
            // with nothing to go on and lands in whatever branch covers that.
            meta: BTreeMap::new(),
            parameters: Vec::new(),
            loaded: false,
        })
        .collect();

    entries.extend(loaded.iter().map(|endpoint| Entry {
        key: endpoint.key(),
        method: endpoint.method.clone(),
        path: endpoint.path.clone(),
        name: endpoint.name.clone(),
        description: endpoint.description.clone(),
        tag: endpoint.tag.clone(),
        meta: endpoint.meta.clone(),
        parameters: endpoint.parameters.clone(),
        loaded: true,
    }));
    entries
}

/// One decision per entry, and the reason if nothing is allowed.
///
/// No policy keeps the rule the collection has always had: read-only unless
/// writes were switched on. A policy replaces that rule outright, GET included
/// — an API where a read can dump the customer table deserves to be able to say
/// so — and a policy that cannot run denies everything rather than falling back
/// to the switch it replaced.
pub fn decide_catalogue(
    section: &crate::store::Section,
    entries: &[Entry],
) -> (Vec<Access>, Option<String>) {
    if section.mcp.policy.trim().is_empty() {
        let accesses = entries
            .iter()
            .map(|entry| {
                if is_read_only(&entry.method) || section.mcp.allow_writes {
                    Access::Allow
                } else {
                    Access::Deny
                }
            })
            .collect();
        return (accesses, None);
    }

    let inputs: Vec<serde_json::Value> =
        entries.iter().map(|entry| entry.facts().input()).collect();
    match decide(&section.mcp.policy, &inputs) {
        Ok(accesses) => (accesses, None),
        Err(message) => (
            vec![Access::Deny; entries.len()],
            Some(format!(
                "collection `{}` denies everything: its access policy failed — {message}",
                section.id
            )),
        ),
    }
}

/// Starting points, offered in the editor.
///
/// Read-only leads because it is what a collection already does before anyone
/// writes a policy. The others are worked examples of reading an API's own
/// vocabulary; `x-kind` is one API's word for it, not one Fiber knows.
pub const TEMPLATES: &[(&str, &str)] = &[
    (
        "Read-only",
        r#"if .method == "GET" or .method == "HEAD" or .method == "OPTIONS"
then "allow" else "deny" end"#,
    ),
    (
        "By x-kind",
        r#"if .meta["x-kind"] == "query" then "allow"
elif .meta["x-kind"] == "command" then "ask"
elif .loaded | not then "ask"
else "deny" end"#,
    ),
    (
        "Ask before anything that isn't a read",
        r#"if .method == "GET" or .method == "HEAD" or .method == "OPTIONS"
then "allow" else "ask" end"#,
    ),
    (
        "Nothing deprecated",
        r#"if .meta.deprecated == true then "deny"
elif .method == "GET" then "allow"
else "ask" end"#,
    ),
];

/// The decision for one call, with the reason a failure would have carried.
/// Any failure is a denial: see the module comment.
pub fn decide_one(policy: &str, facts: &Facts) -> (Access, Option<String>) {
    match decide(policy, &[facts.input()]) {
        Ok(accesses) => (accesses[0], None),
        Err(message) => (Access::Deny, Some(message)),
    }
}

/// The path a policy and the catalogue are compared on: query string and
/// fragment removed, and an absolute URL reduced to its path.
///
/// An absolute URL only reaches here after `join_url_scoped` has established it
/// stays on the collection's own origin, so this is about matching
/// `https://api.example.com/orders/42` to `/orders/{id}` rather than about
/// letting anything through.
pub fn normalize_path(path: &str) -> String {
    let path = path.trim();
    let path = if path.starts_with("http://") || path.starts_with("https://") {
        match reqwest::Url::parse(path) {
            Ok(url) => url.path().to_string(),
            Err(_) => path.to_string(),
        }
    } else {
        path.to_string()
    };
    path.split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Does a catalogue entry describe this call?
///
/// Exact first, then `{name}` placeholders against one path segment each, so
/// `POST /orders/17/refund` finds the `POST /orders/{id}/refund` the manifest
/// listed. A placeholder never matches an empty segment and never spans a `/`:
/// `/orders/{id}` is not `/orders/17/refund`, which would otherwise hand a
/// nested endpoint whatever permission its parent has.
pub fn same_endpoint(entry_method: &str, entry_path: &str, method: &str, path: &str) -> bool {
    if !entry_method.trim().eq_ignore_ascii_case(method.trim()) {
        return false;
    }
    let entry_path = normalize_path(entry_path);
    let path = normalize_path(path);
    if entry_path == path {
        return true;
    }

    let entry_segments: Vec<&str> = entry_path.split('/').collect();
    let segments: Vec<&str> = path.split('/').collect();
    if entry_segments.len() != segments.len() {
        return false;
    }
    entry_segments
        .iter()
        .zip(segments.iter())
        .all(|(entry, segment)| {
            if entry.starts_with('{') && entry.ends_with('}') && entry.len() > 2 {
                !segment.is_empty()
            } else {
                entry == segment
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(pairs: &[(&str, &str)]) -> BTreeMap<String, serde_json::Value> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_string(), serde_json::json!(value)))
            .collect()
    }

    fn facts<'a>(
        method: &'a str,
        path: &'a str,
        meta: &'a BTreeMap<String, serde_json::Value>,
        known: bool,
    ) -> Facts<'a> {
        Facts {
            method,
            path,
            name: "",
            description: "",
            tag: "",
            meta,
            loaded: known,
            known,
        }
    }

    const KIND: &str = r#"if .meta["x-kind"] == "query" then "allow"
                          elif .meta["x-kind"] == "command" then "ask"
                          else "deny" end"#;

    #[test]
    fn a_policy_reads_the_apis_own_vocabulary() {
        let query = meta(&[("x-kind", "query")]);
        let command = meta(&[("x-kind", "command")]);
        let subscription = meta(&[("x-kind", "subscription")]);

        // The whole point: three POSTs, three different answers.
        assert_eq!(
            decide_one(KIND, &facts("POST", "/graphql/read", &query, true)).0,
            Access::Allow
        );
        assert_eq!(
            decide_one(KIND, &facts("POST", "/orders", &command, true)).0,
            Access::Ask
        );
        assert_eq!(
            decide_one(KIND, &facts("POST", "/events", &subscription, true)).0,
            Access::Deny
        );
    }

    #[test]
    fn an_unlisted_path_cannot_borrow_a_listed_ones_permission() {
        // No catalogue entry means no metadata, so a filter keyed off `.meta`
        // falls to its own else branch rather than inheriting anything.
        let none = meta(&[]);
        assert_eq!(
            decide_one(KIND, &facts("POST", "/orders/17/refund", &none, false)).0,
            Access::Deny
        );
    }

    #[test]
    fn a_broken_policy_denies_rather_than_opens() {
        let none = meta(&[]);
        for broken in [
            "",
            "this is not jq",
            r#""maybe""#,
            "42",
            ".meta | keys | length",
        ] {
            let (access, reason) = decide_one(broken, &facts("GET", "/x", &none, true));
            assert_eq!(access, Access::Deny, "`{broken}` must deny");
            assert!(reason.is_some(), "`{broken}` must say why");
        }
    }

    #[test]
    fn every_endpoint_gets_exactly_one_answer() {
        let query = meta(&[("x-kind", "query")]);
        let command = meta(&[("x-kind", "command")]);
        let inputs = vec![
            facts("POST", "/a", &query, true).input(),
            facts("POST", "/b", &command, true).input(),
        ];
        assert_eq!(
            decide(KIND, &inputs).unwrap(),
            vec![Access::Allow, Access::Ask]
        );

        // A filter that fans out or swallows an endpoint is a filter whose
        // answers no longer line up with the list they were asked about.
        assert!(decide(r#""allow", "deny""#, &inputs).is_err());
        assert!(decide(r#"empty"#, &inputs).is_err());
    }

    fn section_with(policy: &str, allow_writes: bool) -> crate::store::Section {
        crate::store::Section {
            id: "acme".into(),
            name: "Acme".into(),
            base_url: "https://api.acme.com".into(),
            mcp: crate::store::McpAccess {
                enabled: true,
                allow_writes,
                policy: policy.to_string(),
            },
            requests: vec![crate::store::SavedRequest {
                id: "req-1".into(),
                name: "Ping".into(),
                method: "GET".into(),
                path: "/ping".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn loaded_endpoint(method: &str, path: &str, kind: &str) -> crate::loader::LoadedEndpoint {
        crate::loader::LoadedEndpoint {
            method: method.into(),
            path: path.into(),
            name: path.into(),
            meta: [("x-kind".to_string(), serde_json::json!(kind))]
                .into_iter()
                .collect(),
            ..Default::default()
        }
    }

    /// The editor's preview and the MCP server's answer are this same call, so
    /// this is the one that has to be right for the two never to disagree.
    #[test]
    fn a_catalogue_covers_both_kinds_of_endpoint() {
        let section = section_with(KIND, false);
        let loaded = vec![
            loaded_endpoint("POST", "/customers/search", "query"),
            loaded_endpoint("POST", "/orders", "command"),
        ];

        let entries = catalogue(&section, &loaded);
        assert_eq!(entries.len(), 3, "one typed request and two loaded");
        assert_eq!(entries[0].key, "req-1");
        assert!(!entries[0].loaded);
        assert_eq!(entries[1].key, "POST /customers/search");
        assert!(entries[1].loaded);

        let (accesses, warning) = decide_catalogue(&section, &entries);
        assert!(warning.is_none());
        // A typed request carries no metadata, so it lands in the else branch —
        // which is why a policy has to say what it wants for those.
        assert_eq!(accesses, vec![Access::Deny, Access::Allow, Access::Ask]);
    }

    #[test]
    fn no_policy_leaves_the_method_rule_in_charge() {
        let loaded = vec![loaded_endpoint("POST", "/orders", "command")];

        let read_only = section_with("", false);
        let entries = catalogue(&read_only, &loaded);
        assert_eq!(
            decide_catalogue(&read_only, &entries).0,
            vec![Access::Allow, Access::Deny],
            "GET allowed, POST not, exactly as before policies existed"
        );

        let writable = section_with("", true);
        assert_eq!(
            decide_catalogue(&writable, &entries).0,
            vec![Access::Allow, Access::Allow]
        );
    }

    #[test]
    fn a_broken_policy_denies_the_whole_catalogue() {
        let section = section_with("this is not jq", true);
        let entries = catalogue(&section, &[loaded_endpoint("GET", "/orders", "query")]);
        let (accesses, warning) = decide_catalogue(&section, &entries);
        assert!(accesses.iter().all(|access| *access == Access::Deny));
        assert!(
            warning.is_some_and(|message| message.contains("acme")),
            "the collection has to be named, or a warning in a list of them is useless"
        );
    }

    #[test]
    fn every_template_compiles_and_answers() {
        let loaded = vec![
            loaded_endpoint("GET", "/orders", "query"),
            loaded_endpoint("POST", "/orders", "command"),
        ];
        for (name, filter) in TEMPLATES {
            let section = section_with(filter, false);
            let entries = catalogue(&section, &loaded);
            let (accesses, warning) = decide_catalogue(&section, &entries);
            assert!(warning.is_none(), "template `{name}` failed: {warning:?}");
            assert_eq!(accesses.len(), entries.len(), "template `{name}`");
        }
    }

    #[test]
    fn placeholders_match_one_segment_and_no_more() {
        assert!(same_endpoint("GET", "/orders/{id}", "get", "/orders/42"));
        assert!(same_endpoint(
            "POST",
            "/orders/{id}/refund",
            "POST",
            "/orders/42/refund"
        ));
        assert!(same_endpoint(
            "GET",
            "/orders/{id}",
            "GET",
            "/orders/42?full=1"
        ));
        assert!(same_endpoint(
            "GET",
            "/orders/{id}",
            "GET",
            "https://api.example.com/orders/42"
        ));

        assert!(!same_endpoint(
            "GET",
            "/orders/{id}",
            "GET",
            "/orders/42/refund"
        ));
        assert!(!same_endpoint("GET", "/orders/{id}", "GET", "/orders/"));
        assert!(!same_endpoint("GET", "/orders/{id}", "POST", "/orders/42"));
        assert!(!same_endpoint("GET", "/orders", "GET", "/customers"));
    }
}
