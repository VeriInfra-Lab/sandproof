//! Business filtering/keysets; wire cursor parsing belongs to API.
use super::{ApplicationError, InstanceInfo};
use std::collections::BTreeMap;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    Asc,
    #[default]
    Desc,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListCursor {
    pub started_at_us: i128,
    pub sandbox_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListSelection {
    pub limit: usize,
    pub running: bool,
    pub metadata: BTreeMap<String, String>,
    pub started_after_us: Option<i128>,
    pub template: Option<String>,
    pub order: Order,
    pub cursor: Option<ListCursor>,
}
impl Default for ListSelection {
    fn default() -> Self {
        Self {
            limit: 100,
            running: true,
            metadata: BTreeMap::new(),
            started_after_us: None,
            template: None,
            order: Order::Desc,
            cursor: None,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstancePage {
    pub items: Vec<InstanceInfo>,
    pub next_cursor: Option<ListCursor>,
    pub total_running: Option<usize>,
}
impl ListSelection {
    /// # Errors
    /// All callers must respect bounded page size.
    pub fn validate(self) -> Result<Self, ApplicationError> {
        if !(1..=100).contains(&self.limit) {
            return Err(ApplicationError::Invalid);
        }
        Ok(self)
    }
    fn compare(&self, a: &ListCursor, b: &ListCursor) -> std::cmp::Ordering {
        let descending = b
            .started_at_us
            .cmp(&a.started_at_us)
            .then_with(|| a.sandbox_id.cmp(&b.sandbox_id));
        match self.order {
            Order::Desc => descending,
            Order::Asc => descending.reverse(),
        }
    }
    /// # Errors
    /// Rejects invalid limits even if query validation was bypassed.
    pub fn page(self, items: Vec<InstanceInfo>) -> Result<InstancePage, ApplicationError> {
        let selection = self.validate()?;
        let mut matching = Vec::new();
        if selection.running {
            for info in items {
                let started = i128::from(info.started_at_ms) * 1_000;
                if selection
                    .started_after_us
                    .is_some_and(|bound| started < bound)
                    || selection
                        .template
                        .as_ref()
                        .is_some_and(|template| *template != info.template_id)
                    || !selection
                        .metadata
                        .iter()
                        .all(|(key, value)| info.metadata.get(key) == Some(value))
                {
                    continue;
                }
                matching.push((
                    ListCursor {
                        started_at_us: started,
                        sandbox_id: info.sandbox_id.clone(),
                    },
                    info,
                ));
            }
        }
        let total_running = selection.running.then_some(matching.len());
        if let Some(cursor) = &selection.cursor {
            matching.retain(|(key, _)| selection.compare(key, cursor).is_gt());
        }
        matching.sort_by(|(a, _), (b, _)| selection.compare(a, b));
        let next_cursor =
            (matching.len() > selection.limit).then(|| matching[selection.limit - 1].0.clone());
        matching.truncate(selection.limit);
        Ok(InstancePage {
            items: matching.into_iter().map(|(_, info)| info).collect(),
            next_cursor,
            total_running,
        })
    }
}
