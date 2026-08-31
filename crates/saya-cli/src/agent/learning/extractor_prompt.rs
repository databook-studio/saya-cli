//! Structured extraction prompt builder — spec F Chunk 2.
//!
//! Generates the precision extraction prompt presenting `TurnRecord` and `TurnObjectTable`
//! with turn-scoped object IDs (`T0..Tn`) to the model.

use saya_agent::{ChatMessage, ChatRequest};

use super::turn_record::TurnRecord;

/// Builds a structured extraction `ChatRequest` for the given `TurnRecord`.
#[allow(dead_code)]
pub fn build_extraction_prompt(record: &TurnRecord, model: &str) -> ChatRequest {
    let mut table_desc = String::new();
    if record.object_table.is_empty() {
        table_desc.push_str("No database objects were registered during this turn.\n");
    } else {
        for entry in record.object_table.entries() {
            table_desc.push_str(&format!(
                "- {} -> {} (profile: {})",
                entry.id, entry.qualified_name, entry.profile
            ));
            if !entry.columns.is_empty() {
                table_desc.push_str(&format!(" [columns: {}]", entry.columns.join(", ")));
            }
            table_desc.push('\n');
        }
    }

    let system_prompt = format!(
        r#"You are SAYA's precision schema knowledge extractor.
Analyze the user conversation, executed actions, and assistant response to extract factual semantic knowledge about database tables and columns.

### STRICT RULES:
1. ONLY propose knowledge for objects listed in the REGISTERED OBJECTS table below using their turn-scoped ID (e.g. "T0", "T1").
   NEVER invent table names or reference objects not in the table.
2. Output valid JSON adhering to the JSON schema below.
3. Classify origin:
   - "user_explicit": The user explicitly stated, asserted, or corrected a rule or definition.
   - "assistant_inferred": The fact was inferred from schema inspection, query patterns, or assistant explanation.
4. Permitted slots:
   - "table.description": Overall purpose or business context of the table.
   - "table.alias": Common human/business alias for the table.
   - "table.grain": Description of what a single row represents (e.g. "one row per order").
   - "table.default_time": The column name to use as default time dimension.
   - "column:<column_name>.description": Purpose or business definition of a specific column.
   - "column:<column_name>.role": One of ["identifier", "dimension", "measure", "timestamp", "sensitive"].
5. A directive claim ("table.grain", "table.default_time", "column:<column_name>.role")
   carries a `reason`: the one-sentence justification a user gave for it — *why* the
   claim holds, not *what* it says. When the user states a justification for a
   directive in the same statement ("use return_date — a rental only counts once it
   comes back"), you MUST attach that justification as the proposal's `reason` (here,
   "a rental only counts once it comes back") and you MUST NOT also emit it as a
   separate `column:<col>.description` or `table.description` claim — one statement,
   one claim. A description claim remains correct when the user describes a column or
   table for its own sake, with no directive attached. Omit `reason` for description
   and alias slots, and when no justification was stated. Keep `reason` to one sentence
   and never put passwords, SQL, or credentials in it.
6. NEVER include passwords, API keys, tokens, or private credentials in extracted values.

### REGISTERED OBJECTS:
{table_desc}

### OUTPUT JSON SCHEMA:
{{
  "proposals": [
    {{
      "object_id": "T0",
      "slot": "table.grain",
      "value": "one row per completed order",
      "reason": "an order only completes when it ships, not when it is placed",
      "origin": "user_explicit",
      "confidence": 1.0
    }}
  ]
}}
If no new knowledge was asserted or discovered, return {{"proposals": []}}."#
    );

    let mut user_context = String::new();
    user_context.push_str("### USER PROMPT:\n");
    user_context.push_str(&record.prompt);
    user_context.push_str("\n\n");

    if !record.user_corrections.is_empty() {
        user_context.push_str("### DETECTED USER CORRECTIONS:\n");
        for corr in &record.user_corrections {
            user_context.push_str(&format!("- {corr}\n"));
        }
        user_context.push('\n');
    }

    if !record.override_findings.is_empty() {
        user_context.push_str("### OBSERVED OVERRIDES / CONTRADICTIONS:\n");
        for finding in &record.override_findings {
            user_context.push_str(&format!(
                "- Claim {}: slot '{}' expected '{}', but observed columns {:?}\n",
                finding.claim_id, finding.kind, finding.claimed_value, finding.observed_columns
            ));
        }
        user_context.push('\n');
    }

    if !record.supplied_claims.is_empty() {
        user_context.push_str("### RECALLED KNOWLEDGE (ALREADY KNOWN):\n");
        for contract in &record.supplied_claims {
            for claim in &contract.claims {
                user_context.push_str(&format!(
                    "- {} ({}): {} = {}\n",
                    contract.object, claim.claim_id, claim.kind, claim.value
                ));
            }
        }
        user_context.push('\n');
    }

    user_context.push_str("### ASSISTANT ANSWER:\n");
    user_context.push_str(&record.assistant_answer);

    ChatRequest {
        model: model.to_string(),
        messages: vec![
            ChatMessage::text("system", system_prompt),
            ChatMessage::text("user", user_context),
        ],
        tools: Vec::new(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::learning::turn_table::TurnObjectTable;
    use saya_agent::OverrideFindingDto;
    use saya_types::ClaimId;

    #[test]
    fn test_build_extraction_prompt_renders_object_table_and_schema() {
        let mut table = TurnObjectTable::new();
        table.register(
            "primary",
            "catalog.public.orders",
            &["id".into(), "created_at".into()],
        );

        let record = TurnRecord {
            prompt: "What is the grain of public.orders?".into(),
            assistant_answer: "Each row in orders represents a customer purchase.".into(),
            object_table: table,
            user_corrections: vec!["orders are daily".into()],
            override_findings: vec![OverrideFindingDto {
                claim_id: ClaimId::parse("c-1").unwrap(),
                kind: "default_time_column".into(),
                claimed_value: "created_at".into(),
                observed_columns: vec!["ordered_at".into()],
            }],
            supplied_claims: Vec::new(),
        };

        let req = build_extraction_prompt(&record, "gemini-2.5-flash");
        assert_eq!(req.model, "gemini-2.5-flash");
        assert_eq!(req.messages.len(), 2);

        let system = &req.messages[0].content;
        assert!(system.contains("T0 -> catalog.public.orders"));
        assert!(system.contains("[columns: id, created_at]"));
        assert!(system.contains("table.grain"));
        assert!(system.contains("\"proposals\":"));

        let user = &req.messages[1].content;
        assert!(user.contains("What is the grain of public.orders?"));
        assert!(user.contains("orders are daily"));
        assert!(user.contains("OBSERVED OVERRIDES"));
        assert!(user.contains("customer purchase"));
    }

    /// Text assertion (not a behavioural test): the rendered prompt must tell the
    /// model that a justification for a directive slot is attached as `reason` and
    /// must NOT be re-emitted as a separate description claim. Model behaviour is
    /// verified manually against the store dump — see the packet.
    #[test]
    fn test_extraction_prompt_directs_reason_attachment_and_forbids_duplicate_description() {
        let table = TurnObjectTable::new();
        let record = TurnRecord {
            prompt: "Use return_date as the default time column for pagila.public.rental \
                     — a rental only counts once it comes back. How many rentals in 2022?"
                .into(),
            assistant_answer: String::new(),
            object_table: table,
            user_corrections: Vec::new(),
            override_findings: Vec::new(),
            supplied_claims: Vec::new(),
        };

        let req = build_extraction_prompt(&record, "deepseek-v4-flash");
        let system = &req.messages[0].content;

        // Directive, not permissive: the reason MUST be attached.
        assert!(
            system.contains("you MUST attach that justification as the proposal's `reason`"),
            "prompt must direct the model to attach the reason"
        );
        // One statement, one claim: no separate description claim for the same
        // justification. The text wraps across lines in the source, so assert the
        // two halves separately rather than as one contiguous substring.
        assert!(
            system.contains("MUST NOT also emit it as a"),
            "prompt must forbid a duplicate description claim"
        );
        assert!(
            system.contains("description` claim — one statement,"),
            "prompt must state one statement, one claim"
        );
        // The directive slots are named so the model knows which slots carry a reason.
        assert!(system.contains("table.default_time"));
        // The credential prohibition on the reason survives.
        assert!(system.contains("never put passwords, SQL, or credentials in it"));
    }
}
