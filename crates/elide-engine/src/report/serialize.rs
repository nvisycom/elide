//! The `serde::Serialize` impl for [`Report`]: the part-grouped
//! `{ body, parts }` wire view a review layer consumes.
//!
//! [`Report`]: super::Report

use std::collections::HashMap;

use super::{EntityGroup, Report};

impl serde::Serialize for Report {
    /// Serialize to `{ body: [entities], parts: { id: [entities] } }`.
    /// `body` is null when no body pipeline ran.
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        // Adapt an erased group to a Serialize value.
        struct Group<'a>(&'a dyn EntityGroup);
        impl serde::Serialize for Group<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                erased_serde::serialize(self.0, s)
            }
        }

        let parts: HashMap<&str, Group<'_>> = self
            .parts
            .iter()
            .map(|(id, p)| (id.as_str(), Group(p.entities.as_ref())))
            .collect();

        let mut state = serializer.serialize_struct("Report", 2)?;
        state.serialize_field(
            "body",
            &self.body.as_ref().map(|b| Group(b.entities.as_ref())),
        )?;
        state.serialize_field("parts", &parts)?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use elide_core::entity::audit::{AuditEvent, AuditLog, PatternEvent};
    use elide_core::entity::{Entity, LabelRef};
    use elide_core::modality::text::{Text, TextLocation};
    use elide_core::primitive::Confidence;

    use super::Report;

    fn text_entity(label: &str) -> Entity<Text> {
        let loc = TextLocation::new(0, 4);
        let event = AuditEvent::pattern("t", Confidence::MAX, loc.clone(), PatternEvent::default());
        Entity::new(
            LabelRef::new(label),
            loc,
            Confidence::MAX,
            AuditLog::new(event),
        )
    }

    #[test]
    fn serializes_body_to_grouped_view() {
        // The part-grouped `{ body, parts }` shape is exercised end to end
        // (with a real container) in the docx integration test; here we
        // check the body group and the empty-parts shape directly.
        let report = Report::new().insert_body::<Text>(vec![text_entity("EMAIL_ADDRESS")]);

        let value = serde_json::to_value(&report).unwrap();
        // body is an array carrying the entity's label; parts is an object.
        assert_eq!(value["body"][0]["label"], "EMAIL_ADDRESS");
        assert!(value["parts"].is_object());
        assert_eq!(value["parts"].as_object().unwrap().len(), 0);

        // No body pipeline ran → body is null.
        let empty = serde_json::to_value(Report::new()).unwrap();
        assert!(empty["body"].is_null());
    }
}
