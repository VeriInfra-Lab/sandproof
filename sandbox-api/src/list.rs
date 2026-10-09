//! Pinned E2B list filters and stable, direction-aware keyset pagination.
//! Callers supply only currently authorized, live instances; a cursor is a
//! position in that result set, never an authorization credential.

use sandproof_controller_service::application::{InstancePage, ListCursor, ListSelection, Order};
use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::URL_SAFE};
use percent_encoding::percent_decode_str;
use serde::Deserialize;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{PortError, SandboxInfo, SandboxPage};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ListOrder {
    Asc,
    #[default]
    Desc,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub limit: Option<u32>,
    #[serde(rename = "nextToken")]
    pub next_token: Option<String>,
    pub metadata: Option<String>,
    pub state: Option<String>,
    #[serde(rename = "startedAfter")]
    pub started_after: Option<String>,
    pub template: Option<String>,
    pub order: Option<ListOrder>,
}

pub(crate) struct Selection {
    pub(crate) application: ListSelection,
    pub(crate) order: ListOrder,
}

// Match the upstream microsecond keyset, including sub-microsecond bounds.
fn timestamp(value: &str) -> Result<i128, PortError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map(|time| time.unix_timestamp_nanos().div_euclid(1_000))
        .map_err(|_| PortError::Invalid)
}

fn unescape(value: &str) -> Result<String, PortError> {
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && !bytes
                .get(index + 1..index + 3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
        {
            return Err(PortError::Invalid);
        }
    }
    percent_decode_str(&value.replace('+', " "))
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|_| PortError::Invalid)
}

fn metadata_filter(value: &str) -> Result<BTreeMap<String, String>, PortError> {
    // The SDK encodes each component, then the metadata query, then the HTTP
    // query. Axum has removed only the outer HTTP layer at this point.
    unescape(value)?
        .split('&')
        .map(|pair| {
            let (key, value) = pair.split_once('=').ok_or(PortError::Invalid)?;
            if value.contains('=') {
                return Err(PortError::Invalid);
            }
            Ok((unescape(key)?, unescape(value)?))
        })
        .collect()
}

fn template_filter(value: &str) -> Result<String, PortError> {
    // Match the immutable template ID reported by each instance. Catalog
    // aliases are deliberately not re-resolved while listing old instances.
    let parts: Vec<_> = value.split('/').map(str::trim).collect();
    if parts.len() > 2
        || parts.iter().any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_".contains(&byte))
        })
    {
        return Err(PortError::Invalid);
    }
    Ok(parts.join("/").to_ascii_lowercase())
}

impl ListQuery {
    pub(crate) fn validate(&self) -> Result<Selection, PortError> {
        let limit = self.limit.unwrap_or(100);
        if !(1..=100).contains(&limit) {
            return Err(PortError::Invalid);
        }
        let running = if let Some(states) = &self.state {
            let mut running = false;
            for state in states.split(',') {
                match state {
                    "running" => running = true,
                    "paused" => {}
                    _ => return Err(PortError::Invalid),
                }
            }
            running
        } else {
            true
        };
        let order = self.order.unwrap_or_default();
        let cursor = self
            .next_token
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| {
                let decoded = URL_SAFE.decode(value).map_err(|_| PortError::Invalid)?;
                let decoded = std::str::from_utf8(&decoded).map_err(|_| PortError::Invalid)?;
                let parts: Vec<_> = decoded.split("__").collect();
                match (order, parts.as_slice()) {
                    (ListOrder::Desc, [time, id]) | (ListOrder::Asc, [time, id, "asc"]) => {
                        Ok((timestamp(time)?, (*id).to_owned()))
                    }
                    _ => Err(PortError::Invalid),
                }
            })
            .transpose()?;
        Ok(Selection {
            order,
            application: ListSelection {
                limit: usize::try_from(limit).map_err(|_| PortError::Invalid)?,
                running,
                metadata: self
                    .metadata
                    .as_deref()
                    .map(metadata_filter)
                    .transpose()?
                    .unwrap_or_default(),
                started_after_us: self.started_after.as_deref().map(timestamp).transpose()?,
                template: self.template.as_deref().map(template_filter).transpose()?,
                order: match order {
                    ListOrder::Asc => Order::Asc,
                    ListOrder::Desc => Order::Desc,
                },
                cursor: cursor.map(|(started_at_us, sandbox_id)| ListCursor {
                    started_at_us,
                    sandbox_id,
                }),
            },
        })
    }
}

