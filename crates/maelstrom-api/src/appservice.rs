//! Application Service registration file parser.
//!
//! Matrix application services (bridges, bots) register with the homeserver
//! via a YAML configuration file.  This module parses that file format into
//! an [`AppServiceRecord`] that can be persisted via [`ApplicationServiceStore`].
//!
//! # YAML format
//!
//! ```yaml
//! id: "slack"
//! url: "http://localhost:9000"
//! as_token: "abc123"
//! hs_token: "def456"
//! sender_localpart: "slackbridge"
//! namespaces:
//!   users:
//!     - exclusive: true
//!       regex: "@slack_.*:example.com"
//!   aliases:
//!     - exclusive: false
//!       regex: "#slack_.*:example.com"
//! rate_limited: false
//! protocols:
//!   - slack
//! ```

use maelstrom_storage::traits::{AppServiceRecord, NamespaceRule};

/// Parse an application service registration YAML string into an [`AppServiceRecord`].
///
/// Returns an error string if the YAML is invalid or required fields are missing.
pub fn parse_appservice_yaml(yaml_str: &str) -> Result<AppServiceRecord, String> {
    let doc: serde_yaml::Value =
        serde_yaml::from_str(yaml_str).map_err(|e| format!("Invalid YAML: {e}"))?;

    let id = doc
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: id")?
        .to_string();

    let url = doc
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: url")?
        .to_string();

    let as_token = doc
        .get("as_token")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: as_token")?
        .to_string();

    let hs_token = doc
        .get("hs_token")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: hs_token")?
        .to_string();

    let sender_localpart = doc
        .get("sender_localpart")
        .and_then(|v| v.as_str())
        .ok_or("Missing required field: sender_localpart")?
        .to_string();

    let rate_limited = doc
        .get("rate_limited")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let protocols = doc
        .get("protocols")
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let namespaces = doc.get("namespaces");

    let user_namespaces = namespaces
        .and_then(|ns| ns.get("users"))
        .and_then(|v| v.as_sequence())
        .map(|seq| parse_namespace_rules(seq))
        .transpose()?
        .unwrap_or_default();

    let alias_namespaces = namespaces
        .and_then(|ns| ns.get("aliases"))
        .and_then(|v| v.as_sequence())
        .map(|seq| parse_namespace_rules(seq))
        .transpose()?
        .unwrap_or_default();

    Ok(AppServiceRecord {
        id,
        url,
        as_token,
        hs_token,
        sender_localpart,
        user_namespaces,
        alias_namespaces,
        rate_limited,
        protocols,
    })
}

/// Parse a YAML sequence of namespace rules into [`NamespaceRule`] values.
fn parse_namespace_rules(seq: &[serde_yaml::Value]) -> Result<Vec<NamespaceRule>, String> {
    seq.iter()
        .map(|entry| {
            let regex = entry
                .get("regex")
                .and_then(|v| v.as_str())
                .ok_or("Namespace rule missing 'regex' field")?
                .to_string();
            let exclusive = entry
                .get("exclusive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Ok(NamespaceRule { regex, exclusive })
        })
        .collect()
}
