// SPDX-License-Identifier: Apache-2.0
//! Redacted structured audit-event helpers.
//!
//! Audit events are deliberately narrow key/value log lines. Callers pass only
//! normalized identities, operation classes, object labels, outcomes, and static
//! reason codes; SQL text, WKB, object-store paths, and secrets are not accepted
//! by this API.

use std::sync::atomic::{AtomicU64, Ordering};

static AUDIT_EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Denied,
    Failed,
    Succeeded,
}

impl AuditOutcome {
    fn as_str(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Failed => "failed",
            Self::Succeeded => "succeeded",
        }
    }
}

pub fn log_auth_failure(user: &str, reason: &str) {
    log_event(
        "auth",
        AuditOutcome::Denied,
        user,
        "<connection>",
        reason,
        &[],
    );
}

pub fn log_authorization_denied(user: &str, statement_kind: &str, target: &str, reason: &str) {
    log_event(
        "authorization",
        AuditOutcome::Denied,
        user,
        target,
        reason,
        &[("statement_kind", statement_kind)],
    );
}

pub fn log_maintenance(
    user: &str,
    operation: &str,
    target: &str,
    outcome: AuditOutcome,
    rows: Option<usize>,
) {
    let rows = rows.map(|rows| rows.to_string());
    let mut fields = vec![("operation", operation)];
    if let Some(rows) = rows.as_deref() {
        fields.push(("rows", rows));
    }
    log_event("maintenance", outcome, user, target, operation, &fields);
}

fn log_event(
    class: &str,
    outcome: AuditOutcome,
    user: &str,
    target: &str,
    reason: &str,
    fields: &[(&str, &str)],
) {
    let event_id = AUDIT_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    log::info!(
        "{}",
        render_audit_event(event_id, class, outcome, user, target, reason, fields)
    );
}

fn render_audit_event(
    event_id: u64,
    class: &str,
    outcome: AuditOutcome,
    user: &str,
    target: &str,
    reason: &str,
    fields: &[(&str, &str)],
) -> String {
    let mut event = format!(
        "quackgis_audit event_id={} class={} outcome={} user={} target={} reason={}",
        event_id,
        sanitize_label(class),
        outcome.as_str(),
        sanitize_label(user),
        sanitize_label(target),
        sanitize_label(reason),
    );
    for (key, value) in fields {
        event.push(' ');
        event.push_str(&sanitize_label(key));
        event.push('=');
        event.push_str(&sanitize_label(value));
    }
    event
}

fn sanitize_label(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars().take(128) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '@' | '<' | '>') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "<empty>".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_rendering_is_key_value_and_sanitized() {
        let event = render_audit_event(
            7,
            "authorization",
            AuditOutcome::Denied,
            "reader user",
            "main.secret\npath",
            "read allowlist denied",
            &[("statement_kind", "query")],
        );
        assert!(event.starts_with("quackgis_audit event_id=7 class=authorization"));
        assert!(event.contains("outcome=denied"));
        assert!(event.contains("user=reader_user"));
        assert!(event.contains("target=main.secret_path"));
        assert!(event.contains("reason=read_allowlist_denied"));
        assert!(event.contains("statement_kind=query"));
        assert!(!event.contains('\n'));
        assert!(!event.contains("password="));
    }
}