pub(crate) fn page_to_sdk(page: InstancePage, order: ListOrder) -> Result<SandboxPage, PortError> {
    let next_token = page
        .next_cursor
        .map(|key| {
            let time = OffsetDateTime::from_unix_timestamp_nanos(
                key.started_at_us
                    .checked_mul(1_000)
                    .ok_or(PortError::Failed)?,
            )
            .map_err(|_| PortError::Failed)?
            .format(&Rfc3339)
            .map_err(|_| PortError::Failed)?;
            let suffix = if order == ListOrder::Asc { "__asc" } else { "" };
            Ok(URL_SAFE.encode(format!("{time}__{}{suffix}", key.sandbox_id)))
        })
        .transpose()?;
    Ok(SandboxPage {
        items: page
            .items
            .into_iter()
            .map(SandboxInfo::try_from)
            .collect::<Result<_, _>>()?,
        next_token,
        total_running: page.total_running,
    })
}

#[cfg(test)]
use sandproof_controller_service::application::InstanceInfo;

#[cfg(test)]
impl Selection {
    fn page(self, items: Vec<SandboxInfo>) -> Result<SandboxPage, PortError> {
        let items = items
            .into_iter()
            .map(|v| {
                Ok(InstanceInfo {
                    template_id: v.template_id,
                    sandbox_id: v.sandbox_id,
                    owner: 1,
                    metadata: v.metadata,
                    started_at_ms: i64::try_from(
                        timestamp(&v.started_at)
                            .map_err(|_| PortError::Failed)?
                            .div_euclid(1_000),
                    )
                    .map_err(|_| PortError::Failed)?,
                    end_at_ms: 0,
                    envd_version: v.envd_version,
                    cpu_count: v.cpu_count,
                    memory_mb: v.memory_mb,
                    disk_size_mb: v.disk_size_mb,
                })
            })
            .collect::<Result<_, PortError>>()?;
        page_to_sdk(self.application.page(items)?, self.order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, started: &str) -> SandboxInfo {
        SandboxInfo {
            template_id: "base".into(),
            sandbox_id: id.into(),
            client_id: "tenant-1".into(),
            metadata: BTreeMap::from([("group".into(), "red".into())]),
            started_at: started.into(),
            end_at: "2026-01-01T01:00:00Z".into(),
            envd_version: "0.9.0".into(),
            cpu_count: 1,
            memory_mb: 512,
            disk_size_mb: 1024,
            state: "running".into(),
            volume_mounts: Vec::new(),
        }
    }

    fn items() -> Vec<SandboxInfo> {
        vec![
            info("c", "2026-01-01T00:00:02Z"),
            info("a", "2026-01-01T00:00:03Z"),
            info("b", "2026-01-01T00:00:02Z"),
            info("d", "2026-01-01T00:00:01Z"),
        ]
    }

    fn ids(page: &SandboxPage) -> Vec<&str> {
        page.items
            .iter()
            .map(|info| info.sandbox_id.as_str())
            .collect()
    }

    #[test]
    fn start_time_order_and_equal_time_ties_span_pages_in_both_directions() {
        for (order, first, second) in [
            (None, ["a", "b"], ["c", "d"]),
            (Some(ListOrder::Asc), ["d", "c"], ["b", "a"]),
        ] {
            let mut query = ListQuery {
                limit: Some(2),
                order,
                ..ListQuery::default()
            };
            let page = query.validate().unwrap().page(items()).unwrap();
            assert_eq!(ids(&page), first);
            assert_eq!(page.total_running, Some(4));
            query.next_token = page.next_token;
            assert!(query.next_token.is_some());
            let page = query.validate().unwrap().page(items()).unwrap();
            assert_eq!(ids(&page), second);
            assert_eq!(page.total_running, Some(4));
            assert!(page.next_token.is_none());
            query.order = Some(if order.is_none() {
                ListOrder::Asc
            } else {
                ListOrder::Desc
            });
            assert!(
                query.validate().is_err(),
                "cursor direction must not be reused"
            );
        }
    }

    #[test]
    fn cursor_survives_removed_anchor_and_does_not_repeat_after_new_creation() {
        let mut query = ListQuery {
            limit: Some(2),
            ..ListQuery::default()
        };
        query.next_token = query.validate().unwrap().page(items()).unwrap().next_token;
        let mut changed = items();
        changed.retain(|item| item.sandbox_id != "b");
        changed.push(info("new", "2026-01-01T00:00:04Z"));
        let page = query.validate().unwrap().page(changed).unwrap();
        assert_eq!(ids(&page), ["c", "d"]);
        assert_eq!(page.total_running, Some(4));
        assert!(page.next_token.is_none());
    }

    #[test]
    fn filters_precede_count_and_cursor_with_inclusive_microsecond_boundary() {
        let mut rows = items();
        rows[0].metadata.insert("group".into(), "blue".into());
        let mut query = ListQuery {
            metadata: Some("group=red".into()),
            started_after: Some("2026-01-01T01:00:02.000000999+01:00".into()),
            template: Some(" BASE ".into()),
            state: Some("running,paused".into()),
            limit: Some(1),
            ..ListQuery::default()
        };
        let page = query.validate().unwrap().page(rows.clone()).unwrap();
        assert_eq!(ids(&page), ["a"]);
        assert_eq!(page.total_running, Some(2));
        query.next_token = page.next_token;
        let page = query.validate().unwrap().page(rows).unwrap();
        assert_eq!(ids(&page), ["b"]);
        assert_eq!(page.total_running, Some(2));
        assert!(page.next_token.is_none());
    }

    #[test]
    fn paused_and_unknown_templates_are_empty_without_inventing_instances() {
        for (state, template, total) in [
            (Some("paused"), None, None),
            (None, Some("other/base"), Some(0)),
            (Some("paused"), Some("other"), None),
        ] {
            let query = ListQuery {
                state: state.map(str::to_owned),
                template: template.map(str::to_owned),
                ..ListQuery::default()
            };
            let page = query.validate().unwrap().page(items()).unwrap();
            assert!(page.items.is_empty());
            assert!(page.next_token.is_none());
            assert_eq!(page.total_running, total);
        }
    }

    #[test]
    fn sdk_metadata_encoding_preserves_unicode_and_reserved_characters() {
        let filter =
            metadata_filter("a%2526%252B%253D=%25E9%259B%25AA%2520%2525%252B%2F%253D&group=red")
                .unwrap();
        assert_eq!(filter.get("a&+="), Some(&"雪 %+/=".to_owned()));
        assert_eq!(filter.get("group"), Some(&"red".to_owned()));
        assert_eq!(
            metadata_filter("x=one&x=two").unwrap().get("x"),
            Some(&"two".to_owned())
        );
        for invalid in ["", "x", "x=a=b", "x=%", "x=%2", "x=%GG", "x=%25GG"] {
            assert!(metadata_filter(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn malformed_filters_and_cursor_timestamps_fail_before_listing() {
        for value in [
            serde_json::json!({"limit":0}),
            serde_json::json!({"limit":101}),
            serde_json::json!({"state":"destroyed"}),
            serde_json::json!({"state":"running,bad"}),
            serde_json::json!({"startedAfter":"yesterday"}),
            serde_json::json!({"startedAfter":"2026-01-01T00:00:00"}),
            serde_json::json!({"template":"base:tag"}),
            serde_json::json!({"template":"a/b/c"}),
            serde_json::json!({"nextToken":"123"}),
            serde_json::json!({"nextToken":URL_SAFE.encode("bad__id")}),
            serde_json::json!({"nextToken":URL_SAFE.encode("2026-01-01T00:00:00Z__id__desc")}),
        ] {
            let query: ListQuery = serde_json::from_value(value.clone()).unwrap();
            assert!(query.validate().is_err(), "{value}");
        }
        assert!(serde_json::from_str::<ListQuery>(r#"{"order":"newest"}"#).is_err());
    }
}
